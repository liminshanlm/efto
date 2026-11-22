// file-share/main.js —— 运行在内嵌 V8（无 Node、无 DOM）。
// 只做编排：共享列表存 plugin kv；HTTP 服务由插件自带的 Node 服务（service.js）提供，
// 本机没有可用 Node 时插件不可用（设置 → 运行时 配置并检测通过）。
//
// 与 service.js 的通道：efto.node.run 执行一段 node 脚本，访问 127.0.0.1:5422 控制口。

'use strict';

var DEFAULT_PORT = 5421;
var TOKEN_KEY = 'svcToken';
// [{ path, name? }]：原样保存用户添加的路径，读不到的由服务单独报回
var rawItems = [];
// 用户是否处于"正在分享"意图：取消分享后删条目不应偷偷重启服务。
// 这个意图要落盘：取消分享后关掉插件再打开，不能又自动变成分享中。
var wantShare = true;
// 服务是否真的在跑（本进程内）；onUiClosed 会停掉它，但不动 wantShare。
var sharing = false;
var state = {
  running: false,
  port: DEFAULT_PORT,
  urls: [],
  urls6: [],
  items: [],
  invalid: [],
  error: ''
};

var token = ensureToken();
var booted = false;
var booting = null;
var applySeq = 0;
var leaseTimer = 0;

function ensureToken() {
  var t = efto.kv.get(TOKEN_KEY);
  if (typeof t !== 'string' || !t) {
    t = Math.random().toString(36).slice(2) + Date.now().toString(36);
    efto.kv.set(TOKEN_KEY, t);
  }
  return t;
}

// 访问控制口；参数：token、操作名、可选 JSON body。
var CALL_CODE = [
  "var http = require('node:http');",
  "var token = process.argv[1] || '';",
  "var op = process.argv[2] || 'status';",
  "var body = process.argv[3] || '';",
  "var req = http.request({ host: '127.0.0.1', port: 5422, method: body ? 'POST' : 'GET',",
  "  path: '/' + op + '?t=' + encodeURIComponent(token), timeout: 12000,",
  "  headers: body ? { 'content-type': 'text/plain', 'content-length': Buffer.byteLength(body) } : {} },",
  "  function (res) { var out = ''; res.setEncoding('utf8');",
  "    res.on('data', function (c) { out += c; });",
  "    res.on('end', function () { process.stdout.write(out); }); });",
  "req.on('timeout', function () { req.destroy(new Error('控制口超时')); });",
  "req.on('error', function (e) { process.stderr.write(String(e && e.message || e)); process.exit(1); });",
  "if (body) req.write(body);",
  "req.end();"
].join('\n');

// 探测旧实例；没有就 detached 拉起 service.js。
var BOOT_CODE = [
  "var http = require('node:http');",
  "var fs = require('node:fs'), os = require('node:os'), path = require('node:path');",
  "var cp = require('node:child_process');",
  "var token = process.argv[1] || '';",
  "function done(v) { process.stdout.write(JSON.stringify(v)); }",
  "function probe() {",
  "  var req = http.get({ host: '127.0.0.1', port: 5422, timeout: 800,",
  "    path: '/status?t=' + encodeURIComponent(token) },",
  "    function (res) { var d = ''; res.on('data', function (c) { d += c; });",
  "      res.on('end', function () { done(res.statusCode === 200 ? { ok: true, reused: true } : { ok: false, error: '控制口被占用（令牌不匹配）' }); }); });",
  "  req.on('timeout', function () { req.destroy(); });",
  "  req.on('error', function () { spawn(); });",
  "}",
  "function spawn() {",
  "  var log = path.join(os.tmpdir(), 'efto-file-share-service.log');",
  "  var fd = null;",
  "  try { fd = fs.openSync(log, 'a'); } catch (e) { fd = null; }",
  "  var child;",
  "  try {",
  "    child = cp.spawn(process.execPath, [path.join(process.cwd(), 'service.js'), token], {",
  "      detached: true, windowsHide: true,",
  "      stdio: ['ignore', fd === null ? 'ignore' : fd, fd === null ? 'ignore' : fd] });",
  "  } catch (e) { done({ ok: false, error: String(e && e.message || e) }); return; }",
  "  child.unref();",
  "  if (fd !== null) { try { fs.closeSync(fd); } catch (e) {} }",
  "  setTimeout(function () {",
  "    if (child.exitCode !== null) {",
  "      var t = '';",
  "      try { t = fs.readFileSync(log, 'utf8').split('\\n').slice(-6).join('\\n'); } catch (e) {}",
  "      done({ ok: false, error: '共享服务启动失败（退出码 ' + child.exitCode + '）' + (t ? '\\n' + t : '') });",
  "    } else { done({ ok: true, started: true }); }",
  "  }, 450);",
  "}",
  "probe();"
].join('\n');

