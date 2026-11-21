// 通用动作窗口：展示抓取到的选中文本，并执行复制 / 粘贴 / 搜索 / 打开。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var textEl = document.getElementById('text');
  var emptyEl = document.getElementById('empty');
  var listEl = document.getElementById('list');
  var statusEl = document.getElementById('status');

  var META = {
    copy: { title: 'actions.copy', hint: 'actions.copyHint' },
    paste: { title: 'actions.paste', hint: 'actions.pasteHint' },
    web: { title: 'actions.web', hint: 'actions.webHint' },
    open: { title: 'actions.open', hint: 'actions.openHint' }
  };
  var ids = [];

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

  function render(data) {
    ids = (data && data.ids) || [];
    var text = (data && data.text) || '';
    textEl.textContent = text;
    emptyEl.hidden = !!text.trim();
    textEl.hidden = !text.trim();
    listEl.innerHTML = '';
    ids.forEach(function (id, i) {
      var meta = META[id];
      if (!meta) return;
      var li = document.createElement('li');
      li.className = 'action' + (i === 0 ? ' first' : '');
      li.innerHTML =
        '<span class="title">' + t(meta.title) + '</span>' +
        '<span class="hint">' + t(meta.hint) + '</span>';
      li.addEventListener('click', function () { run(id); });
      listEl.appendChild(li);
    });
    statusEl.textContent = '';
  }

  function load() {
    invoke('actions_data')
      .then(render)
      .catch(function (e) {
        statusEl.textContent = String(e);
      });
  }

  function run(id) {
    invoke('actions_run', { action: id })
      .then(function (msg) {
        statusEl.textContent = msg || '';
        if (id === 'copy') {
          setTimeout(function () {
            invoke('hide_actions').catch(function () {});
          }, 450);
        } else if (id !== 'paste') {
          invoke('hide_actions').catch(function () {});
        }
      })
      .catch(function (e) {
        statusEl.textContent = String(e);
      });
  }

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      invoke('hide_actions').catch(function () {});
    } else if (e.key === 'Enter' && !e.ctrlKey) {
      e.preventDefault();
      if (ids.length) run(ids[0]);
    }
  });

  document.getElementById('close').addEventListener('click', function () {
    invoke('hide_actions').catch(function () {});
  });

  listen('actions:show', load);
  window.addEventListener('focus', load);
  load();
})();
