'use strict';
// 命令层：文本命令与 JSON 命令共用同一套处理函数。
// 文本面向人（启动器输入框），JSON 面向 agent（cli plugin-run）。

var u = require('./util');
var model = require('./model');
var request = require('./request');
var transfer = require('./transfer');
var http = require('./http');
var swagger = require('./swagger');
var format = require('./format');

var ALLOWED_PATCH = ['name', 'method', 'url', 'params', 'headers', 'body', 'auth', 'timeoutMs', 'folderId'];

function collectIds(ws) {
  var ids = {};
  ws.collections.forEach(function (c) {
    ids[c.id] = true;
    (c.folders || []).forEach(function (f) { ids[f.id] = true; });
    c.requests.forEach(function (r) { ids[r.id] = true; });
  });
  ws.environments.forEach(function (e) { ids[e.id] = true; });
  return ids;
}

function requestSummary(req, col) {
  return {
    id: req.id,
    name: req.name,
    method: req.method,
    url: req.url,
    collection: col.name,
    collectionId: col.id,
    folder: model.folderPath(col, req.folderId),
    folderId: req.folderId || '',
    headerCount: req.headers.filter(function (h) { return h.enabled !== false && h.key; }).length,
    paramCount: req.params.filter(function (p) { return p.enabled !== false && p.key; }).length,
    bodyMode: req.body.mode
  };
}

function sendRows(requestMeta, data) {
  var rows = [];
  if (data.ok === false) {
    rows.push({
      title: '请求失败',
      subtitle: String(data.error || '未知错误'),
      copy: String(data.error || '未知错误')
    });
    return rows;
  }
  rows.push({
    title: String(data.status || 0) + (data.statusText ? ' ' + data.statusText : '') + ' · ' + (data.durationMs || 0) + ' ms',
    subtitle: u.formatBytes(data.size) + ' · ' + requestMeta.method + ' ' + (data.url || requestMeta.url),
    copy: String(data.body == null ? '' : data.body)
  });
  rows.push({
    title: '复制完整结果 JSON',
    subtitle: '状态码 / 响应头 / 响应体 / 耗时' + (data.truncated ? '（响应已截断）' : ''),
    copy: JSON.stringify({ request: requestMeta, response: data })
  });
  if (data.warnings && data.warnings.length) {
    rows.push({
      title: '未解析的变量',
      subtitle: data.warnings.join(', '),
      copy: '未解析的变量: ' + data.warnings.join(', ')
    });
  }
  return rows;
}

function sendResult(requestMeta, r) {
  var data = r.data;
  var payload = {
    request: requestMeta,
    response: data,
    warnings: r.warnings || [],
    env: r.envName || ''
  };
  if (data.ok === false) {
    return {
      result: { ok: false, cmd: 'send', error: data.error || '请求失败', data: payload },
      rows: sendRows(requestMeta, data)
    };
  }
  return format.ok('send', payload, sendRows(requestMeta, data));
}

// ---------- 查询 ----------

function doHelp() {
  var text = [
    'API 管理命令（api）：',
    '  api                       打开图形界面',
    '  api list [关键字]          列出请求',
    '  api show <id|名称>         查看请求 JSON',
    '  api send <id|名称>         发送已保存请求',
    '  api run GET <url>          临时发送（支持 --header K:V / --body / --content-type）',
    '  api add {json}             新增请求',
    '  api update <id> {json}     更新请求字段',
    '  api delete <id>            删除请求',
    '  api collections            列出集合',
    '  api collection add <名称>  新建集合',
    '  api folder list <集合>     列出集合内的分组',
    '  api folder add <集合> <名称>  新建分组（JSON 可用 parent 指定父分组）',
    '  api folder rename <集合> <分组> <新名>  重命名分组',
    '  api folder delete <集合> <分组>         删除分组及其请求',
    '  api env                    列出环境',
    '  api env use <名称>         切换当前环境',
    '  api env add <名称>         新建环境',
    '  api env set <名称> k=v     设置变量（可多个）',
    '  api history [n]            最近请求',
    '  api import {postman}       导入 Postman / 本插件 JSON',
    '  api import --file <path>   从文件导入',
    '  api import --url <swagger> 从 Swagger/OpenAPI 地址导入（如 knife4j 的 doc.html）',
    '  api import ... --collection <集合>  导入到指定集合（不存在则新建）',
    '  api --json-file <path>     从文件读取 JSON 命令（规避 Windows 引号转义）',
    '  api export [集合]          导出（指定集合时是 Postman v2.1）',
    '  api help                   本帮助',
    '',
    'Agent 建议用 JSON 命令：api {"cmd":"send","id":"req-xxx"}',
    '结果行以 EFTO_API 开头；cli plugin-run --json 时从 logs[].text 里取。'
  ].join('\n');
  return format.ok('help', { text: text }, [{ title: 'API 管理命令', subtitle: '回车复制完整帮助', copy: text }]);
}

function doList(ctx, keyword) {
  var kw = u.trim(keyword).toLowerCase();
  var items = [];
  ctx.ws.collections.forEach(function (col) {
    col.requests.forEach(function (req) {
      var folder = model.folderPath(col, req.folderId);
      var hay = (col.name + ' ' + folder + ' ' + req.name + ' ' + req.method + ' ' + req.url).toLowerCase();
      if (kw && hay.indexOf(kw) < 0) return;
      items.push(requestSummary(req, col));
    });
  });
  var rows = items.slice(0, 50).map(function (it) {
    return {
      title: it.method + '  ' + it.name,
      subtitle: it.collection + (it.folder ? ' / ' + it.folder : '') + ' · ' + (it.url || '(空 URL)'),
      copy: 'api send ' + it.id
    };
  });
  if (!rows.length) {
    rows.push({ title: '没有匹配的请求', subtitle: kw ? '关键字: ' + kw : '工作区为空，输入 api 打开界面新建', copy: 'api help' });
  }
  return format.ok('list', { total: items.length, items: items }, rows);
}

