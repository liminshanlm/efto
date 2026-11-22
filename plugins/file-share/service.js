// file-share/service.js —— 本机 Node 常驻服务（由 main.js 通过 efto.node.run 拉起）。
// 只暴露调用方显式传进来的路径；不支持上传、账号与权限。请求路径都在共享根内解析
// （realpath + 前缀校验），`..` / 编码绕过都会被拒绝。
//
// 数据口默认 5421；控制口固定 127.0.0.1:5422，只给插件用。

'use strict';

const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');

const TOKEN = String(process.argv[2] || '');
if (!TOKEN) {
  console.error('file-share: 缺少控制口令牌，拒绝启动');
  process.exit(2);
}

const DEFAULT_PORT = 5421;
const CONTROL_PORT = 5422;
// 数据口在跑时要求 main 定期 heartbeat；超过这个时间没有控制请求就停止共享，
// 避免应用退出后 detached 服务继续对外提供文件。
const LEASE_MS = 120 * 1000;
const IDLE_EXIT_MS = 30 * 60 * 1000;

let server4 = null;
let server6 = null;
let currentPort = 0;
let items = [];
let lastSeen = Date.now();

function log() {
  console.error.apply(console, ['[file-share]'].concat(Array.prototype.slice.call(arguments)));
}

// ---------- 条目校验 ----------

function parseItems(raw) {
  const out = [];
  const invalid = [];
  const seen = new Set();
  const list = Array.isArray(raw) ? raw : [];
  for (const v of list) {
    const text = v && typeof v.path === 'string' ? v.path.trim() : '';
    if (!text) {
      invalid.push({ path: '', reason: '路径为空' });
      continue;
    }
    let st;
    try {
      st = fs.statSync(text);
    } catch (e) {
      invalid.push({ path: text, reason: '无法读取：' + e.message });
      continue;
    }
    let root;
    try {
      root = fs.realpathSync(text);
    } catch (e) {
      invalid.push({ path: text, reason: '路径无效：' + e.message });
      continue;
    }
    const key = process.platform === 'win32' ? root.toLowerCase() : root;
    if (seen.has(key)) continue;
    seen.add(key);
    let name = v && typeof v.name === 'string' ? v.name.trim() : '';
    if (!name) name = path.basename(root) || root;
    out.push({ name: name, path: text, root: root, is_dir: st.isDirectory(), size: st.size });
  }
  return { items: out, invalid: invalid };
}

// ---------- 地址 ----------

function primaryIPv4() {
  const all = os.networkInterfaces();
  for (const name of Object.keys(all)) {
    for (const a of all[name] || []) {
      if (a.family === 'IPv4' && !a.internal && a.address) return a.address;
    }
  }
  return null;
}

function lanUrls(port) {
  const ip = primaryIPv4();
  return ['http://' + (ip || '127.0.0.1') + ':' + port + '/'];
}

function lanUrls6(port) {
  if (!server6) return [];
  const all = os.networkInterfaces();
  for (const name of Object.keys(all)) {
    for (const a of all[name] || []) {
      if (a.family === 'IPv6' && !a.internal && a.address) {
        return ['http://[' + a.address + ']:' + port + '/'];
      }
    }
  }
  return [];
}

function snapshotItems() {
  return items.map(function (it) {
    return { name: it.name, path: it.path, is_dir: it.is_dir, size: it.size };
  });
}

function snapshot(running, port) {
  const on = !!running;
  return {
    running: on,
    port: on ? port : port,
    urls: on ? lanUrls(port) : [],
    urls6: on ? lanUrls6(port) : [],
    items: on ? snapshotItems() : [],
    invalid: []
  };
}

// ---------- 数据口 ----------

const MIME = {
  html: 'text/html; charset=utf-8', htm: 'text/html; charset=utf-8',
  txt: 'text/plain; charset=utf-8', log: 'text/plain; charset=utf-8',
  md: 'text/plain; charset=utf-8', csv: 'text/plain; charset=utf-8',
  json: 'application/json', css: 'text/css; charset=utf-8',
  js: 'application/javascript', png: 'image/png', jpg: 'image/jpeg',
  jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp',
  svg: 'image/svg+xml', pdf: 'application/pdf', mp4: 'video/mp4',
  mp3: 'audio/mpeg', zip: 'application/zip'
};

function mimeFor(p) {
  const ext = path.extname(p).slice(1).toLowerCase();
  return MIME[ext] || 'application/octet-stream';
}

function escapeHtml(s) {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function encodePath(segs) {
  return segs.map(encodeURIComponent).join('/');
}

function contentDisposition(name) {
  const fallback = String(name).replace(/[^A-Za-z0-9\-_. ]/g, '_').trim();
  const encoded = encodeURIComponent(name);
  return 'attachment; filename="' + fallback + '"; filename*=UTF-8\'\'' + encoded;
}

function humanSize(n) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = Number(n) || 0;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return unit === 0 ? Math.round(n) + ' B' : value.toFixed(1) + ' ' + units[unit];
}

