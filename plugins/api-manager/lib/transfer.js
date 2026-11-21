'use strict';
// 导入/导出：支持本插件原生格式与 Postman Collection v2.1。

var u = require('./util');

function postmanUrl(url) {
  if (typeof url === 'string') return url;
  if (!u.isObj(url)) return '';
  if (url.raw) return String(url.raw);
  var s = '';
  if (url.protocol) s += String(url.protocol) + '://';
  if (Array.isArray(url.host)) s += url.host.join('.');
  else if (typeof url.host === 'string') s += url.host;
  var path = Array.isArray(url.path) ? url.path.join('/') : (url.path || '');
  if (path) s += (s && s.slice(-1) !== '/' ? '/' : '') + path;
  if (Array.isArray(url.query) && url.query.length) {
    var q = url.query.filter(function (x) {
      return x && x.key && x.disabled !== true;
    }).map(function (x) {
      return encodeURIComponent(x.key) + '=' + encodeURIComponent(x.value == null ? '' : x.value);
    }).join('&');
    if (q) s += (s.indexOf('?') >= 0 ? '&' : '?') + q;
  }
  return s;
}

function postmanHeaders(list) {
  if (!Array.isArray(list)) return [];
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var h = list[i];
    if (!h || h.key == null) continue;
    out.push({ key: String(h.key), value: String(h.value == null ? '' : h.value), disabled: !!h.disabled });
  }
  return out;
}

function postmanBody(body) {
  if (!u.isObj(body)) return { mode: 'none' };
  if (body.mode === 'raw') {
    var lang = body.options && body.options.raw && body.options.raw.language;
    return {
      mode: 'raw',
      raw: String(body.raw == null ? '' : body.raw),
      options: { raw: { language: lang === 'json' ? 'json' : 'text' } }
    };
  }
  if (body.mode === 'urlencoded' || body.mode === 'formdata') {
    var rows = Array.isArray(body[body.mode]) ? body[body.mode] : [];
    return {
      mode: 'urlencoded',
      urlencoded: rows.filter(function (r) { return r && r.key != null; }).map(function (r) {
        return {
          key: String(r.key),
          value: String(r.value == null ? (r.src || '') : r.value),
          disabled: !!r.disabled
        };
      })
    };
  }
  return { mode: 'none' };
}

function postmanAuth(auth) {
  if (!u.isObj(auth) || !auth.type || auth.type === 'none') return { type: 'noauth' };
  if (auth.type === 'bearer') {
    return { type: 'bearer', bearer: [{ key: 'token', value: String(auth.token || ''), type: 'string' }] };
  }
  if (auth.type === 'basic') {
    return {
      type: 'basic',
      basic: [
        { key: 'username', value: String(auth.username || ''), type: 'string' },
        { key: 'password', value: String(auth.password || ''), type: 'string' }
      ]
    };
  }
  return { type: 'noauth' };
}

function toRequest(item, folderPath) {
  var src = u.isObj(item.request) ? item.request : {};
  var name = u.trim(item.name) || '未命名请求';
  var path = Array.isArray(folderPath) ? folderPath.slice() : [];

  var params = [];
  if (u.isObj(src.url) && Array.isArray(src.url.query)) {
    src.url.query.forEach(function (q) {
      if (!q || q.key == null) return;
      params.push({ key: String(q.key), value: String(q.value == null ? '' : q.value), enabled: q.disabled !== true });
    });
  }

  var auth = { type: 'none', token: '', username: '', password: '' };
  var pa = u.isObj(src.auth) ? src.auth : null;
  if (pa && pa.type === 'bearer') {
    auth.type = 'bearer';
    (pa.bearer || []).forEach(function (x) { if (x && x.key === 'token') auth.token = String(x.value || ''); });
  } else if (pa && pa.type === 'basic') {
    auth.type = 'basic';
    (pa.basic || []).forEach(function (x) {
      if (!x) return;
      if (x.key === 'username') auth.username = String(x.value || '');
      if (x.key === 'password') auth.password = String(x.value || '');
    });
  }

  var bodyMode = src.body && src.body.mode;
  var body;
  if (bodyMode === 'raw') {
    body = {
      mode: (src.body.options && src.body.options.raw && src.body.options.raw.language === 'json') ? 'json' : 'text',
      raw: String(src.body.raw == null ? '' : src.body.raw),
      form: []
    };
  } else if (bodyMode === 'urlencoded' || bodyMode === 'formdata') {
    var rows = Array.isArray(src.body[bodyMode]) ? src.body[bodyMode] : [];
    body = {
      mode: 'form',
      raw: '',
      form: rows.filter(function (r) { return r && r.key != null; }).map(function (r) {
        return { key: String(r.key), value: String(r.value == null ? (r.src || '') : r.value), enabled: r.disabled !== true };
      })
    };
  } else {
    body = { mode: 'none', raw: '', form: [] };
  }

  return {
    name: name,
    folderPath: path,
    method: String(src.method || 'GET').toUpperCase(),
    url: postmanUrl(src.url),
    params: params,
    headers: postmanHeaders(src.header).map(function (h) {
      return { key: h.key, value: h.value, enabled: !h.disabled };
    }),
    body: body,
    auth: auth
  };
}