function doShow(ctx, ref) {
  var m = model.findRequest(ctx.ws, ref);
  if (m.error) return format.fail('show', m.error);
  var data = u.clone(m.request);
  data.collection = m.collection.name;
  data.collectionId = m.collection.id;
  data.folder = model.folderPath(m.collection, m.request.folderId);
  return format.ok('show', data, [
    { title: m.request.method + '  ' + m.request.name, subtitle: m.collection.name + (data.folder ? ' / ' + data.folder : '') + ' · ' + (m.request.url || '(空 URL)'), copy: JSON.stringify(data) },
    { title: '所在集合', subtitle: m.collection.name, copy: m.collection.id }
  ]);
}

function doCollections(ctx) {
  var items = ctx.ws.collections.map(function (c) {
    return { id: c.id, name: c.name, count: c.requests.length, folderCount: (c.folders || []).length, description: c.description };
  });
  var rows = items.map(function (c) {
    return { title: c.name, subtitle: c.count + ' 个请求 · ' + c.folderCount + ' 个分组 · ' + c.id, copy: c.id };
  });
  if (!rows.length) rows.push({ title: '还没有集合', subtitle: '输入 api 打开界面新建，或 api collection add <名称>', copy: 'api collection add 我的集合' });
  return format.ok('collections', { total: items.length, items: items }, rows);
}

function doHistory(ctx, opts) {
  opts = opts || {};
  var ws = ctx.ws;
  if (opts.clear) {
    ws.history = [];
    ctx.persist();
    return format.ok('history', { cleared: true }, [{ title: '历史已清空', subtitle: '', copy: 'ok' }]);
  }
  var limit = Math.max(1, Math.min(100, Number(opts.limit) || 20));
  var items = ws.history.slice(0, limit).map(function (h) {
    return {
      id: h.id,
      ts: h.ts,
      requestId: h.requestId,
      name: h.name,
      method: h.method,
      url: h.url,
      status: h.status,
      durationMs: h.durationMs,
      ok: h.ok
    };
  });
  var rows = items.map(function (h) {
    var status = h.ok ? String(h.status) : '失败';
    return {
      title: status + '  ' + h.method + ' ' + (h.name || h.url),
      subtitle: h.ts + ' · ' + h.durationMs + ' ms',
      copy: h.requestId ? 'api send ' + h.requestId : h.url
    };
  });
  if (!rows.length) rows.push({ title: '还没有请求历史', subtitle: '发送一次请求后会出现在这里', copy: 'api help' });
  return format.ok('history', { total: ws.history.length, items: items }, rows);
}

// ---------- 发送 ----------

function doSend(ctx, ref, opts) {
  var m = model.findRequest(ctx.ws, ref);
  if (m.error) return format.fail('send', m.error);
  if (!u.trim(m.request.url)) return format.fail('send', '请求没有 URL: ' + m.request.name);

  var meta = {
    id: m.request.id,
    name: m.request.name,
    collection: m.collection.name,
    method: m.request.method,
    url: m.request.url
  };
  return request.execute(ctx.ws, m.request, { env: opts.env, vars: opts.vars }).then(function (r) {
    var data = r.data;
    meta.method = r.request.method;
    meta.url = r.request.url;
    model.pushHistory(ctx.ws, {
      requestId: m.request.id,
      name: m.request.name,
      method: r.request.method,
      url: data.url || r.request.url,
      status: data.status,
      statusText: data.statusText,
      durationMs: data.durationMs,
      size: data.size,
      ok: data.ok !== false
    });
    ctx.persist();
    return sendResult(meta, r);
  }).catch(function (e) {
    return format.fail('send', e && e.message ? e.message : String(e));
  });
}

function doRun(ctx, input, opts) {
  if (!u.trim(input.url)) return format.fail('run', '缺少 URL');
  var meta = {
    id: '',
    name: '临时请求',
    collection: '',
    method: String(input.method || 'GET').toUpperCase(),
    url: input.url
  };
  return request.execute(ctx.ws, input, { env: opts.env, vars: opts.vars }).then(function (r) {
    var data = r.data;
    meta.method = r.request.method;
    meta.url = r.request.url;
    model.pushHistory(ctx.ws, {
      requestId: '',
      name: '临时请求',
      method: r.request.method,
      url: data.url || r.request.url,
      status: data.status,
      statusText: data.statusText,
      durationMs: data.durationMs,
      size: data.size,
      ok: data.ok !== false
    });
    ctx.persist();
    return sendResult(meta, r);
  }).catch(function (e) {
    return format.fail('run', e && e.message ? e.message : String(e));
  });
}

// ---------- 增删改 ----------

