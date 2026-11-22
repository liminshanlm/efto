// ipmsg/service.js —— 由插件通过 efto.node 启动的 Node 常驻服务（无第三方依赖）。
// 负责：UDP 广播发现 + TCP 文本消息 + 首次通信授权 + 本地 HTTP 控制口（只给插件 UI 用）。
// 宿主只提供「运行 Node」这一件事；端口、协议、状态都在这里。协议为 efto 自定义，
// 不与标准 IPMSG 互通，全部明文，仅适用于可信局域网。

'use strict';

var dgram = require('node:dgram');
var http = require('node:http');
var net = require('node:net');
var os = require('node:os');

var UDP_PORT = 24250;
var TCP_PORT = 24250;
var HTTP_PORT = 24251;
var BEACON_MS = 2000;
var PEER_TTL = 9000;
var IO_TIMEOUT = 2000;
var OUTBOX_LIMIT = 20;
var TEXT_LIMIT = 4000;
var IDLE_EXIT_MS = 30 * 60 * 1000;

var TOKEN = String(process.argv[2] || '');
if (!TOKEN) {
  console.error('ipmsg: 缺少控制口令牌，拒绝启动');
  process.exit(2);
}
var selfId = os.hostname();
var selfName = selfId;
var selfIp = lanIp();

var peers = new Map(); // id -> { id, name, ip, port, lastSeen }
var trusted = new Set(); // 我同意接收谁的请求
var approved = new Set(); // 谁已同意接收我的消息
var pending = new Set(); // 待我处理的接收请求
var inbox = []; // 待 UI 取走的入站消息
var outbox = []; // 未授权暂存的待发消息
var lastPoll = Date.now();
var shuttingDown = false;

function lanIp() {
  var nets = os.networkInterfaces();
  var names = Object.keys(nets);
  for (var i = 0; i < names.length; i++) {
    var rows = nets[names[i]] || [];
    for (var j = 0; j < rows.length; j++) {
      var ni = rows[j];
      if (ni.family === 'IPv4' && !ni.internal && ni.address.indexOf('169.254.') !== 0) return ni.address;
    }
  }
  return '127.0.0.1';
}

function log() {
  try {
    console.log.apply(console, arguments);
  } catch (e) { /* stdout 被丢弃时忽略 */ }
}

function normIp(ip) {
  return String(ip || '').replace(/^::ffff:/, '');
}

function touch(id, name, ip, port) {
  if (!id || id === selfId) return null;
  var old = peers.get(id);
  var p = old || { id: id, name: id, ip: normIp(ip), port: TCP_PORT, lastSeen: 0 };
  if (name) p.name = String(name);
  if (ip) p.ip = normIp(ip);
  if (port) p.port = Number(port) || TCP_PORT;
  p.lastSeen = Date.now();
  peers.set(id, p);
  return p;
}

function prune() {
  var now = Date.now();
  peers.forEach(function (p, id) {
    if (now - p.lastSeen > PEER_TTL) peers.delete(id);
  });
}

// ---------------- UDP 发现 ----------------

var udp = dgram.createSocket('udp4');
udp.on('error', function (e) {
  log('udp error:', e.message);
  shutdown(1);
});
udp.on('message', function (buf, rinfo) {
  var msg;
  try {
    msg = JSON.parse(buf.toString('utf8'));
  } catch (e) {
    return;
  }
  if (!msg || msg.efto_ipmsg !== 1 || !msg.id) return;
  touch(msg.id, msg.name, rinfo.address, msg.port);
});
udp.bind(UDP_PORT, '0.0.0.0', function () {
  try {
    udp.setBroadcast(true);
  } catch (e) {
    log('setBroadcast failed:', e.message);
  }
  // 广播必须等 setBroadcast 生效后开始，否则首包可能以 EACCES 触发 socket error。
  setInterval(function () {
    prune();
    var beacon = Buffer.from(JSON.stringify({
      efto_ipmsg: 1,
      id: selfId,
      name: selfName,
      port: TCP_PORT
    }));
    udp.send(beacon, 0, beacon.length, UDP_PORT, '255.255.255.255', function () {});
  }, BEACON_MS).unref();
});

// ---------------- TCP 消息 ----------------

function envelope(payload) {
  var out = {
    efto_ipmsg: 1,
    id: selfId,
    name: selfName,
    port: TCP_PORT
  };
  Object.keys(payload).forEach(function (k) { out[k] = payload[k]; });
  return out;
}

