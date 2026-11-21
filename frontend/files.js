// 文件缓冲窗口：批量复制 / 移动 / 删除在搜索结果里攒下的文件。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var listEl = document.getElementById('list');
  var emptyEl = document.getElementById('empty');
  var countEl = document.getElementById('count');
  var statusEl = document.getElementById('status');

  var items = [];
  var active = 0;

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  function t(key) {
    if (window.eftoI18n && typeof window.eftoI18n.t === 'function') {
      return window.eftoI18n.t(key);
    }
    return key;
  }

  function listen(name, handler) {
    if (!I || !I.transformCallback) return;
    var cb = I.transformCallback(function (ev) {
      var p = ev && ev.payload;
      if (typeof p === 'string') { try { p = JSON.parse(p); } catch (e) { /* 原样 */ } }
      handler(p);
    }, false);
    invoke('plugin:event|listen', {
      event: name,
      target: { kind: 'Any' },
      handler: cb
    });
  }

  function baseName(path) {
    var parts = String(path).split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function dirName(path) {
    var s = String(path);
    var i = Math.max(s.lastIndexOf('\\'), s.lastIndexOf('/'));
    return i > 0 ? s.slice(0, i) : s;
  }

  function render() {
    listEl.innerHTML = '';
    countEl.textContent = String(items.length);
    emptyEl.hidden = items.length > 0;
    items.forEach(function (path, i) {
      var li = document.createElement('li');
      li.className = 'item' + (i === active ? ' active' : '');
      li.dataset.index = String(i);
      li.innerHTML =
        '<span class="name">' + esc(baseName(path)) + '</span>' +
        '<span class="dir">' + esc(dirName(path)) + '</span>' +
        '<button class="del" title="移除">×</button>';
      li.addEventListener('click', function (e) {
        if (e.target.classList.contains('del')) return;
        active = i;
        updateActive();
      });
      li.querySelector('.del').addEventListener('click', function (e) {
        e.stopPropagation();
        invoke('file_buffer_remove', { path: path }).then(load);
      });
      listEl.appendChild(li);
    });
  }

  function updateActive() {
    var rows = listEl.querySelectorAll('.item');
    for (var i = 0; i < rows.length; i++) {
      rows[i].classList.toggle('active', Number(rows[i].dataset.index) === active);
    }
  }

  function esc(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function load() {
    invoke('file_buffer_list')
      .then(function (res) {
        items = res || [];
        if (active >= items.length) active = Math.max(0, items.length - 1);
        render();
      })
      .catch(function (e) {
        statusEl.textContent = String(e);
      });
  }

  function bufferAction(action) {
    invoke('file_buffer_action', { action: action })
      .then(function (msg) {
        statusEl.textContent = msg || '';
        load();
      })
      .catch(function (e) {
        statusEl.textContent = String(e);
      });
  }

  function transfer(moveit) {
    if (!items.length) return;
    invoke('file_transfer', { paths: items, moveit: !!moveit })
      .then(function (msg) {
        statusEl.textContent = msg || '';
        load();
      })
      .catch(function (e) {
        if (String(e).indexOf('已取消') === -1) statusEl.textContent = String(e);
      });
  }

  document.getElementById('copy').addEventListener('click', function () { bufferAction('copy'); });
  document.getElementById('delete').addEventListener('click', function () { bufferAction('delete'); });
  document.getElementById('copyTo').addEventListener('click', function () { transfer(false); });
  document.getElementById('moveTo').addEventListener('click', function () { transfer(true); });
  document.getElementById('clear').addEventListener('click', function () {
    invoke('file_buffer_clear').then(function () {
      statusEl.textContent = t('files.cleared');
      active = 0;
      load();
    });
  });

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      invoke('hide_files').catch(function () {});
    } else if (e.key === 'Delete' && items[active]) {
      e.preventDefault();
      invoke('file_buffer_remove', { path: items[active] }).then(load);
    } else if (e.key === 'ArrowDown' && items.length) {
      e.preventDefault();
      active = (active + 1) % items.length;
      updateActive();
    } else if (e.key === 'ArrowUp' && items.length) {
      e.preventDefault();
      active = (active - 1 + items.length) % items.length;
      updateActive();
    }
  });

  listen('files:show', load);
  window.addEventListener('focus', load);
  load();
})();
