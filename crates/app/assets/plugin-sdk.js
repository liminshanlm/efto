// efto 注入给插件 UI 页面的 SDK（无 npm，纯手写，走 Tauri 注入的 __TAURI_INTERNALS__）。
// 页面里可直接使用 window.efto.*，与 main 进程通过宿主互发消息。
(function () {
  'use strict';
  var I = window.__TAURI_INTERNALS__;
  if (!I || typeof I.invoke !== 'function') {
    console.warn('[efto] __TAURI_INTERNALS__ 不可用，插件 SDK 降级为空实现');
    return;
  }

  function invoke(cmd, payload) {
    return I.invoke(cmd, payload || {});
  }

  // 事件名只允许字母数字与 - / : _；插件 id 用 cmd 参数传递，不放事件名里
  var listeners = {};
  var started = false;

  function dispatch(name, data) {
    var arr = listeners[name] || [];
    for (var i = 0; i < arr.length; i++) {
      try { arr[i](data); } catch (e) { console.error('[plugin-sdk] handler 出错', e); }
    }
  }

  function startListening() {
    if (started) return;
    started = true;
    var handler = I.transformCallback(function (ev) {
      var p = ev.payload || {};
      dispatch(p.name, p.data);
    }, false);
    // target 必须给：Tauri 2 的 EventTarget 是 `{kind:"Any"}` 这种结构，
    // 早先漏了这个字段，订阅直接失败 → 页面永远收不到主进程消息（而且报错只在 webview 控制台里）
    invoke('plugin:event|listen', {
      event: 'plugin:message',
      target: { kind: 'Any' },
      handler: handler
    })
      .then(function () {
        // 握手：告诉宿主"监听已就绪，可以补发排队消息了"。
        // 不这样做的话，主进程在 showUI 之前发的消息会在注册完成前被发出而丢掉。
        return invoke('plugin_ui_ready');
      })
      .catch(function (e) {
        console.warn('[plugin-sdk] 事件订阅失败', e);
        // 报给宿主日志：webview 控制台里的报错在宿主侧看不见
        invoke('plugin_ui_log', { level: 'error', message: '事件订阅失败: ' + e }).catch(function () {});
        started = false;
      });
  }

  // 订阅 Tauri 自带事件（例如系统文件拖拽）。这类事件的目标是当前 webview，
  // 不能用 onMessage 的 Any 目标，否则收不到。
  function listenTauriEvent(event, cb) {
    var meta = I.metadata || {};
    var label = (meta.currentWebview && meta.currentWebview.label) || '';
    var target = label ? { kind: 'Webview', label: label } : { kind: 'Any' };
    var handler = I.transformCallback(function (ev) {
      try { cb(ev && ev.payload); } catch (e) { console.error('[plugin-sdk] 事件处理出错', event, e); }
    }, false);
    invoke('plugin:event|listen', { event: event, target: target, handler: handler })
      .catch(function (e) {
        invoke('plugin_ui_log', { level: 'warn', message: event + ' 订阅失败: ' + e }).catch(function () {});
      });
  }

  function on(name, cb) {
    startListening();
    (listeners[name] = listeners[name] || []).push(cb);
    return cb;
  }

  function off(name, cb) {
    var arr = listeners[name] || [];
    for (var i = arr.length - 1; i >= 0; i--) if (arr[i] === cb) arr.splice(i, 1);
  }

  var efto = {
    // 与 main 通信
    ipcMessage: function (name, data) { return invoke('plugin_to_main', { name: name, data: data === undefined ? null : data }); },
    onMessage: on,
    removeListener: off,
    broadcast: function (name, data) { return invoke('plugin_broadcast', { name: name, data: data === undefined ? null : data }); },

    // 窗口
    setSubTitle: function (t) { return invoke('plugin_set_subtitle', { text: String(t == null ? '' : t) }); },
    hideWindow: function () { return invoke('plugin_hide_self'); },
    showMainWindow: function () { return invoke('plugin_show_main'); },
    quit: function () { return invoke('plugin_quit'); },

    // 数据（直接读写本插件的 kv，UI 侧无需绕 main）
    kv: {
      get: function (k) { return invoke('plugin_db_get', { key: String(k) }); },
      set: function (k, v) { return invoke('plugin_db_set', { key: String(k), value: v === undefined ? null : v }); },
      assign: function (k, v) { return efto.kv.set(k, v); },
      remove: function (k) { return invoke('plugin_db_remove', { key: String(k) }); },
      all: function () { return invoke('plugin_db_all'); },
      keys: function () { return invoke('plugin_db_keys'); },
      size: function () { return invoke('plugin_db_size'); }
    },

    // 系统
    copyText: function (t) { return invoke('plugin_copy_text', { text: String(t) }); },
    pasteText: function () { return invoke('plugin_paste_text'); },
    showNotification: function (t, sub) { return invoke('plugin_notify', { text: String(t == null ? '' : t), sub: sub ? String(sub) : '' }); },
    shellOpenPath: function (p) { return invoke('plugin_shell_open', { path: String(p) }); },
    shellShowItemInFolder: function (p) { return invoke('plugin_shell_show_in_folder', { path: String(p) }); },
    openExternal: function (u) { return invoke('plugin_open_external', { url: String(u) }); },

    // 原生文件选择框；取消时 resolve 空数组
    pickFiles: function () { return invoke('plugin_pick_files'); },
    // 文本文件读写：只允许 pickFiles 选中或系统拖入的路径（宿主侧授权，见 fileapi.rs）。
    // read 拒绝非 UTF-8、目录与超过 8 MiB 的文件；saveAs 走原生保存框，取消返回 null。
    file: {
      read: function (path) { return invoke('plugin_file_read', { path: String(path) }); },
      write: function (path, text) { return invoke('plugin_file_write', { path: String(path), text: String(text) }); },
      saveAs: function (name, text) {
        return invoke('plugin_file_save_as', { name: String(name == null ? 'untitled.md' : name), text: String(text) });
      }
    },
    // 系统文件拖拽：cb(paths) 只在松手时触发；cb(bool) 用于高亮拖拽区域
    onFileDrop: function (cb) {
      listenTauriEvent('tauri://drag-drop', function (p) { cb((p && p.paths) || []); });
      return cb;
    },
    onFileDrag: function (cb) {
      listenTauriEvent('tauri://drag-enter', function () { cb(true); });
      listenTauriEvent('tauri://drag-leave', function () { cb(false); });
      return cb;
    },
    redirect: function (label, options) { return invoke('plugin_redirect', { label: String(label), options: options || {} }); },
    platform: 'win32',
    // 主题：必须是函数（插件页面写 `efto.isDarkMode()`）。
    // 判据是窗口的 prefers-color-scheme：宿主打开插件窗口时会按设置里的
    // 跟随系统/浅色/深色锁定窗口主题，和主窗口保持一致。
    isDarkMode: function () { return prefersDark(); },
    isLightMode: function () { return !prefersDark(); }
  };

  function prefersDark() {
    try {
      return !!(window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches);
    } catch (e) {
      return true; // 取不到时回退深色，与插件样式表的默认令牌一致
    }
  }

  window.efto = efto;
})();
