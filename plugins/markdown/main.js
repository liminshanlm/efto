// markdown/main.js —— 运行在内嵌 V8（无 Node、无 DOM）。
// 只负责打开 UI 窗口，并把触发文本或「打开方式」传来的文件路径推给页面。

'use strict';

var UI = { title: 'Markdown', width: 1120, height: 760 };

// 触发词后面的输入（"md # 标题" 里的 "# 标题"）作为空白文档的初始内容
var seed = '';
var seedSent = false;

// 系统「打开方式」传入的文件路径（efto.exe --open-file <路径>）
var filePath = '';
var fileSent = false;

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

function sendOpenFile() {
  if (!filePath || fileSent) return;
  fileSent = true;
  push('open-file', { path: filePath });
}

efto.onPluginEnter(function (info) {
  var payload = info && typeof info.payload === 'string' ? info.payload : '';
  var cmd = info && typeof info.code === 'string' ? info.code : '';

  if (cmd === 'open-file') {
    filePath = payload.trim() ? payload : '';
    seed = '';
  } else {
    filePath = '';
    seed = payload.trim() ? payload : '';
  }
  seedSent = false;
  fileSent = false;

  efto.showUI(UI);

  // 窗口复用时页面不会再握手，这里直接推一次
  sendSeed();
  sendOpenFile();
});

efto.onMainMessage(function (msg) {
  if (msg && msg.name === 'init') {
    push('ready', {});
    sendSeed();
    sendOpenFile();
  }
});

efto.onUiClosed(function () {
  seed = '';
  seedSent = false;
  filePath = '';
  fileSent = false;
});
