// 单张便签：加载文本、去抖自动保存、标题栏拖动、关闭即删除。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var id = window.__EFTO_NOTE;
  var text = document.getElementById('text');
  var bar = document.getElementById('bar');
  var del = document.getElementById('del');

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  if (!id) {
    text.value = '便签参数缺失，请重新打开。';
    text.disabled = true;
    return;
  }

  invoke('note_load', { id: id })
    .then(function (saved) {
      text.value = saved || '';
      if (window.__EFTO_NOTE_FOCUS) text.focus();
    })
    .catch(function () {
      text.value = '';
    });

  var timer = null;
  function save() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    invoke('note_save', { id: id, text: text.value }).catch(function () {});
  }

  text.addEventListener('input', function () {
    if (timer) clearTimeout(timer);
    timer = setTimeout(save, 300);
  });
  text.addEventListener('blur', save);
  window.addEventListener('beforeunload', save);

  bar.addEventListener('mousedown', function (e) {
    if (e.button !== 0 || e.target === del) return;
    invoke('note_drag', { id: id }).catch(function () {});
  });

  del.addEventListener('click', function (e) {
    e.preventDefault();
    e.stopPropagation();
    invoke('note_close', { id: id }).catch(function () {});
  });
})();
