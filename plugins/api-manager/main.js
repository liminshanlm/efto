'use strict';
// efto API 管理插件主进程。
// 职责：工作区持久化、UI IPC、命令入口、启动器搜索项。

var model = require('./lib/model');
var commands = require('./lib/commands');
var request = require('./lib/request');
var transfer = require('./lib/transfer');
var http = require('./lib/http');
var swagger = require('./lib/swagger');
var format = require('./lib/format');

var workspace = null;

function ensure() {
  if (!workspace) workspace = model.load();
  return workspace;
}

function persist() {
  model.save(workspace);
  try {
    syncSearchItems();
  } catch (e) {
    console.error('同步启动器搜索项失败: ' + (e && e.message ? e.message : e));
  }
}

function syncSearchItems() {
  var items = [];
  for (var i = 0; i < workspace.collections.length; i++) {
    var col = workspace.collections[i];
    for (var j = 0; j < col.requests.length; j++) {
      if (items.length >= 200) break;
      var req = col.requests[j];
      var folder = model.folderPath(col, req.folderId);
      var title = (folder ? folder + ' / ' : '') + (req.name || '未命名请求') + ' · ' + req.method;
      if (title.length > 64) title = title.slice(0, 64);
      var cmd = 'api:open:' + req.id;
      if (cmd.length > 128) continue;
      items.push({ title: title, cmd: cmd });
    }
  }
  efto.setSearchItems(items);
}

function pushState() {
  efto.ipcMessage('state', { workspace: workspace });
}

function openUi(id) {
  try {
    efto.showUI({ title: 'API 管理', width: 1180, height: 780 });
    if (id) efto.ipcMessage('select', { id: id });
  } catch (e) {
    console.error('打开 UI 失败: ' + errorText(e));
  }
}

function errorText(e) {
  return e && e.message ? e.message : String(e);
}

function findAnyRequest(id) {
  if (!id) return null;
  for (var i = 0; i < workspace.collections.length; i++) {
    var col = workspace.collections[i];
    for (var j = 0; j < col.requests.length; j++) {
      if (col.requests[j].id === id) return { collection: col, request: col.requests[j] };
    }
  }
  return null;
}

function findCollectionById(id) {
  if (!id) return null;
  for (var i = 0; i < workspace.collections.length; i++) {
    if (workspace.collections[i].id === id) return workspace.collections[i];
  }
  return null;
}

function ensureCollectionFor(id, name) {
  var col = findCollectionById(id);
  if (col) return col;
  if (name) {
    for (var i = 0; i < workspace.collections.length; i++) {
      if (workspace.collections[i].name.toLowerCase() === String(name).toLowerCase()) {
        return workspace.collections[i];
      }
    }
    col = model.normCollection({ name: name });
    workspace.collections.push(col);
    return col;
  }
  return model.ensureCollection(workspace, '');
}

// ---------- UI 消息 ----------

function handleUiSend(data) {
  var input = data && data.input ? data.input : {};
  var requestId = String((data && data.requestId) || input.id || '');
  request.execute(workspace, input, { env: workspace.activeEnvId }).then(function (r) {
    var d = r.data;
    model.pushHistory(workspace, {
      requestId: requestId,
      name: input.name || '临时请求',
      method: r.request.method,
      url: d.url || r.request.url,
      status: d.status,
      statusText: d.statusText,
      durationMs: d.durationMs,
      size: d.size,
      ok: d.ok !== false
    });
    persist();
    pushState();
    efto.ipcMessage('send-result', {
      requestId: requestId,
      ok: d.ok !== false,
      response: d,
      request: r.request,
      warnings: r.warnings,
      env: r.envName,
      error: d.ok === false ? (d.error || '请求失败') : ''
    });
  }).catch(function (e) {
    efto.ipcMessage('send-result', {
      requestId: requestId,
      ok: false,
      error: errorText(e)
    });
  });
}

