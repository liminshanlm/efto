// 钉图窗口：把一张 png 贴在屏幕上。
// 后端通过 initialization_script 注入 window.__EFTO_PIN（钉图 id）。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var id = window.__EFTO_PIN;
  var img = document.getElementById('shot');
  var hint = document.getElementById('hint');
  var zoom = 1;

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  function close() {
    invoke('pin_close', { id: id }).catch(function () {});
  }

  if (!id) {
    hint.textContent = '钉图参数缺失';
    return;
  }

  invoke('pin_load', { id: id })
    .then(function (url) {
      img.src = url;
      hint.style.display = 'none';
    })
    .catch(function (e) {
      hint.textContent = '读取钉图失败：' + e;
    });

  // 拖动窗口（整窗拖拽，和 Snipaste 的默认手感一致）
  document.addEventListener('mousedown', function (e) {
    if (e.button !== 0) return;
    invoke('pin_drag', { id: id }).catch(function () {});
  });

  document.addEventListener('dblclick', close);
  document.addEventListener('contextmenu', function (e) {
    e.preventDefault();
    close();
  });

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      close();
    } else if ((e.ctrlKey || e.metaKey) && (e.key === 'c' || e.key === 'C')) {
      e.preventDefault();
      invoke('pin_copy', { id: id }).catch(function () {});
    } else if (e.key === '+' || e.key === '=') {
      zoom = Math.min(4, zoom + 0.1);
      img.style.transform = 'scale(' + zoom + ')';
    } else if (e.key === '-') {
      zoom = Math.max(0.3, zoom - 0.1);
      img.style.transform = 'scale(' + zoom + ')';
    } else if (e.key === '0') {
      zoom = 1;
      img.style.transform = 'scale(1)';
    }
  });
})();
