// ipmsg/service.js —— 由插件通过 efto.node 启动的 Node 常驻服务（无第三方依赖）。
// 负责：UDP 广播发现 + 按 IP 探测 + TCP 文本消息 / 文件传输 + 首次通信授权
// + 本地 HTTP 控制口（只给插件 UI 用）。
// 宿主只提供「运行 Node」这一件事；端口、协议、状态都在这里。协议为 efto 自定义，
// 不与标准 IPMSG 互通，全部明文，仅适用于可信局域网。收到的文件只接受已授权对方，
// 统一落到下载目录的 efto-ipmsg 子目录。

'use strict';

var dgram = require('node:dgram');
var fs = require('node:fs');
var http = require('node:http');
var net = require('node:net');
var os = require('node:os');
var path = require('node:path');

var UDP_PORT = 24250;
var TCP_PORT = 24250;
var HTTP_PORT = 24251;
var BEACON_MS = 2000;
var PEER_TTL = 9000;
var IO_TIMEOUT = 2000;
var OUTBOX_LIMIT = 20;
var TEXT_LIMIT = 4000;
var IDLE_EXIT_MS = 30 * 60 * 1000;
var FILE_IDLE_MS = 60 * 1000; // 文件传输的空闲超时（有数据就重置）
var PROBE_REFRESH_MS = 4000; // 手动 IP 添加的 peer 用 TCP hello 续探的间隔
var DOWNLOAD_DIR = path.join(os.homedir(), 'Downloads', 'efto-ipmsg');

var TOKEN = String(process.argv[2] || '');
if (!TOKEN) {
  console.error('ipmsg: 缺少控制口令牌，拒绝启动');
  process.exit(2);
}
var selfId = os.hostname();
var selfName = selfId;
var selfIp = lanIp();

var peers = new Map(); // id -> { id, name, ip, port, lastSeen, manual }
var trusted = new Set(); // 我同意接收谁的请求
var approved = new Set(); // 谁已同意接收我的消息
var pending = new Set(); // 待我处理的接收请求
var inbox = []; // 待 UI 取走的入站消息（含收文件回执）
var outbox = []; // 未授权暂存的待发消息 / 文件路径
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

function isIpv4(s) {
  var m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(String(s || ''));
  if (!m) return false;
  for (var i = 1; i <= 4; i++) if (Number(m[i]) > 255) return false;
  return true;
}

function fmtSize(n) {
  n = Number(n) || 0;
  if (n < 1024) return n + ' B';
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + ' KB';
  if (n < 1024 * 1024 * 1024) return (n / 1024 / 1024).toFixed(1) + ' MB';
  return (n / 1024 / 1024 / 1024).toFixed(2) + ' GB';
}

