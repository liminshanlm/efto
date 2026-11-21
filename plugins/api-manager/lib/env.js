'use strict';
// 环境变量与 {{变量}} 替换。

function uuid() {
  var s = '';
  for (var i = 0; i < 32; i++) {
    if (i === 8 || i === 12 || i === 16 || i === 20) s += '-';
    s += Math.floor(Math.random() * 16).toString(16);
  }
  return s;
}

function dynamic(name) {
  if (name === '$timestamp') return String(Math.floor(Date.now() / 1000));
  if (name === '$isoTimestamp') return new Date().toISOString();
  if (name === '$uuid' || name === '$guid') return uuid();
  if (name === '$randomInt') return String(Math.floor(Math.random() * 1000));
  return null;
}

/** 环境变量表：当前环境 + 调用方覆盖（CLI --var / JSON vars）。 */
function collect(ws, envRef, overrides) {
  var env = null;
  if (envRef) {
    for (var i = 0; i < ws.environments.length; i++) {
      var e = ws.environments[i];
      if (e.id === envRef || e.name.toLowerCase() === String(envRef).toLowerCase()) { env = e; break; }
    }
  }
  if (!env) {
    for (var j = 0; j < ws.environments.length; j++) {
      if (ws.environments[j].id === ws.activeEnvId) { env = ws.environments[j]; break; }
    }
  }
  var map = {};
  if (env && Array.isArray(env.vars)) {
    for (var k = 0; k < env.vars.length; k++) {
      var v = env.vars[k];
      if (v && v.enabled !== false && v.key) map[v.key] = String(v.value == null ? '' : v.value);
    }
  }
  if (overrides && typeof overrides === 'object') {
    Object.keys(overrides).forEach(function (key) {
      map[key] = String(overrides[key] == null ? '' : overrides[key]);
    });
  }
  return { env: env, map: map };
}

/**
 * 替换字符串里的 {{name}}。未定义的变量原样保留，并写入 warnings（去重）。
 * 内置动态变量：{{$timestamp}} {{$isoTimestamp}} {{$uuid}} {{$randomInt}}。
 */
function resolveString(value, vars, warnings) {
  return String(value == null ? '' : value).replace(/\{\{\s*([^{}]+?)\s*\}\}/g, function (m, name) {
    if (Object.prototype.hasOwnProperty.call(vars, name)) return vars[name];
    var dyn = dynamic(name);
    if (dyn !== null) return dyn;
    if (warnings && warnings.indexOf(name) < 0) warnings.push(name);
    return m;
  });
}

function resolveObject(obj, vars, warnings) {
  var out = {};
  Object.keys(obj || {}).forEach(function (k) {
    out[k] = resolveString(obj[k], vars, warnings);
  });
  return out;
}

exports.uuid = uuid;
exports.collect = collect;
exports.resolveString = resolveString;
exports.resolveObject = resolveObject;
