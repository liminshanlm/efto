// file-share/main.js —— 运行在内嵌 V8（无 Node、无 DOM）。
// 只做编排：共享列表存 plugin kv；HTTP 服务由宿主 efto.share 提供。
// 局域网文件共享（Apache-2.0 参考实现改写），后端由宿主 Rust 能力提供，不依赖 Node/express。

'use strict';

var DEFAULT_PORT = 5421;
// [{ path, name? }]：原样保存用户添加的路径，读不到的由宿主单独报回
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
      var it = meta[m];
      if (it && typeof it.path === 'string' && it.path) {
        state.items.push({
          name: it.name,
          path: it.path,
          is_dir: !!it.is_dir,
          size: Number(it.size) || 0
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

// 只在 state.items 是宿主校验后的真实条目时调用，供下次以停止状态打开时展示
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

function apply() {
  state.error = '';
  try {
    var res = efto.share.start({ port: state.port, items: rawItems }) || {};
    state.running = !!res.running;
    if (res.port) state.port = res.port;
    state.urls = res.urls || [];
    state.urls6 = res.urls6 || [];
    state.items = res.items || [];
    state.invalid = res.invalid || [];
    saveMeta();
  } catch (e) {
    state.error = (e && e.message) ? e.message : String(e);
    // 启动失败时保留上一次的条目，用户改端口/删路径后可以直接重试
    state.running = false;
    state.urls = [];
    state.urls6 = [];
  }
  push();
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

efto.onPluginEnter(function (info) {
  load();
  var payload = info && typeof info.payload === 'string' ? info.payload.trim() : '';
  if (payload && addPaths(payload)) save();
  // 有配置且用户上次没有主动取消分享，才自动恢复；取消过就停在"未在分享"
  if (rawItems.length && wantShare) {
    sharing = true;
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
    state.error = '';
    try {
      efto.share.stop();
    } catch (e) {
      state.error = (e && e.message) ? e.message : String(e);
    }
    // 取消分享是用户的明确意图，落盘后重开插件不会再自动启动
    wantShare = false;
    sharing = false;
    state.running = false;
    state.urls = [];
    state.urls6 = [];
    save();
    push();
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
efto.onUiClosed(function () {
  try {
    efto.share.stop();
  } catch (e) {
    // 进程退出时忽略
  }
  sharing = false;
  state.running = false;
  state.urls = [];
  state.urls6 = [];
  state.items = [];
  state.invalid = [];
});
