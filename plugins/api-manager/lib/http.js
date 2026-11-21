'use strict';
// HTTP 执行：main 没有 fetch，统一交给设置里配置的 Node 运行时。
// Node 脚本把结果输出成「纯 ASCII 的 JSON」，避免 GUI 下按 GBK 解码中文乱码。

/** 在 Node 里执行的脚本源码（node -e）。参数：process.argv[1] = 请求 JSON。 */
function nodeScript() {
  'use strict';
  var http = require('http');
  var https = require('https');
  var zlib = require('zlib');
  var URL = require('url').URL;

  var MAX_BYTES = 131072;
  var MAX_CHARS = 100000;

  function ascii(s) {
    return s.replace(/[\u007f-\uffff]/g, function (c) {
      return '\\u' + ('0000' + c.charCodeAt(0).toString(16)).slice(-4);
    });
  }

  function lowerHeaders(h) {
    var out = {};
    Object.keys(h || {}).forEach(function (k) {
      var v = h[k];
      if (v === undefined || v === null) return;
      out[String(k).toLowerCase()] = String(v);
    });
    return out;
  }

  var started = Date.now();
  var input;
  try {
    input = JSON.parse(process.argv[1] || '{}');
  } catch (e) {
    process.stdout.write(ascii(JSON.stringify({ ok: false, error: '请求参数不是合法 JSON: ' + e.message })));
    process.exit(0);
    return;
  }

  var method = String(input.method || 'GET').toUpperCase();
  var follow = input.followRedirects !== false;
  var maxRedirects = Math.max(0, Math.min(10, Number(input.maxRedirects) || 5));
  var timeoutMs = Math.max(1000, Math.min(55000, Number(input.timeoutMs) || 30000));
  var baseHeaders = lowerHeaders(input.headers);
  var bodyText = input.body == null ? '' : String(input.body);
  var redirects = [];
  var finished = false;

  function finish(obj) {
    if (finished) return;
    finished = true;
    clearTimeout(timer);
    var payload = ascii(JSON.stringify(obj));
    // 等 stdout 真正刷出去再退出，避免管道场景下截断
    process.stdout.write(payload, function () { process.exit(0); });
  }

  var timer = setTimeout(function () {
    finish({ ok: false, error: '请求超时（' + timeoutMs + 'ms）', durationMs: Date.now() - started });
  }, timeoutMs);

  function send(target, depth) {
    var parsed;
    try {
      parsed = new URL(target);
    } catch (e) {
      finish({ ok: false, error: 'URL 无效: ' + target, durationMs: Date.now() - started });
      return;
    }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      finish({ ok: false, error: '只支持 http/https 协议: ' + parsed.protocol, durationMs: Date.now() - started });
      return;
    }

    var lib = parsed.protocol === 'https:' ? https : http;
    var headers = {};
    Object.keys(baseHeaders).forEach(function (k) { headers[k] = baseHeaders[k]; });
    if (!headers['accept-encoding']) headers['accept-encoding'] = 'gzip, deflate, br';

    var payload = null;
    if (method !== 'GET' && method !== 'HEAD' && bodyText) {
      payload = Buffer.from(bodyText, 'utf8');
      if (!headers['content-length']) headers['content-length'] = String(payload.length);
    }
    if (parsed.username && !headers['authorization']) {
      var cred = decodeURIComponent(parsed.username) + ':' + decodeURIComponent(parsed.password || '');
      headers['authorization'] = 'Basic ' + Buffer.from(cred, 'utf8').toString('base64');
    }

    var req = lib.request({
      method: method,
      protocol: parsed.protocol,
      hostname: parsed.hostname,
      port: parsed.port || undefined,
      path: parsed.pathname + parsed.search,
      headers: headers
    }, function (res) {
      var status = res.statusCode || 0;
      var location = res.headers.location;
      if (follow && location && [301, 302, 303, 307, 308].indexOf(status) >= 0 && depth < maxRedirects) {
        var next = '';
        try { next = new URL(location, target).toString(); } catch (e) { next = ''; }
        if (next) {
          redirects.push({ status: status, from: target, to: next });
          res.resume();
          if (status === 303 || ((status === 301 || status === 302) && method !== 'GET' && method !== 'HEAD')) {
            method = 'GET';
            bodyText = '';
            delete baseHeaders['content-type'];
            delete baseHeaders['content-length'];
          }
          send(next, depth + 1);
          return;
        }
      }

      var chunks = [];
      var size = 0;
      var cut = false;
      var completed = false;

      function done(buf) {
        if (completed) return;
        completed = true;
        var enc = String(res.headers['content-encoding'] || '').toLowerCase();

        function after(err, out) {
          if (finished) return;
          var text;
          try {
            text = (out || buf).toString('utf8');
            if (text.length > MAX_CHARS) {
              text = text.slice(0, MAX_CHARS);
              cut = true;
            }
          } catch (e) {
            finish({ ok: false, error: '读取响应失败: ' + e.message, durationMs: Date.now() - started });
            return;
          }
          var ct = String(res.headers['content-type'] || '');
          var binary = !/text|json|xml|javascript|html|form|urlencoded|csv|yaml|graphql/i.test(ct);
          finish({
            ok: true,
            status: status,
            statusText: res.statusMessage || '',
            headers: res.headers,
            body: text,
            binary: binary,
            size: size,
            durationMs: Date.now() - started,
            url: target,
            redirects: redirects,
            truncated: cut
          });
        }

        try {
          if (cut) {
            // 超限时不再解压，直接把已拿到的原始字节当文本
            after(null, buf);
          } else if (enc === 'gzip' || enc === 'x-gzip') {
            zlib.gunzip(buf, after);
          } else if (enc === 'deflate') {
            zlib.inflate(buf, after);
          } else if (enc === 'br' && zlib.brotliDecompress) {
            zlib.brotliDecompress(buf, after);
          } else {
            after(null, buf);
          }
        } catch (e) {
          after(null, buf);
        }
      }

      res.on('data', function (chunk) {
        size += chunk.length;
        if (size > MAX_BYTES) {
          cut = true;
          var keep = chunk.length - (size - MAX_BYTES);
          if (keep > 0) chunks.push(chunk.slice(0, keep));
          size = MAX_BYTES;
          res.destroy();
          done(Buffer.concat(chunks));
          return;
        }
        chunks.push(chunk);
      });
      res.on('end', function () { done(Buffer.concat(chunks)); });
      res.on('error', function () { done(Buffer.concat(chunks)); });
    });

    req.on('error', function (e) {
      finish({
        ok: false,
        error: String((e && e.message) || e),
        code: e && e.code,
        durationMs: Date.now() - started,
        url: target
      });
    });

    if (payload) req.write(payload);
    req.end();
  }

  if (!input.url) {
    finish({ ok: false, error: '缺少 url', durationMs: Date.now() - started });
    return;
  }
  send(String(input.url), 0);
}

