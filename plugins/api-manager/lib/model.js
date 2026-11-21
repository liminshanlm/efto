'use strict';
// 工作区数据模型：集合 / 请求 / 环境 / 历史。
// 全部存在插件 KV 的单个 key 里，UI 与 CLI 共用同一份数据。

var u = require('./util');

var STORE_KEY = 'workspace';
var HISTORY_LIMIT = 100;

function defaultTimeout(v) {
  var n = Number(v);
  if (!n || isNaN(n)) return 30000;
  return Math.max(1000, Math.min(55000, n));
}

function normKvList(list) {
  if (!Array.isArray(list)) return [];
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var it = list[i];
    if (!u.isObj(it)) continue;
    out.push({
      key: String(it.key == null ? '' : it.key),
      value: String(it.value == null ? '' : it.value),
      enabled: it.enabled !== false
    });
  }
  return out;
}

function normBody(body) {
  body = u.isObj(body) ? body : {};
  var mode = String(body.mode || 'none');
  if (['none', 'json', 'text', 'form'].indexOf(mode) < 0) mode = 'none';
  return {
    mode: mode,
    raw: String(body.raw == null ? '' : body.raw),
    form: normKvList(body.form)
  };
}

function normAuth(auth) {
  auth = u.isObj(auth) ? auth : {};
  var type = String(auth.type || 'none');
  if (['none', 'bearer', 'basic'].indexOf(type) < 0) type = 'none';
  return {
    type: type,
    token: String(auth.token == null ? '' : auth.token),
    username: String(auth.username == null ? '' : auth.username),
    password: String(auth.password == null ? '' : auth.password)
  };
}

function normRequest(raw, existingIds) {
  raw = u.isObj(raw) ? raw : {};
  var id = u.trim(raw.id);
  if (!id || (existingIds && existingIds[id])) id = u.newId('req');
  return {
    id: id,
    name: u.trim(raw.name) || '未命名请求',
    method: String(raw.method || 'GET').toUpperCase(),
    url: String(raw.url == null ? '' : raw.url),
    params: normKvList(raw.params),
    headers: normKvList(raw.headers),
    body: normBody(raw.body),
    auth: normAuth(raw.auth),
    timeoutMs: defaultTimeout(raw.timeoutMs),
    createdAt: raw.createdAt || u.nowIso(),
    updatedAt: raw.updatedAt || u.nowIso()
  };
}

function normCollection(raw, existingIds) {
  raw = u.isObj(raw) ? raw : {};
  var id = u.trim(raw.id);
  if (!id || (existingIds && existingIds[id])) id = u.newId('col');
  var requests = [];
  if (Array.isArray(raw.requests)) {
    for (var i = 0; i < raw.requests.length; i++) {
      var req = normRequest(raw.requests[i], existingIds);
      if (existingIds) existingIds[req.id] = true;
      requests.push(req);
    }
  }
  return {
    id: id,
    name: u.trim(raw.name) || '未命名集合',
    description: String(raw.description == null ? '' : raw.description),
    requests: requests,
    createdAt: raw.createdAt || u.nowIso(),
    updatedAt: raw.updatedAt || u.nowIso()
  };
}

function normEnvironment(raw, existingIds) {
  raw = u.isObj(raw) ? raw : {};
  var id = u.trim(raw.id);
  if (!id || (existingIds && existingIds[id])) id = u.newId('env');
  return {
    id: id,
    name: u.trim(raw.name) || '未命名环境',
    vars: normKvList(raw.vars),
    createdAt: raw.createdAt || u.nowIso(),
    updatedAt: raw.updatedAt || u.nowIso()
  };
}

function normHistory(list) {
  if (!Array.isArray(list)) return [];
  var out = [];
  for (var i = 0; i < list.length && out.length < HISTORY_LIMIT; i++) {
    var h = list[i];
    if (!u.isObj(h)) continue;
    out.push({
      id: String(h.id || u.newId('his')),
      ts: String(h.ts || u.nowIso()),
      requestId: String(h.requestId || ''),
      name: String(h.name || ''),
      method: String(h.method || 'GET').toUpperCase(),
      url: String(h.url || ''),
      status: Number(h.status) || 0,
      statusText: String(h.statusText || ''),
      durationMs: Number(h.durationMs) || 0,
      size: Number(h.size) || 0,
      ok: h.ok !== false
    });
  }
  return out;
}

function seed() {
  var envId = u.newId('env');
  return {
    version: 1,
    collections: [{
      id: u.newId('col'),
      name: '示例',
      description: '开箱即用的示例集合，地址用环境变量 {{baseUrl}} 拼接',
      requests: [{
        id: u.newId('req'),
        name: 'Get Todo',
        method: 'GET',
        url: '{{baseUrl}}/todos/1',
        params: [],
        headers: [{ key: 'Accept', value: 'application/json', enabled: true }],
        body: { mode: 'none', raw: '', form: [] },
        auth: { type: 'none', token: '', username: '', password: '' },
        timeoutMs: 30000,
        createdAt: u.nowIso(),
        updatedAt: u.nowIso()
      }],
      createdAt: u.nowIso(),
      updatedAt: u.nowIso()
    }],
    environments: [{
      id: envId,
      name: '示例环境',
      vars: [{ key: 'baseUrl', value: 'https://jsonplaceholder.typicode.com', enabled: true }],
      createdAt: u.nowIso(),
      updatedAt: u.nowIso()
    }],
    activeEnvId: envId,
    history: []
  };
}