function rpc(peer, payload) {
  return new Promise(function (resolve, reject) {
    var socket = net.connect({ host: peer.ip, port: peer.port });
    var buf = '';
    var done = false;
    var timer = setTimeout(function () { finish(new Error('连接超时')); }, IO_TIMEOUT);

    function finish(err, line) {
      if (done) return;
      done = true;
      clearTimeout(timer);
      socket.destroy();
      if (err) return reject(err);
      try {
        resolve(JSON.parse(line));
      } catch (e) {
        reject(new Error('响应格式错误'));
      }
    }

    socket.setEncoding('utf8');
    socket.on('connect', function () {
      socket.write(JSON.stringify(envelope(payload)) + '\n');
    });
    socket.on('data', function (chunk) {
      buf += chunk;
      if (buf.length > 65536) return finish(new Error('响应过大'));
      var i = buf.indexOf('\n');
      if (i >= 0) finish(null, buf.slice(0, i));
    });
    socket.on('error', function (e) { finish(e); });
    socket.on('close', function () { finish(new Error('连接被关闭')); });
  });
}

function dispatch(msg, remoteIp) {
  if (!msg || msg.efto_ipmsg !== 1 || !msg.id) return { ok: false, error: 'bad_packet' };
  if (msg.id === selfId) return { ok: false, error: 'self' };
  var peer = touch(msg.id, msg.name, remoteIp, msg.port);
  var from = msg.id;
  var type = msg.type;

  if (type === 'hello') {
    return { ok: true, id: selfId, name: selfName };
  }
  if (type === 'request') {
    if (trusted.has(from)) return { ok: true, allowed: true };
    pending.add(from);
    return { ok: true, allowed: false };
  }
  if (type === 'msg') {
    if (!trusted.has(from)) return { ok: false, error: 'not_allowed' };
    var text = String(msg.text == null ? '' : msg.text).trim();
    if (!text) return { ok: false, error: 'empty' };
    inbox.push({
      from: from,
      fromName: msg.name || (peer && peer.name) || from,
      text: text.slice(0, TEXT_LIMIT),
      ts: Date.now()
    });
    return { ok: true };
  }
  if (type === 'approve') {
    approved.add(from);
    flushOutbox(from);
    return { ok: true };
  }
  if (type === 'reject') {
    approved.delete(from);
    outbox = outbox.filter(function (m) { return m.peerId !== from; });
    return { ok: true };
  }
  return { ok: false, error: 'unknown_type' };
}

var tcp = net.createServer(function (socket) {
  var buf = '';
  var handled = false;
  socket.setEncoding('utf8');
  socket.setTimeout(4000, function () { socket.destroy(); });
  socket.on('error', function () {});

  function reply(obj) {
    if (handled) return;
    handled = true;
    try {
      socket.end(JSON.stringify(obj) + '\n');
    } catch (e) {
      socket.destroy();
    }
  }

  socket.on('data', function (chunk) {
    if (handled) return;
    buf += chunk;
    if (buf.length > 65536) return reply({ ok: false, error: 'too_large' });
    var i = buf.indexOf('\n');
    if (i < 0) return;
    var msg;
    try {
      msg = JSON.parse(buf.slice(0, i));
    } catch (e) {
      return reply({ ok: false, error: 'bad_json' });
    }
    var res;
    try {
      res = dispatch(msg, socket.remoteAddress);
    } catch (e) {
      res = { ok: false, error: 'internal' };
      log('dispatch error:', e && e.message);
    }
    reply(res);
  });
  socket.on('close', function () {});
});

tcp.on('error', function (e) {
  log('tcp error:', e.message);
  shutdown(1);
});
tcp.listen(TCP_PORT, '0.0.0.0');

// ---------------- 发送 / 授权 ----------------

async function sendMessage(peerId, text) {
  var peer = peers.get(String(peerId));
  if (!peer) return { ok: false, error: '对方不在线' };
  text = String(text == null ? '' : text).trim().slice(0, TEXT_LIMIT);
  if (!text) return { ok: false, error: '消息不能为空' };

  if (approved.has(peer.id)) {
    var r;
    try {
      r = await rpc(peer, { type: 'msg', text: text });
    } catch (e) {
      return { ok: false, error: '发送失败：' + e.message };
    }
    if (r && r.ok) return { ok: true, delivered: true };
    if (r && r.error === 'not_allowed') approved.delete(peer.id);
  }

  var q;
  try {
    q = await rpc(peer, { type: 'request' });
  } catch (e) {
    return { ok: false, error: '发送失败：' + e.message };
  }
  if (q && q.allowed) {
    approved.add(peer.id);
    var m;
    try {
      m = await rpc(peer, { type: 'msg', text: text });
    } catch (e) {
      return { ok: false, error: '发送失败：' + e.message };
    }
    if (m && m.ok) return { ok: true, delivered: true };
  }

  outbox.push({ peerId: peer.id, text: text });
  if (outbox.length > OUTBOX_LIMIT) outbox.splice(0, outbox.length - OUTBOX_LIMIT);
  return { ok: true, pending: true };
}

async function flushOutbox(peerId) {
  var peer = peers.get(peerId);
  if (!peer) return;
  var mine = outbox.filter(function (m) { return m.peerId === peerId; });
  outbox = outbox.filter(function (m) { return m.peerId !== peerId; });
  for (var i = 0; i < mine.length; i++) {
    try {
      var r = await rpc(peer, { type: 'msg', text: mine[i].text });
      if (!r || !r.ok) break;
    } catch (e) {
      break;
    }
  }
}