function doAdd(ctx, raw, collectionName, folderRef) {
  var obj = raw;
  if (typeof obj === 'string') {
    try {
      obj = JSON.parse(obj);
    } catch (e) {
      return format.fail('add', '请求 JSON 解析失败: ' + e.message);
    }
  }
  if (!u.isObj(obj)) return format.fail('add', '需要一个请求对象或导入 JSON');

  if (Array.isArray(obj.collections) || obj.info || Array.isArray(obj.item) || Array.isArray(obj.values)) {
    return applyImport(ctx, transfer.parseImport(obj));
  }

  var ws = ctx.ws;
  var ids = collectIds(ws);
  var col = model.ensureCollection(ws, collectionName || obj.collection || '');
  var req = model.normRequest(obj, ids);
  var targetFolder = folderRef || obj.folder || obj.folderPath || '';
  req.folderId = model.resolveFolder(col, targetFolder);
  col.requests.push(req);
  col.updatedAt = u.nowIso();
  ctx.persist();
  var folder = model.folderPath(col, req.folderId);
  return format.ok('add', {
    id: req.id,
    collection: col.name,
    collectionId: col.id,
    folder: folder,
    request: req
  }, [
    { title: '已新增 ' + req.method + ' ' + req.name, subtitle: col.name + (folder ? ' / ' + folder : '') + ' · ' + (req.url || '(空 URL)'), copy: req.id },
    { title: '复制请求 JSON', subtitle: '便于继续编辑', copy: JSON.stringify(req) }
  ]);
}

function doUpdate(ctx, ref, patch) {
  var m = model.findRequest(ctx.ws, ref);
  if (m.error) return format.fail('update', m.error);
  if (!u.isObj(patch)) return format.fail('update', 'patch 必须是 JSON 对象');

  var merged = u.clone(m.request);
  var changed = [];
  ALLOWED_PATCH.forEach(function (key) {
    if (Object.prototype.hasOwnProperty.call(patch, key)) {
      merged[key] = patch[key];
      changed.push(key);
    }
  });
  if (Object.prototype.hasOwnProperty.call(patch, 'folder')) {
    merged.folderId = model.resolveFolder(m.collection, patch.folder);
    changed.push('folder');
  }
  if (!changed.length) return format.fail('update', '没有可更新字段（支持: ' + ALLOWED_PATCH.join(', ') + ', folder）');
  if (merged.folderId && !m.collection.folders.some(function (f) { return f.id === merged.folderId; })) {
    return format.fail('update', '分组不存在: ' + merged.folderId);
  }
  merged.updatedAt = u.nowIso();
  var normed = model.normRequest(merged);
  normed.id = m.request.id;
  normed.createdAt = m.request.createdAt;

  var idx = m.collection.requests.indexOf(m.request);
  m.collection.requests[idx] = normed;
  m.collection.updatedAt = u.nowIso();
  ctx.persist();
  return format.ok('update', { id: normed.id, changed: changed, request: normed }, [
    { title: '已更新 ' + normed.name, subtitle: changed.join(', '), copy: normed.id }
  ]);
}

function doDelete(ctx, ref) {
  var m = model.findRequest(ctx.ws, ref);
  if (m.error) return format.fail('delete', m.error);
  var idx = m.collection.requests.indexOf(m.request);
  if (idx >= 0) m.collection.requests.splice(idx, 1);
  m.collection.updatedAt = u.nowIso();
  ctx.persist();
  return format.ok('delete', { id: m.request.id, name: m.request.name, collection: m.collection.name }, [
    { title: '已删除 ' + m.request.name, subtitle: m.collection.name, copy: m.request.id }
  ]);
}

function doCollectionAction(ctx, action, opts) {
  action = String(action || 'list').toLowerCase();
  if (action === 'list' || action === 'ls') return doCollections(ctx);
  var ws = ctx.ws;

  if (action === 'add' || action === 'create') {
    var name = u.trim(opts.name || opts.ref);
    if (!name) return format.fail('collection', '缺少集合名称');
    var ids = collectIds(ws);
    var col = model.normCollection({ name: name, description: opts.description || '' }, ids);
    ws.collections.push(col);
    ctx.persist();
    return format.ok('collection', { action: 'add', id: col.id, name: col.name }, [
      { title: '已新建集合 ' + col.name, subtitle: col.id, copy: col.id }
    ]);
  }

  var target = model.findCollection(ws, opts.ref || opts.id || opts.name);
  if (target.error) return format.fail('collection', target.error);

  if (action === 'delete' || action === 'remove') {
    var index = ws.collections.indexOf(target);
    if (index >= 0) ws.collections.splice(index, 1);
    ctx.persist();
    return format.ok('collection', { action: 'delete', id: target.id, name: target.name }, [
      { title: '已删除集合 ' + target.name, subtitle: target.requests.length + ' 个请求', copy: target.id }
    ]);
  }

  if (action === 'rename') {
    var newName = u.trim(opts.newName || opts.to || opts.name);
    if (!newName) return format.fail('collection', '缺少新名称');
    target.name = newName;
    target.updatedAt = u.nowIso();
    ctx.persist();
    return format.ok('collection', { action: 'rename', id: target.id, name: target.name }, [
      { title: '已重命名集合', subtitle: newName, copy: target.id }
    ]);
  }

  return format.fail('collection', '未知集合操作: ' + action);
}