function handleSaveRequest(data) {
  var input = data && data.request ? data.request : {};
  var targetCol = ensureCollectionFor(
    (data && data.collectionId) || input._collectionId || input.collectionId || '',
    input.collection || ''
  );
  var existing = findAnyRequest(input.id);
  var normed = model.normRequest(input);
  normed.folderId = '';
  var wantedFolder = String((data && data.folderId) || input.folderId || '');
  if (wantedFolder && (targetCol.folders || []).some(function (f) { return f.id === wantedFolder; })) {
    normed.folderId = wantedFolder;
  }

  if (existing) {
    normed.id = existing.request.id;
    normed.createdAt = existing.request.createdAt;
    if (existing.collection.id === targetCol.id) {
      var idx = existing.collection.requests.indexOf(existing.request);
      existing.collection.requests[idx] = normed;
    } else {
      var oldIdx = existing.collection.requests.indexOf(existing.request);
      if (oldIdx >= 0) existing.collection.requests.splice(oldIdx, 1);
      targetCol.requests.push(normed);
      existing.collection.updatedAt = new Date().toISOString();
    }
  } else {
    // 新请求：忽略客户端可能带来的重复 id
    normed = model.normRequest(input, collectExistingRequestIds());
    targetCol.requests.push(normed);
  }
  targetCol.updatedAt = new Date().toISOString();
  persist();
  pushState();
  efto.ipcMessage('saved', { id: normed.id, collectionId: targetCol.id, folderId: normed.folderId || '' });
}

function collectExistingRequestIds() {
  var ids = {};
  workspace.collections.forEach(function (col) {
    (col.folders || []).forEach(function (f) { ids[f.id] = true; });
    col.requests.forEach(function (req) { ids[req.id] = true; });
  });
  return ids;
}

function handleDeleteRequest(data) {
  var found = findAnyRequest(data && data.id);
  if (!found) {
    efto.ipcMessage('error', { message: '请求不存在' });
    return;
  }
  var idx = found.collection.requests.indexOf(found.request);
  if (idx >= 0) found.collection.requests.splice(idx, 1);
  persist();
  pushState();
  efto.ipcMessage('deleted', { id: found.request.id });
}

function handleSaveCollection(data) {
  var input = data && data.collection ? data.collection : {};
  var existing = findCollectionById(input.id);
  if (existing) {
    existing.name = String(input.name || '').trim() || existing.name;
    existing.description = String(input.description == null ? existing.description : input.description);
    existing.updatedAt = new Date().toISOString();
  } else {
    var col = model.normCollection({ name: input.name, description: input.description });
    workspace.collections.push(col);
    existing = col;
  }
  persist();
  pushState();
  efto.ipcMessage('collection-saved', { id: existing.id });
}

function handleDeleteCollection(data) {
  var col = findCollectionById(data && data.id);
  if (!col) {
    efto.ipcMessage('error', { message: '集合不存在' });
    return;
  }
  var idx = workspace.collections.indexOf(col);
  if (idx >= 0) workspace.collections.splice(idx, 1);
  persist();
  pushState();
  efto.ipcMessage('deleted', { collectionId: col.id });
}

function findFolderById(col, id) {
  if (!id) return null;
  for (var i = 0; i < (col.folders || []).length; i++) {
    if (col.folders[i].id === id) return col.folders[i];
  }
  return null;
}

