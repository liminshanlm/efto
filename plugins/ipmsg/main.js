// ipmsg/main.js —— 运行在内嵌 V8（无 Node、无 DOM）。
// 宿主只提供「运行 Node」这一件事：这里用 efto.node.run 启动一个 detached 的 Node 服务
// （同目录 service.js），真正的 UDP 发现 / TCP 收发 / 授权状态都在 Node 里。
// UI 直接访问 127.0.0.1 上的本地控制口，main 只负责拉起服务、下发端口与令牌。

'use strict';

var CONTROL_PORT = 24251;
var TOKEN_KEY = 'token';
var SHOW_UI = { title: '局域网消息', width: 880, height: 640 };

// 在 node -e 里执行：先探测同令牌的旧实例，没有再拉起 service.js。
// 工作目录就是插件根目录，所以直接用 process.cwd() 找 service.js。
var BOOTSTRAP = [
  "const http = require('node:http');",
  "const { spawn } = require('node:child_process');",
  "const fs = require('node:fs');",
  "const os = require('node:os');",
  "const path = require('node:path');",
  "const TOKEN = process.argv[1] || '';",
  "const PORT = 24251;",
  "let done = false;",
  "let starting = false;",
  "function finish(obj) {",
  "  if (done) return;",
  "  done = true;",
  "  process.stdout.write(JSON.stringify(obj));",
  "  process.exit(0);",
  "}",
  "function start() {",
  "  if (starting || done) return;",
  "  starting = true;",
  "  const service = path.join(process.cwd(), 'service.js');",
  "  if (!fs.existsSync(service)) return finish({ ok: false, error: '插件缺少 service.js' });",
  "  let out = 'ignore';",
  "  let logFile = '';",
  "  try {",
  "    logFile = path.join(os.tmpdir(), 'efto-ipmsg-service.log');",
  "    out = fs.openSync(logFile, 'w');",
  "  } catch (e) { out = 'ignore'; }",
  "  let child;",
  "  try {",
  "    child = spawn(process.execPath, [service, TOKEN], {",
  "      cwd: process.cwd(),",
  "      detached: true,",
  "      stdio: ['ignore', out, out],",
  "      windowsHide: true",
  "    });",
  "  } catch (e) {",
  "    return finish({ ok: false, error: '无法启动 Node 服务：' + e.message });",
  "  }",
  "  child.unref();",
  "  setTimeout(function () {",
  "    if (child.exitCode === null) return finish({ ok: true });",
  "    let log = '';",
  "    try { log = fs.readFileSync(logFile, 'utf8').slice(-600); } catch (e) {}",
  "    finish({ ok: false, error: '消息服务启动失败（端口被占用或 Node 不可用）', log: log });",
  "  }, 450);",
  "}",
  "function probe() {",
  "  const req = http.get({",
  "    host: '127.0.0.1',",
  "    port: PORT,",
  "    path: '/status?t=' + encodeURIComponent(TOKEN),",
  "    timeout: 600",
  "  }, function (res) {",
  "    res.resume();",
  "    if (res.statusCode === 200) finish({ ok: true, reused: true });",
  "    else start();",
  "  });",
  "  req.on('timeout', function () { req.destroy(); start(); });",
  "  req.on('error', function () { start(); });",
  "}",
  "probe();"
].join('\n');

var token = '';
var booting = false;
var svc = { state: 'starting', error: '' };

function errText(e) {
  return e && e.message ? e.message : String(e);
}

function pushSvc() {
  try {
    efto.ipcMessage('svc', {
      state: svc.state,
      port: CONTROL_PORT,
      token: token,
      error: svc.error || ''
    });
  } catch (e) {
    // UI 还没打开时宿主会排队
  }
}

function ensureToken() {
  var t = efto.kv.get(TOKEN_KEY);
  if (typeof t !== 'string' || t.length < 8) {
    t = Math.random().toString(36).slice(2) + Date.now().toString(36);
    try {
      efto.kv.set(TOKEN_KEY, t);
    } catch (e) {
      // 存储失败也还能用，只是每次启动换令牌
    }
  }
  return t;
}

function boot() {
  if (booting) return;
  booting = true;
  svc = { state: 'starting', error: '' };
  pushSvc();

  var promise;
  try {
    promise = efto.node.run({ code: BOOTSTRAP, args: [token], timeoutMs: 10000 });
  } catch (e) {
    booting = false;
    svc = { state: 'error', error: errText(e) };
    pushSvc();
    return;
  }

  promise.then(function (r) {
    booting = false;
    var out = null;
    try {
      out = JSON.parse(String((r && r.stdout) || '').trim());
    } catch (e) {
      out = null;
    }
    if (!out) {
      var detail = String((r && (r.stderr || r.stdout)) || '').slice(0, 300);
      svc = { state: 'error', error: 'Node 启动输出无法解析' + (detail ? '：' + detail : '') };
    } else if (!out.ok) {
      svc = {
        state: 'error',
        error: String(out.error || '消息服务启动失败') + (out.log ? '\n' + out.log : '')
      };
    } else {
      svc = { state: 'ready', error: '' };
    }
    pushSvc();
  }).catch(function (e) {
    booting = false;
    svc = { state: 'error', error: errText(e) };
    pushSvc();
  });
}

efto.onPluginEnter(function () {
  token = ensureToken();
  if (svc.state === 'ready') pushSvc();
  else boot();
  efto.showUI(SHOW_UI);
});

efto.onMainMessage(function (msg) {
  if (!msg) return;
  if (msg.name === 'init') pushSvc();
  else if (msg.name === 'restart') boot();
});

// 主动停服由 UI 的 pagehide 发本地 /stop 完成：main 再发一次会和「刚关就重开」抢跑。
// 万一 pagehide 没触发，service.js 有 30 分钟空闲自退兜底。
