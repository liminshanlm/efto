// 单张便签：加载标题与文本、去抖自动保存、标题改名、折叠/展开、关闭即删除。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var id = window.__EFTO_NOTE;
  var text = document.getElementById('text');
  var title = document.getElementById('title');
  var bar = document.getElementById('bar');
  var fold = document.getElementById('fold');
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

  function applyCollapsed(collapsed) {
    document.body.classList.toggle('collapsed', collapsed);
    fold.textContent = collapsed ? '▸' : '▾';
    fold.title = collapsed ? '展开内容' : '收起内容';
  }

  invoke('note_load', { id: id })
    .then(function (saved) {
      title.value = (saved && saved.title) || '';
      text.value = (saved && saved.text) || '';
      applyCollapsed(!!(saved && saved.collapsed));
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
    invoke('note_save', { id: id, title: title.value, text: text.value }).catch(function () {});
  }
  function schedule() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(save, 300);
  }

  text.addEventListener('input', schedule);
  text.addEventListener('blur', save);
  title.addEventListener('input', schedule);
  title.addEventListener('blur', save);
  window.addEventListener('beforeunload', save);

  bar.addEventListener('mousedown', function (e) {
    // 标题与按钮自己处理，别把点击当拖动
    if (e.button !== 0 || e.target.closest('button, input')) return;
    invoke('note_drag', { id: id }).catch(function () {});
  });

  fold.addEventListener('click', function (e) {
    e.stopPropagation();
    var collapsed = !document.body.classList.contains('collapsed');
    // 先落一次盘，把展开高度交出去，折叠后的窗口不影响它
    save();
    applyCollapsed(collapsed);
    invoke('note_toggle', { id: id, collapsed: collapsed }).catch(function () {});
  });

  del.addEventListener('click', function (e) {
    e.preventDefault();
    e.stopPropagation();
    invoke('note_close', { id: id }).catch(function () {});
  });
})();