function normalize(raw) {
  if (!u.isObj(raw) || !Array.isArray(raw.collections)) return null;
  var ids = {};
  var collections = [];
  for (var i = 0; i < raw.collections.length; i++) {
    var c = normCollection(raw.collections[i], ids);
    ids[c.id] = true;
    collections.push(c);
  }
  var environments = [];
  if (Array.isArray(raw.environments)) {
    for (var j = 0; j < raw.environments.length; j++) {
      var e = normEnvironment(raw.environments[j], ids);
      ids[e.id] = true;
      environments.push(e);
    }
  }
  var activeEnvId = String(raw.activeEnvId || '');
  if (activeEnvId && !ids[activeEnvId]) activeEnvId = '';
  if (!activeEnvId && environments.length) activeEnvId = environments[0].id;
  return {
    version: 1,
    collections: collections,
    environments: environments,
    activeEnvId: activeEnvId,
    history: normHistory(raw.history)
  };
}

function load() {
  var raw = efto.kv.get(STORE_KEY);
  var ws = normalize(raw);
  if (!ws) {
    ws = seed();
    save(ws);
  }
  return ws;
}

function save(ws) {
  efto.kv.set(STORE_KEY, ws);
}

/** 在集合/请求里查找：id > 精确名字 > id 前缀 > 名字包含；返回唯一匹配或错误。 */
function findRequest(ws, ref) {
  var needle = u.trim(ref);
  if (!needle) return { error: '缺少请求 id 或名称' };
  var all = [];
  for (var i = 0; i < ws.collections.length; i++) {
    var col = ws.collections[i];
    for (var j = 0; j < col.requests.length; j++) {
      all.push({ collection: col, request: col.requests[j] });
    }
  }
  var lower = needle.toLowerCase();
  var hit = all.filter(function (x) { return x.request.id === needle; });
  if (hit.length === 1) return hit[0];
  hit = all.filter(function (x) { return x.request.name.toLowerCase() === lower; });
  if (hit.length === 1) return hit[0];
  hit = all.filter(function (x) { return x.request.id.toLowerCase().indexOf(lower) === 0; });
  if (hit.length === 1) return hit[0];
  hit = all.filter(function (x) { return x.request.name.toLowerCase().indexOf(lower) >= 0; });
  if (hit.length === 1) return hit[0];
  if (hit.length > 1) {
    return { error: '匹配到多个请求，请用 id：' + hit.map(function (x) { return x.request.id; }).join(', ') };
  }
  return { error: '没有找到请求: ' + needle };
}

function findCollection(ws, ref) {
  var needle = u.trim(ref);
  if (!needle) return { error: '缺少集合 id 或名称' };
  var lower = needle.toLowerCase();
  var hit = ws.collections.filter(function (c) { return c.id === needle; });
  if (hit.length === 1) return hit[0];
  hit = ws.collections.filter(function (c) { return c.name.toLowerCase() === lower; });
  if (hit.length === 1) return hit[0];
  hit = ws.collections.filter(function (c) { return c.id.toLowerCase().indexOf(lower) === 0; });
  if (hit.length === 1) return hit[0];
  hit = ws.collections.filter(function (c) { return c.name.toLowerCase().indexOf(lower) >= 0; });
  if (hit.length === 1) return hit[0];
  if (hit.length > 1) return { error: '匹配到多个集合，请用 id' };
  return { error: '没有找到集合: ' + needle };
}

function findEnvironment(ws, ref) {
  var needle = u.trim(ref);
  if (!needle) return { error: '缺少环境 id 或名称' };
  var lower = needle.toLowerCase();
  var hit = ws.environments.filter(function (e) { return e.id === needle; });
  if (hit.length === 1) return hit[0];
  hit = ws.environments.filter(function (e) { return e.name.toLowerCase() === lower; });
  if (hit.length === 1) return hit[0];
  hit = ws.environments.filter(function (e) { return e.name.toLowerCase().indexOf(lower) >= 0; });
  if (hit.length === 1) return hit[0];
  if (hit.length > 1) return { error: '匹配到多个环境，请用 id' };
  return { error: '没有找到环境: ' + needle };
}

/** 找不到同名集合就新建一个。 */
function ensureCollection(ws, name) {
  var target = u.trim(name);
  if (target) {
    for (var i = 0; i < ws.collections.length; i++) {
      if (ws.collections[i].name.toLowerCase() === target.toLowerCase()) return ws.collections[i];
    }
  } else if (ws.collections.length) {
    return ws.collections[0];
  }
  var col = normCollection({ name: target || '默认集合' });
  ws.collections.push(col);
  return col;
}

function pushHistory(ws, entry) {
  var h = {
    id: u.newId('his'),
    ts: u.nowIso(),
    requestId: String(entry.requestId || ''),
    name: String(entry.name || ''),
    method: String(entry.method || 'GET').toUpperCase(),
    url: String(entry.url || ''),
    status: Number(entry.status) || 0,
    statusText: String(entry.statusText || ''),
    durationMs: Number(entry.durationMs) || 0,
    size: Number(entry.size) || 0,
    ok: entry.ok !== false
  };
  ws.history.unshift(h);
  if (ws.history.length > HISTORY_LIMIT) ws.history.length = HISTORY_LIMIT;
  return h;
}

exports.STORE_KEY = STORE_KEY;
exports.HISTORY_LIMIT = HISTORY_LIMIT;
exports.load = load;
exports.save = save;
exports.seed = seed;
exports.normalize = normalize;
exports.normRequest = normRequest;
exports.normCollection = normCollection;
exports.normEnvironment = normEnvironment;
exports.normKvList = normKvList;
exports.findRequest = findRequest;
exports.findCollection = findCollection;
exports.findEnvironment = findEnvironment;
exports.ensureCollection = ensureCollection;
exports.pushHistory = pushHistory;
