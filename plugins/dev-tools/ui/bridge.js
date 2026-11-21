// efto 适配层：在 Ctool 主包加载前完成三件事。
//   1. 用宿主剪贴板替换 navigator.clipboard / permissions：WebView2 里没有浏览器剪贴板授权，
//      Ctool 的「复制」「自动读取剪贴板」会整体降级为不可用；
//   2. 先向 main 要本次要打开的路由，改好 hash 再加载主包（主包一 import 就会挂载 Vue Router，
//      路由必须在它求值之前就位）；
//   3. UI 窗口会被宿主复用：第二次触发时整页 reload，让组件重新 setup 以吃到新路由与预填输入。
(function () {
  'use strict';

  var bridge = window.efto;
  var APP_ASSET = './assets/tool-4bde39df.js';

  function asText(v) { return v == null ? '' : String(v); }

  // ---------------- 0. 存储兜底 ----------------
  // Ctool 的设置与历史都存在 localStorage（store2）。WebView2 里一般可用，
  // 但一旦不可用整个应用会在初始化阶段抛错白屏，所以先探一次，不行就换成内存实现。
  try {
    window.localStorage.setItem('__probe__', '1');
    window.localStorage.removeItem('__probe__');
  } catch (e) {
    console.warn('[dev-tools] localStorage 不可用，改用内存存储', e);
    var mem = {};
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      value: {
        get length() { return Object.keys(mem).length; },
        key: function (i) { return Object.keys(mem)[i] == null ? null : Object.keys(mem)[i]; },
        getItem: function (k) { return Object.prototype.hasOwnProperty.call(mem, k) ? mem[k] : null; },
        setItem: function (k, v) { mem[k] = asText(v); },
        removeItem: function (k) { delete mem[k]; },
        clear: function () { mem = {}; }
      }
    });
  }

  // ---------------- 1. 剪贴板 ----------------
  // UI 侧的 efto.* 全部走 IPC、返回 Promise，这里原样把 Promise 交出去。
  try {
    var clipboard = {
      writeText: function (t) {
        return bridge.copyText(asText(t)).then(function () { }, function () { });
      },
      readText: function () {
        return bridge.pasteText().then(function (v) { return asText(v); }, function () { return ''; });
      },
      // 宿主只做文本剪贴板，图片复制明确失败，不静默成功。
      write: function () {
        return Promise.reject(new Error('当前宿主不支持复制图片，请改用文本复制'));
      }
    };
    Object.defineProperty(navigator, 'clipboard', { value: clipboard, configurable: true });
  } catch (e) {
    console.warn('[dev-tools] 剪贴板替换失败', e);
  }

  if (typeof window.ClipboardItem !== 'function') {
    window.ClipboardItem = function ClipboardItem(parts) { this.parts = parts; };
  }

  // Ctool 用 navigator.permissions.query({name:'clipboard-read'}) 探测读取权限，探不到就当作拒绝。
  // 宿主侧读写都可用，这里对剪贴板权限直接回 granted，其余权限透传。
  try {
    if (navigator.permissions) {
      var nativeQuery = navigator.permissions.query;
      navigator.permissions.query = function (desc) {
        var name = desc && desc.name ? String(desc.name) : '';
        if (name.indexOf('clipboard') === 0) {
          return Promise.resolve({ state: 'granted', name: name, onchange: null });
        }
        return nativeQuery
          ? nativeQuery.call(navigator.permissions, desc)
          : Promise.resolve({ state: 'prompt', name: name, onchange: null });
      };
    }
  } catch (e) {
    console.warn('[dev-tools] permissions 替换失败', e);
  }

  // Ctool 的 web 运行时会用 window.open 打开外链，WebView2 里会弹一个空窗口；
  // 改成交给系统浏览器。
  var nativeOpen = window.open;
  window.open = function (url) {
    if (url && bridge.openExternal) { bridge.openExternal(asText(url)); return null; }
    return nativeOpen ? nativeOpen.apply(window, arguments) : null;
  };

  // ---------------- 2. 路由 ----------------
  var mounted = false;   // 主包是否已经注入
  var applied = '';      // 已经生效的 hash

  function buildHash(route) {
    var hash = (route && route.route) || '/';
    if (hash.charAt(0) !== '/') hash = '/' + hash;
    var input = route && route.input ? String(route.input) : '';
    if (input) {
      hash += (hash.indexOf('?') >= 0 ? '&' : '?') + 'input=' + encodeURIComponent(input);
    }
    return hash;
  }

  function fatal(text) {
    var box = document.getElementById('app');
    if (box) {
      box.innerHTML = '<p style="font:14px/1.7 system-ui,sans-serif;color:#b42318;padding:28px">'
        + text + '</p>';
    }
  }

  function mount(hash) {
    if (mounted) return;
    mounted = true;
    applied = hash;
    // 必须在主包 import 之前改 hash：Vue Router 在模块求值阶段就读取 location。
    try { history.replaceState(null, '', '#' + hash); }
    catch (e) { window.location.hash = hash; }

    var s = document.createElement('script');
    s.type = 'module';
    s.src = APP_ASSET;
    s.onerror = function () {
      fatal('Ctool 主应用包加载失败：请确认插件目录下的 assets/ 完整。');
    };
    document.head.appendChild(s);
  }

  function onRoute(route) {
    var hash = buildHash(route);
    if (!mounted) { mount(hash); return; }
    if (hash === applied) return;
    // 宿主复用同一个 UI 窗口，页面不会自己重载；reload 之后由 pull 拿到最新路由。
    try { history.replaceState(null, '', '#' + hash); } catch (e) { /* 忽略 */ }
    window.location.reload();
  }

  // ---------------- 3. 与 main 握手 ----------------
  try {
    bridge.onMessage('route', onRoute);
    // 主动拉一次：reload 之后宿主的补发队列已空，只能靠这次握手要到最新路由。
    bridge.ipcMessage('pull', {});
  } catch (e) {
    console.warn('[dev-tools] 与宿主通信失败，按默认路由启动', e);
  }

  // 宿主没应答也不能白屏：超时按首页启动。
  setTimeout(function () { if (!mounted) mount('/'); }, 1500);
})();