function doFolderAction(ctx, action, opts) {
  action = String(action || 'list').toLowerCase();
  opts = opts || {};
  var ws = ctx.ws;
  var col = model.findCollection(ws, opts.collection || opts.col);
  if (col.error) return format.fail('folder', col.error);

  if (action === 'list' || action === 'ls') {
    var items = (col.folders || []).map(function (f) {
      return {
        id: f.id,
        name: f.name,
        path: model.folderPath(col, f.id),
        parentId: f.parentId || '',
        requestCount: model.countFolderRequests(col, f.id)
      };
    });
    var rows = items.map(function (f) {
      return { title: f.path, subtitle: f.requestCount + ' 个请求 · ' + f.id, copy: 'api folder delete ' + col.id + ' ' + f.id };
    });
    if (!rows.length) rows.push({ title: '还没有分组', subtitle: 'api folder add <集合> <名称>', copy: 'api folder add ' + col.id + ' 我的分组' });
    return format.ok('folder', { action: 'list', collection: col.name, collectionId: col.id, total: items.length, items: items }, rows);
  }

  if (action === 'add' || action === 'create') {
    var name = u.trim(opts.name || opts.ref);
    if (!name) return format.fail('folder', '缺少分组名称');
    var parentId = '';
    if (opts.parent) {
      var parent = model.findFolder(col, opts.parent);
      if (parent.error) return format.fail('folder', parent.error);
      parentId = parent.id;
    }
    var dup = (col.folders || []).some(function (f) {
      return String(f.parentId || '') === parentId && f.name.toLowerCase() === name.toLowerCase();
    });
    if (dup) return format.fail('folder', '同级已存在同名分组: ' + name);
    var folder = model.normFolder({ name: name, parentId: parentId });
    col.folders.push(folder);
    col.updatedAt = u.nowIso();
    ctx.persist();
    return format.ok('folder', {
      action: 'add',
      id: folder.id,
      name: folder.name,
      path: model.folderPath(col, folder.id),
      collection: col.name,
      collectionId: col.id
    }, [
      { title: '已新建分组 ' + folder.name, subtitle: col.name + ' / ' + model.folderPath(col, folder.id), copy: folder.id }
    ]);
  }

  var target = model.findFolder(col, opts.ref || opts.id || opts.name);
  if (target.error) return format.fail('folder', target.error);

  if (action === 'rename') {
    var newName = u.trim(opts.newName || opts.to);
    if (!newName) return format.fail('folder', '缺少新名称');
    var clash = (col.folders || []).some(function (f) {
      return f.id !== target.id && String(f.parentId || '') === String(target.parentId || '') && f.name.toLowerCase() === newName.toLowerCase();
    });
    if (clash) return format.fail('folder', '同级已存在同名分组: ' + newName);
    target.name = newName;
    col.updatedAt = u.nowIso();
    ctx.persist();
    return format.ok('folder', { action: 'rename', id: target.id, name: target.name, path: model.folderPath(col, target.id), collection: col.name }, [
      { title: '已重命名分组', subtitle: newName, copy: target.id }
    ]);
  }

  if (action === 'delete' || action === 'remove') {
    var ids = model.descendantFolderIds(col, target.id);
    var removedRequests = 0;
    col.requests = col.requests.filter(function (r) {
      if (ids[r.folderId]) { removedRequests++; return false; }
      return true;
    });
    var removedFolders = 0;
    col.folders = col.folders.filter(function (f) {
      if (ids[f.id]) { removedFolders++; return false; }
      return true;
    });
    col.updatedAt = u.nowIso();
    ctx.persist();
    return format.ok('folder', {
      action: 'delete',
      id: target.id,
      name: target.name,
      removedFolders: removedFolders,
      removedRequests: removedRequests,
      collection: col.name
    }, [
      { title: '已删除分组 ' + target.name, subtitle: removedFolders + ' 个分组 · ' + removedRequests + ' 个请求', copy: target.id }
    ]);
  }

  return format.fail('folder', '未知分组操作: ' + action);
}

function doEnvAction(ctx, action, opts) {
  action = String(action || 'list').toLowerCase();
  var ws = ctx.ws;

  if (action === 'list' || action === 'ls') {
    var items = ws.environments.map(function (e) {
      return {
        id: e.id,
        name: e.name,
        active: e.id === ws.activeEnvId,
        vars: e.vars.map(function (v) { return { key: v.key, value: v.value, enabled: v.enabled }; })
      };
    });
    var rows = items.map(function (e) {
      return {
        title: e.name + (e.active ? '（当前）' : ''),
        subtitle: e.vars.length + ' 个变量 · ' + e.id,
        copy: 'api env use ' + e.id
      };
    });
    if (!rows.length) rows.push({ title: '还没有环境', subtitle: 'api env add <名称>', copy: 'api env add 我的环境' });
    return format.ok('env', { activeEnvId: ws.activeEnvId, items: items }, rows);
  }

  if (action === 'add' || action === 'create') {
    var name = u.trim(opts.name || opts.ref);
    if (!name) return format.fail('env', '缺少环境名称');
    var ids = collectIds(ws);
    var env = model.normEnvironment({ name: name }, ids);
    if (opts.vars) env.vars = normalizeVars(opts.vars);
    ws.environments.push(env);
    if (!ws.activeEnvId) ws.activeEnvId = env.id;
    ctx.persist();
    return format.ok('env', { action: 'add', id: env.id, name: env.name }, [
      { title: '已新建环境 ' + env.name, subtitle: env.id, copy: env.id }
    ]);
  }

  var target = null;
  if (opts.ref || opts.id || opts.name) {
    target = model.findEnvironment(ws, opts.ref || opts.id || opts.name);
    if (target.error) return format.fail('env', target.error);
  }

  if (action === 'use' || action === 'switch') {
    if (!target) return format.fail('env', '缺少环境 id 或名称');
    ws.activeEnvId = target.id;
    ctx.persist();
    return format.ok('env', { action: 'use', id: target.id, name: target.name }, [
      { title: '当前环境：' + target.name, subtitle: target.id, copy: target.id }
    ]);
  }

  if (action === 'set') {
    if (!target) return format.fail('env', '缺少环境 id 或名称');
    var vars = normalizeVars(opts.vars);
    if (!vars.length) return format.fail('env', '缺少变量（k=v 或 {"k":"v"}）');
    vars.forEach(function (v) {
      var hit = null;
      for (var i = 0; i < target.vars.length; i++) {
        if (target.vars[i].key === v.key) { hit = target.vars[i]; break; }
      }
      if (hit) {
        hit.value = v.value;
        hit.enabled = true;
      } else {
        target.vars.push(v);
      }
    });
    target.updatedAt = u.nowIso();
    ctx.persist();
    return format.ok('env', { action: 'set', id: target.id, name: target.name, vars: vars }, [
      { title: '已写入 ' + vars.length + ' 个变量', subtitle: target.name, copy: target.id }
    ]);
  }

  if (action === 'unset' || action === 'remove') {
    if (!target) return format.fail('env', '缺少环境 id 或名称');
    var keys = Array.isArray(opts.keys) ? opts.keys : (opts.key ? [opts.key] : []);
    if (!keys.length) return format.fail('env', '缺少要删除的变量名');
    target.vars = target.vars.filter(function (v) { return keys.indexOf(v.key) < 0; });
    target.updatedAt = u.nowIso();
    ctx.persist();
    return format.ok('env', { action: 'unset', id: target.id, keys: keys }, [
      { title: '已删除变量', subtitle: keys.join(', '), copy: target.id }
    ]);
  }

  if (action === 'delete') {
    if (!target) return format.fail('env', '缺少环境 id 或名称');
    var idx = ws.environments.indexOf(target);
    if (idx >= 0) ws.environments.splice(idx, 1);
    if (ws.activeEnvId === target.id) {
      ws.activeEnvId = ws.environments.length ? ws.environments[0].id : '';
    }
    ctx.persist();
    return format.ok('env', { action: 'delete', id: target.id, name: target.name }, [
      { title: '已删除环境 ' + target.name, subtitle: '', copy: target.id }
    ]);
  }

  if (action === 'show' || action === 'get') {
    if (!target) return format.fail('env', '缺少环境 id 或名称');
    return format.ok('env', { action: 'show', environment: target }, [
      { title: target.name, subtitle: target.vars.length + ' 个变量', copy: JSON.stringify(target) }
    ]);
  }

  return format.fail('env', '未知环境操作: ' + action);
}