function sendText(res, status, text) {
  const body = Buffer.from(String(text), 'utf8');
  res.writeHead(status, {
    'Content-Type': 'text/plain; charset=utf-8',
    'Content-Length': body.length,
    'Connection': 'close'
  });
  res.end(body);
}

function sendHtml(res, method, status, body) {
  const buf = Buffer.from(body, 'utf8');
  res.writeHead(status, {
    'Content-Type': 'text/html; charset=utf-8',
    'Content-Length': buf.length,
    'Connection': 'close'
  });
  res.end(method === 'HEAD' ? undefined : buf);
}

const STYLE = [
  ':root{color-scheme:light dark}',
  '*{box-sizing:border-box}',
  'body{margin:0;padding:36px 18px;font:15px/1.6 "Segoe UI","Microsoft YaHei",system-ui,sans-serif;background:#f2f4f7;color:#1d2230}',
  'main{max-width:760px;margin:0 auto;background:#fff;border-radius:14px;padding:26px 28px 18px;box-shadow:0 8px 28px rgba(0,0,0,.08)}',
  'h1{font-size:20px;margin:0 0 14px;word-break:break-all}',
  '.hint{margin:0 0 10px;color:#6b7484;font-size:13px}',
  '.list{list-style:none;margin:0;padding:0}',
  '.list li{display:flex;align-items:center;gap:12px;padding:9px 2px;border-bottom:1px solid #eceef2}',
  '.list a{color:#2e5ce7;text-decoration:none;word-break:break-all}',
  '.list a:hover{text-decoration:underline}',
  '.list .meta{margin-left:auto;flex:0 0 auto;color:#6b7484;font-size:12px}',
  '.up{color:#2e5ce7;text-decoration:none;font-size:13px}',
  '.foot{margin:18px 0 0;color:#99a1b3;font-size:11.5px}',
  '@media (prefers-color-scheme:dark){body{background:#15171c;color:#e8ebf2}main{background:#1d2027;box-shadow:none}.list li{border-color:#2a2f39}.list a,.up{color:#6f8dff}.hint,.list .meta{color:#99a1b3}}'
].join('');

function page(title, body) {
  return '<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">' +
    '<meta name="viewport" content="width=device-width,initial-scale=1">' +
    '<title>' + escapeHtml(title) + '</title><style>' + STYLE + '</style></head><body><main>' +
    '<h1>' + escapeHtml(title) + '</h1>' + body +
    '<p class="foot">efto · 局域网只读共享</p></main></body></html>';
}

function row(href, name, kind) {
  return '<li><a href="' + href + '">' + escapeHtml(name) + '</a><span class="meta">' + escapeHtml(kind) + '</span></li>';
}

function listIndex() {
  let rows = '';
  for (let i = 0; i < items.length; i++) {
    const it = items[i];
    rows += row(it.is_dir ? '/s/' + i + '/' : '/f/' + i, it.name, it.is_dir ? '文件夹' : humanSize(it.size));
  }
  return '<p class="hint">共 ' + items.length + ' 项 · 只读共享</p><ul class="list">' + rows + '</ul>';
}

function listDir(idx, rel, dir) {
  let entries = [];
  try {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      let size = 0;
      try { size = fs.statSync(path.join(dir, e.name)).size; } catch (x) { continue; }
      entries.push({ name: e.name, isDir: e.isDirectory(), size: size });
    }
  } catch (e) {
    entries = [];
  }
  entries.sort(function (a, b) {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return a.name.toLowerCase() < b.name.toLowerCase() ? -1 : a.name.toLowerCase() > b.name.toLowerCase() ? 1 : 0;
  });
  let body = '';
  if (rel.length) {
    body += '<p><a class="up" href="/s/' + idx + '/' + encodePath(rel.slice(0, -1)) + '">返回上级</a></p>';
  }
  body += '<ul class="list">';
  for (const e of entries) {
    const child = rel.concat([e.name]);
    body += row('/s/' + idx + '/' + encodePath(child), e.name, e.isDir ? '文件夹' : humanSize(e.size));
  }
  body += '</ul>';
  const title = rel.length ? rel[rel.length - 1] : '共享目录';
  return { title: title, body: body };
}

function parseRange(value) {
  if (!value) return null;
  const m = /^bytes=(\d*)-(\d*)$/.exec(String(value).trim());
  if (!m || (!m[1] && !m[2])) return null;
  if (!m[1]) return { suffix: Number(m[2]) };
  return { start: Number(m[1]), end: m[2] ? Number(m[2]) : null };
}

