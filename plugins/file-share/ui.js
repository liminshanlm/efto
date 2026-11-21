// file-share/ui.js —— 插件 UI（普通 webview，无 Node 权限）。
// 状态全由 main 推送；这里只发意图并渲染。

(function () {
  'use strict';

  var statusEl = document.getElementById('status');
  var statusTextEl = document.getElementById('statusText');
  var statusHintEl = document.getElementById('statusHint');
  var toggleEl = document.getElementById('toggle');
  var linkRowEl = document.getElementById('linkRow');
  var shareUrlEl = document.getElementById('shareUrl');
  var copyUrlEl = document.getElementById('copyUrl');
  var toggleV6El = document.getElementById('toggleV6');
  var v6LabelEl = document.getElementById('v6Label');
  var clearEl = document.getElementById('clear');
  var dropEl = document.getElementById('drop');
  var itemsEl = document.getElementById('items');
  var itemCountEl = document.getElementById('itemCount');
  var emptyHintEl = document.getElementById('emptyHint');
  var errorEl = document.getElementById('error');
  var toastEl = document.getElementById('toast');

  var state = { running: false, port: 5421, urls: [], urls6: [], items: [], invalid: [], error: '' };
  var preferV6 = false;
  var toastTimer = 0;

  var ICONS = {
    link: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>',
    folder: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="m6 6 1 14h10l1-14"/></svg>'
  };

  function send(name, data) {
    efto.ipcMessage(name, data || {}).catch(function (e) {
      toast('发送失败：' + e);
    });
  }

  function toast(text) {
    toastEl.textContent = text;
    toastEl.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { toastEl.hidden = true; }, 1500);
  }

  function copy(text, okText) {
    efto.copyText(text).then(function () {
      toast(okText || '已复制');
    }).catch(function (e) {
      toast('复制失败：' + e);
    });
  }

  function usingV6() {
    return preferV6 && !!(state.urls6 && state.urls6.length);
  }

  function baseUrl() {
    var urls = usingV6() ? state.urls6 : (state.urls || []);
    var url = urls.length ? String(urls[0]) : '';
    return url.replace(/\/+$/, '');
  }

  function itemUrl(item, index) {
    var base = baseUrl();
    if (!base) return '';
    return base + (item.is_dir ? '/s/' + index + '/' : '/f/' + index);
  }

  function hostOf(url) {
    if (!url) return '';
    var a = document.createElement('a');
    a.href = url + '/';
    return String(a.hostname || '').replace(/^\[|\]$/g, '');
  }

  function baseName(path) {
    var parts = String(path || '').split(/[\\/]/);
    return parts[parts.length - 1] || String(path || '');
  }

  function itemName(item) {
    return item.name || baseName(item.path);
  }

  function iconButton(icon, title, extraClass, onClick) {
    var b = document.createElement('button');
    b.type = 'button';
    b.className = 'icon-btn' + (extraClass ? ' ' + extraClass : '');
    b.title = title;
    b.setAttribute('aria-label', title);
    b.innerHTML = icon;
    b.addEventListener('click', onClick);
    return b;
  }

  function itemRow(item, index, badReason) {
    var li = document.createElement('li');
    if (badReason) li.className = 'bad';

    var ip = document.createElement('span');
    ip.className = 'item-ip';
    ip.textContent = hostOf(baseUrl());
    li.appendChild(ip);

    var name = document.createElement('span');
    name.className = 'item-name';
    name.textContent = itemName(item);
    name.title = badReason ? badReason : (item.path || '');
    li.appendChild(name);

    var actions = document.createElement('span');
    actions.className = 'row-actions';

    // 文件夹在网页端本身就是目录入口，没有单独的"分享链接"可复制
    if (badReason) {
      actions.appendChild(iconButton(ICONS.trash, '移除', 'danger', function () {
        send('remove', { path: item.path });
      }));
    } else {
      if (!item.is_dir && baseUrl()) {
        var url = itemUrl(item, index);
        actions.appendChild(iconButton(ICONS.link, '复制链接', '', function () {
          if (!url) return toast('还没有分享链接');
          copy(url, '链接已复制');
        }));
      }
      actions.appendChild(iconButton(ICONS.folder, item.is_dir ? '打开文件夹' : '在文件夹中显示', '', function () {
        var p = item.is_dir ? efto.shellOpenPath(item.path) : efto.shellShowItemInFolder(item.path);
        p.catch(function (e) { toast('打开失败：' + e); });
      }));
      actions.appendChild(iconButton(ICONS.trash, '移除', 'danger', function () {
        send('remove', { path: item.path });
      }));
    }

    li.appendChild(actions);
    return li;
  }

  function render() {
    var base = baseUrl();

    statusTextEl.textContent = state.running ? '正在分享' : '未在分享';
    statusEl.dataset.state = state.running ? 'on' : 'off';
    statusHintEl.textContent = state.running
      ? '同网段设备可在浏览器中打开下方地址。'
      : '添加文件后，同网段设备即可在浏览器中访问。';
    toggleEl.textContent = state.running ? '取消分享' : '开始分享';
    toggleEl.className = state.running ? 'btn danger' : 'btn';

    linkRowEl.hidden = !base;
    shareUrlEl.textContent = base;
    shareUrlEl.title = base;
    copyUrlEl.disabled = !base;

    var hasV6 = !!(state.urls6 && state.urls6.length);
    toggleV6El.disabled = !hasV6;
    toggleV6El.title = hasV6 ? '' : '当前网络没有可用的 IPv6 地址';
    v6LabelEl.textContent = usingV6() ? '切换ipv4' : '切换ipv6';

    errorEl.textContent = state.error || '';
    errorEl.hidden = !state.error;

    itemsEl.innerHTML = '';
    var items = state.items || [];
    for (var i = 0; i < items.length; i++) itemsEl.appendChild(itemRow(items[i], i, ''));
    var invalid = state.invalid || [];
    for (var j = 0; j < invalid.length; j++) {
      itemsEl.appendChild(itemRow({ path: invalid[j].path }, j, invalid[j].reason || '路径不可用'));
    }

    var total = items.length + invalid.length;
    itemCountEl.textContent = String(total);
    emptyHintEl.hidden = total > 0;
  }

  function pickFiles() {
    if (typeof efto.pickFiles !== 'function') {
      return toast('当前 efto 未加载文件选择能力，请重启应用');
    }
    efto.pickFiles().then(function (paths) {
      if (paths && paths.length) send('add', { paths: paths });
    }).catch(function (e) {
      toast('选择文件失败：' + e);
    });
  }

  toggleEl.addEventListener('click', function () {
    if (state.running) {
      send('stop');
      return;
    }
    if (!(state.items && state.items.length)) {
      toast('请先添加要分享的文件');
      return;
    }
    send('start', { port: Number(state.port) || 5421 });
  });

  copyUrlEl.addEventListener('click', function () {
    var base = baseUrl();
    if (base) copy(base, '链接已复制');
  });

  toggleV6El.addEventListener('click', function () {
    if (!(state.urls6 && state.urls6.length)) return;
    preferV6 = !preferV6;
    render();
  });

  clearEl.addEventListener('click', function () {
    if (!window.confirm('清空全部共享项并停止分享？')) return;
    send('clear');
  });

  dropEl.addEventListener('click', pickFiles);
  dropEl.addEventListener('keydown', function (ev) {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      pickFiles();
    }
  });

  if (typeof efto.onFileDrag === 'function') {
    efto.onFileDrag(function (inside) {
      dropEl.classList.toggle('over', !!inside);
    });
  }
  if (typeof efto.onFileDrop === 'function') {
    efto.onFileDrop(function (paths) {
      dropEl.classList.remove('over');
      if (paths && paths.length) send('add', { paths: paths });
    });
  }

  efto.onMessage('state', function (next) {
    state = next || state;
    render();
  });
  send('init');
})();
