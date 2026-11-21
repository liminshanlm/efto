'use strict';
// Swagger / OpenAPI 导入。
// 规范可能是几 MB，而插件只能从 Node 拿到 1MB stdout，所以「抓取 + 摘要」全部放在
// Node 脚本里完成，主进程只接收按 tag 归好类的操作列表。
// 支持：直接给 api-docs JSON / YAML 之外的 OpenAPI 地址、knife4j 的 doc.html
// （自动探测 swagger-config、swagger-resources、v2/v3/api-docs）。

function swaggerScript() {
  'use strict';
  var http = require('http');
  var https = require('https');
  var zlib = require('zlib');
  var URL = require('url').URL;

  var MAX_SPEC = 8 * 1024 * 1024;
  var MAX_OUT = 900000;
  var TIMEOUT = 15000;
  var METHODS = ['get', 'post', 'put', 'patch', 'delete', 'head', 'options'];

  function ascii(s) {
    return s.replace(/[\u007f-\uffff]/g, function (c) {
      return '\\u' + ('0000' + c.charCodeAt(0).toString(16)).slice(-4);
    });
  }

  function finish(obj) {
    var text = ascii(JSON.stringify(obj));
    if (text.length > MAX_OUT) {
      text = ascii(JSON.stringify({ ok: false, error: 'Swagger 摘要超过输出上限（' + text.length + ' 字符）' }));
    }
    process.stdout.write(text, function () { process.exit(0); });
  }

  function get(url, depth) {
    return new Promise(function (resolve, reject) {
      var parsed;
      try { parsed = new URL(url); } catch (e) { return reject(new Error('URL 无效: ' + url)); }
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        return reject(new Error('只支持 http/https: ' + url));
      }
      var lib = parsed.protocol === 'https:' ? https : http;
      var req = lib.request({
        method: 'GET',
        hostname: parsed.hostname,
        port: parsed.port || undefined,
        path: parsed.pathname + parsed.search,
        headers: {
          accept: 'application/json, text/plain, */*',
          'accept-encoding': 'gzip, deflate, br'
        }
      }, function (res) {
        var status = res.statusCode || 0;
        var loc = res.headers.location;
        if (loc && [301, 302, 303, 307, 308].indexOf(status) >= 0 && depth < 5) {
          res.resume();
          var next = '';
          try { next = new URL(loc, url).toString(); } catch (e) { next = ''; }
          if (next) { resolve(get(next, depth + 1)); return; }
        }
        var chunks = [];
        var size = 0;
        var tooBig = false;
        res.on('data', function (c) {
          size += c.length;
          if (size > MAX_SPEC) { tooBig = true; res.destroy(); return; }
          chunks.push(c);
        });
        res.on('error', reject);
        res.on('end', function () {
          if (tooBig) return reject(new Error('规范超过 8MB: ' + url));
          var buf = Buffer.concat(chunks);
          var enc = String(res.headers['content-encoding'] || '').toLowerCase();
          function done(err, out) {
            if (err) return reject(err);
            resolve({ status: status, text: (out || buf).toString('utf8') });
          }
          if (enc === 'gzip') zlib.gunzip(buf, done);
          else if (enc === 'br' && zlib.brotliDecompress) zlib.brotliDecompress(buf, done);
          else if (enc === 'deflate') zlib.inflate(buf, done);
          else done(null, buf);
        });
      });
      req.setTimeout(TIMEOUT, function () { req.destroy(new Error('请求超时（' + TIMEOUT + 'ms）: ' + url)); });
      req.on('error', reject);
      req.end();
    });
  }

  function getJson(url) {
    return get(url, 0).then(function (r) {
      if (r.status !== 200) throw new Error('HTTP ' + r.status + ' · ' + url);
      try {
        return JSON.parse(r.text);
      } catch (e) {
        throw new Error('响应不是 JSON（可能是 YAML 或页面）: ' + url);
      }
    });
  }

  function resolveUrl(base, ref) {
    try { return new URL(String(ref || ''), base).toString(); } catch (e) { return ''; }
  }

  function discover(start) {
    if (/api-docs/i.test(start) || /\.json(\?|$)/i.test(start)) {
      return getJson(start).then(function (spec) {
        return [{ name: '', spec: spec, specUrl: start }];
      });
    }
    var u = new URL(start);
    var origin = u.protocol + '//' + u.host;
    var dir = u.pathname.replace(/\/[^/]*$/, '');
    var prefixes = [];
    if (dir && dir !== '/') prefixes.push(origin + dir);
    if (prefixes.indexOf(origin) < 0) prefixes.push(origin);

    var errors = [];
    var chain = Promise.resolve(null);

    function attempt(fn) {
      chain = chain.then(function (found) {
        if (found) return found;
        return fn().then(function (got) { return got || null; });
      });
      return chain;
    }

    prefixes.forEach(function (p) {
      // SpringDoc / knife4j 的分组配置
      attempt(function () {
        return getJson(p + '/v3/api-docs/swagger-config').then(function (cfg) {
          var urls = cfg && Array.isArray(cfg.urls) ? cfg.urls : [];
          if (!urls.length) {
            return getJson(p + '/v3/api-docs').then(function (spec) {
              return [{ name: '', spec: spec, specUrl: p + '/v3/api-docs' }];
            });
          }
          var groups = [];
          return urls.reduce(function (prev, item) {
            return prev.then(function () {
              item = item || {};
              var specUrl = resolveUrl(p + '/', item.url);
              if (!specUrl) return;
              return getJson(specUrl).then(function (spec) {
                groups.push({ name: String(item.name || ''), spec: spec, specUrl: specUrl });
              });
            });
          }, Promise.resolve()).then(function () {
            return groups.length ? groups : null;
          });
        }).catch(function (e) { errors.push(String(e.message || e)); return null; });
      });

      // springfox / swagger-resources
      attempt(function () {
        return getJson(p + '/swagger-resources').then(function (resources) {
          if (!Array.isArray(resources) || !resources.length) return null;
          var groups = [];
          return resources.reduce(function (prev, res) {
            return prev.then(function () {
              res = res || {};
              var rawUrl = String(res.url || res.location || '');
              if (!rawUrl) return;
              var groupName = String(res.name || '');
              if (rawUrl.indexOf('group=') < 0 && groupName) {
                rawUrl += (rawUrl.indexOf('?') >= 0 ? '&' : '?') + 'group=' + encodeURIComponent(groupName);
              }
              var abs = resolveUrl(p + '/', rawUrl);
              if (!abs) return;
              return getJson(abs).then(function (spec) {
                groups.push({ name: groupName, spec: spec, specUrl: abs });
              });
            });
          }, Promise.resolve()).then(function () {
            return groups.length ? groups : null;
          });
        }).catch(function (e) { errors.push(String(e.message || e)); return null; });
      });

      ['/v3/api-docs', '/v2/api-docs'].forEach(function (suffix) {
        attempt(function () {
          var specUrl = p + suffix;
          return getJson(specUrl).then(function (spec) {
            return [{ name: '', spec: spec, specUrl: specUrl }];
          }).catch(function (e) { errors.push(String(e.message || e)); return null; });
        });
      });
    });

    return chain.then(function (found) {
      if (found) return found;
      throw new Error('没有找到可用的 OpenAPI 规范。已尝试: ' + errors.join(' | '));
    });
  }

  function resolveRef(schema, spec, depth) {
    var guard = 0;
    while (schema && schema.$ref && depth-- > 0 && guard++ < 8) {
      var name = String(schema.$ref).split('/').pop();
      var defs = (spec.components && spec.components.schemas) || spec.definitions || {};
      schema = defs[name];
    }
    return schema;
  }

  function example(schema, spec, depth) {
    if (!schema || depth > 5) return null;
    if (schema.$ref) return example(resolveRef(schema, spec, 3), spec, depth + 1);
    if (schema.example !== undefined) return schema.example;
    if (schema.default !== undefined) return schema.default;
    if (Array.isArray(schema.enum) && schema.enum.length) return schema.enum[0];
    if (Array.isArray(schema.allOf)) {
      var merged = {};
      schema.allOf.forEach(function (s) {
        var e = example(s, spec, depth + 1);
        if (e && typeof e === 'object' && !Array.isArray(e)) {
          Object.keys(e).forEach(function (k) { merged[k] = e[k]; });
        }
      });
      return merged;
    }
    if (schema.properties) {
      var obj = {};
      Object.keys(schema.properties).forEach(function (k) {
        obj[k] = example(schema.properties[k], spec, depth + 1);
      });
      return obj;
    }
    if (schema.type === 'array') return [example(schema.items, spec, depth + 1)];
    if (schema.type === 'integer' || schema.type === 'number') return 0;
    if (schema.type === 'boolean') return false;
    return '';
  }

  function bodyOf(spec, op, isV3) {
    var content = null;
    if (isV3 && op.requestBody && op.requestBody.content) {
      var cs = op.requestBody.content;
      content = cs['application/json'] || cs['*/*'] || cs[Object.keys(cs)[0]];
    }
    if (!content && !isV3 && Array.isArray(op.parameters)) {
      for (var i = 0; i < op.parameters.length; i++) {
        if (op.parameters[i] && op.parameters[i].in === 'body') {
          content = { schema: op.parameters[i].schema };
          break;
        }
      }
    }
    if (!content || !content.schema) return null;
    var value = example(content.schema, spec, 0);
    if (value === null || value === undefined || value === '') return null;
    return JSON.stringify(value, null, 2);
  }

  function summarize(spec, groupName, specUrl) {
    var isV3 = !!spec.openapi;
    var info = spec.info || {};
    var title = String(info.title || groupName || 'Swagger');

    var base = '';
    if (isV3) {
      var servers = Array.isArray(spec.servers) ? spec.servers : [];
      if (servers.length) base = String(servers[0].url || '');
    } else {
      var host = String(spec.host || '');
      var bp = String(spec.basePath || '');
      var scheme = (spec.schemes && spec.schemes[0]) || 'http';
      base = host ? scheme + '://' + host + bp : bp;
    }
    base = base.replace(/\/+$/, '');

    var ops = [];
    var paths = spec.paths || {};
    Object.keys(paths).forEach(function (p) {
      var item = paths[p] || {};
      METHODS.forEach(function (m) {
        var op = item[m];
        if (!op || typeof op !== 'object') return;
        var tag = (op.tags && op.tags.length ? String(op.tags[0]) : '') || '默认';
        var params = [];
        (op.parameters || []).forEach(function (ps) {
          if (!ps) return;
          var schema = ps.schema || {};
          var sample = ps.example;
          if (sample === undefined) sample = ps.default;
          if (sample === undefined) sample = schema.default;
          if (sample === undefined) sample = '';
          params.push({
            name: String(ps.name || ''),
            in: String(ps.in || ''),
            required: ps.required === true,
            value: sample
          });
        });
        var name = String(op.summary || op.operationId || (m.toUpperCase() + ' ' + p));
        ops.push({
          method: m.toUpperCase(),
          path: p,
          tag: tag,
          name: name.replace(/\s+/g, ' ').trim().slice(0, 64),
          params: params,
          body: bodyOf(spec, op, isV3)
        });
      });
    });
    return { name: groupName || title, title: title, base: base, specUrl: specUrl, operations: ops };
  }

  (function main() {
    var input;
    try {
      input = JSON.parse(process.argv[1] || '{}');
    } catch (e) {
      finish({ ok: false, error: '参数不是合法 JSON' });
      return;
    }
    var start = String(input.url || '').trim();
    if (!start) {
      finish({ ok: false, error: '缺少 Swagger 地址' });
      return;
    }
    discover(start).then(function (groups) {
      var out = [];
      groups.forEach(function (g) {
        var spec = g.spec;
        if (!spec || (!spec.paths && !spec.openapi && !spec.swagger)) {
          throw new Error('不是 OpenAPI/Swagger 文档: ' + g.specUrl);
        }
        out.push(summarize(spec, g.name, g.specUrl));
      });
      var total = 0;
      out.forEach(function (g) { total += g.operations.length; });
      finish({ ok: true, url: start, groups: out, operationCount: total });
    }).catch(function (e) {
      finish({ ok: false, error: String((e && e.message) || e) });
    });
  })();
}