function serveFile(req, res, filePath, name, range) {
  let st;
  try {
    st = fs.statSync(filePath);
  } catch (e) {
    return sendText(res, 404, '文件不存在');
  }
  const len = st.size;
  let status = 200;
  let start = 0;
  let end = len ? len - 1 : 0;
  if (range) {
    if (range.suffix !== undefined) {
      start = Math.max(0, len - range.suffix);
      end = len ? len - 1 : 0;
    } else {
      start = range.start;
      end = range.end === null ? (len ? len - 1 : 0) : Math.min(range.end, len ? len - 1 : 0);
    }
    if (start >= len || start > end) {
      res.writeHead(416, {
        'Content-Type': 'text/plain; charset=utf-8',
        'Content-Range': 'bytes */' + len,
        'Connection': 'close'
      });
      return res.end('请求范围无效');
    }
    status = 206;
  }
  const contentLen = len === 0 ? 0 : end - start + 1;
  const headers = {
    'Content-Type': mimeFor(filePath),
    'Content-Length': contentLen,
    'Content-Disposition': contentDisposition(name),
    'Accept-Ranges': 'bytes',
    'Connection': 'close'
  };
  if (status === 206) headers['Content-Range'] = 'bytes ' + start + '-' + end + '/' + len;
  res.writeHead(status, headers);
  if (req.method === 'HEAD' || contentLen === 0) return res.end();
  const stream = fs.createReadStream(filePath, { start: start, end: end });
  stream.on('error', function () { try { res.destroy(); } catch (e) { /* ignore */ } });
  stream.pipe(res);
}

function resolveUnder(root, segs) {
  let p = root;
  for (const s of segs) {
    if (!s || s === '.' || s === '..' || s.indexOf('\\') >= 0 || s.indexOf('/') >= 0 ||
        s.indexOf(':') >= 0 || s.indexOf('\0') >= 0) {
      return null;
    }
    p = path.join(p, s);
  }
  let canon;
  try {
    canon = fs.realpathSync(p);
  } catch (e) {
    return null;
  }
  const base = root.endsWith(path.sep) ? root : root + path.sep;
  return canon === root || canon.startsWith(base) ? canon : null;
}

function handleData(req, res) {
  const method = req.method || 'GET';
  if (method !== 'GET' && method !== 'HEAD') return sendText(res, 405, '只支持 GET / HEAD');

  const target = String(req.url || '/').split('?')[0] || '/';
  let segs;
  try {
    segs = target.split('/').filter(Boolean).map(function (s) { return decodeURIComponent(s); });
  } catch (e) {
    return sendText(res, 400, '路径编码无效');
  }

  if (!segs.length) return sendHtml(res, method, 200, page('FileShare', listIndex()));

  const idx = /^\d+$/.test(segs[1] || '') ? Number(segs[1]) : -1;
  if (idx < 0 || idx >= items.length) return sendText(res, 404, '没有这个共享项');
  const item = items[idx];

  if (segs[0] === 'f') {
    if (item.is_dir) {
      res.writeHead(302, { Location: '/s/' + idx + '/', 'Content-Length': 0, 'Connection': 'close' });
      return res.end();
    }
    return serveFile(req, res, item.root, item.name, parseRange(req.headers.range));
  }

  if (segs[0] === 's') {
    if (!item.is_dir) return sendText(res, 404, '该项不是文件夹');
    const rel = segs.slice(2);
    const resolved = resolveUnder(item.root, rel);
    if (!resolved) return sendText(res, 404, '路径无效');
    let st;
    try {
      st = fs.statSync(resolved);
    } catch (e) {
      return sendText(res, 404, '文件不存在');
    }
    if (st.isDirectory()) {
      const listed = listDir(idx, rel, resolved);
      return sendHtml(res, method, 200, page(listed.title, listed.body));
    }
    return serveFile(req, res, resolved, path.basename(resolved) || 'download', parseRange(req.headers.range));
  }

  return sendText(res, 404, '没有这个页面');
}

// ---------- 启停 ----------

function stopServers() {
  if (server4) {
    try { server4.close(); } catch (e) { /* ignore */ }
    server4 = null;
  }
  if (server6) {
    try { server6.close(); } catch (e) { /* ignore */ }
    server6 = null;
  }
  currentPort = 0;
  items = [];
}

function listenOn(server, port, host, ipv6Only) {
  return new Promise(function (resolve, reject) {
    function onError(e) {
      server.removeListener('listening', onListening);
      reject(e);
    }
    function onListening() {
      server.removeListener('error', onError);
      resolve(server.address().port);
    }
    server.once('error', onError);
    server.once('listening', onListening);
    const opts = { port: port, host: host };
    if (ipv6Only) opts.ipv6Only = true;
    server.listen(opts);
  });
}