function normalizeVars(input) {
  var out = [];
  if (Array.isArray(input)) {
    input.forEach(function (v) {
      if (!u.isObj(v) || v.key == null) return;
      out.push({ key: String(v.key), value: String(v.value == null ? '' : v.value), enabled: v.enabled !== false });
    });
    return out;
  }
  if (u.isObj(input)) {
    Object.keys(input).forEach(function (k) {
      out.push({ key: k, value: String(input[k] == null ? '' : input[k]), enabled: true });
    });
  }
  return out;
}

// ---------- 导入导出 ----------

/** 导入目标集合：指定 id 用现有集合；指定名称时同名复用、否则新建；都不给则按来源建集合。 */
function importTarget(ws, ids, opts) {
  opts = opts || {};
  var id = u.trim(opts.collectionId);
  if (id) {
    for (var i = 0; i < ws.collections.length; i++) {
      if (ws.collections[i].id === id) return { col: ws.collections[i] };
    }
    return { error: '目标集合不存在: ' + id };
  }
  var name = u.trim(opts.collection);
  if (!name) return { col: null };
  var col = model.ensureCollection(ws, name);
  ids[col.id] = true;
  return { col: col };
}

function applyImport(ctx, parsed, opts) {
  var ws = ctx.ws;
  var ids = collectIds(ws);
  var addedRequests = 0;
  var collectionNames = [];
  var target = importTarget(ws, ids, opts);
  if (target.error) return format.fail('import', target.error);
  var targetCol = target.col;
  var wrapGroup = targetCol && (parsed.collections || []).length > 1;

  (parsed.collections || []).forEach(function (pc) {
    var col = targetCol || null;
    if (!col) {
      for (var i = 0; i < ws.collections.length; i++) {
        if (ws.collections[i].name.toLowerCase() === String(pc.name).toLowerCase()) {
          col = ws.collections[i];
          break;
        }
      }
    }
    if (!col) {
      col = model.normCollection({ name: pc.name, description: pc.description }, ids);
      ids[col.id] = true;
      ws.collections.push(col);
    }
    (pc.requests || []).forEach(function (pr) {
      var input = u.clone(pr);
      var folderPath = Array.isArray(input.folderPath) ? input.folderPath : null;
      delete input.folderPath;
      if (wrapGroup) folderPath = [pc.name].concat(folderPath || []);
      if (folderPath && folderPath.length) {
        input.folderId = model.ensureFolderPath(col, folderPath);
      }
      var req = model.normRequest(input, ids);
      ids[req.id] = true;
      col.requests.push(req);
      addedRequests++;
    });
    col.updatedAt = u.nowIso();
    if (collectionNames.indexOf(col.name) < 0) collectionNames.push(col.name);
  });

  var addedEnvs = 0;
  var firstEnvId = '';
  (parsed.environments || []).forEach(function (pe) {
    var existing = null;
    for (var i = 0; i < ws.environments.length; i++) {
      if (ws.environments[i].name.toLowerCase() === String(pe.name).toLowerCase()) {
        existing = ws.environments[i];
        break;
      }
    }
    if (existing) {
      (pe.vars || []).forEach(function (v) {
        var hit = null;
        for (var j = 0; j < existing.vars.length; j++) {
          if (existing.vars[j].key === v.key) { hit = existing.vars[j]; break; }
        }
        if (hit) { hit.value = v.value; hit.enabled = v.enabled !== false; }
        else existing.vars.push({ key: v.key, value: v.value, enabled: v.enabled !== false });
      });
      existing.updatedAt = u.nowIso();
    } else {
      var env = model.normEnvironment(pe, ids);
      ids[env.id] = true;
      ws.environments.push(env);
      if (!ws.activeEnvId) ws.activeEnvId = env.id;
    }
    if (!firstEnvId) firstEnvId = existing ? existing.id : env.id;
    addedEnvs++;
  });

  // Swagger 导入带 activateEnv：请求用的是 {{baseUrl}}，必须切到刚导入的环境才可发送。
  if (parsed.activateEnv === true && firstEnvId) ws.activeEnvId = firstEnvId;

  ctx.persist();
  return format.ok('import', {
    collections: collectionNames,
    addedRequests: addedRequests,
    addedEnvironments: addedEnvs
  }, [
    {
      title: '导入完成',
      subtitle: collectionNames.join(', ') + ' · ' + addedRequests + ' 个请求 · ' + addedEnvs + ' 个环境',
      copy: JSON.stringify({ collections: collectionNames, addedRequests: addedRequests, addedEnvironments: addedEnvs })
    }
  ]);
}

