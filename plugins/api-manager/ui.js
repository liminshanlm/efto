(function () {
  'use strict';

  var efto = window.efto || null;

  var state = {
    ws: null,
    draft: null,
    selectedId: '',
    dirty: false,
    sideTab: 'collections',
    reqTab: 'params',
    respTab: 'body',
    response: null,
    respRaw: '',
    busy: false,
    collapsed: {}
  };

  var el = {};
  var toastTimer = 0;

  function $(id) { return document.getElementById(id); }

  function h(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined && text !== null) node.textContent = text;
    return node;
  }

  function button(text, cls) {
    var b = h('button', cls || '', text);
    b.type = 'button';
    return b;
  }

  function iconButton(text, title, fn) {
    var b = button(text, 'icon-btn');
    b.title = title;
    b.addEventListener('click', function (e) {
      e.stopPropagation();
      fn();
    });
    return b;
  }

  function textInput(value, placeholder) {
    var i = document.createElement('input');
    i.type = 'text';
    i.value = value == null ? '' : value;
    i.placeholder = placeholder || '';
    i.spellcheck = false;
    return i;
  }

  function clone(v) { return JSON.parse(JSON.stringify(v)); }

  function formatBytes(n) {
    n = Number(n) || 0;
    if (n < 1024) return n + ' B';
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + ' KB';
    return (n / (1024 * 1024)).toFixed(1) + ' MB';
  }

  function toast(msg) {
    el.toast.textContent = String(msg || '');
    el.toast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { el.toast.hidden = true; }, 2600);
  }

  // ---------- data helpers ----------

  function findRequestById(id) {
    if (!state.ws || !id) return null;
    for (var i = 0; i < state.ws.collections.length; i++) {
      var col = state.ws.collections[i];
      for (var j = 0; j < col.requests.length; j++) {
        if (col.requests[j].id === id) return { collection: col, request: col.requests[j] };
      }
    }
    return null;
  }

  function firstRequest() {
    if (!state.ws) return null;
    for (var i = 0; i < state.ws.collections.length; i++) {
      if (state.ws.collections[i].requests.length) {
        return { collection: state.ws.collections[i], request: state.ws.collections[i].requests[0] };
      }
    }
    return null;
  }

  function defaultCollectionId() {
    if (!state.ws || !state.ws.collections.length) return '';
    if (state.draft && state.draft._collectionId) return state.draft._collectionId;
    return state.ws.collections[0].id;
  }

  function blankRequest(collectionId, folderId) {
    return {
      id: '',
      name: '',
      method: 'GET',
      url: '',
      params: [],
      headers: [],
      body: { mode: 'none', raw: '', form: [] },
      auth: { type: 'none', token: '', username: '', password: '' },
      timeoutMs: 30000,
      folderId: folderId || '',
      _collectionId: collectionId || defaultCollectionId()
    };
  }

  // ---------- rendering ----------

  function folderChildren(col, parentId) {
    return (col.folders || []).filter(function (f) {
      return String(f.parentId || '') === String(parentId || '');
    });
  }

  function folderRequests(col, folderId) {
    return col.requests.filter(function (r) {
      return String(r.folderId || '') === String(folderId || '');
    });
  }

  function countFolderRequests(col, folderId) {
    var ids = {};
    var stack = [String(folderId || '')];
    while (stack.length) {
      var cur = stack.pop();
      if (!cur || ids[cur]) continue;
      ids[cur] = true;
      (col.folders || []).forEach(function (f) {
        if (String(f.parentId || '') === cur) stack.push(f.id);
      });
    }
    return col.requests.filter(function (r) { return ids[r.folderId]; }).length;
  }

  function renderRequestRow(box, col, req, depth) {
    var active = state.draft && state.draft.id && state.draft.id === req.id;
    var item = h('div', 'req-item' + (active ? ' active' : ''));
    if (depth) item.style.paddingLeft = (10 + depth * 14) + 'px';
    item.appendChild(h('span', 'method ' + String(req.method || 'GET').toLowerCase(), req.method || 'GET'));

    var text = h('div', 'req-text');
    text.appendChild(h('span', 'req-name', req.name || '未命名请求'));
    text.appendChild(h('span', 'req-url', req.url || '(空 URL)'));
    item.appendChild(text);
    item.appendChild(iconButton('×', '删除请求', function () { deleteRequest(req); }));
    item.addEventListener('click', function () { loadRequest(req.id); });
    box.appendChild(item);
  }

  function renderFolderNode(box, col, folder, depth) {
    var key = col.id + ':' + folder.id;
    var collapsed = !!state.collapsed[key];
    var head = h('div', 'folder-head');
    head.style.paddingLeft = (8 + depth * 14) + 'px';
    head.appendChild(h('span', 'folder-caret', collapsed ? '▸' : '▾'));
    head.appendChild(h('span', 'folder-name', folder.name));
    head.appendChild(h('span', 'folder-count', String(countFolderRequests(col, folder.id))));
    head.appendChild(iconButton('＋', '在这个分组里新建请求', function () { startNewRequest(col.id, folder.id); }));
    head.appendChild(iconButton('⊞', '新建子分组', function () { createFolder(col, folder.id); }));
    head.appendChild(iconButton('✎', '重命名分组', function () { renameFolder(col, folder); }));
    head.appendChild(iconButton('×', '删除分组及其请求', function () { deleteFolder(col, folder); }));
    head.addEventListener('click', function () {
      if (collapsed) delete state.collapsed[key];
      else state.collapsed[key] = true;
      renderSidebar();
    });
    box.appendChild(head);

    if (collapsed) return;
    folderChildren(col, folder.id).forEach(function (sub) { renderFolderNode(box, col, sub, depth + 1); });
    folderRequests(col, folder.id).forEach(function (req) { renderRequestRow(box, col, req, depth + 1); });
  }

  function renderSidebar() {
    var box = el.sideList;
    box.textContent = '';
    if (!state.ws) return;

    if (state.sideTab === 'history') {
      renderHistory(box);
      return;
    }

    if (!state.ws.collections.length) {
      box.appendChild(h('div', 'side-empty', '还没有集合，点上方「新集合」或导入 Postman 集合。'));
      return;
    }

    state.ws.collections.forEach(function (col) {
      var head = h('div', 'col-head');
      head.appendChild(h('span', 'col-name', col.name + ' · ' + col.requests.length));
      head.appendChild(iconButton('＋', '在这个集合里新建请求', function () { startNewRequest(col.id, ''); }));
      head.appendChild(iconButton('⊞', '新建分组', function () { createFolder(col, ''); }));
      head.appendChild(iconButton('✎', '重命名集合', function () { renameCollection(col); }));
      head.appendChild(iconButton('×', '删除集合', function () { deleteCollection(col); }));
      box.appendChild(head);

      folderChildren(col, '').forEach(function (folder) { renderFolderNode(box, col, folder, 0); });
      folderRequests(col, '').forEach(function (req) { renderRequestRow(box, col, req, 0); });
    });
  }

  function renderHistory(box) {
    var tools = h('div', 'history-tools');
    var clear = button('清空历史', 'ghost');
    clear.addEventListener('click', function () {
      if (!confirm('清空全部请求历史？')) return;
      efto.ipcMessage('clear-history');
    });
    tools.appendChild(clear);
    if (state.ws.history.length) box.appendChild(tools);

    if (!state.ws.history.length) {
      box.appendChild(h('div', 'history-empty', '还没有请求历史。'));
      return;
    }

    state.ws.history.forEach(function (item) {
      var row = h('div', 'history-item');
      row.appendChild(h('span', 'method ' + String(item.method || 'GET').toLowerCase(), item.method || 'GET'));
      var text = h('div', 'hist-text');
      text.appendChild(h('span', 'hist-name', (item.ok ? String(item.status) : '失败') + ' · ' + (item.name || item.url || '请求')));
      text.appendChild(h('span', 'hist-meta', item.ts + ' · ' + item.durationMs + ' ms'));
      row.appendChild(text);
      row.addEventListener('click', function () { loadHistory(item); });
      box.appendChild(row);
    });
  }

  function renderEnvSelect() {
    if (!state.ws) return;
    var value = state.ws.activeEnvId || '';
    el.envSelect.textContent = '';
    var none = document.createElement('option');
    none.value = '';
    none.textContent = '无环境';
    el.envSelect.appendChild(none);
    state.ws.environments.forEach(function (env) {
      var opt = document.createElement('option');
      opt.value = env.id;
      opt.textContent = env.name;
      el.envSelect.appendChild(opt);
    });
    el.envSelect.value = value;
  }

  function renderEditor() {
    if (!state.draft) return;
    var d = state.draft;
    el.method.value = d.method || 'GET';
    el.url.value = d.url || '';
    el.reqName.value = d.name || '';

    renderKv(el.kvParams, d.params, markDirty);
    renderKv(el.kvHeaders, d.headers, markDirty);
    renderBody();
    renderAuth();
    refreshCounts();
    renderReqTabs();
    refreshDirty();
  }

  function renderKv(container, list, onChange) {
    container.textContent = '';
    if (!Array.isArray(list)) return;

    list.forEach(function (item, index) {
      var row = h('div', 'kv-row');

      var cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = item.enabled !== false;
      cb.title = '启用/禁用';
      cb.addEventListener('change', function () {
        item.enabled = cb.checked;
        onChange();
      });

      var key = textInput(item.key, '键');
      key.addEventListener('input', function () {
        item.key = key.value;
        onChange();
      });

      var value = textInput(item.value, '值');
      value.addEventListener('input', function () {
        item.value = value.value;
        onChange();
      });

      var del = button('×', 'kv-del');
      del.title = '删除这一行';
      del.addEventListener('click', function () {
        list.splice(index, 1);
        renderKv(container, list, onChange);
        onChange();
      });

      row.appendChild(cb);
      row.appendChild(key);
      row.appendChild(value);
      row.appendChild(del);
      container.appendChild(row);
    });

    var add = button('＋ 添加一行', 'kv-add');
    add.addEventListener('click', function () {
      list.push({ key: '', value: '', enabled: true });
      renderKv(container, list, onChange);
      onChange();
    });
    container.appendChild(add);
  }

  function renderBody() {
    var body = state.draft.body || { mode: 'none', raw: '', form: [] };
    el.bodyMode.value = body.mode || 'none';
    var isForm = body.mode === 'form';
    el.bodyForm.hidden = !isForm;
    el.bodyRaw.hidden = isForm;
    el.bodyRaw.disabled = body.mode === 'none';

    if (isForm) {
      if (!Array.isArray(body.form)) body.form = [];
      renderKv(el.kvBodyForm, body.form, markDirty);
      el.bodyHint.textContent = '按 application/x-www-form-urlencoded 发送';
    } else if (body.mode === 'json') {
      el.bodyRaw.value = body.raw || '';
      el.bodyHint.textContent = 'Content-Type 默认 application/json';
    } else if (body.mode === 'text') {
      el.bodyRaw.value = body.raw || '';
      el.bodyHint.textContent = 'Content-Type 默认 text/plain';
    } else {
      el.bodyRaw.value = body.raw || '';
      el.bodyHint.textContent = '不发送请求体';
    }
  }

  function renderAuth() {
    var auth = state.draft.auth || { type: 'none' };
    el.authType.value = auth.type || 'none';
    el.authBearer.hidden = auth.type !== 'bearer';
    el.authBasic.hidden = auth.type !== 'basic';
    el.authToken.value = auth.token || '';
    el.authUser.value = auth.username || '';
    el.authPass.value = auth.password || '';
  }

  function renderReqTabs() {
    var tabs = el.reqTabs.querySelectorAll('button[data-tab]');
    for (var i = 0; i < tabs.length; i++) {
      tabs[i].classList.toggle('active', tabs[i].getAttribute('data-tab') === state.reqTab);
    }
    el.paneParams.hidden = state.reqTab !== 'params';
    el.paneHeaders.hidden = state.reqTab !== 'headers';
    el.paneBody.hidden = state.reqTab !== 'body';
    el.paneAuth.hidden = state.reqTab !== 'auth';
  }

  function renderRespTabs() {
    var tabs = el.respActions.querySelectorAll('button[data-resp]');
    for (var i = 0; i < tabs.length; i++) {
      tabs[i].classList.toggle('active', tabs[i].getAttribute('data-resp') === state.respTab);
    }
    el.respBody.hidden = state.respTab !== 'body';
    el.respHeaders.hidden = state.respTab !== 'headers';
  }

  function refreshCounts() {
    if (!state.draft) return;
    el.countParams.textContent = String(enabledCount(state.draft.params));
    el.countHeaders.textContent = String(enabledCount(state.draft.headers));
  }

  function enabledCount(list) {
    if (!Array.isArray(list)) return 0;
    return list.filter(function (x) { return x && x.enabled !== false && String(x.key || '').trim(); }).length;
  }

  function markDirty() {
    state.dirty = true;
    refreshDirty();
    refreshCounts();
  }

  function refreshDirty() {
    el.dirty.hidden = !state.dirty;
    el.btnSaveCurrent.disabled = !state.draft;
  }

  function renderResponseEmpty(text) {
    el.respStatus.className = 'resp-status idle';
    el.respStatus.textContent = '未发送';
    el.respMeta.textContent = '';
    el.respBody.hidden = false;
    el.respHeaders.hidden = true;
    el.respBody.className = 'resp-body';
    el.respBody.textContent = text || '发送一个请求后，这里显示响应。';
    state.respRaw = '';
    state.respTab = 'body';
    renderRespTabs();
  }

  function renderResponse(payload) {
    var r = payload.response;
    var ok = r.status >= 200 && r.status < 300;
    var cls = ok ? 'ok' : (r.status >= 400 ? 'err' : 'warn');
    el.respStatus.className = 'resp-status ' + cls;
    el.respStatus.textContent = String(r.status || 0) + (r.statusText ? ' ' + r.statusText : '');

    var meta = (r.durationMs || 0) + ' ms · ' + formatBytes(r.size);
    if (r.truncated) meta += ' · 响应已截断';
    if (r.binary) meta += ' · 二进制';
    if (payload.warnings && payload.warnings.length) meta += ' · 未解析变量: ' + payload.warnings.join(', ');
    el.respMeta.textContent = meta;

    el.respBody.className = 'resp-body';
    el.respBody.textContent = r.binary ? String(r.body || '[二进制内容]') : prettyBody(r.body);
    state.respRaw = String(r.body == null ? '' : r.body);

    renderResponseHeaders(r.headers);
    state.respTab = 'body';
    renderRespTabs();
  }

  function prettyBody(body) {
    var text = String(body == null ? '' : body);
    var trimmed = text.trim();
    if (!trimmed || (trimmed.charAt(0) !== '{' && trimmed.charAt(0) !== '[')) return text || '(空响应体)';
    try {
      return JSON.stringify(JSON.parse(trimmed), null, 2);
    } catch (e) {
      return text;
    }
  }

  function renderResponseHeaders(headers) {
    el.respHeaders.textContent = '';
    var keys = Object.keys(headers || {}).sort();
    if (!keys.length) {
      el.respHeaders.appendChild(h('div', 'side-empty', '没有响应头'));
      return;
    }
    keys.forEach(function (key) {
      var row = h('div', 'header-row');
      row.appendChild(h('span', 'hk', key));
      row.appendChild(h('span', '', String(headers[key])));
      el.respHeaders.appendChild(row);
    });
  }

  // ---------- actions ----------

  function loadRequest(id, force) {
    var found = findRequestById(id);
    if (!found) {
      toast('请求不存在');
      return;
    }
    if (!force && state.dirty && !confirm('当前请求有未保存修改，切换会丢失。继续？')) return;
    state.draft = clone(found.request);
    state.draft._collectionId = found.collection.id;
    state.selectedId = id;
    state.dirty = false;
    state.reqTab = 'params';
    renderEditor();
    renderSidebar();
    renderResponseEmpty('发送一个请求后，这里显示响应。');
  }

  function loadHistory(item) {
    if (item.requestId) {
      var found = findRequestById(item.requestId);
      if (found) {
        loadRequest(item.requestId);
        return;
      }
    }
    if (state.dirty && !confirm('当前请求有未保存修改，切换会丢失。继续？')) return;
    var draft = blankRequest(defaultCollectionId());
    draft.method = item.method || 'GET';
    draft.url = item.url || '';
    draft.name = (item.name || item.url || '历史请求') + '（历史）';
    state.draft = draft;
    state.selectedId = '';
    state.dirty = false;
    renderEditor();
    renderSidebar();
    toast('历史记录已填入编辑器，保存后会进入集合');
  }

  function startNewRequest(collectionId, folderId) {
    if (state.dirty && !confirm('当前请求有未保存修改，新建会丢失。继续？')) return;
    state.draft = blankRequest(collectionId || defaultCollectionId(), folderId || '');
    state.selectedId = '';
    state.dirty = false;
    state.reqTab = 'params';
    renderEditor();
    renderSidebar();
    renderResponseEmpty('发送一个请求后，这里显示响应。');
    el.url.focus();
  }

  function saveCurrent() {
    if (!state.draft) return;
    var input = clone(state.draft);
    input._collectionId = state.draft._collectionId;
    efto.ipcMessage('save-request', {
      collectionId: input._collectionId,
      folderId: input.folderId || '',
      request: input
    });
  }

  function sendRequest() {
    if (!state.draft || state.busy) return;
    if (!String(state.draft.url || '').trim()) {
      toast('请先填写 URL');
      el.url.focus();
      return;
    }
    var input = clone(state.draft);
    delete input._collectionId;
    state.busy = true;
    el.btnSend.disabled = true;
    el.btnSend.textContent = '发送中…';
    el.respStatus.className = 'resp-status idle';
    el.respStatus.textContent = '发送中…';
    el.respMeta.textContent = '';
    efto.ipcMessage('send', { input: input, requestId: state.draft.id || '' });
  }

  function createCollection() {
    var name = prompt('新集合名称');
    if (name === null) return;
    name = name.trim();
    if (!name) {
      toast('集合名称不能为空');
      return;
    }
    efto.ipcMessage('save-collection', { collection: { name: name } });
  }

  function renameCollection(col) {
    var name = prompt('集合名称', col.name);
    if (name === null) return;
    name = name.trim();
    if (!name) return;
    efto.ipcMessage('save-collection', { collection: { id: col.id, name: name, description: col.description } });
  }

  function deleteCollection(col) {
    if (!confirm('删除集合「' + col.name + '」及其中 ' + col.requests.length + ' 个请求？')) return;
    efto.ipcMessage('delete-collection', { id: col.id });
  }

  function createFolder(col, parentId) {
    var name = prompt('新分组名称');
    if (name === null) return;
    name = name.trim();
    if (!name) {
      toast('分组名称不能为空');
      return;
    }
    efto.ipcMessage('save-folder', { collectionId: col.id, parentId: parentId || '', name: name });
  }

  function renameFolder(col, folder) {
    var name = prompt('分组名称', folder.name);
    if (name === null) return;
    name = name.trim();
    if (!name || name === folder.name) return;
    efto.ipcMessage('save-folder', { collectionId: col.id, folderId: folder.id, name: name });
  }

  function deleteFolder(col, folder) {
    var n = countFolderRequests(col, folder.id);
    if (!confirm('删除分组「' + folder.name + '」及其中 ' + n + ' 个请求？')) return;
    efto.ipcMessage('delete-folder', { collectionId: col.id, id: folder.id });
  }

  function deleteRequest(req) {
    if (!confirm('删除请求「' + (req.name || '未命名请求') + '」？')) return;
    efto.ipcMessage('delete-request', { id: req.id });
  }

  // ---------- messages ----------

  function onState(data) {
    if (!data || !data.workspace) return;
    state.ws = data.workspace;

    if (!state.draft) {
      var first = firstRequest();
      if (first) loadRequest(first.request.id, true);
      else {
        state.draft = blankRequest('');
        renderEditor();
      }
    } else if (state.draft.id && !findRequestById(state.draft.id)) {
      var f = firstRequest();
      if (f) loadRequest(f.request.id, true);
      else {
        state.draft = blankRequest('');
        state.selectedId = '';
        state.dirty = false;
        renderEditor();
        renderResponseEmpty();
      }
    }

    renderSidebar();
    renderEnvSelect();
  }

  function onSendResult(data) {
    state.busy = false;
    el.btnSend.disabled = false;
    el.btnSend.textContent = '发送';

    if (!data || data.ok === false) {
      var msg = (data && (data.error || (data.response && data.response.error))) || '请求失败';
      el.respStatus.className = 'resp-status err';
      el.respStatus.textContent = '请求失败';
      el.respMeta.textContent = data && data.request ? (data.request.method + ' ' + data.request.url) : '';
      el.respBody.hidden = false;
      el.respHeaders.hidden = true;
      el.respBody.className = 'resp-body error';
      el.respBody.textContent = msg;
      state.respRaw = msg;
      state.respTab = 'body';
      renderRespTabs();
      toast('请求失败');
      return;
    }

    state.response = data.response;
    renderResponse(data);
    toast('收到响应');
  }

  function onSaved(data) {
    if (!data || !state.draft) return;
    state.draft.id = data.id;
    state.draft._collectionId = data.collectionId || state.draft._collectionId;
    if (data.folderId !== undefined) state.draft.folderId = data.folderId;
    state.selectedId = data.id;
    state.dirty = false;
    var found = findRequestById(data.id);
    if (found) {
      state.draft.name = found.request.name;
      el.reqName.value = found.request.name;
    }
    refreshDirty();
    renderSidebar();
    toast('已保存');
  }

  function onDeleted(data) {
    renderSidebar();
    if (data && data.folderId) toast('分组已删除');
    else if (data && data.collectionId) toast('集合已删除');
  }

  function onImportResult(data) {
    if (!data || !data.ok) {
      var retry = el.modalRoot.querySelector('.import-url button');
      if (retry) {
        retry.disabled = false;
        retry.textContent = '从 Swagger 导入';
      }
      toast('导入失败: ' + ((data && data.error) || '未知错误'));
      return;
    }
    closeModal();
    var info = data.data || {};
    toast('导入完成：' + (info.addedRequests || 0) + ' 个请求 · ' + (info.addedEnvironments || 0) + ' 个环境');
  }

  function onExportResult(data) {
    if (!data || !data.ok) {
      toast('导出失败: ' + ((data && data.error) || '未知错误'));
      return;
    }
    showExportText(data);
  }

  // ---------- modals ----------

  function showModal(card) {
    el.modalRoot.hidden = false;
    el.modalRoot.textContent = '';
    el.modalRoot.appendChild(card);
  }

  function closeModal() {
    el.modalRoot.hidden = true;
    el.modalRoot.textContent = '';
  }

  function openImportModal() {
    var card = h('div', 'modal-card');
    card.appendChild(h('h2', '', '导入 API'));
    card.appendChild(h('p', 'modal-sub', '支持 Postman Collection v2.1、Postman 环境、本插件导出的 JSON，以及 Swagger/OpenAPI 地址（含 knife4j 的 doc.html）。'));

    var targetRow = h('div', 'import-target');
    var targetSelect = document.createElement('select');
    var autoOpt = document.createElement('option');
    autoOpt.value = '';
    autoOpt.textContent = '自动：按来源创建/合并集合';
    targetSelect.appendChild(autoOpt);
    (state.ws ? state.ws.collections : []).forEach(function (col) {
      var opt = document.createElement('option');
      opt.value = col.id;
      opt.textContent = '导入到：' + col.name;
      targetSelect.appendChild(opt);
    });
    var newOpt = document.createElement('option');
    newOpt.value = '__new__';
    newOpt.textContent = '＋ 新建集合…';
    targetSelect.appendChild(newOpt);
    targetRow.appendChild(targetSelect);
    var newNameInput = textInput('', '新集合名称');
    newNameInput.hidden = true;
    targetRow.appendChild(newNameInput);
    targetSelect.addEventListener('change', function () {
      newNameInput.hidden = targetSelect.value !== '__new__';
      if (!newNameInput.hidden) newNameInput.focus();
    });
    card.appendChild(targetRow);

    function targetPayload() {
      if (targetSelect.value === '__new__') {
        var name = newNameInput.value.trim();
        if (!name) { toast('请输入新集合名称'); newNameInput.focus(); return null; }
        return { collection: name };
      }
      return targetSelect.value ? { collectionId: targetSelect.value } : {};
    }

    var urlRow = h('div', 'import-url');
    var urlInput = textInput('', 'Swagger 地址，如 http://10.10.58.153:32300/doc.html');
    urlRow.appendChild(urlInput);
    var urlBtn = button('从 Swagger 导入', 'primary');
    urlBtn.addEventListener('click', function () {
      var url = urlInput.value.trim();
      if (!url) { toast('请输入 Swagger 地址'); urlInput.focus(); return; }
      var target = targetPayload();
      if (!target) return;
      urlBtn.disabled = true;
      urlBtn.textContent = '抓取中…';
      toast('正在读取 Swagger…');
      efto.ipcMessage('import-url', {
        url: url,
        collection: target.collection,
        collectionId: target.collectionId
      });
    });
    urlRow.appendChild(urlBtn);
    card.appendChild(urlRow);

    var area = document.createElement('textarea');
    area.placeholder = '把 Postman Collection / Environment 或本插件导出的 JSON 粘贴到这里…';
    card.appendChild(area);

    var foot = h('div', 'modal-foot');
    var fileBtn = button('选择文件…', 'ghost left');
    fileBtn.addEventListener('click', function () {
      if (!efto.pickFiles) { toast('当前宿主不支持选择文件'); return; }
      var target = targetPayload();
      if (!target) return;
      efto.pickFiles().then(function (paths) {
        var path = paths && paths[0];
        if (!path) return;
        toast('正在读取文件…');
        efto.ipcMessage('import-file', {
          path: path,
          collection: target.collection,
          collectionId: target.collectionId
        });
      }).catch(function (e) {
        toast('选择文件失败: ' + (e && e.message ? e.message : e));
      });
    });
    foot.appendChild(fileBtn);

    var cancel = button('取消', 'ghost');
    cancel.addEventListener('click', closeModal);
    foot.appendChild(cancel);

    var ok = button('导入', 'primary');
    ok.addEventListener('click', function () {
      if (!area.value.trim()) { toast('请粘贴 JSON 或选择文件'); return; }
      var target = targetPayload();
      if (!target) return;
      efto.ipcMessage('import', {
        text: area.value,
        collection: target.collection,
        collectionId: target.collectionId
      });
    });
    foot.appendChild(ok);
    card.appendChild(foot);
    showModal(card);
    area.focus();
  }

  function showExportText(data) {
    var card = h('div', 'modal-card');
    card.appendChild(h('h2', '', '导出：' + (data.name || 'API')));
    card.appendChild(h('p', 'modal-sub', data.format === 'postman'
      ? 'Postman Collection v2.1，可直接导入 Postman。'
      : '本插件原生格式，包含集合、环境与当前环境设置。'));

    var area = document.createElement('textarea');
    area.value = data.text || '';
    area.readOnly = true;
    card.appendChild(area);

    var foot = h('div', 'modal-foot');
    var copy = button('复制 JSON', 'primary');
    copy.addEventListener('click', function () {
      efto.copyText(area.value);
      toast('已复制');
    });
    foot.appendChild(copy);
    var close = button('关闭', 'ghost');
    close.addEventListener('click', closeModal);
    foot.appendChild(close);
    card.appendChild(foot);
    showModal(card);
  }

  function openExportModal() {
    var card = h('div', 'modal-card');
    card.appendChild(h('h2', '', '导出 API'));
    card.appendChild(h('p', 'modal-sub', '导出全部工作区（原生格式），或把某个集合导出为 Postman v2.1。'));

    var select = document.createElement('select');
    var all = document.createElement('option');
    all.value = '';
    all.textContent = '全部工作区（原生 JSON）';
    select.appendChild(all);
    (state.ws ? state.ws.collections : []).forEach(function (col) {
      var opt = document.createElement('option');
      opt.value = col.id;
      opt.textContent = col.name + '（Postman 集合）';
      select.appendChild(opt);
    });
    select.style.width = '100%';
    card.appendChild(select);

    var foot = h('div', 'modal-foot');
    var cancel = button('取消', 'ghost');
    cancel.addEventListener('click', closeModal);
    foot.appendChild(cancel);
    var go = button('生成', 'primary');
    go.addEventListener('click', function () {
      efto.ipcMessage('export', { collectionId: select.value });
    });
    foot.appendChild(go);
    card.appendChild(foot);
    showModal(card);
  }

  function openEnvModal() {
    var envs = clone((state.ws && state.ws.environments) || []);
    var activeId = (state.ws && state.ws.activeEnvId) || '';
    var card = h('div', 'modal-card');
    card.appendChild(h('h2', '', '环境变量'));
    card.appendChild(h('p', 'modal-sub', '在 URL、Header、Body、Auth 中用 {{变量名}} 引用；发送时按当前环境替换。'));

    var body = h('div', '');
    card.appendChild(body);

    function render() {
      body.textContent = '';
      if (!envs.length) body.appendChild(h('div', 'side-empty', '还没有环境，点下方「新建环境」。'));

      envs.forEach(function (env, index) {
        var box = h('div', 'env-card');
        var head = h('div', 'env-card-head');

        var name = textInput(env.name, '环境名称');
        name.addEventListener('input', function () { env.name = name.value; });
        head.appendChild(name);

        if (env.id === activeId) {
          head.appendChild(h('span', 'active-tag', '当前'));
        } else {
          var use = button('设为当前', 'ghost');
          use.addEventListener('click', function () { activeId = env.id; render(); });
          head.appendChild(use);
        }

        var del = button('删除', 'ghost');
        del.addEventListener('click', function () {
          if (!confirm('删除环境「' + (env.name || '未命名') + '」？')) return;
          envs.splice(index, 1);
          if (activeId === env.id) activeId = envs.length ? envs[0].id : '';
          render();
        });
        head.appendChild(del);
        box.appendChild(head);

        if (!Array.isArray(env.vars)) env.vars = [];
        var kv = h('div', 'kv');
        box.appendChild(kv);
        renderKv(kv, env.vars, function () {});
        body.appendChild(box);
      });

      var add = button('＋ 新建环境', 'kv-add');
      add.addEventListener('click', function () {
        envs.push({ id: '', name: '新环境', vars: [{ key: '', value: '', enabled: true }] });
        render();
      });
      body.appendChild(add);
    }

    render();

    var foot = h('div', 'modal-foot');
    var cancel = button('取消', 'ghost');
    cancel.addEventListener('click', closeModal);
    foot.appendChild(cancel);
    var save = button('保存', 'primary');
    save.addEventListener('click', function () {
      efto.ipcMessage('env-save-all', { environments: envs, activeEnvId: activeId });
    });
    foot.appendChild(save);
    card.appendChild(foot);
    showModal(card);
  }

  // ---------- bindings ----------

  function bind() {
    el.method.addEventListener('change', function () {
      state.draft.method = el.method.value;
      markDirty();
    });
    el.url.addEventListener('input', function () {
      state.draft.url = el.url.value;
      markDirty();
    });
    el.reqName.addEventListener('input', function () {
      state.draft.name = el.reqName.value;
      markDirty();
    });

    el.bodyMode.addEventListener('change', function () {
      state.draft.body.mode = el.bodyMode.value;
      if (!state.draft.body.raw) state.draft.body.raw = '';
      if (!Array.isArray(state.draft.body.form)) state.draft.body.form = [];
      renderBody();
      markDirty();
    });
    el.bodyRaw.addEventListener('input', function () {
      state.draft.body.raw = el.bodyRaw.value;
      markDirty();
    });

    el.authType.addEventListener('change', function () {
      state.draft.auth.type = el.authType.value;
      renderAuth();
      markDirty();
    });
    el.authToken.addEventListener('input', function () {
      state.draft.auth.token = el.authToken.value;
      markDirty();
    });
    el.authUser.addEventListener('input', function () {
      state.draft.auth.username = el.authUser.value;
      markDirty();
    });
    el.authPass.addEventListener('input', function () {
      state.draft.auth.password = el.authPass.value;
      markDirty();
    });

    el.btnSend.addEventListener('click', sendRequest);
    el.btnSaveCurrent.addEventListener('click', function () { saveCurrent(); });
    el.btnNewRequest.addEventListener('click', function () { startNewRequest(''); });
    el.btnNewCollection.addEventListener('click', createCollection);
    el.btnEnvEdit.addEventListener('click', openEnvModal);
    el.btnImport.addEventListener('click', openImportModal);
    el.btnExport.addEventListener('click', openExportModal);

    el.envSelect.addEventListener('change', function () {
      efto.ipcMessage('env-use', { id: el.envSelect.value });
    });

    el.reqTabs.addEventListener('click', function (e) {
      var btn = e.target.closest('button[data-tab]');
      if (!btn) return;
      state.reqTab = btn.getAttribute('data-tab');
      renderReqTabs();
    });

    el.respActions.addEventListener('click', function (e) {
      var btn = e.target.closest('button[data-resp]');
      if (!btn) return;
      state.respTab = btn.getAttribute('data-resp');
      renderRespTabs();
    });

    el.sideTabs.addEventListener('click', function (e) {
      var btn = e.target.closest('button[data-side]');
      if (!btn) return;
      state.sideTab = btn.getAttribute('data-side');
      var tabs = el.sideTabs.querySelectorAll('button[data-side]');
      for (var i = 0; i < tabs.length; i++) {
        tabs[i].classList.toggle('active', tabs[i].getAttribute('data-side') === state.sideTab);
      }
      renderSidebar();
    });

    el.btnCopyResponse.addEventListener('click', function () {
      var text = state.respTab === 'headers' && state.response
        ? JSON.stringify(state.response.headers || {}, null, 2)
        : state.respRaw;
      if (!text) { toast('没有可复制的内容'); return; }
      efto.copyText(text);
      toast('已复制');
    });

    el.modalRoot.addEventListener('click', function (e) {
      if (e.target === el.modalRoot) closeModal();
    });

    document.addEventListener('keydown', function (e) {
      var mod = e.ctrlKey || e.metaKey;
      if (e.key === 'Escape' && !el.modalRoot.hidden) {
        closeModal();
        return;
      }
      if (mod && e.key === 'Enter') {
        e.preventDefault();
        sendRequest();
      } else if (mod && (e.key === 's' || e.key === 'S')) {
        e.preventDefault();
        saveCurrent();
      }
    });
  }

  function cacheEls() {
    el.sideList = $('side-list');
    el.sideTabs = document.querySelector('.side-tabs');
    el.method = $('method');
    el.url = $('url');
    el.reqName = $('req-name');
    el.dirty = $('dirty');
    el.envSelect = $('env-select');
    el.btnSend = $('btn-send');
    el.btnSaveCurrent = $('btn-save-current');
    el.btnNewRequest = $('btn-new-request');
    el.btnNewCollection = $('btn-new-collection');
    el.btnEnvEdit = $('btn-env-edit');
    el.btnImport = $('btn-import');
    el.btnExport = $('btn-export');
    el.reqTabs = $('req-tabs');
    el.paneParams = $('pane-params');
    el.paneHeaders = $('pane-headers');
    el.paneBody = $('pane-body');
    el.paneAuth = $('pane-auth');
    el.kvParams = $('kv-params');
    el.kvHeaders = $('kv-headers');
    el.kvBodyForm = $('kv-body-form');
    el.bodyMode = $('body-mode');
    el.bodyForm = $('body-form');
    el.bodyRaw = $('body-raw');
    el.bodyHint = $('body-hint');
    el.authType = $('auth-type');
    el.authBearer = $('auth-bearer');
    el.authBasic = $('auth-basic');
    el.authToken = $('auth-token');
    el.authUser = $('auth-user');
    el.authPass = $('auth-pass');
    el.countParams = $('count-params');
    el.countHeaders = $('count-headers');
    el.respStatus = $('resp-status');
    el.respMeta = $('resp-meta');
    el.respBody = $('resp-body');
    el.respHeaders = $('resp-headers');
    el.respActions = document.querySelector('.resp-actions');
    el.btnCopyResponse = $('btn-copy-response');
    el.modalRoot = $('modal-root');
    el.toast = $('toast');
  }

  function init() {
    cacheEls();
    if (!efto) {
      document.body.innerHTML = '<div class="empty-main">请在 efto 中通过「api」命令打开此页面。</div>';
      return;
    }

    efto.onMessage('state', onState);
    efto.onMessage('select', function (data) {
      if (data && data.id) loadRequest(data.id, true);
    });
    efto.onMessage('send-result', onSendResult);
    efto.onMessage('saved', onSaved);
    efto.onMessage('deleted', onDeleted);
    efto.onMessage('error', function (data) {
      toast((data && data.message) || '操作失败');
    });
    efto.onMessage('import-result', onImportResult);
    efto.onMessage('export-result', onExportResult);
    efto.onMessage('collection-saved', function () { toast('集合已保存'); });
    efto.onMessage('folder-saved', function () { toast('分组已保存'); });
    efto.onMessage('env-saved', function (data) {
      if (data) state.ws.activeEnvId = data.activeEnvId;
      closeModal();
      toast('环境已保存');
    });

    bind();
    renderResponseEmpty();
    efto.ipcMessage('init');
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