function handleSaveFolder(data) {
  var col = findCollectionById(data && data.collectionId);
  if (!col) {
    efto.ipcMessage('error', { message: '集合不存在' });
    return;
  }
  var name = String((data && data.name) || '').trim();
  if (!name) {
    efto.ipcMessage('error', { message: '分组名称不能为空' });
    return;
  }
  var existing = findFolderById(col, data && data.folderId);
  var parentId = existing ? String(existing.parentId || '') : String((data && data.parentId) || '');
  if (parentId && !findFolderById(col, parentId)) parentId = '';
  var dup = (col.folders || []).some(function (f) {
    return f !== existing && String(f.parentId || '') === parentId && f.name.toLowerCase() === name.toLowerCase();
  });
  if (dup) {
    efto.ipcMessage('error', { message: '同级已存在同名分组: ' + name });
    return;
  }
  if (existing) {
    existing.name = name;
  } else {
    existing = model.normFolder({ name: name, parentId: parentId });
    col.folders.push(existing);
  }
  col.updatedAt = new Date().toISOString();
  persist();
  pushState();
  efto.ipcMessage('folder-saved', { id: existing.id, collectionId: col.id });
}

function handleDeleteFolder(data) {
  var col = findCollectionById(data && data.collectionId);
  if (!col) {
    efto.ipcMessage('error', { message: '集合不存在' });
    return;
  }
  var folder = findFolderById(col, data && data.id);
  if (!folder) {
    efto.ipcMessage('error', { message: '分组不存在' });
    return;
  }
  var ids = model.descendantFolderIds(col, folder.id);
  col.requests = col.requests.filter(function (r) { return !ids[r.folderId]; });
  col.folders = col.folders.filter(function (f) { return !ids[f.id]; });
  col.updatedAt = new Date().toISOString();
  persist();
  pushState();
  efto.ipcMessage('deleted', { folderId: folder.id, collectionId: col.id });
}

function handleSaveEnvironments(data) {
  var list = data && Array.isArray(data.environments) ? data.environments : [];
  var next = list.map(function (e) { return model.normEnvironment(e); });
  workspace.environments = next;
  var active = String((data && data.activeEnvId) || workspace.activeEnvId || '');
  var activeOk = next.some(function (e) { return e.id === active; });
  workspace.activeEnvId = activeOk ? active : (next.length ? next[0].id : '');
  persist();
  pushState();
  efto.ipcMessage('env-saved', { activeEnvId: workspace.activeEnvId });
}

function findEnvById(id) {
  for (var i = 0; i < workspace.environments.length; i++) {
    if (workspace.environments[i].id === id) return workspace.environments[i];
  }
  return null;
}

function handleEnvUse(data) {
  var id = String((data && data.id) || '');
  if (id && !findEnvById(id)) {
    efto.ipcMessage('error', { message: '环境不存在' });
    return;
  }
  workspace.activeEnvId = id;
  persist();
  pushState();
}

function uiContext() {
  return {
    ws: workspace,
    persist: persist,
    openUi: openUi
  };
}

function importTargetOf(data) {
  return {
    collection: String((data && data.collection) || '').trim(),
    collectionId: String((data && data.collectionId) || '')
  };
}

function handleUiImport(data) {
  var text = String((data && data.text) || '');
  var obj;
  try {
    obj = JSON.parse(text);
  } catch (e) {
    efto.ipcMessage('import-result', { ok: false, error: 'JSON 解析失败: ' + errorText(e) });
    return;
  }
  var res = commands.importData(uiContext(), obj, importTargetOf(data));
  if (!res.result.ok) {
    efto.ipcMessage('import-result', { ok: false, error: res.result.error });
    return;
  }
  pushState();
  efto.ipcMessage('import-result', { ok: true, data: res.result.data });
}

function handleUiImportFile(data) {
  var path = String((data && data.path) || '');
  http.readJsonText(path).then(function (text) {
    var obj;
    try {
      obj = JSON.parse(text);
    } catch (e) {
      throw new Error('文件不是合法 JSON: ' + errorText(e));
    }
    var res = commands.importData(uiContext(), obj, importTargetOf(data));
    if (!res.result.ok) throw new Error(res.result.error);
    pushState();
    efto.ipcMessage('import-result', { ok: true, data: res.result.data });
  }).catch(function (e) {
    efto.ipcMessage('import-result', { ok: false, error: errorText(e) });
  });
}

