// markdown/ui.js —— 插件 UI（普通 webview，无 Node 权限）。
// 渲染在本地完成（render.js 是纯函数），文件读写只走宿主受限 API。

(function () {
  'use strict';

  var MD = (typeof MarkdownRender !== 'undefined' && MarkdownRender) || null;
  var host = (typeof efto !== 'undefined' && efto) ? efto : null;

  var docNameEl = document.getElementById('docName');
  var docDirtyEl = document.getElementById('docDirty');
  var modesEl = document.getElementById('modes');
  var toolbarEl = document.getElementById('toolbar');
  var editorEl = document.getElementById('editor');
  var previewWrapEl = document.getElementById('previewWrap');
  var previewEl = document.getElementById('preview');
  var emptyEl = document.getElementById('empty');
  var dropMaskEl = document.getElementById('dropMask');
  var statusEl = document.getElementById('status');
  var metaEl = document.getElementById('meta');
  var pathInfoEl = document.getElementById('pathInfo');
  var toastEl = document.getElementById('toast');
  var openBtn = document.getElementById('openBtn');
  var saveBtn = document.getElementById('saveBtn');
  var exportBtn = document.getElementById('exportBtn');
  var printBtn = document.getElementById('printBtn');
  var emptyOpen = document.getElementById('emptyOpen');
  var emptyNew = document.getElementById('emptyNew');

  var state = {
    mode: 'preview',
    path: '',
    name: '未命名.md',
    text: '',
    dirty: false
  };

  // ---------- 小工具 ----------

  function toast(text, bad) {
    toastEl.textContent = text;
    toastEl.className = 'toast' + (bad ? ' bad' : '');
    toastEl.hidden = false;
    clearTimeout(toast._t);
    toast._t = setTimeout(function () { toastEl.hidden = true; }, 2200);
  }

  function setStatus(text, kind) {
    statusEl.textContent = text;
    statusEl.className = 'status' + (kind ? ' ' + kind : '');
  }

  function errText(e) {
    if (e && e.message) return e.message;
    return String(e == null ? '未知错误' : e);
  }

  function baseName(p) {
    var s = String(p || '');
    var i = Math.max(s.lastIndexOf('/'), s.lastIndexOf('\\'));
    return i >= 0 ? s.slice(i + 1) : s;
  }

  function updateMeta() {
    var t = state.text;
    if (!t) {
      metaEl.textContent = '空';
      return;
    }
    var lines = t.split(/\r\n|\r|\n/).length;
    metaEl.textContent = t.length + ' 字符 · ' + lines + ' 行';
  }

  function updatePath() {
    pathInfoEl.textContent = state.path || '未保存';
    pathInfoEl.title = state.path || '';
  }

  function refreshTitle() {
    docNameEl.textContent = state.name;
    docNameEl.title = state.name;
    docDirtyEl.hidden = !state.dirty;
    document.title = state.name + (state.dirty ? ' *' : '');
  }

  function setDirty(v) {
    state.dirty = !!v;
    refreshTitle();
  }

  // ---------- 渲染与模式 ----------

  function renderPreview() {
    if (!MD) {
      setStatus('渲染器未加载', 'bad');
      toast('render.js 缺失或语法错误，无法渲染', true);
      return;
    }
    previewEl.innerHTML = state.text.trim() ? MD.renderMarkdown(state.text) : '';
  }

  function updateEmpty() {
    emptyEl.hidden = !(state.mode === 'preview' && !state.text.trim());
  }

  function switchMode(next) {
    if (next !== 'preview' && next !== 'source') return;
    state.mode = next;

    var items = modesEl.querySelectorAll('.seg-item');
    for (var i = 0; i < items.length; i++) {
      items[i].className =
        'seg-item' + (items[i].getAttribute('data-mode') === next ? ' on' : '');
    }

    var source = next === 'source';
    toolbarEl.hidden = !source;
    editorEl.hidden = !source;
    previewWrapEl.hidden = source;

    if (source) {
      editorEl.value = state.text;
      editorEl.focus();
    } else {
      renderPreview();
    }
    updateEmpty();
  }

  function applyDoc(path, content, dirty) {
    state.path = path || '';
    state.name = state.path ? baseName(state.path) : '未命名.md';
    state.text = content || '';
    editorEl.value = state.text;
    setDirty(!!dirty);
    updatePath();
    updateMeta();
    updateEmpty();
    if (state.mode === 'preview') renderPreview();
  }

  function confirmDiscard() {
    if (!state.dirty) return true;
    return window.confirm('当前文档有未保存的修改，继续会丢弃它们。');
  }

  // ---------- 打开与保存 ----------

  function loadPath(p) {
    if (!host || !host.file || !host.file.read) {
      toast('当前环境不支持读取文件', true);
      return;
    }
    host.file.read(p)
      .then(function (res) {
        applyDoc(res.path, res.content, false);
        setStatus('已打开 ' + baseName(res.path), 'ok');
      })
      .catch(function (e) {
        setStatus('读取失败', 'bad');
        toast('读取失败：' + errText(e), true);
      });
  }

  function openFile() {
    if (!confirmDiscard()) return;
    if (!host || !host.pickFiles) {
      toast('当前环境不支持选择文件', true);
      return;
    }
    host.pickFiles()
      .then(function (paths) {
        if (!paths || !paths.length) return;
        loadPath(paths[0]);
      })
      .catch(function (e) {
        toast('选择文件失败：' + errText(e), true);
      });
  }

  function newDoc() {
    if (!confirmDiscard()) return;
    applyDoc('', '', false);
    switchMode('source');
    setStatus('新建空白文档');
  }

  function save() {
    if (!host || !host.file || !host.file.write) {
      toast('当前环境不支持保存文件', true);
      return;
    }
    if (!state.path) {
      saveAs();
      return;
    }
    host.file.write(state.path, state.text)
      .then(function (res) {
        setDirty(false);
        setStatus('已保存 ' + baseName(res.path), 'ok');
        toast('已保存');
      })
      .catch(function (e) {
        setStatus('保存失败', 'bad');
        toast('保存失败：' + errText(e), true);
      });
  }

  function saveAs() {
    if (!host || !host.file || !host.file.saveAs) {
      toast('当前环境不支持保存文件', true);
      return;
    }
    host.file.saveAs(state.name, state.text)
      .then(function (p) {
        if (!p) {
          setStatus('已取消保存');
          return;
        }
        state.path = p;
        state.name = baseName(p);
        setDirty(false);
        updatePath();
        setStatus('已保存到 ' + p, 'ok');
        toast('已保存');
      })
      .catch(function (e) {
        setStatus('保存失败', 'bad');
        toast('保存失败：' + errText(e), true);
      });
  }

  // ---------- 导出与打印 ----------

  var EXPORT_CSS = [
    'body{margin:0;padding:48px 28px;background:#fff;color:#1a1e28;',
    'font:15px/1.85 -apple-system,"Segoe UI","Microsoft YaHei",sans-serif;}',
    'article{max-width:780px;margin:0 auto;word-wrap:break-word;}',
    'h1,h2,h3,h4,h5,h6{margin:1.6em 0 .6em;line-height:1.35;font-weight:650;}',
    'h1{font-size:1.85em;}h2{font-size:1.45em;}h3{font-size:1.22em;}',
    'h1,h2{padding-bottom:.3em;border-bottom:1px solid #e2e6ee;}',
    'p{margin:.85em 0;}a{color:#3b6fe0;text-decoration:none;}',
    'ul,ol{margin:.8em 0;padding-left:1.6em;}li{margin:.28em 0;}',
    'li.task{list-style:none;margin-left:-1.4em;}li.task input{margin-right:6px;}',
    'blockquote{margin:1em 0;padding:2px 16px;border-left:3px solid #b9c8ea;',
    'background:#f3f6fd;border-radius:0 8px 8px 0;color:#3f4657;}',
    'code{padding:2px 6px;border-radius:5px;background:#f2f4f8;',
    'font:.9em/1.6 "Cascadia Code","Consolas",monospace;}',
    'pre{margin:1em 0;padding:14px 18px;overflow-x:auto;border-radius:9px;',
    'background:#f2f4f8;border:1px solid #e2e6ee;}',
    'pre code{padding:0;background:transparent;font-size:12.5px;line-height:1.7;}',
    'table{width:100%;margin:1em 0;border-collapse:collapse;font-size:.94em;}',
    'th,td{padding:7px 12px;border:1px solid #d8dee9;}th{background:#f2f4f8;}',
    'hr{margin:1.8em 0;border:0;border-top:1px solid #d8dee9;}',
    'img{max-width:100%;border-radius:8px;}',
    'del{color:#8d95a8;}',
    '@media print{body{padding:0;}}'
  ].join('');

  function exportName() {
    var n = state.name || 'untitled.md';
    if (/\.(md|markdown|txt)$/i.test(n)) return n.replace(/\.(md|markdown|txt)$/i, '') + '.html';
    if (!/\.[a-z0-9]+$/i.test(n)) return n + '.html';
    return n;
  }

  function buildExportHtml() {
    var title = state.name.replace(/\.(md|markdown|txt|html?)$/i, '') || 'Markdown';
    var esc = MD && MD.escapeHtml ? MD.escapeHtml : function (s) { return String(s); };
    return '<!doctype html>\n<html lang="zh-CN">\n<head>\n<meta charset="utf-8">\n' +
      '<meta name="viewport" content="width=device-width, initial-scale=1">\n' +
      '<title>' + esc(title) + '</title>\n<style>\n' + EXPORT_CSS + '\n</style>\n</head>\n' +
      '<body>\n<article>\n' + MD.renderMarkdown(state.text) + '\n</article>\n</body>\n</html>\n';
  }

  function exportHtml() {
    if (!state.text.trim()) {
      toast('没有可导出的内容', true);
      return;
    }
    if (!MD) {
      toast('渲染器未加载，无法导出', true);
      return;
    }
    if (!host || !host.file || !host.file.saveAs) {
      toast('当前环境不支持导出文件', true);
      return;
    }
    host.file.saveAs(exportName(), buildExportHtml())
      .then(function (p) {
        if (!p) return;
        setStatus('已导出 ' + baseName(p), 'ok');
        toast('已导出 ' + baseName(p));
      })
      .catch(function (e) {
        setStatus('导出失败', 'bad');
        toast('导出失败：' + errText(e), true);
      });
  }

  function printDoc() {
    if (!state.text.trim()) {
      toast('没有可打印的内容', true);
      return;
    }
    switchMode('preview');
    setTimeout(function () {
      try {
        window.print();
      } catch (e) {
        toast('打印失败：' + errText(e) + '，可先导出 HTML 再打印', true);
      }
    }, 80);
  }

  // ---------- 插入工具栏 ----------

  function afterInsert() {
    state.text = editorEl.value;
    setDirty(true);
    updateMeta();
  }

  function surround(before, after, placeholder) {
    var s = editorEl.selectionStart;
    var e = editorEl.selectionEnd;
    var v = editorEl.value;
    var sel = v.slice(s, e);
    if (!sel) sel = placeholder || '';
    var text = before + sel + after;
    editorEl.value = v.slice(0, s) + text + v.slice(e);
    editorEl.selectionStart = s + before.length;
    editorEl.selectionEnd = s + before.length + sel.length;
    afterInsert();
    editorEl.focus();
  }

  function prefixLines(prefix) {
    var s = editorEl.selectionStart;
    var e = editorEl.selectionEnd;
    var v = editorEl.value;
    var start = v.lastIndexOf('\n', s - 1) + 1;
    var end = v.indexOf('\n', e);
    if (end < 0) end = v.length;
    var lines = v.slice(start, end).split('\n');
    var replaced = lines
      .map(function (line, i) {
        return (typeof prefix === 'function' ? prefix(i) : prefix) + line;
      })
      .join('\n');
    editorEl.value = v.slice(0, start) + replaced + v.slice(end);
    editorEl.selectionStart = start;
    editorEl.selectionEnd = start + replaced.length;
    afterInsert();
    editorEl.focus();
  }

  function insertBlock(block) {
    var s = editorEl.selectionStart;
    var e = editorEl.selectionEnd;
    var v = editorEl.value;
    var before = v.slice(0, s);
    var after = v.slice(e);
    var text = block;
    if (before && !/\n$/.test(before)) text = '\n' + text;
    if (after && !/^\n/.test(after)) text = text + '\n';
    editorEl.value = before + text + after;
    var caret = before.length + text.length;
    editorEl.selectionStart = caret;
    editorEl.selectionEnd = caret;
    afterInsert();
    editorEl.focus();
  }

  function runAction(act) {
    switch (act) {
      case 'h1': prefixLines('# '); break;
      case 'h2': prefixLines('## '); break;
      case 'h3': prefixLines('### '); break;
      case 'bold': surround('**', '**', '粗体'); break;
      case 'italic': surround('*', '*', '斜体'); break;
      case 'strike': surround('~~', '~~', '删除线'); break;
      case 'code': surround('`', '`', '代码'); break;
      case 'ul': prefixLines('- '); break;
      case 'ol': prefixLines(function (i) { return (i + 1) + '. '; }); break;
      case 'task': prefixLines('- [ ] '); break;
      case 'quote': prefixLines('> '); break;
      case 'link': surround('[', '](https://)', '链接文字'); break;
      case 'image': surround('![', '](https://)', '图片描述'); break;
      case 'table': insertBlock('| 列 1 | 列 2 |\n| --- | --- |\n| 内容 | 内容 |'); break;
      case 'codeblock': surround('```\n', '\n```', ''); break;
      case 'hr': insertBlock('---'); break;
    }
  }

  // ---------- 事件 ----------

  modesEl.addEventListener('click', function (e) {
    var t = e.target;
    if (!t || !t.getAttribute) return;
    var mode = t.getAttribute('data-mode');
    if (mode) switchMode(mode);
  });

  toolbarEl.addEventListener('click', function (e) {
    var btn = e.target && e.target.closest ? e.target.closest('button[data-act]') : null;
    if (!btn) return;
    runAction(btn.getAttribute('data-act'));
  });

  editorEl.addEventListener('input', function () {
    state.text = editorEl.value;
    setDirty(true);
    updateMeta();
  });

  editorEl.addEventListener('keydown', function (e) {
    if (e.ctrlKey && (e.key === 's' || e.key === 'S')) {
      e.preventDefault();
      save();
    }
  });

  openBtn.addEventListener('click', openFile);
  saveBtn.addEventListener('click', save);
  exportBtn.addEventListener('click', exportHtml);
  printBtn.addEventListener('click', printDoc);
  emptyOpen.addEventListener('click', openFile);
  emptyNew.addEventListener('click', newDoc);

  // 预览里的链接：外链交给系统浏览器，相对路径明确告知打不开
  previewEl.addEventListener('click', function (e) {
    var a = e.target && e.target.closest ? e.target.closest('a') : null;
    if (!a) return;
    var href = a.getAttribute('href') || '';
    if (!href || href.charAt(0) === '#') return;
    e.preventDefault();
    if (/^(https?:|mailto:)/i.test(href)) {
      if (host && host.openExternal) {
        host.openExternal(href).catch(function (err) {
          toast('打开链接失败：' + errText(err), true);
        });
      }
      return;
    }
    toast('相对链接无法直接打开：' + href, true);
  });

  document.addEventListener('keydown', function (e) {
    if (!(e.ctrlKey || e.metaKey)) return;
    var k = String(e.key || '').toLowerCase();
    if (k === 's') {
      e.preventDefault();
      if (e.shiftKey) saveAs();
      else save();
    } else if (k === 'o') {
      e.preventDefault();
      openFile();
    } else if (k === 'p') {
      e.preventDefault();
      printDoc();
    } else if (k === 'e') {
      e.preventDefault();
      switchMode(state.mode === 'preview' ? 'source' : 'preview');
    }
  });

  // 默认拖放会让 webview 直接导航到文件，这里统一吞掉，改用宿主事件
  document.addEventListener('dragover', function (e) { e.preventDefault(); });
  document.addEventListener('drop', function (e) { e.preventDefault(); });

  if (host && host.onFileDrag) {
    host.onFileDrag(function (inside) {
      dropMaskEl.hidden = !inside;
    });
  }

  if (host && host.onFileDrop) {
    host.onFileDrop(function (paths) {
      dropMaskEl.hidden = true;
      if (!paths || !paths.length) return;
      if (!confirmDiscard()) return;
      loadPath(paths[0]);
    });
  }

  if (host && host.onMessage) {
    host.onMessage('seed', function (data) {
      var text = data && typeof data.text === 'string' ? data.text : '';
      if (!text.trim() || state.text.trim()) return;
      applyDoc('', text, true);
      switchMode('source');
      setStatus('已载入输入内容');
    });
    // 资源管理器「打开方式」传来的文件
    host.onMessage('open-file', function (data) {
      var p = data && typeof data.path === 'string' ? data.path : '';
      if (!p) return;
      if (!confirmDiscard()) return;
      loadPath(p);
    });
  }

  // ---------- 初始化 ----------

  applyDoc('', '', false);
  switchMode('preview');

  if (host && host.ipcMessage) {
    host.ipcMessage('init', {}).catch(function () {
      // 握手失败不影响本地编辑，只是拿不到触发词后的初始内容
    });
  }
})();