function respond(peerId, accept) {
  var id = String(peerId || '');
  if (!id) return { ok: false, error: '缺少 peerId' };
  pending.delete(id);
  if (accept) trusted.add(id);
  else trusted.delete(id);

  var peer = peers.get(id);
  if (!peer) return { ok: true, accepted: !!accept, offline: true };
  rpc(peer, { type: accept ? 'approve' : 'reject' }).catch(function () {});
  return { ok: true, accepted: !!accept, offline: false };
}

function initState(body) {
  var t = body && Array.isArray(body.trusted) ? body.trusted : [];
  var a = body && Array.isArray(body.approved) ? body.approved : [];
  trusted = new Set(t.filter(function (x) { return typeof x === 'string' && x; }));
  approved = new Set(a.filter(function (x) { return typeof x === 'string' && x; }));
}

function status() {
  prune();
  var now = Date.now();
  var rows = Array.from(peers.values()).sort(function (a, b) {
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
  var requestIds = Array.from(pending).sort();
  var messages = inbox.splice(0, inbox.length);
  return {
    running: true,
    port: TCP_PORT,
    self: { id: selfId, name: selfName, ip: selfIp },
    peers: rows.map(function (p) {
      return {
        id: p.id,
        name: p.name,
        ip: p.ip,
        addr: p.ip + ':' + p.port,
        allowed: trusted.has(p.id),
        outboundOk: approved.has(p.id),
        pending: pending.has(p.id),
        online: now - p.lastSeen <= PEER_TTL
      };
    }),
    requests: requestIds.map(function (id) {
      var p = peers.get(id);
      return { id: id, name: p ? p.name : id, online: !!p };
    }),
    messages: messages,
    outbox: outbox.length
  };
}

// ---------------- 本地 HTTP 控制口（只给插件 UI） ----------------

function cors(res) {
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'content-type, x-efto-ipmsg');
  res.setHeader('Access-Control-Allow-Private-Network', 'true');
}

function sendJson(res, code, obj) {
  var body = Buffer.from(JSON.stringify(obj), 'utf8');
  res.writeHead(code, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': body.length,
    'cache-control': 'no-store'
  });
  res.end(body);
}

function readBody(req) {
  return new Promise(function (resolve, reject) {
    var buf = '';
    req.setEncoding('utf8');
    req.on('data', function (chunk) {
      buf += chunk;
      if (buf.length > 65536) {
        reject(new Error('请求过大'));
        req.destroy();
      }
    });
    req.on('end', function () {
      if (!buf) return resolve({});
      try {
        resolve(JSON.parse(buf));
      } catch (e) {
        reject(new Error('请求格式错误'));
      }
    });
    req.on('error', reject);
  });
}

var control = http.createServer(function (req, res) {
  cors(res);
  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }
  var url;
  try {
    url = new URL(req.url, 'http://127.0.0.1');
  } catch (e) {
    return sendJson(res, 400, { ok: false, error: 'bad_url' });
  }
  var token = url.searchParams.get('t') || req.headers['x-efto-ipmsg'] || '';
  if (token !== TOKEN) return sendJson(res, 403, { ok: false, error: 'forbidden' });

  lastPoll = Date.now();

  if (req.method === 'GET' && url.pathname === '/status') {
    return sendJson(res, 200, status());
  }
  if (req.method === 'POST' && url.pathname === '/stop') {
    sendJson(res, 200, { ok: true });
    setTimeout(function () { shutdown(0); }, 30);
    return;
  }
  if (req.method !== 'POST') return sendJson(res, 405, { ok: false, error: 'method_not_allowed' });

  readBody(req).then(function (body) {
    if (url.pathname === '/init') {
      initState(body);
      return sendJson(res, 200, { ok: true });
    }
    if (url.pathname === '/send') {
      return sendMessage(body.peerId, body.text).then(function (r) {
        sendJson(res, 200, r);
      });
    }
    if (url.pathname === '/respond') {
      return sendJson(res, 200, respond(body.peerId, !!body.accept));
    }
    sendJson(res, 404, { ok: false, error: 'not_found' });
  }).catch(function (e) {
    sendJson(res, 200, { ok: false, error: e && e.message ? e.message : String(e) });
  });
});

control.on('error', function (e) {
  log('http error:', e.message);
  shutdown(1);
});
control.listen(HTTP_PORT, '127.0.0.1');

// ---------------- 退出 ----------------

function shutdown(code) {
  if (shuttingDown) return;
  shuttingDown = true;
  try { udp.close(); } catch (e) {}
  try { tcp.close(); } catch (e) {}
  try { control.close(); } catch (e) {}
  setTimeout(function () { process.exit(code || 0); }, 60);
}

setInterval(function () {
  if (Date.now() - lastPoll > IDLE_EXIT_MS) shutdown(0);
}, 5000).unref();

process.on('uncaughtException', function (e) {
  log('uncaught:', e && e.stack ? e.stack : e);
});