function doImportInput(ctx, opts) {
  var target = { collection: opts.collection, collectionId: opts.collectionId };
  if (opts.data !== undefined) {
    return applyImport(ctx, transfer.parseImport(opts.data), target);
  }
  if (opts.json) {
    var parsed;
    try {
      parsed = JSON.parse(opts.json);
    } catch (e) {
      return format.fail('import', 'JSON 解析失败: ' + e.message);
    }
    return applyImport(ctx, transfer.parseImport(parsed), target);
  }
  if (opts.file) {
    return http.readJsonText(opts.file).then(function (text) {
      var parsed;
      try {
        parsed = JSON.parse(text);
      } catch (e) {
        throw new Error('文件不是合法 JSON: ' + e.message);
      }
      return applyImport(ctx, transfer.parseImport(parsed), target);
    });
  }
  if (opts.url) {
    return swagger.importFromUrl(opts.url).then(function (parsed) {
      return applyImport(ctx, parsed, target);
    });
  }
  return format.fail('import', '需要 JSON 内容、--file <path> 或 --url <swagger 地址>');
}

function doExport(ctx, opts) {
  opts = opts || {};
  if (opts.ref) {
    var col = model.findCollection(ctx.ws, opts.ref);
    if (col.error) return format.fail('export', col.error);
    var pm = transfer.exportPostmanCollection(col);
    var text = JSON.stringify(pm, null, 2);
    return format.ok('export', { format: 'postman', collection: col.name, text: text }, [
      { title: '导出 Postman 集合：' + col.name, subtitle: col.requests.length + ' 个请求', copy: text }
    ]);
  }
  var native = transfer.exportNative(ctx.ws);
  var nativeText = JSON.stringify(native, null, 2);
  var total = ctx.ws.collections.reduce(function (n, c) { return n + c.requests.length; }, 0);
  return format.ok('export', { format: 'native', text: nativeText }, [
    { title: '导出工作区', subtitle: ctx.ws.collections.length + ' 个集合 · ' + total + ' 个请求 · ' + ctx.ws.environments.length + ' 个环境', copy: nativeText }
  ]);
}

function doOpen(ctx, ref) {
  var id = '';
  if (ref) {
    var m = model.findRequest(ctx.ws, ref);
    if (m.error) return format.fail('open', m.error);
    id = m.request.id;
  }
  ctx.openUi(id);
  return format.ok('open', { id: id }, [
    { title: id ? '已打开界面并定位请求' : '已打开 API 管理界面', subtitle: id, copy: id || 'api' }
  ]);
}

// ---------- 解析入口 ----------

function parseOptions(tokens) {
  var o = { pos: [], headers: {}, vars: {}, env: '', timeoutMs: 0, body: null, contentType: '', follow: true };
  for (var i = 0; i < tokens.length; i++) {
    var t = tokens[i];
    if (t === '--env') {
      o.env = tokens[++i] || '';
    } else if (t === '--var') {
      var kv = tokens[++i] || '';
      var eq = kv.indexOf('=');
      if (eq > 0) o.vars[kv.slice(0, eq)] = kv.slice(eq + 1);
    } else if (t === '--header' || t === '-H') {
      var hv = tokens[++i] || '';
      var colon = hv.indexOf(':');
      if (colon > 0) o.headers[hv.slice(0, colon).trim()] = hv.slice(colon + 1).trim();
    } else if (t === '--body') {
      o.body = tokens[++i] || '';
    } else if (t === '--content-type') {
      o.contentType = tokens[++i] || '';
    } else if (t === '--timeout') {
      o.timeoutMs = Number(tokens[++i]) || 0;
    } else if (t === '--no-follow') {
      o.follow = false;
    } else {
      o.pos.push(t);
    }
  }
  return o;
}

