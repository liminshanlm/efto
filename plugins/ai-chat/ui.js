/* AI 对话 UI：普通 webview，无 Node 权限；模型调用统一走 main。 */
(function () {
  'use strict';

  var STORE_KEY = 'ai-chat.v1';
  var IMAGE_EXT = /\.(png|jpe?g|webp|gif|bmp)$/i;
  var MAX_IMAGES = 4;
  var MAX_DOCS = 3;

  var el = {
    conversations: document.getElementById('conversations'),
    count: document.getElementById('count'),
    clearAll: document.getElementById('clear-all'),
    newChat: document.getElementById('new-chat'),
    title: document.getElementById('chat-title'),
    sub: document.getElementById('chat-sub'),
    model: document.getElementById('model'),
    messages: document.getElementById('messages'),
    empty: document.getElementById('empty'),
    attachments: document.getElementById('attachments'),
    docs: document.getElementById('docs'),
    hint: document.getElementById('hint'),
    input: document.getElementById('input'),
    pick: document.getElementById('pick'),
    pickFile: document.getElementById('pick-file'),
    web: document.getElementById('web'),
    send: document.getElementById('send'),
    modelState: document.getElementById('model-state')
  };

  var state = {
    conversations: [],
    currentId: '',
    models: [],
    defaultModel: '',
    model: '',
    attachments: [],
    docs: [],
    webSearch: false,
    streaming: null,
    previewSeq: 0,
    ocrSeq: 0,
    previewPaths: {}
  };

  var saveTimer = 0;

  // ---------------- 基础 ----------------

  function uid() {
    return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
  }

  function now() {
    return Date.now();
  }

  function findConv(id) {
    for (var i = 0; i < state.conversations.length; i++) {
      if (state.conversations[i].id === id) return state.conversations[i];
    }
    return null;
  }

  function currentConv() {
    return findConv(state.currentId);
  }

  function modelById(id) {
    for (var i = 0; i < state.models.length; i++) {
      if (state.models[i].id === id) return state.models[i];
    }
    return null;
  }

  function fmtTime(ts) {
    if (!ts) return '';
    var d = new Date(ts);
    var today = new Date();
    var sameDay = d.toDateString() === today.toDateString();
    var pad = function (n) { return n < 10 ? '0' + n : String(n); };
    return sameDay
      ? pad(d.getHours()) + ':' + pad(d.getMinutes())
      : (d.getMonth() + 1) + '-' + d.getDate();
  }

  function deriveTitle(conv) {
    for (var i = 0; i < conv.messages.length; i++) {
      var m = conv.messages[i];
      if (m.role !== 'user') continue;
      var t = (m.text || '').replace(/\s+/g, ' ').trim();
      if (t) return t.length > 28 ? t.slice(0, 28) + '…' : t;
      if (m.images && m.images.length) return '图片对话';
      if (m.docs && m.docs.length) return m.docs[0].name || '文档对话';
    }
    return '新对话';
  }

  function showHint(text) {
    el.hint.textContent = text || '';
    el.hint.hidden = !text;
  }

  function clearHint() {
    showHint('');
  }

  // ---------------- 持久化 ----------------

  // 图片的 base64 预览不进 kv：单插件配额只有 10MB，一张大图就能把整个历史写失败。
  // 预览在下次打开时按 path 重新生成，历史里只留文件路径。
  function persistable() {
    return state.conversations.map(function (conv) {
      return {
        id: conv.id,
        title: conv.title,
        createdAt: conv.createdAt,
        updatedAt: conv.updatedAt,
        messages: conv.messages.map(function (m) {
          if (m.role === 'user') {
            return {
              role: 'user',
              text: m.text,
              images: (m.images || []).map(function (img) { return { path: img.path }; }),
              docs: (m.docs || []).map(function (d) {
                return { name: d.name, path: d.path, pages: d.pages, text: String(d.text || '').slice(0, 60000) };
              }),
              ts: m.ts
            };
          }
          return {
            role: 'assistant',
            content: m.content,
            reasoning: m.reasoning,
            error: m.error,
            sources: m.sources || [],
            ts: m.ts
          };
        })
      };
    });
  }

  function saveSoon() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(function () {
      var payload = { conversations: persistable(), model: state.model, webSearch: state.webSearch };
      try {
        var p = efto.kv.set(STORE_KEY, payload);
        if (p && p.catch) p.catch(function (e) { showHint('保存失败：' + errorText(e)); });
      } catch (e) {
        showHint('保存失败：' + errorText(e));
      }
    }, 200);
  }

  function normalizeConv(raw) {
    if (!raw || typeof raw !== 'object') return null;
    var conv = {
      id: typeof raw.id === 'string' && raw.id ? raw.id : uid(),
      title: typeof raw.title === 'string' ? raw.title : '新对话',
      createdAt: Number(raw.createdAt) || now(),
      updatedAt: Number(raw.updatedAt) || now(),
      messages: []
    };
    var list = Array.isArray(raw.messages) ? raw.messages : [];
    for (var i = 0; i < list.length; i++) {
      var m = list[i];
      if (!m || (m.role !== 'user' && m.role !== 'assistant')) continue;
      if (m.role === 'user') {
        conv.messages.push({
          role: 'user',
          text: typeof m.text === 'string' ? m.text : '',
          images: (Array.isArray(m.images) ? m.images : [])
            .filter(function (img) { return img && typeof img.path === 'string'; })
            .map(function (img) { return { path: img.path }; }),
          docs: (Array.isArray(m.docs) ? m.docs : [])
            .filter(function (d) { return d && typeof d.text === 'string' && d.text; })
            .map(function (d) {
              return {
                name: typeof d.name === 'string' ? d.name : '文档',
                path: typeof d.path === 'string' ? d.path : '',
                pages: Number(d.pages) || 0,
                text: d.text
              };
            }),
          ts: Number(m.ts) || now()
        });
      } else {
        var content = typeof m.content === 'string' ? m.content : '';
        var reasoning = typeof m.reasoning === 'string' ? m.reasoning : '';
        var error = typeof m.error === 'string' ? m.error : '';
        if (!content && !reasoning && !error) continue;
        conv.messages.push({
          role: 'assistant',
          content: content,
          reasoning: reasoning,
          error: error,
          sources: Array.isArray(m.sources) ? m.sources.filter(function (s) {
            return s && typeof s.url === 'string';
          }) : [],
          pending: false,
          ts: Number(m.ts) || now()
        });
      }
    }
    return conv;
  }

  function loadSaved() {
    var raw = null;
    try {
      raw = efto.kv.get(STORE_KEY);
    } catch (e) {
      raw = null;
    }
    Promise.resolve(raw).then(function (data) {
      if (typeof data === 'string') {
        try { data = JSON.parse(data); } catch (e) { data = null; }
      }
      data = data && typeof data === 'object' ? data : {};
      var list = Array.isArray(data.conversations) ? data.conversations : [];
      state.conversations = list
        .map(normalizeConv)
        .filter(function (c) { return c && c.messages.length > 0; });
      if (typeof data.model === 'string') state.model = data.model;
      state.webSearch = data.webSearch === true;
      ensureConversation();
      renderAll();
      applyDefaultModel();
      syncWebButton();
      hydratePreviews();
      setTimeout(function () { el.input.focus(); }, 30);
    }).catch(function (e) {
      showHint('读取历史失败：' + errorText(e));
      ensureConversation();
      renderAll();
    });
  }

  function ensureConversation() {
    var conv = currentConv();
    if (conv) return conv;
    conv = {
      id: uid(),
      title: '新对话',
      createdAt: now(),
      updatedAt: now(),
      messages: []
    };
    state.conversations.unshift(conv);
    state.currentId = conv.id;
    return conv;
  }

  // 历史里的图片只有路径，打开时按 path 让 main 重新读盘生成预览。
  function hydratePreviews() {
    var seen = {};
    var missing = [];
    state.conversations.forEach(function (conv) {
      conv.messages.forEach(function (m) {
        (m.images || []).forEach(function (img) {
          if (img.preview || seen[img.path]) return;
          seen[img.path] = true;
          missing.push(img.path);
        });
      });
    });
    if (!missing.length) return;
    var id = 'hist-' + (++state.previewSeq);
    state.previewPaths[id] = missing;
    efto.ipcMessage('preview', { id: id, paths: missing });
  }

  // ---------------- 模型 ----------------

  function applyDefaultModel() {
    if (!state.models.length) return;
    var ids = state.models.map(function (m) { return m.id; });
    var pick = state.model;
    if (ids.indexOf(pick) < 0) {
      pick = state.defaultModel && ids.indexOf(state.defaultModel) >= 0
        ? state.defaultModel
        : ids[0];
    }
    state.model = pick;
    el.model.value = pick;
    renderHeader();
    updateSendState();
  }

  function renderModels(data) {
    data = data && typeof data === 'object' ? data : {};
    state.models = Array.isArray(data.models) ? data.models : [];
    state.defaultModel = typeof data.default === 'string' ? data.default : '';

    el.model.innerHTML = '';
    if (!state.models.length) {
      el.model.disabled = true;
      var opt = document.createElement('option');
      opt.value = '';
      opt.textContent = '未配置模型';
      el.model.appendChild(opt);
      el.modelState.textContent = '请先在 设置 → 模型 中配置并设为默认';
      el.modelState.title = '';
      updateSendState();
      renderHeader();
      return;
    }

    el.model.disabled = false;
    state.models.forEach(function (m) {
      var o = document.createElement('option');
      o.value = m.id;
      o.textContent = m.name || m.model || m.id;
      o.title = m.model || '';
      el.model.appendChild(o);
    });
    el.modelState.textContent = '';
    applyDefaultModel();
  }

  // ---------------- 侧栏 ----------------

  function renderConversations() {
    el.conversations.innerHTML = '';
    var frag = document.createDocumentFragment();
    state.conversations.forEach(function (conv) {
      var row = document.createElement('div');
      row.className = 'conv' + (conv.id === state.currentId ? ' on' : '');
      row.dataset.id = conv.id;

      var title = document.createElement('div');
      title.className = 'conv-title';
      title.textContent = conv.title || deriveTitle(conv);
      title.title = title.textContent;

      var meta = document.createElement('div');
      meta.className = 'conv-meta';
      var n = conv.messages.length;
      meta.textContent = fmtTime(conv.updatedAt) + (n ? ' · ' + n + ' 条' : '');

      var del = document.createElement('button');
      del.className = 'conv-del';
      del.type = 'button';
      del.title = '删除会话';
      del.setAttribute('aria-label', '删除会话 ' + (conv.title || '新对话'));
      del.textContent = '×';

      row.appendChild(title);
      row.appendChild(meta);
      row.appendChild(del);
      frag.appendChild(row);
    });
    el.conversations.appendChild(frag);
    el.count.textContent = state.conversations.length + ' 个会话';
  }

  function bindConversationEvents() {
    el.conversations.addEventListener('click', function (e) {
      var del = e.target.closest('.conv-del');
      var row = e.target.closest('.conv');
      if (!row) return;
      var id = row.dataset.id;
      if (del) {
        e.stopPropagation();
        var conv = findConv(id);
        if (conv && window.confirm('删除会话「' + (conv.title || '新对话') + '」？')) {
          state.conversations = state.conversations.filter(function (c) { return c.id !== id; });
          if (state.currentId === id) state.currentId = '';
          ensureConversation();
          renderAll();
          saveSoon();
        }
        return;
      }
      if (id !== state.currentId) {
        state.currentId = id;
        clearHint();
        renderAll();
      }
    });

    el.conversations.addEventListener('dblclick', function (e) {
      var title = e.target.closest('.conv-title');
      var row = e.target.closest('.conv');
      if (!title || !row) return;
      var conv = findConv(row.dataset.id);
      if (!conv) return;
      var next = window.prompt('重命名会话', conv.title || '新对话');
      if (next && next.trim()) {
        conv.title = next.trim().slice(0, 60);
        conv.updatedAt = now();
        renderAll();
        saveSoon();
      }
    });
  }

  // ---------------- Markdown ----------------

  function escapeHtml(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }

  function inline(text) {
    var codes = [];
    var s = escapeHtml(text);
    // 行内代码先抠出来，避免其中的 * / [ ] 被当成 Markdown
    s = s.replace(/`([^`]+)`/g, function (_, code) {
      codes.push(code);
      return '\u0000' + (codes.length - 1) + '\u0000';
    });
    s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    s = s.replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<em>$2</em>');
    s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="#" data-href="$2">$1</a>');
    s = s.replace(/\u0000(\d+)\u0000/g, function (_, i) {
      return '<code>' + codes[Number(i)] + '</code>';
    });
    return s;
  }

  function renderMarkdown(src) {
    var lines = String(src || '').replace(/\r\n/g, '\n').split('\n');
    var out = [];
    var para = [];
    var i = 0;

    function flushPara() {
      if (!para.length) return;
      out.push('<p>' + inline(para.join('\n')).replace(/\n/g, '<br>') + '</p>');
      para = [];
    }

    while (i < lines.length) {
      var line = lines[i];
      var m;

      if (/^\s*```/.test(line)) {
        flushPara();
        var lang = line.replace(/^\s*```/, '').trim();
        var buf = [];
        i++;
        while (i < lines.length && !/^\s*```/.test(lines[i])) {
          buf.push(lines[i]);
          i++;
        }
        if (i < lines.length) i++;
        out.push(
          '<pre><div class="code-head"><span>' + escapeHtml(lang || 'code') +
          '</span><button class="copy-code" type="button">复制</button></div><code>' +
          escapeHtml(buf.join('\n')) + '</code></pre>'
        );
        continue;
      }

      if (!line.trim()) {
        flushPara();
        i++;
        continue;
      }

      m = line.match(/^(#{1,6})\s+(.*)$/);
      if (m) {
        flushPara();
        var lv = m[1].length;
        out.push('<h' + lv + '>' + inline(m[2]) + '</h' + lv + '>');
        i++;
        continue;
      }

      if (/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
        flushPara();
        out.push('<hr>');
        i++;
        continue;
      }

      if (/^>\s?/.test(line)) {
        flushPara();
        var quote = [];
        while (i < lines.length && /^>\s?/.test(lines[i])) {
          quote.push(lines[i].replace(/^>\s?/, ''));
          i++;
        }
        out.push('<blockquote>' + inline(quote.join('\n')).replace(/\n/g, '<br>') + '</blockquote>');
        continue;
      }

      var ul = /^\s*[-*+]\s+/;
      if (ul.test(line)) {
        flushPara();
        var items = [];
        while (i < lines.length && ul.test(lines[i])) {
          items.push(lines[i].replace(ul, ''));
          i++;
        }
        out.push('<ul>' + items.map(function (t) { return '<li>' + inline(t) + '</li>'; }).join('') + '</ul>');
        continue;
      }

      var ol = /^\s*\d+\.\s+/;
      if (ol.test(line)) {
        flushPara();
        var oitems = [];
        while (i < lines.length && ol.test(lines[i])) {
          oitems.push(lines[i].replace(ol, ''));
          i++;
        }
        out.push('<ol>' + oitems.map(function (t) { return '<li>' + inline(t) + '</li>'; }).join('') + '</ol>');
        continue;
      }

      para.push(line);
      i++;
    }

    flushPara();
    return out.join('');
  }

  // ---------------- 消息渲染 ----------------

  function assistantBubble(msg) {
    var bubble = document.createElement('div');
    bubble.className = 'bubble';

    if (msg.reasoning) {
      var details = document.createElement('details');
      details.className = 'reasoning';
      if (msg.pending) details.open = true;
      var summary = document.createElement('summary');
      summary.textContent = '思考过程';
      var body = document.createElement('div');
      body.className = 'reasoning-body';
      body.textContent = msg.reasoning;
      details.appendChild(summary);
      details.appendChild(body);
      bubble.appendChild(details);
    }

    if (msg.toolStatus) {
      var tool = document.createElement('div');
      tool.className = 'msg-tool-status';
      tool.textContent = msg.toolStatus;
      bubble.appendChild(tool);
    }

    var content = document.createElement('div');
    content.className = 'markdown bubble-body';
    if (!msg.content && msg.pending) {
      content.innerHTML = '<span class="typing"><i></i><i></i><i></i></span>';
    } else {
      content.innerHTML = renderMarkdown(msg.content);
    }
    bubble.appendChild(content);

    if (msg.sources && msg.sources.length) {
      var box = document.createElement('div');
      box.className = 'msg-sources';
      var label = document.createElement('div');
      label.className = 'msg-sources-title';
      label.textContent = '参考来源';
      box.appendChild(label);
      msg.sources.forEach(function (s, i) {
        var a = document.createElement('a');
        a.href = '#';
        a.dataset.href = s.url;
        a.textContent = (i + 1) + '. ' + (s.title || s.url);
        box.appendChild(a);
      });
      bubble.appendChild(box);
    }

    if (msg.error) {
      var err = document.createElement('div');
      err.className = 'msg-error';
      err.textContent = msg.error;
      bubble.appendChild(err);
    }
    return bubble;
  }

  function messageEl(conv, msg, index) {
    var row = document.createElement('div');
    row.className = 'msg ' + msg.role;
    row.dataset.index = String(index);

    var avatar = document.createElement('div');
    avatar.className = 'avatar';
    avatar.textContent = msg.role === 'user' ? '你' : 'AI';

    var main = document.createElement('div');
    main.className = 'msg-main';

    if (msg.role === 'user') {
      var bubble = document.createElement('div');
      bubble.className = 'bubble';
      if (msg.images && msg.images.length) {
        var grid = document.createElement('div');
        grid.className = 'msg-images';
        msg.images.forEach(function (img) {
          var name = img.path.split(/[\\/]/).pop();
          // 历史预览是异步补的，先显示文件名，别把空 src 挂上去触发一次无谓加载。
          if (!img.preview) {
            var ph = document.createElement('div');
            ph.className = 'msg-image-ph';
            ph.textContent = name;
            ph.title = img.path;
            grid.appendChild(ph);
            return;
          }
          var im = document.createElement('img');
          im.src = img.preview;
          im.alt = name;
          im.title = img.path;
          grid.appendChild(im);
        });
        bubble.appendChild(grid);
      }
      if (msg.docs && msg.docs.length) {
        var docBox = document.createElement('div');
        docBox.className = 'msg-docs';
        msg.docs.forEach(function (d) {
          var chip = document.createElement('span');
          chip.className = 'msg-doc-chip';
          chip.textContent = '文档 · ' + d.name + (d.pages ? '（' + d.pages + ' 页）' : '');
          chip.title = d.path || d.name;
          docBox.appendChild(chip);
        });
        bubble.appendChild(docBox);
      }
      if (msg.text) {
        var text = document.createElement('div');
        text.textContent = msg.text;
        bubble.appendChild(text);
      }
      main.appendChild(bubble);
    } else {
      main.appendChild(assistantBubble(msg));
    }

    var actions = document.createElement('div');
    actions.className = 'msg-actions';
    var copy = document.createElement('button');
    copy.type = 'button';
    copy.className = 'act-copy';
    copy.textContent = '复制';
    actions.appendChild(copy);

    var last = index === conv.messages.length - 1;
    if (msg.role === 'assistant' && last && !msg.pending) {
      var again = document.createElement('button');
      again.type = 'button';
      again.className = 'act-again';
      again.textContent = '重新生成';
      actions.appendChild(again);
    }
    main.appendChild(actions);

    row.appendChild(avatar);
    row.appendChild(main);
    return row;
  }

  function renderMessages(keepScroll) {
    var conv = currentConv();
    el.messages.innerHTML = '';
    if (!conv || !conv.messages.length) {
      el.empty.hidden = false;
      return;
    }
    el.empty.hidden = true;
    var frag = document.createDocumentFragment();
    conv.messages.forEach(function (msg, i) {
      frag.appendChild(messageEl(conv, msg, i));
    });
    el.messages.appendChild(frag);
    if (!keepScroll) scrollToBottom(true);
  }

  function patchMessage(index) {
    var conv = currentConv();
    if (!conv) return;
    var msg = conv.messages[index];
    var node = el.messages.querySelector('.msg[data-index="' + index + '"]');
    if (!msg || !node) {
      renderMessages(true);
      return;
    }
    var old = node.querySelector('.bubble');
    var next = assistantBubble(msg);
    old.replaceWith(next);
    scrollToBottom(false);
  }

  function nearBottom() {
    return el.messages.scrollHeight - el.messages.scrollTop - el.messages.clientHeight < 120;
  }

  function scrollToBottom(force) {
    if (force || nearBottom()) el.messages.scrollTop = el.messages.scrollHeight;
  }

  function renderHeader() {
    var conv = currentConv();
    el.title.textContent = conv ? (conv.title || '新对话') : '新对话';
    var m = modelById(state.model);
    el.sub.textContent = m ? (m.model || '') : '';
  }

  function renderAll() {
    renderConversations();
    renderHeader();
    renderMessages();
    renderAttachments();
    renderDocs();
    updateSendState();
  }

  // ---------------- 附件 ----------------

  function renderAttachments() {
    el.attachments.innerHTML = '';
    el.attachments.hidden = state.attachments.length === 0;
    state.attachments.forEach(function (att, i) {
      var box = document.createElement('div');
      box.className = 'thumb';
      var img = document.createElement('img');
      img.src = att.preview;
      img.alt = att.path.split(/[\\/]/).pop();
      img.title = att.path;
      var del = document.createElement('button');
      del.type = 'button';
      del.textContent = '×';
      del.title = '移除';
      del.setAttribute('aria-label', '移除图片');
      del.addEventListener('click', function () {
        state.attachments.splice(i, 1);
        renderAttachments();
      });
      box.appendChild(img);
      box.appendChild(del);
      el.attachments.appendChild(box);
    });
  }

  function renderDocs() {
    el.docs.innerHTML = '';
    el.docs.hidden = state.docs.length === 0;
    state.docs.forEach(function (doc, i) {
      var chip = document.createElement('div');
      chip.className = 'doc-chip';
      chip.title = doc.path || doc.name;
      var label = document.createElement('span');
      label.textContent = doc.name + (doc.pages ? ' · ' + doc.pages + ' 页' : '');
      var del = document.createElement('button');
      del.type = 'button';
      del.textContent = '×';
      del.title = '移除';
      del.setAttribute('aria-label', '移除文档');
      del.addEventListener('click', function () {
        state.docs.splice(i, 1);
        renderDocs();
        updateSendState();
      });
      chip.appendChild(label);
      chip.appendChild(del);
      el.docs.appendChild(chip);
    });
  }

  function addDocPaths(paths) {
    paths = Array.isArray(paths) ? paths.filter(function (p) { return typeof p === 'string' && p; }) : [];
    if (!paths.length) return;
    var room = MAX_DOCS - state.docs.length;
    if (room <= 0) {
      showHint('一条消息最多 ' + MAX_DOCS + ' 个文档');
      return;
    }
    paths = paths.slice(0, room);
    showHint('正在识别文档，请稍候…');
    efto.ipcMessage('ocr', { id: String(++state.ocrSeq), paths: paths });
  }

  function addPaths(paths) {
    paths = Array.isArray(paths) ? paths.filter(function (p) { return typeof p === 'string' && p; }) : [];
    if (!paths.length) return;
    var images = paths.filter(function (p) { return IMAGE_EXT.test(p); });
    var others = paths.filter(function (p) { return !IMAGE_EXT.test(p); });
    if (others.length) addDocPaths(others);
    if (!images.length) return;
    var room = MAX_IMAGES - state.attachments.length;
    if (room <= 0) {
      showHint('一条消息最多 ' + MAX_IMAGES + ' 张图片');
      return;
    }
    images = images.slice(0, room);
    var id = String(++state.previewSeq);
    state.previewPaths[id] = images;
    efto.ipcMessage('preview', { id: id, paths: images });
  }

  // ---------------- 请求 ----------------

  function buildApiMessages(conv, pendingIndex) {
    var out = [];
    conv.messages.forEach(function (m, i) {
      if (i === pendingIndex) return;
      if (m.role === 'user') {
        var text = m.text || '';
        if (m.docs && m.docs.length) {
          var docs = m.docs.map(function (d) {
            return '【文档：' + d.name + '】\n' + d.text;
          }).join('\n\n');
          text = text ? text + '\n\n' + docs : docs;
        }
        if (m.images && m.images.length) {
          var parts = [];
          if (text) parts.push({ type: 'text', text: text });
          m.images.forEach(function (img) { parts.push({ type: 'image', path: img.path }); });
          out.push({ role: 'user', content: parts });
        } else if (text) {
          out.push({ role: 'user', content: text });
        }
      } else if (m.role === 'assistant' && m.content && !m.pending && !m.error) {
        out.push({ role: 'assistant', content: m.content });
      }
    });
    return out;
  }

  function startRequest(conv) {
    var pendingIndex = conv.messages.length - 1;
    var msg = conv.messages[pendingIndex];
    if (!msg || msg.role !== 'assistant') return;
    msg.pending = true;
    var requestId = uid();
    state.streaming = { id: requestId, convId: conv.id, index: pendingIndex };
    updateSendState();

    var messages = buildApiMessages(conv, pendingIndex);
    if (!messages.length) {
      msg.pending = false;
      msg.error = '没有可发送的消息';
      patchMessage(pendingIndex);
      state.streaming = null;
      updateSendState();
      return;
    }
    efto.ipcMessage('ask', {
      id: requestId,
      model: state.model || '',
      messages: messages,
      webSearch: state.webSearch === true
    });
  }

  function send() {
    if (state.streaming) return;
    clearHint();
    if (!state.models.length) {
      showHint('请先在 设置 → 模型 中配置模型并设为默认');
      return;
    }
    var text = el.input.value.trim();
    var images = state.attachments.slice();
    var docs = state.docs.slice();
    if (!text && !images.length && !docs.length) return;

    var conv = ensureConversation();
    conv.messages.push({
      role: 'user',
      text: text,
      images: images,
      docs: docs,
      ts: now()
    });
    if (!conv.title || conv.title === '新对话') conv.title = deriveTitle(conv);
    conv.updatedAt = now();
    conv.messages.push({
      role: 'assistant',
      content: '',
      reasoning: '',
      error: '',
      pending: true,
      ts: now()
    });

    el.input.value = '';
    autoGrow();
    state.attachments = [];
    state.docs = [];
    renderAll();
    startRequest(conv);
    el.input.focus();
  }

  function regenerate() {
    if (state.streaming) return;
    var conv = currentConv();
    if (!conv) return;
    var last = conv.messages.length - 1;
    if (last < 0 || conv.messages[last].role !== 'assistant') return;
    conv.messages.pop();
    conv.messages.push({
      role: 'assistant',
      content: '',
      reasoning: '',
      error: '',
      pending: true,
      ts: now()
    });
    conv.updatedAt = now();
    renderMessages();
    startRequest(conv);
  }

  function stopStreaming() {
    var s = state.streaming;
    if (!s) return;
    state.streaming = null;
    var conv = findConv(s.convId);
    if (conv) {
      var msg = conv.messages[s.index];
      if (msg) {
        msg.pending = false;
        if (!msg.content && !msg.reasoning) conv.messages.splice(s.index, 1);
      }
      conv.updatedAt = now();
      saveSoon();
      if (conv.id === state.currentId) renderMessages(true);
      else renderConversations();
    }
    updateSendState();
  }

  function updateSendState() {
    el.send.textContent = state.streaming ? '停止' : '发送';
    el.send.disabled = !state.streaming && !state.models.length;
  }

  function syncWebButton() {
    el.web.classList.toggle('on', state.webSearch === true);
    el.web.setAttribute('aria-pressed', state.webSearch === true ? 'true' : 'false');
  }

  // DeepSeek 的 server_tool_event 形状见文档；这里只取到能显示的一句话。
  function toolStatusText(ev) {
    if (!ev || typeof ev !== 'object') return '';
    var kind = ev.kind || ev.type || '';
    var action = '';
    if (ev.call && typeof ev.call === 'object') {
      action = (ev.call.action && ev.call.action.type) || ev.call.kind || '';
    }
    var reading = action === 'read_link' || action === 'open_page' || action === 'find_in_page';
    if (kind === 'call_started') return reading ? '正在阅读网页…' : '正在联网搜索…';
    if (kind === 'call_completed') return reading ? '已读取网页' : '已完成搜索';
    if (kind === 'step_reasoning_done') return '';
    return '';
  }

  // 从 server_tool_calls / annotations 里收集去重后的引用链接。
  function collectSources(data) {
    var out = [];
    var seen = {};
    function add(url, title) {
      if (!url || seen[url]) return;
      seen[url] = true;
      out.push({ url: url, title: title || url });
    }
    (Array.isArray(data.serverToolCalls) ? data.serverToolCalls : []).forEach(function (step) {
      (step.calls || []).forEach(function (call) {
        (call.sources || []).forEach(function (s) { add(s.url, s.title); });
        if (call.page && call.page.url) add(call.page.url, call.page.title);
      });
    });
    (Array.isArray(data.annotations) ? data.annotations : []).forEach(function (a) {
      var c = a && a.url_citation;
      if (c) add(c.url, c.title);
    });
    return out.slice(0, 20);
  }

  // ---------------- 事件 ----------------

  function errorText(e) {
    if (!e) return '未知错误';
    if (typeof e === 'string') return e;
    return e.message || String(e);
  }

  function autoGrow() {
    el.input.style.height = 'auto';
    el.input.style.height = Math.min(el.input.scrollHeight, 168) + 'px';
  }

  function bind() {
    try {
      document.documentElement.dataset.theme = efto.isDarkMode() ? 'dark' : 'light';
    } catch (e) {
      document.documentElement.dataset.theme = 'dark';
    }

    el.send.addEventListener('click', function () {
      if (state.streaming) stopStreaming();
      else send();
    });
    el.newChat.addEventListener('click', function () {
      if (state.streaming) stopStreaming();
      var conv = {
        id: uid(),
        title: '新对话',
        createdAt: now(),
        updatedAt: now(),
        messages: []
      };
      state.conversations.unshift(conv);
      state.currentId = conv.id;
      state.attachments = [];
      state.docs = [];
      clearHint();
      renderAll();
      saveSoon();
      el.input.focus();
    });

    el.clearAll.addEventListener('click', function () {
      if (!state.conversations.length) return;
      if (!window.confirm('清空全部 ' + state.conversations.length + ' 个会话？')) return;
      if (state.streaming) {
        state.streaming = null;
        updateSendState();
      }
      state.conversations = [];
      state.currentId = '';
      ensureConversation();
      renderAll();
      saveSoon();
    });

    el.model.addEventListener('change', function () {
      state.model = el.model.value;
      renderHeader();
      saveSoon();
    });

    el.input.addEventListener('input', autoGrow);
    el.input.addEventListener('keydown', function (e) {
      if (e.isComposing || e.keyCode === 229) return;
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        send();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && (e.key === 'v' || e.key === 'V')) {
        efto.ipcMessage('paste-image', {});
      }
    });

    el.pick.addEventListener('click', function () {
      try {
        efto.pickFiles().then(function (paths) { addPaths(paths); }).catch(function (e) {
          showHint('选择图片失败：' + errorText(e));
        });
      } catch (e) {
        showHint('选择图片失败：' + errorText(e));
      }
    });

    el.pickFile.addEventListener('click', function () {
      try {
        efto.pickFiles().then(function (paths) { addDocPaths(paths); }).catch(function (e) {
          showHint('选择文档失败：' + errorText(e));
        });
      } catch (e) {
        showHint('选择文档失败：' + errorText(e));
      }
    });

    el.web.addEventListener('click', function () {
      state.webSearch = !state.webSearch;
      syncWebButton();
      saveSoon();
      if (state.webSearch) showHint('已开启联网：由 DeepSeek 服务端搜索与抓取网页，需要 deepseek-flash 模型');
    });

    try {
      efto.onFileDrag(function (inside) {
        document.body.classList.toggle('dragging', !!inside);
      });
      efto.onFileDrop(function (paths) {
        document.body.classList.remove('dragging');
        addPaths(paths);
      });
    } catch (e) {
      // 老版本宿主没有拖拽能力时不阻塞输入
    }

    el.messages.addEventListener('click', function (e) {
      var codeBtn = e.target.closest('.copy-code');
      if (codeBtn) {
        var code = codeBtn.closest('pre').querySelector('code');
        efto.copyText(code ? code.textContent : '');
        var old = codeBtn.textContent;
        codeBtn.textContent = '已复制';
        setTimeout(function () { codeBtn.textContent = old; }, 1000);
        return;
      }
      var link = e.target.closest('a[data-href]');
      if (link) {
        e.preventDefault();
        efto.openExternal(link.getAttribute('data-href'));
        return;
      }
      var row = e.target.closest('.msg');
      if (!row) return;
      var conv = currentConv();
      if (!conv) return;
      var index = Number(row.dataset.index);
      var msg = conv.messages[index];
      if (!msg) return;
      if (e.target.closest('.act-copy')) {
        var content = msg.role === 'user' ? msg.text : msg.content;
        if (content) efto.copyText(content);
        return;
      }
      if (e.target.closest('.act-again')) {
        regenerate();
      }
    });

    efto.onMessage('models', renderModels);
    efto.onMessage('delta', function (data) {
      var s = state.streaming;
      if (!s || !data || s.id !== data.id) return;
      var conv = findConv(s.convId);
      if (!conv) return;
      var msg = conv.messages[s.index];
      if (!msg) return;
      if (data.text) msg.content += data.text;
      if (data.reasoning) msg.reasoning += data.reasoning;
      if (data.toolEvent) msg.toolStatus = toolStatusText(data.toolEvent);
      if (conv.id === state.currentId) patchMessage(s.index);
    });
    efto.onMessage('done', function (data) {
      var s = state.streaming;
      if (!s || !data || s.id !== data.id) return;
      var conv = findConv(s.convId);
      state.streaming = null;
      updateSendState();
      if (!conv) return;
      var msg = conv.messages[s.index];
      if (msg) {
        msg.pending = false;
        if (data.content) msg.content = data.content;
        if (data.reasoning) msg.reasoning = data.reasoning;
        msg.sources = collectSources(data);
        msg.error = '';
        msg.toolStatus = '';
        msg.ts = now();
      }
      conv.updatedAt = now();
      if (conv.id === state.currentId) {
        renderMessages(true);
        scrollToBottom(false);
      }
      renderConversations();
      renderHeader();
      saveSoon();
    });
    efto.onMessage('error', function (data) {
      var s = state.streaming;
      if (!s || !data || s.id !== data.id) return;
      var conv = findConv(s.convId);
      state.streaming = null;
      updateSendState();
      if (!conv) return;
      var msg = conv.messages[s.index];
      if (msg) {
        msg.pending = false;
        msg.error = data.message || '请求失败';
        msg.toolStatus = '';
        msg.ts = now();
      }
      conv.updatedAt = now();
      if (conv.id === state.currentId) {
        renderMessages(true);
        scrollToBottom(false);
      }
      renderConversations();
      saveSoon();
    });
    efto.onMessage('previews', function (data) {
      if (!data) return;
      var paths = state.previewPaths[data.id] || [];
      delete state.previewPaths[data.id];
      var items = Array.isArray(data.items) ? data.items : [];
      if (String(data.id).indexOf('hist-') === 0) {
        var byPath = {};
        items.forEach(function (item) {
          if (item.preview) byPath[item.path] = item.preview;
        });
        var changed = false;
        state.conversations.forEach(function (conv) {
          conv.messages.forEach(function (m) {
            (m.images || []).forEach(function (img) {
              if (!img.preview && byPath[img.path]) {
                img.preview = byPath[img.path];
                changed = true;
              }
            });
          });
        });
        if (changed) renderMessages(true);
        return;
      }
      items.forEach(function (item) {
        if (item.preview) {
          var dup = state.attachments.some(function (a) { return a.path === item.path; });
          if (!dup && state.attachments.length < MAX_IMAGES) {
            state.attachments.push({ path: item.path, preview: item.preview });
          }
        } else if (item.error) {
          showHint('图片读取失败：' + item.error);
        }
      });
      if (!items.length && paths.length) showHint('图片读取失败');
      renderAttachments();
    });
    efto.onMessage('paste-image', function (data) {
      if (!data) return;
      if (data.error) {
        showHint('粘贴图片失败：' + data.error);
        return;
      }
      if (!data.path || !data.preview) return;
      if (state.attachments.length >= MAX_IMAGES) {
        showHint('一条消息最多 ' + MAX_IMAGES + ' 张图片');
        return;
      }
      state.attachments.push({ path: data.path, preview: data.preview });
      renderAttachments();
    });
    efto.onMessage('ocr', function (data) {
      if (!data) return;
      if (!data.ok) {
        showHint('文档识别失败：' + (data.error || data.path || ''));
        return;
      }
      if (state.docs.length >= MAX_DOCS) {
        showHint('一条消息最多 ' + MAX_DOCS + ' 个文档');
        return;
      }
      state.docs.push({
        name: data.name || '文档',
        path: data.path || '',
        pages: data.pageCount || 0,
        text: data.text || ''
      });
      clearHint();
      renderDocs();
      updateSendState();
    });
    efto.onMessage('prefill', function (data) {
      if (!data || typeof data.text !== 'string' || !data.text) return;
      el.input.value = data.text;
      autoGrow();
      el.input.focus();
    });
  }

  bind();
  bindConversationEvents();
  loadSaved();
  efto.ipcMessage('init', {});
})();
