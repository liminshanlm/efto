// hello-js/main.js —— 运行在内嵌 V8 里（无 Node、无 DOM）
// 用 efto 注册式 API 挂钩子。

'use strict';

var launchCount = 0;

efto.onPluginEnter(function (info) {
  launchCount += 1;
  var payload = info.payload;
  var text = typeof payload === 'string' && payload.trim() ? payload.trim() : '世界';

  // 1) 记住最近一次打招呼内容（efto.db 是文档接口，KV 用 efto.kv）
  var doc = { _id: 'last', text: text, at: new Date().toISOString() };
  var put = efto.db.put(doc);

  // 2) 用 KV 存计数
  var prev = Number(efto.kv.get('count') || 0);
  efto.kv.set('count', String(prev + 1));

  // 3) 通知 + 打开 UI 页面
  efto.showNotification('已向「' + text + '」打招呼（第 ' + (prev + 1) + ' 次）');
  efto.setSubTitle('hello-js · rev ' + (put && put.rev ? put.rev : '-'));
  efto.showUI();

  // 4) 把数据推给 UI 页面
  efto.ipcMessage('greet', {
    text: text,
    count: prev + 1,
    launches: launchCount,
    locale: efto.locale,
    docs: efto.db.allDocs({ prefix: '' }).total_rows
  });
});

// UI 页面通过 ipcMessage 发来的消息在这里接收
efto.onMainMessage(function (msg) {
  var name = msg && msg.name;
  var data = (msg && msg.data) || {};
  if (name === 'copy-greet') {
    efto.copyText(data.text);
    efto.showNotification('已复制到剪贴板');
    efto.ipcMessage('copied', { ok: true });
  } else if (name === 'reset') {
    efto.kv.remove('count');
    efto.db.remove({ _id: 'last' });
    efto.ipcMessage('reset-done', { ok: true });
  } else if (name === 'whoami') {
    efto.ipcMessage('whoami', {
      platform: efto.platform,
      isWindows: efto.isWindows(),
      version: efto.version,
      dbPath: efto.dbPath,
      timers: typeof setTimeout === 'function'
    });
  }
});

efto.onPluginOut(function () {
  // 隐藏 UI，后端 isolate 继续存活
  efto.hideUI();
});