function walk(items, folderPath, sink) {
  for (var i = 0; i < items.length; i++) {
    var item = items[i];
    if (!u.isObj(item)) continue;
    if (u.isObj(item.request)) {
      sink.push(toRequest(item, folderPath));
    } else if (Array.isArray(item.item)) {
      var name = u.trim(item.name);
      walk(item.item, name ? folderPath.concat([name]) : folderPath, sink);
    }
  }
}

function parseEnvironment(raw) {
  var vars = Array.isArray(raw.values) ? raw.values : (Array.isArray(raw.vars) ? raw.vars : null);
  if (!vars) return null;
  return {
    name: u.trim(raw.name) || '导入的环境',
    vars: vars.filter(function (v) { return v && v.key != null; }).map(function (v) {
      return { key: String(v.key), value: String(v.value == null ? '' : v.value), enabled: v.enabled !== false };
    })
  };
}

/**
 * 解析导入内容。返回 { collections:[{name,description,requests}], environments:[...] }。
 * 支持：本插件原生导出、Postman Collection v2.x、Postman 环境、单条请求数组。
 */
function parseImport(raw) {
  var out = { collections: [], environments: [] };

  if (Array.isArray(raw)) {
    // 请求数组
    out.collections.push({ name: '导入的请求', description: '', requests: raw.map(function (r) { return toRequest({ name: r.name, request: r }, ''); }) });
    return out;
  }
  if (!u.isObj(raw)) throw new Error('导入内容必须是 JSON 对象或数组');

  // 原生导出
  if (Array.isArray(raw.collections)) {
    raw.collections.forEach(function (c) {
      if (!u.isObj(c)) return;
      var requests = [];
      var folderMap = {};
      if (Array.isArray(c.folders)) {
        c.folders.forEach(function (f) {
          if (u.isObj(f) && f.id) folderMap[String(f.id)] = f;
        });
      }
      if (Array.isArray(c.requests)) {
        c.requests.forEach(function (r) {
          if (!u.isObj(r)) return;
          var copy = u.clone(r);
          if (copy.folderId) {
            var names = [];
            var fid = String(copy.folderId);
            var guard = 0;
            while (fid && folderMap[fid] && guard++ < 32) {
              names.unshift(String(folderMap[fid].name || ''));
              fid = String(folderMap[fid].parentId || '');
            }
            if (names.length) copy.folderPath = names;
          }
          delete copy.folderId;
          requests.push(copy);
        });
      }
      out.collections.push({
        name: u.trim(c.name) || '导入的集合',
        description: String(c.description || ''),
        requests: requests
      });
    });
    if (Array.isArray(raw.environments)) {
      raw.environments.forEach(function (e) {
        var env = parseEnvironment(e);
        if (env) out.environments.push(env);
      });
    }
    return out;
  }

  // Postman 环境
  if (Array.isArray(raw.values)) {
    var env = parseEnvironment(raw);
    if (env) { out.environments.push(env); return out; }
  }

  // Postman Collection
  if (raw.info || Array.isArray(raw.item)) {
    var requests = [];
    walk(Array.isArray(raw.item) ? raw.item : [], [], requests);
    out.collections.push({
      name: u.trim(raw.info && raw.info.name) || u.trim(raw.name) || '导入的集合',
      description: String((raw.info && raw.info.description) || ''),
      requests: requests
    });
    return out;
  }

  // 单条请求
  if (raw.method || raw.url) {
    out.collections.push({
      name: '导入的请求',
      description: '',
      requests: [toRequest({ name: raw.name, request: raw }, '')]
    });
    return out;
  }

  throw new Error('无法识别的导入格式（支持本插件导出、Postman Collection/Environment、请求对象）');
}