function runJson(j, ctx) {
  if (!u.isObj(j)) return format.fail('json', 'JSON 命令必须是对象');

  var cmd = String(j.cmd || '').toLowerCase();
  if (!cmd) {
    if (Array.isArray(j.collections) || j.info || Array.isArray(j.item) || Array.isArray(j.values)) {
      return applyImport(ctx, transfer.parseImport(j));
    }
    return format.fail('json', '缺少 cmd 字段，例如 {"cmd":"list"}');
  }

  switch (cmd) {
    case 'help':
      return doHelp();
    case 'list':
      return doList(ctx, j.keyword || j.filter || '');
    case 'show':
    case 'get':
      return doShow(ctx, j.id || j.ref || j.name);
    case 'send':
      return doSend(ctx, j.id || j.ref || j.name, {
        env: j.env || '',
        vars: j.vars || {}
      });
    case 'run':
      return doRun(ctx, {
        method: j.method || 'GET',
        url: j.url || '',
        headers: toHeaderList(j.headers, j.contentType),
        params: [],
        body: {
          mode: j.bodyMode === 'json' || j.contentType === 'application/json' ? 'json' : (j.body == null ? 'none' : 'text'),
          raw: j.body == null ? '' : String(j.body),
          form: []
        },
        auth: j.auth || { type: 'none' },
        timeoutMs: j.timeoutMs || 0,
        followRedirects: j.followRedirects !== false
      }, { env: j.env || '', vars: j.vars || {} });
    case 'add':
      var addBody = j.request || j.data || (function () {
        var copy = u.clone(j);
        delete copy.cmd;
        delete copy.collection;
        delete copy.folder;
        return copy;
      })();
      return doAdd(ctx, addBody, j.collection || '', j.folder || '');
    case 'update':
      var patch = j.patch || j.request || j.data || (function () {
        var copy = u.clone(j);
        delete copy.cmd;
        delete copy.id;
        delete copy.ref;
        delete copy.folder;
        return copy;
      })();
      if (j.folder && u.isObj(patch) && patch.folder === undefined) {
        patch = u.clone(patch);
        patch.folder = j.folder;
      }
      return doUpdate(ctx, j.id || j.ref || j.name, patch);
    case 'delete':
      return doDelete(ctx, j.id || j.ref || j.name);
    case 'collections':
      return doCollections(ctx);
    case 'collection':
      return doCollectionAction(ctx, j.action || 'list', {
        name: j.name,
        ref: j.ref || j.id,
        newName: j.newName,
        description: j.description
      });
    case 'folder':
      return doFolderAction(ctx, j.action || 'list', {
        collection: j.collection || j.col,
        ref: j.ref || j.id || j.name,
        name: j.name,
        parent: j.parent || j.parentId,
        newName: j.newName || j.to
      });
    case 'env':
      return doEnvAction(ctx, j.action || 'list', {
        name: j.name,
        ref: j.ref || j.id,
        vars: j.vars,
        keys: j.keys,
        key: j.key
      });
    case 'history':
      return doHistory(ctx, { limit: j.limit, clear: j.clear });
    case 'import':
      return doImportInput(ctx, {
        data: j.data,
        json: j.json,
        file: j.file,
        url: j.url,
        collection: j.collection,
        collectionId: j.collectionId
      });
    case 'export':
      return doExport(ctx, { ref: j.collection || j.ref || j.id, postman: j.postman });
    case 'open':
    case 'ui':
      return doOpen(ctx, j.id || j.ref);
    default:
      return format.fail(cmd, '未知命令: ' + cmd + '（api help）');
  }
}

function toHeaderList(headers, contentType) {
  var list = [];
  if (u.isObj(headers)) {
    Object.keys(headers).forEach(function (k) {
      list.push({ key: k, value: String(headers[k] == null ? '' : headers[k]), enabled: true });
    });
  }
  if (contentType) {
    var has = list.some(function (h) { return h.key.toLowerCase() === 'content-type'; });
    if (!has) list.push({ key: 'Content-Type', value: contentType, enabled: true });
  }
  return list;
}

