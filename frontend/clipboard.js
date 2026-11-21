// 剪贴板历史窗口。列表来自 ~/.efto/clipboard.json（后端轮询写入）。
// 支持文本、图片（缩略图）、文件列表，以及置顶与粘贴到前台窗口。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var q = document.getElementById('q');
  var list = document.getElementById('list');
  var empty = document.getElementById('empty');
  var count = document.getElementById('count');
  var statusEl = document.getElementById('status');

  var items = [];
  var active = 0;
  var timer = null;

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  function listen(name, handler) {
    if (!I || !I.transformCallback) return;
    var cb = I.transformCallback(function (ev) {
      var p = ev && ev.payload;
      if (typeof p === 'string') { try { p = JSON.parse(p); } catch (e) { /* 原样 */ } }
      handler(p);
    }, false);
    // 同 app.js：Tauri 2 的 EventTarget 必填，否则订阅静默失败
    invoke('plugin:event|listen', {
      event: name,
      target: { kind: 'Any' },
      handler: cb
    });
  }

  function esc(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function baseName(path) {
    var parts = String(path).split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function timeLabel(ts) {
    if (!ts) return '';
    var d = new Date(ts * 1000);
    var now = new Date();
    var hm = String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
    if (d.toDateString() === now.toDateString()) return hm;
    return (d.getMonth() + 1) + '-' + d.getDate() + ' ' + hm;
  }

  function bodyHtml(item) {
    if (item.kind === 'image') {
      return '<span class="thumb placeholder"></span>' +
        '<span class="text">图片 ' + esc(item.text) + '</span>';
    }
    if (item.kind === 'files') {
      var files = item.files || [];
      var shown = files.slice(0, 3).map(function (p) {
        return '<span class="file">' + esc(baseName(p)) + '</span>';
      }).join('');
      var more = files.length > 3 ? ' +' + (files.length - 3) : '';
      return '<span class="text files">' + shown +
        '<span class="file-count">' + esc(more || '') + ' · ' + files.length + ' 个文件</span>' +
        '<span class="paths">' + esc(files.join('\n')) + '</span></span>';
    }
    return '<span class="text">' + esc(item.text) + '</span>';
  }

  function render() {
    list.innerHTML = '';
    empty.hidden = items.length > 0;
    count.textContent = items.length ? items.length + ' 条' : '空';
    items.forEach(function (item, i) {
      var li = document.createElement('li');
      li.className = 'item' + (i === active ? ' active' : '') + (item.pinned ? ' pinned' : '');
      li.dataset.index = String(i);
      li.innerHTML =
        bodyHtml(item) +
        '<span class="meta">' + esc(timeLabel(item.ts)) + '</span>' +
        '<button class="pin' + (item.pinned ? ' on' : '') + '" title="置顶 / 取消置顶">' +
        (item.pinned ? '★' : '☆') + '</button>' +
        '<button class="del" title="删除这条">×</button>';

      if (item.kind === 'image') {
        (function (row, id) {
          invoke('clipboard_thumb', { id: id }).then(function (url) {
            if (!url) return;
            var img = document.createElement('img');
            img.className = 'thumb';
            img.alt = '';
            img.src = url;
            var ph = row.querySelector('.thumb');
            if (ph) row.replaceChild(img, ph);
          }).catch(function () { /* 缩略图失败就留占位 */ });
        })(li, item.id);
      }

      li.addEventListener('click', function (e) {
        if (e.target.classList.contains('del') || e.target.classList.contains('pin')) return;
        active = i;
        copy(item.id);
      });
      li.querySelector('.pin').addEventListener('click', function (e) {
        e.stopPropagation();
        pin(item.id);
      });
      li.querySelector('.del').addEventListener('click', function (e) {
        e.stopPropagation();
        remove(item.id);
      });
      list.appendChild(li);
    });
  }

  function updateActive(scroll) {
    var rows = list.querySelectorAll('.item');
    for (var i = 0; i < rows.length; i++) {
      rows[i].classList.toggle('active', Number(rows[i].dataset.index) === active);
    }
    if (scroll !== false) {
      var cur = list.querySelector('.item[data-index="' + active + '"]');
      if (cur && cur.scrollIntoView) cur.scrollIntoView({ block: 'nearest' });
    }
  }

  function load() {
    invoke('clipboard_list', { query: q.value })
      .then(function (res) {
        items = res || [];
        if (active >= items.length) active = 0;
        render();
        statusEl.textContent = items.length ? '单击或回车复制' : '就绪';
      })
      .catch(function (e) {
        statusEl.textContent = '读取失败：' + e;
      });
  }

  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(load, 60);
  }

  function copy(id) {
    invoke('clipboard_copy', { id: id })
      .then(function (msg) {
        statusEl.textContent = msg || '已复制';
      })
      .catch(function (e) {
        statusEl.textContent = '复制失败：' + e;
      });
  }

  function paste(id) {
    invoke('clipboard_paste', { id: id })
      .then(function (msg) {
        statusEl.textContent = msg || '已粘贴';
      })
      .catch(function (e) {
        statusEl.textContent = '粘贴失败：' + e;
      });
  }

  function pin(id) {
    invoke('clipboard_pin', { id: id }).then(function (pinned) {
      if (pinned == null) return;
      load();
    });
  }

  function remove(id) {
    invoke('clipboard_remove', { id: id }).then(function () {
      load();
    });
  }

  q.addEventListener('input', function () { active = 0; schedule(); });

  document.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (items.length) { active = (active + 1) % items.length; updateActive(); }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (items.length) { active = (active - 1 + items.length) % items.length; updateActive(); }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (!items[active]) return;
      if (e.ctrlKey) paste(items[active].id);
      else copy(items[active].id);
    } else if (e.key === 'Delete') {
      e.preventDefault();
      if (items[active]) remove(items[active].id);
    } else if (e.key === 'p' || e.key === 'P') {
      if (document.activeElement === q) return;
      e.preventDefault();
      if (items[active]) pin(items[active].id);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      invoke('hide_clipboard').catch(function () {});
    }
  });

  document.getElementById('clear').addEventListener('click', function () {
    invoke('clipboard_clear').then(function () {
      active = 0;
      load();
    });
  });

  listen('clipboard:show', function () {
    q.focus();
    q.select();
    load();
  });

  window.addEventListener('focus', function () {
    q.focus();
    load();
  });

  load();
})();