function errText(e) {
  var msg = (e && e.message) ? e.message : String(e);
  return msg.trim() || '未知错误';
}

function parseLastLine(stdout) {
  var text = String(stdout || '').trim();
  if (!text) return null;
  var lines = text.split('\n');
  try {
    return JSON.parse(lines[lines.length - 1]);
  } catch (e) {
    return null;
  }
}

function ensureService() {
  if (booted) return Promise.resolve(true);
  if (booting) return booting;
  booting = efto.node.run({ code: BOOT_CODE, args: [token], timeoutMs: 15000 }).then(function (r) {
    var hit = parseLastLine(r && r.stdout);
    if (!hit || !hit.ok) {
      var why = (hit && hit.error) || (r && String(r.stderr || '').trim()) || 'Node 运行时不可用';
      throw new Error(why);
    }
    booted = true;
    booting = null;
    return true;
  }).catch(function (e) {
    booting = null;
    throw e;
  });
  return booting;
}

function control(op, payload) {
  var body = payload ? JSON.stringify(payload) : '';
  return ensureService().then(function () {
    return efto.node.run({ code: CALL_CODE, args: [token, op, body], timeoutMs: 15000 });
  }).then(function (r) {
    if (!r || !r.ok) {
      throw new Error(String((r && r.stderr) || '').trim() || '共享服务调用失败');
    }
    var out = parseLastLine(r.stdout);
    if (!out) throw new Error('共享服务没有返回结果');
    if (out.error) throw new Error(out.error);
    return out;
  });
}

function load() {
  var stored = efto.kv.get('items');
  rawItems = [];
  if (Array.isArray(stored)) {
    for (var i = 0; i < stored.length; i++) {
      var it = stored[i];
      if (it && typeof it.path === 'string' && it.path.trim()) {
        rawItems.push({ path: it.path.trim(), name: it.name || undefined });
      }
    }
  }
  var port = Number(efto.kv.get('port'));
  state.port = port >= 1 && port <= 65535 ? port : DEFAULT_PORT;
  // 缺省视为"要分享"，兼容升级前只存了 items 的老数据
  var want = efto.kv.get('wantShare');
  wantShare = want === false ? false : true;
  // 上次的条目元信息（is_dir/size）：停止状态下也要能正常列出文件与文件夹
  var meta = efto.kv.get('itemsMeta');
  state.items = [];
  if (Array.isArray(meta)) {
    for (var m = 0; m < meta.length; m++) {
      var it2 = meta[m];
      if (it2 && typeof it2.path === 'string' && it2.path) {
        state.items.push({
          name: it2.name,
          path: it2.path,
          is_dir: !!it2.is_dir,
          size: Number(it2.size) || 0
        });
      }
    }
  }
}

function save() {
  try {
    efto.kv.set('items', rawItems);
    efto.kv.set('port', state.port);
    efto.kv.set('wantShare', wantShare);
  } catch (e) {
    // 存储失败不影响本次运行
  }
}

// 只在 state.items 是服务校验后的真实条目时调用，供下次以停止状态打开时展示
function saveMeta() {
  try {
    efto.kv.set('itemsMeta', state.items);
  } catch (e) {
    // 存储失败不影响本次运行
  }
}

// 路径匹配用：去掉 Windows canonicalize 的 \\?\ / \\?\UNC\ 前缀并统一大小写。
// 老版本宿主回传的是带前缀的路径，KV 里可能还留着，匹配时要能对上。
function normPath(p) {
  return String(p == null ? '' : p)
    .replace(/^\\\\\?\\UNC\\/i, '\\\\')
    .replace(/^\\\\\?\\/i, '')
    .toLowerCase();
}

function push() {
  try {
    efto.ipcMessage('state', state);
  } catch (e) {
    // UI 还没打开时宿主会排队，这里的异常只可能是插件进程正在退出
  }
}

// 数据口在跑时定期 heartbeat，service 用租约判断"插件是否还活着"。
function setLease(on) {
  if (on && !leaseTimer) {
    leaseTimer = setInterval(function () {
      if (!sharing) return;
      control('status').catch(function () { /* 下次 apply 会重新拉起 */ });
    }, 45000);
  } else if (!on && leaseTimer) {
    clearInterval(leaseTimer);
    leaseTimer = 0;
  }
}

function applyState(res) {
  state.error = '';
  state.running = !!res.running;
  if (res.port) state.port = res.port;
  state.urls = res.urls || [];
  state.urls6 = res.urls6 || [];
  state.items = res.items || [];
  state.invalid = res.invalid || [];
  sharing = state.running;
  saveMeta();
  setLease(sharing);
  push();
}