function runText(payload, ctx) {
  var s = u.trim(payload);
  if (!s) return format.fail('', '缺少命令（api help）');

  if (s.indexOf('--json-file') === 0) {
    var filePath = u.trim(s.slice('--json-file'.length));
    if (!filePath) return format.fail('json', '缺少文件路径（api --json-file <path>）');
    if (filePath.charAt(0) === '"' || filePath.charAt(0) === "'") filePath = filePath.slice(1, -1);
    return http.readJsonText(filePath).then(function (text) {
      var obj;
      try {
        obj = JSON.parse(text);
      } catch (e) {
        return format.fail('json', 'JSON 解析失败: ' + e.message);
      }
      return Array.isArray(obj) ? applyImport(ctx, transfer.parseImport(obj)) : runJson(obj, ctx);
    });
  }

  if (s.charAt(0) === '{' || s.charAt(0) === '[') {
    var obj;
    try {
      obj = JSON.parse(s);
    } catch (e) {
      return format.fail('json', 'JSON 解析失败: ' + e.message);
    }
    return Array.isArray(obj) ? applyImport(ctx, transfer.parseImport(obj)) : runJson(obj, ctx);
  }

  var tokens = u.tokenize(s);
  var cmd = String(tokens.shift() || '').toLowerCase();
  var restRaw = s.slice(cmd.length).trim();

  switch (cmd) {
    case 'help':
    case '?':
      return doHelp();
    case 'list':
    case 'ls':
      return doList(ctx, tokens.join(' '));
    case 'show':
    case 'get':
      return doShow(ctx, tokens[0]);
    case 'send': {
      var opts = parseOptions(tokens.slice(1));
      return doSend(ctx, tokens[0], { env: opts.env, vars: opts.vars });
    }
    case 'run': {
      var ro = parseOptions(tokens);
      var method = 'GET';
      var url = '';
      if (ro.pos.length >= 2) {
        method = ro.pos[0];
        url = ro.pos[1];
      } else if (ro.pos.length === 1) {
        url = ro.pos[0];
      }
      var bodyMode = 'none';
      if (ro.body !== null) bodyMode = ro.contentType === 'application/json' ? 'json' : 'text';
      return doRun(ctx, {
        method: method,
        url: url,
        headers: toHeaderList(ro.headers, ro.contentType),
        params: [],
        body: { mode: bodyMode, raw: ro.body == null ? '' : ro.body, form: [] },
        auth: { type: 'none' },
        timeoutMs: ro.timeoutMs,
        followRedirects: ro.follow
      }, { env: ro.env, vars: ro.vars });
    }
    case 'add':
      return doAdd(ctx, restRaw, '');
    case 'update': {
      var ref = tokens[0];
      if (!ref) return format.fail('update', '缺少请求 id 或名称');
      var at = restRaw.indexOf(ref);
      var patchText = at >= 0 ? restRaw.slice(at + ref.length).trim() : '';
      if (!patchText) return format.fail('update', '缺少 patch JSON');
      var patchObj;
      try {
        patchObj = JSON.parse(patchText);
      } catch (e) {
        return format.fail('update', 'patch JSON 解析失败: ' + e.message);
      }
      return doUpdate(ctx, ref, patchObj);
    }
    case 'delete':
    case 'rm':
      return doDelete(ctx, tokens[0]);
    case 'collections':
      return doCollections(ctx);
    case 'collection': {
      var sub = String(tokens[0] || 'list').toLowerCase();
      if (sub === 'add' || sub === 'create') {
        return doCollectionAction(ctx, 'add', { name: restRaw.slice(restRaw.indexOf(tokens[0]) + tokens[0].length).trim() });
      }
      if (sub === 'delete' || sub === 'remove') {
        return doCollectionAction(ctx, 'delete', { ref: tokens[1] });
      }
      if (sub === 'rename') {
        return doCollectionAction(ctx, 'rename', { ref: tokens[1], newName: tokens.slice(2).join(' ') });
      }
      return doCollectionAction(ctx, sub, { ref: tokens[1] });
    }
    case 'folder': {
      var fsub = String(tokens[0] || 'list').toLowerCase();
      if (fsub === 'list' || fsub === 'ls') return doFolderAction(ctx, 'list', { collection: tokens[1] });
      if (fsub === 'add' || fsub === 'create') {
        return doFolderAction(ctx, 'add', { collection: tokens[1], name: tokens.slice(2).join(' ') });
      }
      if (fsub === 'rename') {
        return doFolderAction(ctx, 'rename', { collection: tokens[1], ref: tokens[2], newName: tokens.slice(3).join(' ') });
      }
      if (fsub === 'delete' || fsub === 'remove' || fsub === 'rm') {
        return doFolderAction(ctx, 'delete', { collection: tokens[1], ref: tokens.slice(2).join(' ') });
      }
      return format.fail('folder', '未知分组操作: ' + fsub);
    }
    case 'env': {
      var esub = String(tokens[0] || 'list').toLowerCase();
      if (esub === 'list' || esub === 'ls') return doEnvAction(ctx, 'list', {});
      if (esub === 'add' || esub === 'create') {
        return doEnvAction(ctx, 'add', { name: restRaw.slice(restRaw.indexOf(tokens[0]) + tokens[0].length).trim() });
      }
      if (esub === 'use' || esub === 'switch') {
        return doEnvAction(ctx, 'use', { ref: tokens[1] });
      }
      if (esub === 'show' || esub === 'get') {
        return doEnvAction(ctx, 'show', { ref: tokens[1] });
      }
      if (esub === 'delete' || esub === 'remove') {
        return doEnvAction(ctx, 'delete', { ref: tokens[1] });
      }
      if (esub === 'set') {
        var ref2 = tokens[1];
        var varsObj = {};
        tokens.slice(2).forEach(function (t) {
          var eq = t.indexOf('=');
          if (eq > 0) varsObj[t.slice(0, eq)] = t.slice(eq + 1);
        });
        return doEnvAction(ctx, 'set', { ref: ref2, vars: varsObj });
      }
      if (esub === 'unset') {
        return doEnvAction(ctx, 'unset', { ref: tokens[1], keys: tokens.slice(2) });
      }
      return format.fail('env', '未知环境操作: ' + esub);
    }
    case 'history': {
      if (String(tokens[0] || '').toLowerCase() === 'clear') {
        return doHistory(ctx, { clear: true });
      }
      return doHistory(ctx, { limit: Number(tokens[0]) || 20 });
    }
    case 'import': {
      var io = { file: '', url: '', collection: '', collectionId: '' };
      for (var k = 0; k < tokens.length; k++) {
        var key = tokens[k];
        if (key === '--file') { io.file = tokens[++k] || ''; }
        else if (key === '--url') { io.url = tokens[++k] || ''; }
        else if (key === '--collection' || key === '--col') { io.collection = tokens[++k] || ''; }
        else if (key === '--collection-id') { io.collectionId = tokens[++k] || ''; }
      }
      if (io.file) return doImportInput(ctx, io);
      if (io.url) return doImportInput(ctx, io);
      return doImportInput(ctx, { json: restRaw, collection: io.collection, collectionId: io.collectionId });
    }
    case 'export': {
      var eo = parseOptions(tokens);
      return doExport(ctx, { ref: eo.pos.join(' ') });
    }
    case 'open':
    case 'ui':
      return doOpen(ctx, tokens[0]);
    default:
      return format.fail(cmd || '', '未知命令: ' + (cmd || '(空)') + '（api help）');
  }
}

/**
 * 统一入口。返回 {result, rows} 或 Promise<{result, rows}>。
 */
function run(payload, ctx) {
  try {
    var out = runText(payload, ctx);
    if (out && typeof out.then === 'function') {
      return out.catch(function (e) {
        return format.fail('command', e && e.message ? e.message : String(e));
      });
    }
    return out;
  } catch (e) {
    return format.fail('command', e && e.message ? e.message : String(e));
  }
}

exports.run = run;
exports.runJson = runJson;
exports.runText = runText;
exports.importData = function (ctx, obj, opts) {
  try {
    return applyImport(ctx, transfer.parseImport(obj), opts);
  } catch (e) {
    return format.fail('import', e && e.message ? e.message : String(e));
  }
};
exports.applyParsed = applyImport;