async function startItems(portOpt, rawItems) {
  let requested = Number(portOpt);
  if (!isFinite(requested) || requested < 0 || requested > 65535) requested = DEFAULT_PORT;
  requested = Math.floor(requested);

  const parsed = parseItems(rawItems);
  if (!parsed.items.length) {
    stopServers();
    return {
      running: false, port: requested, urls: [], urls6: [],
      items: [], invalid: parsed.invalid
    };
  }

  const needRestart = !server4 || (requested !== 0 && requested !== currentPort);
  if (needRestart) {
    stopServers();
    const next4 = http.createServer(handleData);
    next4.on('clientError', function (_e, socket) { try { socket.destroy(); } catch (x) { /* ignore */ } });
    let actual;
    try {
      actual = await listenOn(next4, requested, '0.0.0.0', false);
    } catch (e) {
      return { running: false, port: 0, urls: [], urls6: [], items: [], invalid: parsed.invalid, error: '端口 ' + requested + ' 无法监听：' + e.message };
    }
    server4 = next4;
    currentPort = actual;

    // IPv6 是可选入口；系统没开 IPv6 或端口被占时跳过，不影响 IPv4。
    const next6 = http.createServer(handleData);
    next6.on('clientError', function (_e, socket) { try { socket.destroy(); } catch (x) { /* ignore */ } });
    try {
      await listenOn(next6, actual, '::', true);
      server6 = next6;
    } catch (e) {
      try { next6.close(); } catch (x) { /* ignore */ }
    }
  }

  items = parsed.items;
  return {
    running: true,
    port: currentPort,
    urls: lanUrls(currentPort),
    urls6: lanUrls6(currentPort),
    items: snapshotItems(),
    invalid: parsed.invalid
  };
}

function stopShare() {
  stopServers();
  return { running: false, port: 0, urls: [], urls6: [], items: [], invalid: [] };
}

// ---------- 控制口 ----------

function readBody(req, limit) {
  return new Promise(function (resolve, reject) {
    let buf = '';
    req.setEncoding('utf8');
    req.on('data', function (c) {
      buf += c;
      if (buf.length > limit) {
        reject(new Error('请求体过大'));
        req.destroy();
      }
    });
    req.on('end', function () { resolve(buf); });
    req.on('error', reject);
  });
}

function json(res, status, value) {
  const body = Buffer.from(JSON.stringify(value), 'utf8');
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': body.length,
    'Access-Control-Allow-Origin': '*',
    'Connection': 'close'
  });
  res.end(body);
}

const control = http.createServer(async function (req, res) {
  lastSeen = Date.now();
  res.setHeader('Access-Control-Allow-Origin', '*');
  if (req.method === 'OPTIONS') {
    res.writeHead(204, {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Private-Network': 'true',
      'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
      'Access-Control-Allow-Headers': 'content-type',
      'Connection': 'close'
    });
    return res.end();
  }

  const url = new URL(req.url || '/', 'http://127.0.0.1');
  if (url.searchParams.get('t') !== TOKEN) return sendText(res, 403, 'forbidden');

  if (req.method === 'GET' && url.pathname === '/status') {
    return json(res, 200, snapshot(!!server4, currentPort));
  }

  if (req.method === 'POST' && url.pathname === '/start') {
    let body;
    try {
      body = JSON.parse((await readBody(req, 1024 * 1024)) || '{}');
    } catch (e) {
      return json(res, 400, { error: '请求体不是合法 JSON：' + e.message });
    }
    try {
      return json(res, 200, await startItems(body.port, body.items));
    } catch (e) {
      return json(res, 200, { running: false, port: 0, urls: [], urls6: [], items: [], invalid: [], error: e.message });
    }
  }

  if (req.method === 'POST' && url.pathname === '/stop') {
    return json(res, 200, stopShare());
  }

  if (req.method === 'POST' && url.pathname === '/shutdown') {
    json(res, 200, { ok: true });
    return setTimeout(shutdown, 60);
  }

  return sendText(res, 404, '没有这个接口');
});

control.on('error', function (e) {
  log('控制口 127.0.0.1:' + CONTROL_PORT + ' 无法监听：' + e.message);
  process.exit(1);
});

control.listen(CONTROL_PORT, '127.0.0.1');

// 应用退出后没人 heartbeat：数据口在租约到期后自动停，避免 detached 服务继续对外共享。
setInterval(function () {
  if (!server4) {
    if (Date.now() - lastSeen > IDLE_EXIT_MS) shutdown();
    return;
  }
  if (Date.now() - lastSeen > LEASE_MS) {
    log('控制口长时间无请求，停止共享');
    stopServers();
  }
}, 15000).unref();

function shutdown() {
  stopServers();
  try { control.close(); } catch (e) { /* ignore */ }
  setTimeout(function () { process.exit(0); }, 60);
}

log('control listening on 127.0.0.1:' + CONTROL_PORT);