function handleUiImportUrl(data) {
  var url = String((data && data.url) || '').trim();
  if (!url) {
    efto.ipcMessage('import-result', { ok: false, error: '请输入 Swagger 地址' });
    return;
  }
  swagger.importFromUrl(url).then(function (parsed) {
    var res = commands.applyParsed(uiContext(), parsed, importTargetOf(data));
    if (!res.result.ok) throw new Error(res.result.error);
    pushState();
    efto.ipcMessage('import-result', { ok: true, data: res.result.data });
  }).catch(function (e) {
    efto.ipcMessage('import-result', { ok: false, error: errorText(e) });
  });
}

function handleUiExport(data) {
  var collectionId = String((data && data.collectionId) || '');
  try {
    if (collectionId) {
      var col = findCollectionById(collectionId);
      if (!col) throw new Error('集合不存在');
      efto.ipcMessage('export-result', {
        ok: true,
        format: 'postman',
        name: col.name,
        text: JSON.stringify(transfer.exportPostmanCollection(col), null, 2)
      });
    } else {
      efto.ipcMessage('export-result', {
        ok: true,
        format: 'native',
        name: 'efto-api-workspace',
        text: JSON.stringify(transfer.exportNative(workspace), null, 2)
      });
    }
  } catch (e) {
    efto.ipcMessage('export-result', { ok: false, error: errorText(e) });
  }
}

function handleUiMessage(msg) {
  var name = msg && msg.name;
  var data = (msg && msg.data) || {};
  switch (name) {
    case 'init':
      pushState();
      break;
    case 'send':
      handleUiSend(data);
      break;
    case 'save-request':
      handleSaveRequest(data);
      break;
    case 'delete-request':
      handleDeleteRequest(data);
      break;
    case 'save-collection':
      handleSaveCollection(data);
      break;
    case 'delete-collection':
      handleDeleteCollection(data);
      break;
    case 'save-folder':
      handleSaveFolder(data);
      break;
    case 'delete-folder':
      handleDeleteFolder(data);
      break;
    case 'env-save-all':
      handleSaveEnvironments(data);
      break;
    case 'env-use':
      handleEnvUse(data);
      break;
    case 'import':
      handleUiImport(data);
      break;
    case 'import-file':
      handleUiImportFile(data);
      break;
    case 'import-url':
      handleUiImportUrl(data);
      break;
    case 'export':
      handleUiExport(data);
      break;
    case 'clear-history':
      workspace.history = [];
      persist();
      pushState();
      break;
    default:
      efto.ipcMessage('error', { message: '未知 UI 消息: ' + name });
  }
}

// ---------- 命令 ----------

function handleCommand(payload) {
  var ctx = {
    ws: workspace,
    persist: persist,
    openUi: openUi
  };
  Promise.resolve(commands.run(payload, ctx)).then(function (res) {
    format.emit(res);
  }).catch(function (e) {
    format.emit(format.fail('command', errorText(e)));
  });
}

// ---------- 生命周期 ----------

efto.onPluginEnter(function (info) {
  ensure();
  try {
    syncSearchItems();
  } catch (e) {
    console.error('同步启动器搜索项失败: ' + errorText(e));
  }

  var code = String((info && info.code) || '');
  var payload = info && typeof info.payload === 'string' ? info.payload.trim() : '';

  if (code.indexOf('api:open:') === 0) {
    openUi(code.slice('api:open:'.length));
    return;
  }
  if (!payload || payload === 'ui' || payload === 'open') {
    openUi('');
    return;
  }
  handleCommand(payload);
});

efto.onMainMessage(function (msg) {
  try {
    ensure();
    handleUiMessage(msg);
  } catch (e) {
    console.error('处理 UI 消息失败: ' + errorText(e));
    efto.ipcMessage('error', { message: errorText(e) });
  }
});

efto.onUiClosed(function () {
  // UI 关闭后主进程仍保留工作区，下次打开直接复用
});
