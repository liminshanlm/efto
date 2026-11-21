// text-beautify/main.js —— 运行在内嵌 V8（无 Node、无 DOM）。
// 只做窗口与剪贴板的中转：真正的美化在 UI 页面里完成（lib/fmt-*.js 是纯函数，Node 与浏览器双用）。

'use strict';

var UI = { title: '文本美化', width: 1120, height: 720 };

// 触发时关键字后面那段输入（"美化 {"a":1}" 里的 JSON），进窗口后预填到左侧
var seed = '';
// 本次进入是否已经把种子推给页面：窗口复用（页面早已加载、不会再握手）时靠它避免重复推送
var seedSent = false;

function push(name, data) {
  try {
    efto.ipcMessage(name, data);
  } catch (e) {
    // UI 还没打开或插件正在退出：宿主会排队或丢弃，这里不必抛出
  }
}

function sendSeed() {
  if (!seed || seedSent) return;
  seedSent = true;
  push('seed', { text: seed });
}

efto.onPluginEnter(function (info) {
  var payload = info && typeof info.payload === 'string' ? info.payload : '';
  seed = payload.trim() ? payload : '';
  seedSent = false;

  efto.showUI(UI);

  // 直接推一次：窗口是复用的（页面已加载完毕）时不会再收到 init，只能靠这里
  sendSeed();
});

efto.onMainMessage(function (msg) {
  var name = msg && msg.name;

  if (name === 'init') {
    // 新页面加载完成会主动握手，补推一次可覆盖"推得太早、页面还没注册监听"的情况
    push('ready', { platform: efto.platform, version: efto.version });
    sendSeed();
    return;
  }

  if (name === 'paste') {
    var text = '';
    var error = '';
    try {
      text = efto.pasteText() || '';
    } catch (e) {
      // CLI 宿主与 GUI 宿主的剪贴板能力不同，差异要如实回报，
      // 不能让用户以为是"剪贴板里没有文字"
      error = (e && e.message) ? e.message : String(e);
    }
    push('pasted', { text: text, error: error });
  }
});

// 关掉 UI 就清掉种子文本，下次进来不会把上次的输入又贴一遍
efto.onUiClosed(function () {
  seed = '';
  seedSent = false;
});