var SOURCE = '(' + swaggerScript.toString() + ')();';

function fetchGroups(url) {
  var target = String(url || '').trim();
  if (!target) return Promise.reject(new Error('缺少 Swagger 地址'));
  var payload = JSON.stringify({ url: target });
  return efto.node.run({ code: SOURCE, args: [payload], timeoutMs: 60000 }).then(function (r) {
    if (r.code !== 0) {
      throw new Error('Node 执行失败（exit ' + r.code + '）: ' + String(r.stderr || '').trim());
    }
    var text = String(r.stdout || '').trim();
    var out;
    try {
      out = JSON.parse(text);
    } catch (e) {
      throw new Error('Node 输出不是 JSON: ' + text.slice(0, 200));
    }
    if (!out.ok) throw new Error(out.error || '读取 Swagger 失败');
    return out;
  });
}

function originOf(url) {
  var m = /^([a-z][a-z0-9+.-]*:\/\/[^/?#]+)/i.exec(String(url || ''));
  return m ? m[1] : '';
}

/** 规范里的 base 可能是相对路径（springfox 没写 host），用规范地址的 origin 补齐。 */
function absoluteBase(g) {
  var base = String(g.base || '').replace(/\/+$/, '');
  if (base && !/^[a-z][a-z0-9+.-]*:\/\//i.test(base)) {
    var origin = originOf(g.specUrl);
    if (origin) base = origin + (base.charAt(0) === '/' ? base : '/' + base);
  }
  return base;
}

function envNameFor(base) {
  return base.replace(/^[a-z][a-z0-9+.-]*:\/\//i, '') || base;
}

/**
 * 把 Node 摘要转成与 transfer.parseImport 相同形状（请求带 folderPath）。
 * URL 一律写成 `{{baseUrl}}` + 路径，并生成同名环境保存真实地址，
 * 导入后可直接切环境改地址，不用逐条改请求。
 */
function toParsed(result) {
  var collections = [];
  var envBases = [];
  (result.groups || []).forEach(function (g) {
    var base = absoluteBase(g);
    if (base && envBases.indexOf(base) < 0) envBases.push(base);
    var colName = String(g.name || g.title || 'Swagger 导入');
    var requests = [];
    (g.operations || []).forEach(function (op) {
      var path = String(op.path || '') || '/';
      if (path.charAt(0) !== '/') path = '/' + path;
      path = path.replace(/\{([^{}\/]+)\}/g, '{{$1}}');
      var url = base ? '{{baseUrl}}' + path : path;
      var params = [];
      var headers = [];
      (op.params || []).forEach(function (p) {
        if (!p.name) return;
        if (p.in === 'query') {
          params.push({ key: p.name, value: p.value == null ? '' : String(p.value), enabled: true });
        } else if (p.in === 'header') {
          headers.push({ key: p.name, value: p.value == null ? '' : String(p.value), enabled: true });
        }
      });
      requests.push({
        name: op.name || (op.method + ' ' + op.path),
        method: op.method,
        url: url,
        params: params,
        headers: headers,
        body: op.body
          ? { mode: 'json', raw: op.body, form: [] }
          : { mode: 'none', raw: '', form: [] },
        folderPath: [op.tag || '默认']
      });
    });
    collections.push({
      name: colName,
      description: '来自 Swagger: ' + (g.specUrl || ''),
      requests: requests
    });
  });
  return {
    collections: collections,
    environments: envBases.map(function (b) {
      return { name: envNameFor(b), vars: [{ key: 'baseUrl', value: b, enabled: true }] };
    }),
    activateEnv: true
  };
}

function importFromUrl(url) {
  return fetchGroups(url).then(toParsed);
}

exports.importFromUrl = importFromUrl;
