'use strict';
// 通用小工具：ID、时间、深拷贝、命令行分词、UTF-8 Base64。

function isObj(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function newId(prefix) {
  return String(prefix || 'id') + '-' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

function nowIso() {
  return new Date().toISOString();
}

function clone(v) {
  if (v === undefined) return undefined;
  return JSON.parse(JSON.stringify(v));
}

function trim(s) {
  return String(s == null ? '' : s).trim();
}

function truncate(s, max) {
  s = String(s == null ? '' : s);
  if (s.length <= max) return s;
  return s.slice(0, Math.max(0, max)) + '…';
}

function formatBytes(n) {
  n = Number(n) || 0;
  if (n < 1024) return n + ' B';
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + ' KB';
  return (n / (1024 * 1024)).toFixed(1) + ' MB';
}

/** 处理双引号/单引号与反斜杠转义的简单分词。 */
function tokenize(input) {
  var s = String(input == null ? '' : input);
  var out = [];
  var cur = '';
  var quote = '';
  var escaped = false;
  for (var i = 0; i < s.length; i++) {
    var ch = s.charAt(i);
    if (escaped) {
      cur += ch;
      escaped = false;
      continue;
    }
    if (quote) {
      if (ch === '\\') { escaped = true; continue; }
      if (ch === quote) { quote = ''; continue; }
      cur += ch;
      continue;
    }
    if (ch === '"' || ch === "'") { quote = ch; continue; }
    if (/\s/.test(ch)) {
      if (cur) { out.push(cur); cur = ''; }
      continue;
    }
    cur += ch;
  }
  if (cur) out.push(cur);
  return out;
}

var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function utf8Bytes(str) {
  var bytes = [];
  for (var i = 0; i < str.length; i++) {
    var c = str.charCodeAt(i);
    if (c < 0x80) {
      bytes.push(c);
    } else if (c < 0x800) {
      bytes.push(0xc0 | (c >> 6), 0x80 | (c & 63));
    } else if (c >= 0xd800 && c <= 0xdbff && i + 1 < str.length) {
      var c2 = str.charCodeAt(i + 1);
      var cp = 0x10000 + ((c - 0xd800) << 10) + (c2 - 0xdc00);
      bytes.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 63), 0x80 | ((cp >> 6) & 63), 0x80 | (cp & 63));
      i++;
    } else {
      bytes.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
  }
  return bytes;
}

/** UTF-8 字符串 → Base64（Basic Auth 用，main 里没有 btoa）。 */
function base64(str) {
  var bytes = utf8Bytes(String(str == null ? '' : str));
  var out = '';
  for (var i = 0; i < bytes.length; i += 3) {
    var b0 = bytes[i];
    var b1 = i + 1 < bytes.length ? bytes[i + 1] : NaN;
    var b2 = i + 2 < bytes.length ? bytes[i + 2] : NaN;
    out += B64.charAt(b0 >> 2);
    out += B64.charAt(((b0 & 3) << 4) | (isNaN(b1) ? 0 : b1 >> 4));
    out += isNaN(b1) ? '=' : B64.charAt(((b1 & 15) << 2) | (isNaN(b2) ? 0 : b2 >> 6));
    out += isNaN(b2) ? '=' : B64.charAt(b2 & 63);
  }
  return out;
}

exports.isObj = isObj;
exports.newId = newId;
exports.nowIso = nowIso;
exports.clone = clone;
exports.trim = trim;
exports.truncate = truncate;
exports.formatBytes = formatBytes;
exports.tokenize = tokenize;
exports.base64 = base64;
