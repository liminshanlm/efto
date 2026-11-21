// fmt-json.js —— JSON 美化器（零依赖，浏览器与 Node 双用）。
// 不经过 JSON.parse/JSON.stringify：数字字面量、键的顺序、字符串转义一律原样保留，
// 避免 1.0 变 1、大整数丢精度这类"美化把数据改坏"的问题。
// 宽容处理：允许 // 与 /* */ 注释（会被丢弃）、允许尾随逗号。

(function (root) {
  'use strict';

  function Err(message, pos) {
    this.message = message;
    this.pos = pos;
  }

  function repeat(s, n) {
    var out = '';
    for (var i = 0; i < n; i++) out += s;
    return out;
  }

  function lineCol(src, pos) {
    var p = Math.max(0, Math.min(Number(pos) || 0, src.length));
    var line = 1;
    var last = 0;
    for (var i = 0; i < p; i++) {
      if (src.charAt(i) === '\n') {
        line++;
        last = i + 1;
      }
    }
    return { line: line, column: p - last + 1 };
  }

  function isSpace(c) {
    return c === ' ' || c === '\t' || c === '\r' || c === '\n' || c === '\uFEFF' || c === '\u00A0';
  }

  var PUNCT = '{}[],:';

  function tokenize(src) {
    var tokens = [];
    var i = 0;
    var n = src.length;

    while (i < n) {
      var c = src.charAt(i);

      if (isSpace(c)) {
        i++;
        continue;
      }

      // 注释：JSON 本身不支持，但用户粘过来的多是 JSONC，直接丢弃
      if (c === '/' && src.charAt(i + 1) === '/') {
        while (i < n && src.charAt(i) !== '\n') i++;
        continue;
      }
      if (c === '/' && src.charAt(i + 1) === '*') {
        var endc = src.indexOf('*/', i + 2);
        if (endc < 0) throw new Err('块注释没有结束（缺少 */）', i);
        i = endc + 2;
        continue;
      }

      if (c === '"') {
        var s0 = i;
        i++;
        var closed = false;
        while (i < n) {
          var ch = src.charAt(i);
          if (ch === '\\') {
            i += 2;
            continue;
          }
          if (ch === '"') {
            i++;
            closed = true;
            break;
          }
          if (ch === '\n') break;
          i++;
        }
        if (!closed) throw new Err('字符串没有结束（缺少收尾的双引号）', s0);
        tokens.push({ type: 'string', text: src.slice(s0, i), pos: s0 });
        continue;
      }

      if (PUNCT.indexOf(c) >= 0) {
        tokens.push({ type: 'punct', text: c, pos: i });
        i++;
        continue;
      }

      if (c === '-' || (c >= '0' && c <= '9')) {
        var num = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?/.exec(src.slice(i));
        if (!num) throw new Err('无法识别的数字字面量', i);
        var n0 = i;
        i += num[0].length;
        var next = src.charAt(i);
        if (next && (/[0-9A-Za-z_$]/.test(next) || next === '.')) {
          throw new Err('数字后面紧跟了非法字符 "' + next + '"', i);
        }
        tokens.push({ type: 'number', text: src.slice(n0, i), pos: n0 });
        continue;
      }

      var lit = /^(true|false|null)/.exec(src.slice(i));
      if (lit) {
        var after = src.charAt(i + lit[0].length);
        if (!after || !/[A-Za-z0-9_$]/.test(after)) {
          tokens.push({ type: 'literal', text: lit[0], pos: i });
          i += lit[0].length;
          continue;
        }
      }

      throw new Err('无法识别的字符 ' + JSON.stringify(c) + '（JSON 的键与字符串必须用双引号）', i);
    }

    return tokens;
  }

  function findClose(tokens, from) {
    var depth = 0;
    for (var j = from; j < tokens.length; j++) {
      var t = tokens[j].text;
      if (t === '[' || t === '{') depth++;
      else if (t === ']' || t === '}') {
        depth--;
        if (depth === 0) return j;
      }
    }
    return -1;
  }

  function format(text, opts) {
    var src = String(text == null ? '' : text);
    var options = opts || {};
    var indent = typeof options.indent === 'string' && options.indent.length ? options.indent : '  ';
    var nl = options.newline === '\r\n' ? '\r\n' : '\n';
    var compactArrays = options.compactArrays !== false;
    var maxWidth = Number(options.maxWidth) > 0 ? Number(options.maxWidth) : 72;
    var maxDepth = 200;

    try {
      var tokens = tokenize(src);
      if (!tokens.length) return { ok: false, error: '没有可美化的内容' };

      var out = '';
      var depth = 0;
      var idx = 0;

      function nlIndent() {
        out += nl + repeat(indent, depth);
      }

      function value() {
        var t = tokens[idx];
        if (!t) throw new Err('内容在值出现之前就结束了', src.length);
        if (t.text === '{') return obj();
        if (t.text === '[') return arr();
        if (t.type === 'string' || t.type === 'number' || t.type === 'literal') {
          out += t.text;
          idx++;
          return;
        }
        throw new Err('这里应该是一个值，却遇到 ' + JSON.stringify(t.text), t.pos);
      }

      function obj() {
        var open = tokens[idx];
        out += '{';
        idx++;
        if (tokens[idx] && tokens[idx].text === '}') {
          out += '}';
          idx++;
          return;
        }
        if (depth >= maxDepth) throw new Err('嵌套层数过深（超过 ' + maxDepth + ' 层）', open.pos);
        depth++;
        for (;;) {
          var t = tokens[idx];
          if (!t) throw new Err('对象没有闭合（缺少 }）', open.pos);
          if (t.text === '}') break;
          if (t.type !== 'string') {
            throw new Err('对象的键必须是双引号字符串，却遇到 ' + JSON.stringify(t.text), t.pos);
          }
          nlIndent();
          out += t.text;
          idx++;
          var colon = tokens[idx];
          if (!colon || colon.text !== ':') {
            throw new Err('键后面缺少冒号', (colon || t).pos);
          }
          out += ': ';
          idx++;
          value();
          var sep = tokens[idx];
          if (sep && sep.text === ',') {
            // 尾随逗号在 JSON 里非法：吃掉逗号但不再输出，避免美化后仍是坏 JSON
            var afterComma = tokens[idx + 1];
            if (afterComma && afterComma.text === '}') {
              idx++;
              break;
            }
            out += ',';
            idx++;
            continue;
          }
          if (sep && sep.text === '}') break;
          if (!sep) throw new Err('对象没有闭合（缺少 }）', open.pos);
          throw new Err('对象成员之间缺少逗号', sep.pos);
        }
        depth--;
        nlIndent();
        out += '}';
        idx++;
      }

      function arr() {
        var open = tokens[idx];

        // 元素全是标量的短数组保持单行，读起来比一列数字清爽
        if (compactArrays) {
          var closeIdx = findClose(tokens, idx);
          if (closeIdx > idx) {
            var simple = true;
            var items = [];
            var cur = [];
            for (var j = idx + 1; j < closeIdx; j++) {
              var tk = tokens[j];
              if (tk.text === ',') {
                // 连续逗号（"1,,2"）不是合法数组：放弃单行快路径，交给下面的完整解析报错
                if (!cur.length) {
                  simple = false;
                  break;
                }
                items.push(cur.join(' '));
                cur = [];
                continue;
              }
              if (tk.text === '[' || tk.text === '{') {
                simple = false;
                break;
              }
              cur.push(tk.text);
            }
            if (simple) {
              if (cur.length) items.push(cur.join(' '));
              var joined = '[' + items.join(', ') + ']';
              if (items.length && repeat(indent, depth).length + joined.length <= maxWidth) {
                out += joined;
                idx = closeIdx + 1;
                return;
              }
            }
          }
        }

        out += '[';
        idx++;
        if (tokens[idx] && tokens[idx].text === ']') {
          out += ']';
          idx++;
          return;
        }
        if (depth >= maxDepth) throw new Err('嵌套层数过深（超过 ' + maxDepth + ' 层）', open.pos);
        depth++;
        for (;;) {
          var t = tokens[idx];
          if (!t) throw new Err('数组没有闭合（缺少 ]）', open.pos);
          if (t.text === ']') break;
          nlIndent();
          value();
          var sep = tokens[idx];
          if (sep && sep.text === ',') {
            var afterComma = tokens[idx + 1];
            if (afterComma && afterComma.text === ']') {
              idx++; // 尾随逗号不输出
              break;
            }
            out += ',';
            idx++;
            continue;
          }
          if (sep && sep.text === ']') break;
          if (!sep) throw new Err('数组没有闭合（缺少 ]）', open.pos);
          throw new Err('数组元素之间缺少逗号', sep.pos);
        }
        depth--;
        nlIndent();
        out += ']';
        idx++;
      }

      value();
      if (idx < tokens.length) {
        var extra = tokens[idx];
        throw new Err('一个 JSON 只能有一个根值，这里多出了 ' + JSON.stringify(extra.text), extra.pos);
      }

      return { ok: true, text: out };
    } catch (e) {
      if (e instanceof Err) {
        var lc = lineCol(src, e.pos);
        return { ok: false, error: '第 ' + lc.line + ' 行第 ' + lc.column + ' 列：' + e.message, line: lc.line, column: lc.column };
      }
      return { ok: false, error: '美化失败：' + (e && e.message ? e.message : String(e)) };
    }
  }

  var api = { format: format };

  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  if (root) {
    root.TBFormat = root.TBFormat || {};
    root.TBFormat.json = api;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