exports.SOURCE = '(' + nodeScript.toString() + ')();';

/**
 * 执行一次请求。input:
 * { method, url, headers:{k:v}, body:string|null, timeoutMs, followRedirects, maxRedirects }
 */
function perform(input) {
  var text = JSON.stringify(input);
  // Windows 命令行上限 32767 字符，给脚本名与解释器参数留余量
  if (text.length > 24000) {
    return Promise.reject(new Error(
      '请求序列化后超过 24000 字符（当前 ' + text.length + '），URL/请求头/请求体需要精简'
    ));
  }
  var timeout = Math.max(5000, Math.min(60000, (Number(input.timeoutMs) || 30000) + 4000));
  return efto.node.run({ code: exports.SOURCE, args: [text], timeoutMs: timeout }).then(function (r) {
    if (r.code !== 0) {
      throw new Error('Node 执行失败（exit ' + r.code + '）: ' + String(r.stderr || '').trim());
    }
    var out = String(r.stdout || '').trim();
    if (!out) {
      throw new Error('Node 没有输出' + (r.stderr ? '（stderr: ' + String(r.stderr).trim() + '）' : ''));
    }
    var data;
    try {
      data = JSON.parse(out);
    } catch (e) {
      throw new Error('响应不是合法 JSON: ' + out.slice(0, 200));
    }
    if (r.truncated) data.truncated = true;
    return data;
  }).catch(function (e) {
    var msg = String((e && e.message) || e);
    if (msg.indexOf('未配置') >= 0) {
      throw new Error(msg + '（API 管理通过 Node 发送请求：设置 → 运行时 → Node.js）');
    }
    throw new Error(msg);
  });
}

/** 读取一个 JSON 文本文件（Node 执行，非 ASCII 转义后返回）。 */
function readScript() {
  'use strict';
  var fs = require('fs');
  var path = process.argv[1];
  var text = fs.readFileSync(path, 'utf8');
  if (text.charCodeAt(0) === 0xfeff) text = text.slice(1);
  process.stdout.write(text.replace(/[\u007f-\uffff]/g, function (c) {
    return '\\u' + ('0000' + c.charCodeAt(0).toString(16)).slice(-4);
  }));
}
var READ_SOURCE = '(' + readScript.toString() + ')();';

function readJsonText(path) {
  if (!path) return Promise.reject(new Error('缺少文件路径'));
  return efto.node.run({ code: READ_SOURCE, args: [String(path)], timeoutMs: 15000 }).then(function (r) {
    if (r.code !== 0) throw new Error('读取文件失败: ' + String(r.stderr || '').trim());
    return String(r.stdout || '');
  });
}

exports.perform = perform;
exports.readJsonText = readJsonText;