// 收文件时只保留名字：路径分隔符、Windows 非法字符与开头的点一律换掉，
// 防止对方用 ../ 或盘符把文件写到下载目录之外。
function safeName(name) {
  var base = path.basename(String(name || '').replace(/\\/g, '/'));
  base = base.replace(/[<>:"|?*\x00-\x1f]/g, '_').replace(/^\.+/, '').trim().slice(0, 120);
  return base || 'file';
}

function uniquePath(dir, name) {
  var ext = path.extname(name);
  var stem = name.slice(0, name.length - ext.length);
  var dest = path.join(dir, name);
  var free = function (p) { return !fs.existsSync(p) && !fs.existsSync(p + '.part'); };
  for (var n = 1; !free(dest) && n < 1000; n++) {
    dest = path.join(dir, stem + ' (' + n + ')' + ext);
  }
  return dest;
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
    refreshManual();
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
  // 文件接收要流式落盘，真正的接收流程在连接处理里；这里只做授权判定。
  if (type === 'file-offer') {
    if (!trusted.has(from)) return { ok: false, error: 'not_allowed' };
    return { ok: true, accept: true };
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
  // 首行是 JSON 文本请求；file-offer 通过后，同一个连接后续字节全是文件内容，
  // 因此这里不能 setEncoding('utf8')，否则二进制会被替换字符破坏。
  var buf = Buffer.alloc(0);
  var handled = false; // 普通请求已回复，不再读输入
  var rx = null; // 进行中的文件接收

  socket.setNoDelay(true);
  socket.setTimeout(4000);
  socket.on('error', function () {});
  socket.on('timeout', function () {
    if (rx) abortFile('接收超时');
    else socket.destroy();
  });

  function endWith(obj) {
    if (handled) return;
    handled = true;
    try {
      socket.end(JSON.stringify(obj) + '\n');
    } catch (e) {
      socket.destroy();
    }
  }

  function abortFile(reason) {
    if (!rx) return;
    var state = rx;
    rx = null;
    handled = true;
    try { state.ws.destroy(); } catch (e) { /* 已关闭 */ }
    fs.unlink(state.part, function () {});
    try {
      socket.end(JSON.stringify({ ok: false, error: reason }) + '\n');
    } catch (e) {
      socket.destroy();
    }
  }

  function finishFile() {
    var state = rx;
    rx = null;
    handled = true;
    socket.setTimeout(0); // 只等本地落盘，不再受空闲超时影响
    state.ws.end(function () {
      fs.rename(state.part, state.dest, function (err) {
        if (err) {
          log('file rename error:', err.message);
          try {
            socket.end(JSON.stringify({ ok: false, error: '保存失败' }) + '\n');
          } catch (e) {
            socket.destroy();
          }
          return;
        }
        inbox.push({
          from: state.from,
          fromName: state.fromName,
          text: '收到文件：' + state.name + '（' + fmtSize(state.size) + '）\n已保存到 ' + state.dest,
          ts: Date.now()
        });
        try {
          socket.end(JSON.stringify({ ok: true, saved: state.dest }) + '\n');
        } catch (e) {
          socket.destroy();
        }
      });
    });
  }

  function pushBytes(chunk) {
    if (!rx) return;
    var need = rx.size - rx.got;
    if (chunk.length > need) return abortFile('数据超出声明大小');
    if (chunk.length) {
      var needsDrain = !rx.ws.write(chunk);
      rx.got += chunk.length;
      // 磁盘慢于网络时暂停 socket，避免整份文件堆在 Node 内存里
      if (needsDrain) {
        socket.pause();
        rx.ws.once('drain', function () {
          if (rx) socket.resume();
        });
      }
    }
    if (rx && rx.got >= rx.size) finishFile();
  }

  function startFile(msg, size) {
    var name = safeName(msg.name);
    var dest;
    try {
      fs.mkdirSync(DOWNLOAD_DIR, { recursive: true });
      dest = uniquePath(DOWNLOAD_DIR, name);
    } catch (e) {
      log('file dir error:', e && e.message);
      return endWith({ ok: false, error: '无法创建下载目录' });
    }
    var part = dest + '.part';
    var ws = fs.createWriteStream(part);
    rx = {
      ws: ws,
      part: part,
      dest: dest,
      size: size,
      got: 0,
      name: name,
      from: msg.id,
      fromName: String(msg.name || msg.id).slice(0, 120)
    };
    ws.on('error', function (e) {
      log('file write error:', e && e.message);
      abortFile('写入失败');
    });
    socket.setTimeout(FILE_IDLE_MS);
    try {
      socket.write(JSON.stringify({ ok: true, accept: true }) + '\n');
    } catch (e) {
      abortFile('连接已断开');
      return;
    }
    if (size === 0) finishFile();
  }

  function handleLine(line) {
    var msg;
    try {
      msg = JSON.parse(line.toString('utf8'));
    } catch (e) {
      return endWith({ ok: false, error: 'bad_json' });
    }
    var res;
    try {
      res = dispatch(msg, socket.remoteAddress);
    } catch (e) {
      res = { ok: false, error: 'internal' };
      log('dispatch error:', e && e.message);
    }
    if (!msg || msg.type !== 'file-offer' || !res || !res.ok) {
      return endWith(res || { ok: false, error: 'internal' });
    }
    var size = Number(msg.size);
    if (!Number.isSafeInteger(size) || size < 0) return endWith({ ok: false, error: 'bad_size' });
    startFile(msg, size);
  }

  socket.on('data', function (chunk) {
    if (rx) return pushBytes(chunk);
    if (handled) return;
    buf = Buffer.concat([buf, chunk]);
    if (buf.length > 65536) return endWith({ ok: false, error: 'too_large' });
    var i = buf.indexOf(10);
    if (i < 0) return;
    var rest = buf.slice(i + 1);
    handleLine(buf.slice(0, i));
    if (rx && rest.length) pushBytes(rest);
  });

  socket.on('close', function () {
    if (!rx) return;
    var state = rx;
    rx = null;
    try { state.ws.destroy(); } catch (e) { /* 已关闭 */ }
    fs.unlink(state.part, function () {});
    log('file receive aborted:', state.name);
  });
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

async function ensureApproved(peer) {
  if (approved.has(peer.id)) return { ok: true };
  var q;
  try {
    q = await rpc(peer, { type: 'request' });
  } catch (e) {
    return { ok: false, error: '发送失败：' + e.message };
  }
  if (q && q.allowed) {
    approved.add(peer.id);
    return { ok: true };
  }
  return { ok: false, pending: true };
}

function codeText(code) {
  if (code === 'self') return '这是本机地址';
  if (code === 'not_allowed') return '对方尚未同意接收';
  if (code === 'unknown_type') return '对方版本不支持文件传输';
  if (code === 'bad_size') return '文件大小无效';
  return String(code || '对方拒绝接收');
}

function fileMeta(filePath) {
  var p = String(filePath || '');
  if (!p) return { ok: false, error: '缺少文件路径' };
  var st;
  try {
    st = fs.statSync(p);
  } catch (e) {
    return { ok: false, error: '文件不存在或无法读取' };
  }
  if (!st.isFile()) return { ok: false, error: '只能发送文件' };
  return { ok: true, file: { path: p, name: path.basename(p), size: st.size } };
}

function queueFile(peerId, filePath) {
  outbox.push({ peerId: peerId, file: filePath });
  if (outbox.length > OUTBOX_LIMIT) outbox.splice(0, outbox.length - OUTBOX_LIMIT);
}

// 文件传输：同一 TCP 连接先发 file-offer，对方回 accept 后原样写入字节，
// 写完等对方回一条保存结果。接收端按声明的 size 判定结束。
function transferFile(peer, file) {
  return new Promise(function (resolve) {
    var socket = net.connect({ host: peer.ip, port: peer.port });
    var buf = '';
    var stage = 'offer';
    var done = false;
    var rs = null;
    var timer = setTimeout(function () { finish({ ok: false, error: '对方没有响应' }); }, IO_TIMEOUT);

    function bump(ms) {
      clearTimeout(timer);
      timer = setTimeout(function () { finish({ ok: false, error: '传输超时' }); }, ms);
    }

    function finish(res) {
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (rs) {
        try { rs.destroy(); } catch (e) { /* 已关闭 */ }
      }
      socket.destroy();
      resolve(res);
    }

    socket.setEncoding('utf8'); // 只解析响应行；文件内容走写方向，不受编码影响
    socket.on('connect', function () {
      socket.write(JSON.stringify(envelope({
        type: 'file-offer',
        name: file.name,
        size: file.size
      })) + '\n');
    });
    socket.on('data', function (chunk) {
      buf += chunk;
      if (buf.length > 65536) return finish({ ok: false, error: '响应过大' });
      var i;
      while ((i = buf.indexOf('\n')) >= 0) {
        var line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        var msg;
        try {
          msg = JSON.parse(line);
        } catch (e) {
          return finish({ ok: false, error: '响应格式错误' });
        }
        if (stage === 'offer') {
          if (!msg || !msg.ok) {
            return finish({ ok: false, error: codeText(msg && msg.error), code: msg && msg.error });
          }
          stage = 'data';
          bump(FILE_IDLE_MS);
          rs = fs.createReadStream(file.path);
          rs.on('error', function (e) {
            finish({ ok: false, error: '读取文件失败：' + e.message });
          });
          rs.on('data', function () { bump(FILE_IDLE_MS); });
          rs.pipe(socket, { end: false });
        } else {
          return finish(msg && msg.ok
            ? { ok: true, saved: msg.saved }
            : { ok: false, error: (msg && msg.error) || '对方保存失败' });
        }
      }
    });
    socket.on('error', function (e) { finish({ ok: false, error: e.message }); });
    socket.on('close', function () { finish({ ok: false, error: '连接被关闭' }); });
  });
}

async function sendFile(peerId, filePath) {
  var peer = peers.get(String(peerId));
  if (!peer) return { ok: false, error: '对方不在线' };
  var meta = fileMeta(filePath);
  if (!meta.ok) return meta;

  var gate = await ensureApproved(peer);
  if (!gate.ok) {
    if (!gate.pending) return gate;
    queueFile(peer.id, meta.file.path);
    return { ok: true, pending: true };
  }

  var r = await transferFile(peer, meta.file);
  if (r && r.code === 'not_allowed') {
    approved.delete(peer.id);
    var again = await ensureApproved(peer);
    if (!again.ok) {
      if (again.pending) {
        queueFile(peer.id, meta.file.path);
        return { ok: true, pending: true };
      }
      return again;
    }
    r = await transferFile(peer, meta.file);
  }
  return r;
}

async function flushOutbox(peerId) {
  var peer = peers.get(peerId);
  if (!peer) return;
  var mine = outbox.filter(function (m) { return m.peerId === peerId; });
  outbox = outbox.filter(function (m) { return m.peerId !== peerId; });
  for (var i = 0; i < mine.length; i++) {
    try {
      if (mine[i].file) {
        var meta = fileMeta(mine[i].file);
        if (!meta.ok) continue; // 文件已被移动/删除，跳过
        var f = await transferFile(peer, meta.file);
        if (!f || !f.ok) {
          if (f && f.code === 'not_allowed') approved.delete(peer.id);
          break;
        }
      } else {
        var r = await rpc(peer, { type: 'msg', text: mine[i].text });
        if (!r || !r.ok) break;
      }
    } catch (e) {
      break;
    }
  }
}

// ---------------- 按 IP 探测 ----------------

async function probePeer(ip, port) {
  var addr = String(ip || '').trim();
  var p = Number(port) || TCP_PORT;
  if (!isIpv4(addr)) return { ok: false, error: 'IP 地址格式不正确' };
  if (addr === selfIp) return { ok: false, error: '这是本机地址' };
  var r;
  try {
    r = await rpc({ ip: addr, port: p }, { type: 'hello' });
  } catch (e) {
    return { ok: false, error: '无法连接 ' + addr + ':' + p + '（' + e.message + '）' };
  }
  if (!r || !r.ok || !r.id) return { ok: false, error: codeText(r && r.error) };
  if (r.id === selfId) return { ok: false, error: '这是本机地址' };
  var peer = touch(r.id, r.name, addr, p);
  if (!peer) return { ok: false, error: '对方标识无效' };
  peer.manual = true;
  peers.set(peer.id, peer);
  return {
    ok: true,
    peer: { id: peer.id, name: peer.name, ip: peer.ip, addr: peer.ip + ':' + peer.port }
  };
}

// 手动按 IP 添加的 peer 收不到广播时，用 TCP hello 续探，掉线后按 PEER_TTL 自然过期。
function refreshManual() {
  var now = Date.now();
  peers.forEach(function (p) {
    if (!p.manual || p.probing || now - p.lastSeen < PROBE_REFRESH_MS) return;
    p.probing = true;
    rpc({ ip: p.ip, port: p.port }, { type: 'hello' }).then(function (r) {
      p.probing = false;
      if (!r || !r.ok || r.id !== p.id) return;
      p.lastSeen = Date.now();
      if (r.name) p.name = String(r.name);
    }).catch(function () {
      p.probing = false;
    });
  });
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
    downloads: DOWNLOAD_DIR,
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
        manual: !!p.manual,
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
    if (url.pathname === '/sendfile') {
      return sendFile(body.peerId, body.path).then(function (r) {
        sendJson(res, 200, r);
      });
    }
    if (url.pathname === '/probe') {
      return probePeer(body.ip, body.port).then(function (r) {
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
