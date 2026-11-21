// efto 主题：所有本地窗口共用一套解析规则。
//
// 在 <head> 里、样式表之前同步加载：先用本地缓存把 html[data-theme] 打上，
// 避免浅色环境下首帧闪一下深色；随后再用 theme_mode 与磁盘设置对齐。
// 模式只有三种：system（跟随系统）/ dark / light。
(function () {
  'use strict';

  var KEY = 'efto.theme';
  var mq = window.matchMedia('(prefers-color-scheme: dark)');
  var mode = localStorage.getItem(KEY) || 'system';

  function resolve(m) {
    if (m === 'dark' || m === 'light') return m;
    return mq.matches ? 'dark' : 'light';
  }

  function paint(m) {
    document.documentElement.dataset.theme = resolve(m);
  }

  function set(m) {
    mode = m === 'dark' || m === 'light' ? m : 'system';
    localStorage.setItem(KEY, mode);
    paint(mode);
  }

  paint(mode);

  // 跟随系统时，Windows 的明暗切换要即时反映到已打开的窗口
  function onSystemChange() {
    if (mode === 'system') paint(mode);
  }
  if (mq.addEventListener) mq.addEventListener('change', onSystemChange);
  else if (mq.addListener) mq.addListener(onSystemChange);

  // 供页面调用：设置页切换主题后立刻重画自己
  window.eftoTheme = {
    current: function () { return mode; },
    set: set
  };

  var I = window.__TAURI_INTERNALS__;
  if (!I || typeof I.invoke !== 'function') return;

  // 本地缓存可能缺失（清过 WebView 数据）或被手改过 settings.json：以磁盘为准
  I.invoke('theme_mode')
    .then(function (m) {
      if (typeof m === 'string') set(m);
    })
    .catch(function () {});

  // 设置页保存主题后广播给其它已打开的窗口
  try {
    var cb = I.transformCallback(function (ev) {
      var p = ev && ev.payload;
      set(typeof p === 'string' ? p : String(p));
    }, false);
    I.invoke('plugin:event|listen', {
      event: 'settings:theme',
      target: { kind: 'Any' },
      handler: cb
    }).catch(function () {});
  } catch (e) {
    // 没有事件通道（例如非 Tauri 环境）时只保留本地缓存效果
  }
})();
