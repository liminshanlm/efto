'use strict';
// 把保存的请求（或 UI 里的草稿）整理成 Node HTTP 请求并执行。

var u = require('./util');
var env = require('./env');
var http = require('./http');

function splitHash(url) {
  var i = url.indexOf('#');
  if (i < 0) return { base: url, hash: '' };
  return { base: url.slice(0, i), hash: url.slice(i) };
}

function appendQuery(url, pieces) {
  if (!pieces.length) return url;
  var parts = splitHash(url);
  var sep = '?';
  if (parts.base.indexOf('?') >= 0) sep = /[?&]$/.test(parts.base) ? '' : '&';
  return parts.base + sep + pieces.join('&') + parts.hash;
}

function hasHeader(headers, name) {
  var lower = name.toLowerCase();
  return Object.keys(headers).some(function (k) { return k.toLowerCase() === lower; });
}

function normalizeUrl(url) {
  var s = u.trim(url);
  if (!s) return s;
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(s)) return s;
  if (s.indexOf('{{') === 0 || s.charAt(0) === '/') return s;
  if (/^(localhost|127\.|0\.0\.0\.0|\[::1\])/i.test(s)) return 'http://' + s;
  return 'https://' + s;
}

/**
 * 生成 Node 请求。
 * input: { method, url, params?, headers?, body?, auth?, timeoutMs?, followRedirects? }
 * opts:  { env?, vars? }
 */
function build(ws, input, opts) {
  opts = opts || {};
  input = u.isObj(input) ? input : {};
  var warnings = [];
  var resolvedEnv = env.collect(ws, opts.env || '', opts.vars || {});
  var vars = resolvedEnv.map;

  var url = env.resolveString(normalizeUrl(input.url), vars, warnings);
  var params = Array.isArray(input.params) ? input.params : [];
  var pieces = [];
  for (var i = 0; i < params.length; i++) {
    var p = params[i];
    if (!p || p.enabled === false || !u.trim(p.key)) continue;
    pieces.push(
      encodeURIComponent(env.resolveString(p.key, vars, warnings)) + '=' +
      encodeURIComponent(env.resolveString(p.value, vars, warnings))
    );
  }
  url = appendQuery(url, pieces);

  var headers = {};
  var list = Array.isArray(input.headers) ? input.headers : [];
  for (var j = 0; j < list.length; j++) {
    var h = list[j];
    if (!h || h.enabled === false || !u.trim(h.key)) continue;
    headers[env.resolveString(h.key, vars, warnings)] = env.resolveString(h.value, vars, warnings);
  }

  var auth = u.isObj(input.auth) ? input.auth : { type: 'none' };
  if (auth.type === 'bearer' && u.trim(auth.token)) {
    headers.Authorization = 'Bearer ' + env.resolveString(auth.token, vars, warnings);
  } else if (auth.type === 'basic' && (u.trim(auth.username) || u.trim(auth.password))) {
    headers.Authorization = 'Basic ' + u.base64(
      env.resolveString(auth.username, vars, warnings) + ':' + env.resolveString(auth.password, vars, warnings)
    );
  }

  var body = u.isObj(input.body) ? input.body : { mode: 'none' };
  var mode = body.mode || 'none';
  var bodyText = '';
  if (mode === 'json' || mode === 'text') {
    bodyText = env.resolveString(body.raw, vars, warnings);
    if (mode === 'json' && !hasHeader(headers, 'content-type')) {
      headers['Content-Type'] = 'application/json';
    }
    if (mode === 'text' && !hasHeader(headers, 'content-type')) {
      headers['Content-Type'] = 'text/plain; charset=utf-8';
    }
  } else if (mode === 'form') {
    var forms = Array.isArray(body.form) ? body.form : [];
    var pairs = [];
    for (var k = 0; k < forms.length; k++) {
      var f = forms[k];
      if (!f || f.enabled === false || !u.trim(f.key)) continue;
      pairs.push(
        encodeURIComponent(env.resolveString(f.key, vars, warnings)) + '=' +
        encodeURIComponent(env.resolveString(f.value, vars, warnings))
      );
    }
    bodyText = pairs.join('&');
    if (!hasHeader(headers, 'content-type')) {
      headers['Content-Type'] = 'application/x-www-form-urlencoded';
    }
  }

  return {
    nodeRequest: {
      method: String(input.method || 'GET').toUpperCase(),
      url: url,
      headers: headers,
      body: bodyText || null,
      timeoutMs: Math.max(1000, Math.min(55000, Number(input.timeoutMs) || 30000)),
      followRedirects: input.followRedirects !== false,
      maxRedirects: 5
    },
    meta: {
      envName: resolvedEnv.env ? resolvedEnv.env.name : '',
      envId: resolvedEnv.env ? resolvedEnv.env.id : '',
      warnings: warnings
    }
  };
}

/**
 * 执行请求。返回 Promise<{
 *   data: Node 原始响应, request:{method,url}, envName, warnings
 * }>
 */
function execute(ws, input, opts) {
  var built;
  try {
    built = build(ws, input, opts);
  } catch (e) {
    return Promise.reject(e);
  }
  return http.perform(built.nodeRequest).then(function (data) {
    return {
      data: data,
      request: { method: built.nodeRequest.method, url: built.nodeRequest.url },
      envName: built.meta.envName,
      warnings: built.meta.warnings
    };
  });
}

exports.build = build;
exports.execute = execute;
