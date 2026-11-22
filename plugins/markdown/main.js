// markdown/main.js —— 运行在内嵌 V8（无 Node、无 DOM）。
// 只负责打开 UI 窗口并把触发词后面的文本作为初始内容推给页面。

'use strict';

var UI = { title: 'Markdown', width: 1120, height: 760 };

// 触发词后面的输入（"md # 标题" 里的 "# 标题"）作为空白文档的初始内容
var seed = '';
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

  // 窗口复用时页面不会再握手，这里直接推一次
  sendSeed();
});

efto.onMainMessage(function (msg) {
  if (msg && msg.name === 'init') {
    push('ready', {});
    sendSeed();
  }
});

efto.onUiClosed(function () {
  seed = '';
  seedSent = false;
});