function exportNative(ws) {
  return {
    kind: 'efto-api-export',
    version: 1,
    exportedAt: u.nowIso(),
    collections: u.clone(ws.collections),
    environments: u.clone(ws.environments),
    activeEnvId: ws.activeEnvId
  };
}

function toPostmanRequest(req) {
  var query = (req.params || []).filter(function (p) { return p && p.key; }).map(function (p) {
    return { key: p.key, value: p.value, disabled: p.enabled === false };
  });
  var raw = String(req.url || '');
  var pairs = query.filter(function (q) { return !q.disabled; }).map(function (q) {
    return encodeURIComponent(q.key) + '=' + encodeURIComponent(q.value == null ? '' : q.value);
  });
  if (pairs.length) raw += (raw.indexOf('?') >= 0 ? '&' : '?') + pairs.join('&');

  var body = { mode: 'none' };
  if (req.body && req.body.mode === 'json') {
    body = { mode: 'raw', raw: String(req.body.raw || ''), options: { raw: { language: 'json' } } };
  } else if (req.body && req.body.mode === 'text') {
    body = { mode: 'raw', raw: String(req.body.raw || ''), options: { raw: { language: 'text' } } };
  } else if (req.body && req.body.mode === 'form') {
    body = {
      mode: 'urlencoded',
      urlencoded: (req.body.form || []).filter(function (f) { return f && f.key; }).map(function (f) {
        return { key: f.key, value: f.value, disabled: f.enabled === false };
      })
    };
  }

  var header = (req.headers || []).filter(function (h) { return h && h.key; }).map(function (h) {
    return { key: h.key, value: h.value, disabled: h.enabled === false };
  });
  if (req.auth && req.auth.type === 'bearer' && req.auth.token) {
    // Postman 推荐走 auth 而不是手写 Header；这里保持最直观的 Header 形式
    header.push({ key: 'Authorization', value: 'Bearer ' + req.auth.token, disabled: false });
  } else if (req.auth && req.auth.type === 'basic' && (req.auth.username || req.auth.password)) {
    header.push({ key: 'Authorization', value: 'Basic ' + u.base64(req.auth.username + ':' + req.auth.password), disabled: false });
  }

  return {
    name: req.name || '未命名请求',
    request: {
      method: String(req.method || 'GET').toUpperCase(),
      header: header,
      url: { raw: raw, query: query },
      body: body,
      auth: postmanAuth(req.auth)
    }
  };
}

function buildPostmanItems(col) {
  var folderNodes = {};
  var roots = [];
  (col.folders || []).forEach(function (f) {
    if (!f || !f.id) return;
    folderNodes[f.id] = { name: f.name || '未命名分组', item: [] };
  });
  (col.folders || []).forEach(function (f) {
    if (!f || !f.id) return;
    var node = folderNodes[f.id];
    var parent = f.parentId ? folderNodes[f.parentId] : null;
    (parent ? parent.item : roots).push(node);
  });
  (col.requests || []).forEach(function (req) {
    var node = toPostmanRequest(req);
    var folder = req.folderId ? folderNodes[req.folderId] : null;
    (folder ? folder.item : roots).push(node);
  });
  return roots;
}

function exportPostmanCollection(col) {
  return {
    info: {
      name: col.name,
      description: col.description || '',
      schema: 'https://schema.getpostman.com/json/collection/v2.1.0/collection.json'
    },
    item: buildPostmanItems(col)
  };
}

exports.parseImport = parseImport;
exports.exportNative = exportNative;
exports.exportPostmanCollection = exportPostmanCollection;
exports.parseEnvironment = parseEnvironment;