function apply() {
  var seq = ++applySeq;
  return control('start', { port: state.port, items: rawItems }).then(function (res) {
    if (seq !== applySeq) return;
    applyState(res);
  }).catch(function (e) {
    if (seq !== applySeq) return;
    state.error = errText(e);
    // 启动失败时保留上一次的条目，用户改端口/删路径后可以直接重试
    state.running = false;
    state.urls = [];
    state.urls6 = [];
    sharing = false;
    setLease(false);
    push();
  });
}

// 一次可粘贴多行（也接受分号分隔）；从选择框/拖拽进来的是数组，不能再按分号拆
// （Windows 文件名允许分号）。重复路径按大小写不敏感去重。
function addPaths(input) {
  var parts = Array.isArray(input)
    ? input
    : String(input == null ? '' : input).split(/[\r\n;]+/);
  var added = 0;
  for (var i = 0; i < parts.length; i++) {
    var p = parts[i].trim().replace(/^"(.*)"$/, '$1').replace(/^'(.*)'$/, '$1');
    if (!p) continue;
    var dup = false;
    for (var j = 0; j < rawItems.length; j++) {
      if (normPath(rawItems[j].path) === normPath(p)) {
        dup = true;
        break;
      }
    }
    if (dup) continue;
    rawItems.push({ path: p });
    added++;
  }
  return added;
}

// 停止分享只改服务状态，保留 state.items（UI 里仍要能看到已添加的条目）。
function applyStopped() {
  state.error = '';
  state.running = false;
  state.urls = [];
  state.urls6 = [];
  state.invalid = [];
  sharing = false;
  setLease(false);
  push();
}

function stopShare() {
  applySeq++;
  if (!booted) {
    applyStopped();
    return Promise.resolve();
  }
  return control('stop').then(function () {
    applyStopped();
  }).catch(function (e) {
    state.error = errText(e);
    applyStopped();
  });
}

efto.onPluginEnter(function (info) {
  load();
  var payload = info && typeof info.payload === 'string' ? info.payload.trim() : '';
  if (payload && addPaths(payload)) save();
  // 有配置且用户上次没有主动取消分享，才自动恢复；取消过就停在"未在分享"
  if (rawItems.length && wantShare) {
    sharing = true;
    state.error = '';
    state.running = false;
    push();
    apply();
  } else {
    state.error = '';
    state.running = false;
    state.urls = [];
    state.urls6 = [];
    state.invalid = [];
    push();
  }
  efto.showUI({ title: 'FileShare', width: 780, height: 640 });
});

efto.onMainMessage(function (msg) {
  var name = msg && msg.name;
  var data = (msg && msg.data) || {};

  if (name === 'init') {
    push();
  } else if (name === 'start') {
    var port = Number(data.port);
    if (port >= 1 && port <= 65535) state.port = port;
    wantShare = true;
    sharing = true;
    save();
    apply();
  } else if (name === 'stop') {
    // 取消分享是用户的明确意图，落盘后重开插件不会再自动启动
    wantShare = false;
    save();
    stopShare();
  } else if (name === 'add') {
    if (addPaths(Array.isArray(data.paths) ? data.paths : data.text)) {
      wantShare = true;
      sharing = true;
      save();
      apply();
    }
  } else if (name === 'remove') {
    var target = normPath(data.path);
    if (!target) return;
    var next = [];
    for (var i = 0; i < rawItems.length; i++) {
      if (normPath(rawItems[i].path) !== target) next.push(rawItems[i]);
    }
    rawItems = next;
    save();
    if (sharing) {
      apply();
    } else {
      state.items = state.items.filter(function (it) {
        return normPath(it.path) !== target;
      });
      state.invalid = state.invalid.filter(function (it) {
        return normPath(it.path) !== target;
      });
      saveMeta();
      push();
    }
  } else if (name === 'clear') {
    rawItems = [];
    wantShare = false;
    sharing = false;
    save();
    apply();
  }
});

// 关闭 UI 窗口即停止服务：避免服务在用户看不见的地方继续跑。
// service 另有租约兜底（应用被强杀时 2 分钟内自动停）。
efto.onUiClosed(function () {
  applySeq++; // 丢掉还在路上的 start 结果，避免窗口关了又恢复分享
  setLease(false);
  sharing = false;
  state.running = false;
  state.urls = [];
  state.urls6 = [];
  state.items = [];
  state.invalid = [];
  if (booted) control('stop').catch(function () { /* 进程退出时忽略 */ });
});
