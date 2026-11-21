/*!
 * fmt-js.js — 无第三方依赖的 JavaScript / TypeScript 源码美化器
 * 挂载：root.TBFormat.js.format(text, opts)
 * 契约：永不抛异常，返回 { ok: true, text } 或 { ok: false, error }
 */
(function (root) {
  'use strict';

  /* ------------------------------------------------------------------ *
   * 基础词法表
   * ------------------------------------------------------------------ */

  var KEYWORDS = {};
  (function () {
    var list = (
      'break case catch class const continue debugger default delete do else export extends finally for ' +
      'function if import in instanceof new return switch this throw try typeof var void while with yield let ' +
      'static async await of as from get set ' +
      'enum implements interface package private protected public abstract ' +
      'boolean number string any unknown never object symbol bigint ' +
      'declare namespace module type keyof readonly infer is asserts satisfies override ' +
      'true false null undefined'
    ).split(' ');
    for (var i = 0; i < list.length; i++) {
      if (list[i]) KEYWORDS[list[i]] = true;
    }
  })();

  /* 作为「值」的关键字：其后出现的 / 是除号而非正则起始 */
  var VALUE_KW = { 'true': 1, 'false': 1, 'null': 1, 'this': 1, 'super': 1 };
  /* 其后不需要补空格的词 */
  var NO_SPACE_AFTER = {
    'true': 1, 'false': 1, 'null': 1, 'undefined': 1, 'this': 1, 'super': 1,
    'break': 1, 'continue': 1, 'debugger': 1
  };
  /* 其后紧跟 { 视为代码块的关键字 */
  var BLOCK_AFTER_NAME = {
    'else': 1, 'do': 1, 'try': 1, 'finally': 1, 'static': 1, 'switch': 1, 'catch': 1
  };
  /* 其后紧跟 ( 需要空格的词 */
  var PAREN_SPACE = {
    'if': 1, 'for': 1, 'while': 1, 'switch': 1, 'catch': 1, 'with': 1,
    'return': 1, 'typeof': 1, 'delete': 1, 'void': 1, 'new': 1, 'in': 1,
    'of': 1, 'instanceof': 1, 'await': 1, 'yield': 1, 'throw': 1, 'case': 1,
    'do': 1, 'else': 1
  };

  /* 表达式延续词：行首出现时不视为新语句，也不在 "}" 后另起一行 */
  var CONTINUATION = {
    'else': 1, 'catch': 1, 'finally': 1, 'while': 1, 'as': 1, 'from': 1, 'in': 1, 'of': 1,
    'instanceof': 1, 'extends': 1, 'implements': 1, 'satisfies': 1, 'is': 1, 'keyof': 1
  };

  /* 多字符运算符，长的优先 */
  var PUNCTUATORS = [
    '>>>=',
    '...', '===', '!==', '**=', '<<=', '>>=', '>>>', '&&=', '||=', '??=',
    '=>', '==', '!=', '<=', '>=', '&&', '||', '??', '?.', '++', '--',
    '+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '<<', '>>', '**',
    '{', '}', '(', ')', '[', ']', ';', ',', '<', '>', '+', '-', '*', '/',
    '%', '&', '|', '^', '!', '~', '?', ':', '=', '.', '@', '#'
  ];

  /* ------------------------------------------------------------------ *
   * 字符判定
   * ------------------------------------------------------------------ */

  function isLineBreak(ch) {
    return ch === '\n' || ch === '\r' || ch === '\u2028' || ch === '\u2029';
  }

  function isDigit(ch) {
    return ch >= '0' && ch <= '9';
  }

  function isIdStart(ch) {
    if (!ch) return false;
    if ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || ch === '_' || ch === '$') return true;
    var c = ch.charCodeAt(0);
    if (c < 0x80) return false;
    /* 允许 Unicode 标识符（中文等）：排除常见全角/中文标点与空白 */
    if (c === 0x00a0 || c === 0xfeff) return false;
    if (c >= 0x2000 && c <= 0x206f) return false;
    if (c >= 0x3000 && c <= 0x303f) return false;
    if (c >= 0xff00 && c <= 0xff0f) return false;
    if (c === 0xff1a || c === 0xff1b || c === 0xff08 || c === 0xff09) return false;
    if (c >= 0xff1c && c <= 0xff20) return false;
    if (c >= 0xff3b && c <= 0xff40) return false;
    return true;
  }

  function isIdPart(ch) {
    if (!ch) return false;
    if ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9')) return true;
    if (ch === '_' || ch === '$') return true;
    if (ch === '\u200c' || ch === '\u200d') return true;
    return isIdStart(ch);
  }

  function isRegexFlagChar(ch) {
    return (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z');
  }

  /* ------------------------------------------------------------------ *
   * 词法错误
   * ------------------------------------------------------------------ */

  function makeErr(msg) {
    var e = new Error(msg);
    e.__fmt = msg;
    return e;
  }

  /* ------------------------------------------------------------------ *
   * 子扫描器（不产生 token，只定位边界）
   * ------------------------------------------------------------------ */

  function scanString(src, i) {
    var q = src.charAt(i);
    var n = src.length;
    var j = i + 1;
    while (j < n) {
      var c = src.charAt(j);
      if (c === '\\') { j += 2; continue; }
      if (c === q) return j + 1;
      if (c === '\n' || c === '\r') break;
      j++;
    }
    throw makeErr('字符串字面量未闭合（缺少结束的 ' + q + '）');
  }

  function scanTemplate(src, i) {
    var n = src.length;
    var j = i + 1;
    while (j < n) {
      var c = src.charAt(j);
      if (c === '\\') { j += 2; continue; }
      if (c === '`') return j + 1;
      if (c === '$' && src.charAt(j + 1) === '{') {
        j = scanTemplateExpr(src, j + 2);
        continue;
      }
      j++;
    }
    throw makeErr('模板字符串未闭合（缺少结束的反引号）');
  }

  /* 从 `${` 之后开始扫描，返回匹配 `}` 之后的位置 */
  function scanTemplateExpr(src, i) {
    var n = src.length;
    var j = i;
    var depth = 1;
    while (j < n) {
      var c = src.charAt(j);
      if (c === '\\') { j += 2; continue; }
      if (c === '"' || c === "'") { j = scanString(src, j); continue; }
      if (c === '`') { j = scanTemplate(src, j); continue; }
      if (c === '/' && src.charAt(j + 1) === '/') {
        while (j < n && !isLineBreak(src.charAt(j))) j++;
        continue;
      }
      if (c === '/' && src.charAt(j + 1) === '*') {
        var endB = src.indexOf('*/', j + 2);
        if (endB < 0) throw makeErr('块注释未闭合（缺少 "*/"）');
        j = endB + 2;
        continue;
      }
      if (c === '{') { depth++; j++; continue; }
      if (c === '}') {
        depth--;
        j++;
        if (depth === 0) return j;
        continue;
      }
      j++;
    }
    throw makeErr('模板字符串中的 "${" 未闭合（缺少配对的 "}"）');
  }

  function scanRegex(src, i) {
    var n = src.length;
    var j = i + 1;
    var inClass = false;
    var closed = false;
    while (j < n) {
      var c = src.charAt(j);
      if (c === '\\') { j += 2; continue; }
      if (c === '\n' || c === '\r' || c === '\u2028' || c === '\u2029') break;
      if (c === '[') { inClass = true; j++; continue; }
      if (c === ']') { inClass = false; j++; continue; }
      if (c === '/' && !inClass) { j++; closed = true; break; }
      j++;
    }
    if (!closed) throw makeErr('正则字面量未闭合（缺少结束的 "/"）');
    while (j < n && isRegexFlagChar(src.charAt(j))) j++;
    return j;
  }

  function scanNumber(src, i) {
    var n = src.length;
    var j = i;
    var c1 = src.charAt(j + 1);
    if (src.charAt(j) === '0' && (c1 === 'x' || c1 === 'X' || c1 === 'b' || c1 === 'B' || c1 === 'o' || c1 === 'O')) {
      j += 2;
      while (j < n && /[0-9a-fA-F_]/.test(src.charAt(j))) j++;
    } else {
      while (j < n && /[0-9_]/.test(src.charAt(j))) j++;
      if (src.charAt(j) === '.') {
        j++;
        while (j < n && /[0-9_]/.test(src.charAt(j))) j++;
      }
      var e = src.charAt(j);
      if (e === 'e' || e === 'E') {
        var k = j + 1;
        if (src.charAt(k) === '+' || src.charAt(k) === '-') k++;
        if (/[0-9]/.test(src.charAt(k))) {
          j = k;
          while (j < n && /[0-9_]/.test(src.charAt(j))) j++;
        }
      }
    }
    if (src.charAt(j) === 'n') j++;
    return j;
  }

  function readPunct(src, i) {
    for (var k = 0; k < PUNCTUATORS.length; k++) {
      var p = PUNCTUATORS[k];
      if (src.substr(i, p.length) === p) return p;
    }
    return src.charAt(i);
  }

  /* 拆分模板字符串为 raw / expr 片段（${ 归入前一段，} 归入后一段） */
  function splitTemplateParts(full) {
    var parts = [];
    var n = full.length;
    var i = 1;
    var rawStart = 0;
    while (i < n) {
      var c = full.charAt(i);
      if (c === '\\') { i += 2; continue; }
      if (c === '`') { i++; break; }
      if (c === '$' && full.charAt(i + 1) === '{') {
        parts.push({ type: 'raw', v: full.slice(rawStart, i + 2) });
        var after = scanTemplateExpr(full, i + 2);
        parts.push({ type: 'expr', src: full.slice(i + 2, after - 1) });
        i = after;
        rawStart = after - 1;
        continue;
      }
      i++;
    }
    if (rawStart < n) parts.push({ type: 'raw', v: full.slice(rawStart) });
    return parts;
  }

  function regexAllowed(prev) {
    if (!prev) return true;
    if (prev.t === 'number' || prev.t === 'string' || prev.t === 'template' || prev.t === 'regex') return false;
    if (prev.t === 'name') {
      if (VALUE_KW[prev.v]) return false;
      return !!KEYWORDS[prev.v];
    }
    if (prev.t === 'punct') {
      var v = prev.v;
      if (v === ')' || v === ']' || v === '}') return false;
      if (v === '++' || v === '--') return false;
      return true;
    }
    return true;
  }

  /* ------------------------------------------------------------------ *
   * Tokenizer
   * ------------------------------------------------------------------ */

  function tokenizeInner(src) {
    var tokens = [];
    var n = src.length;
    var i = 0;
    var nl = 0;
    var prev = null;

    while (i < n) {
      var ch = src.charAt(i);

      if (ch === '\r') {
        if (src.charAt(i + 1) === '\n') i++;
        i++; nl++; continue;
      }
      if (ch === '\n' || ch === '\u2028' || ch === '\u2029') { i++; nl++; continue; }
      if (ch === ' ' || ch === '\t' || ch === '\f' || ch === '\v' || ch === '\u00a0' || ch === '\ufeff') { i++; continue; }

      var start = i;
      var tok = null;

      if (ch === '/' && src.charAt(i + 1) === '/') {
        i += 2;
        while (i < n && !isLineBreak(src.charAt(i))) i++;
        tok = { t: 'lineComment', v: src.slice(start, i) };
      } else if (ch === '/' && src.charAt(i + 1) === '*') {
        var endC = src.indexOf('*/', i + 2);
        if (endC < 0) throw makeErr('块注释未闭合（缺少 "*/"）');
        i = endC + 2;
        tok = { t: 'blockComment', v: src.slice(start, i) };
      } else if (ch === '"' || ch === "'") {
        i = scanString(src, i);
        tok = { t: 'string', v: src.slice(start, i) };
      } else if (ch === '`') {
        i = scanTemplate(src, i);
        var full = src.slice(start, i);
        var parts = null;
        try { parts = splitTemplateParts(full); } catch (e2) { parts = null; }
        tok = { t: 'template', v: full, parts: parts };
      } else if (ch === '/' && regexAllowed(prev)) {
        i = scanRegex(src, i);
        tok = { t: 'regex', v: src.slice(start, i) };
      } else if (isDigit(ch) || (ch === '.' && isDigit(src.charAt(i + 1)))) {
        i = scanNumber(src, i);
        tok = { t: 'number', v: src.slice(start, i) };
      } else if (isIdStart(ch)) {
        i++;
        while (i < n && isIdPart(src.charAt(i))) i++;
        tok = { t: 'name', v: src.slice(start, i) };
      } else {
        var p = readPunct(src, i);
        i += p.length;
        tok = { t: 'punct', v: p };
      }

      tok.nl = nl;
      nl = 0;
      tokens.push(tok);
      if (tok.t !== 'lineComment' && tok.t !== 'blockComment') prev = tok;
    }
    return tokens;
  }

  function tokenize(src) {
    try {
      return { tokens: tokenizeInner(src), error: null };
    } catch (e) {
      return {
        tokens: [],
        error: (e && e.__fmt) || ('词法分析失败：' + (e && e.message ? e.message : String(e)))
      };
    }
  }

  /* ------------------------------------------------------------------ *
   * 泛型尖括号预判（只影响空格，不影响 token 顺序）
   * ------------------------------------------------------------------ */

  function markGenerics(tokens) {
    var open = {};
    var close = {};
    var lastSig = null;
    var breakers = {
      '(': 1, ')': 1, '{': 1, '}': 1, ';': 1, '&&': 1, '||': 1, '=>': 1, '=': 1,
      '==': 1, '===': 1, '!=': 1, '!==': 1, '?': 1, '+': 1, '-': 1, '*': 1,
      '/': 1, '!': 1, '%': 1, '^': 1, '|': 1, '&': 1
    };
    for (var i = 0; i < tokens.length; i++) {
      var t = tokens[i];
      if (t.t === 'lineComment' || t.t === 'blockComment') continue;
      if (t.t === 'punct' && t.v === '<') {
        var prev = lastSig;
        var canBe = false;
        if (prev) {
          if (prev.t === 'name' && VALUE_KW[prev.v]) canBe = true;
          else if (prev.t === 'name' && !KEYWORDS[prev.v]) canBe = true;
          else if (prev.t === 'punct' && (prev.v === ')' || prev.v === ']')) canBe = true;
        }
        if (canBe) {
          var depth = 0;
          var ok = false;
          var endIdx = -1;
          var compound = false;
          for (var j = i; j < tokens.length && j < i + 30; j++) {
            var u = tokens[j];
            if (u.t === 'lineComment' || u.t === 'blockComment') break;
            if (u.t === 'template' || u.t === 'regex') break;
            if (j > i && u.nl > 0) break;
            if (u.t === 'punct') {
              var uv = u.v;
              if (uv === '<') { depth++; }
              else if (uv === '>') { depth--; if (depth === 0) { ok = true; endIdx = j; break; } }
              else if (uv === '>>') { depth -= 2; if (depth <= 0) { ok = true; endIdx = j; break; } }
              else if (uv === '>>>') { depth -= 3; if (depth <= 0) { ok = true; endIdx = j; break; } }
              else if (uv === '>=' && depth === 1) { ok = true; compound = true; endIdx = j; break; }
              else if (breakers[uv]) break;
            } else if (u.t === 'name') {
              /* 类型名、extends、keyof 等，放行 */
            } else if (u.t === 'number' || u.t === 'string') {
              /* 字面量类型，放行 */
            } else {
              break;
            }
          }
          if (ok) {
            open[i] = true;
            close[endIdx] = true;
            tokens[i].__genOpen = true;
            tokens[endIdx].__genClose = true;
            if (compound) tokens[endIdx].__genCloseCompound = true;
          }
        }
      }
      lastSig = t;
    }
    return { open: open, close: close };
  }

  /* ------------------------------------------------------------------ *
   * 引号归一化
   * ------------------------------------------------------------------ */

  function normalizeQuote(v, singleQuote) {
    if (!singleQuote) return v;
    if (v.charAt(0) !== '"' || v.length < 2) return v;
    var body = v.slice(1, -1);
    if (body.indexOf("'") >= 0) return v;
    if (body.indexOf('\\') >= 0) return v;
    if (body.indexOf('\n') >= 0 || body.indexOf('\r') >= 0) return v;
    if (v.charAt(v.length - 1) !== '"') return v;
    return "'" + body + "'";
  }

  /* ------------------------------------------------------------------ *
   * 排版器
   * ------------------------------------------------------------------ */

  function render(tokens, opts) {
    var indentStr = opts.indent;
    var singleQuote = opts.singleQuote;
    var generics = markGenerics(tokens);

    var out = [];
    var cur = '';
    var level = 0;
    var stack = [];
    var pendingSpace = false;
    var caseLabelPending = false;
    var pendingSwitch = false;
    var pendingDecl = false;
    var decActive = false;
    var decFirst = false;
    var decParen = 0;
    var ternCount = 0;

    function ind() {
      if (level <= 0) return '';
      var s = '';
      for (var i = 0; i < level; i++) s += indentStr;
      return s;
    }
    function writeRaw(str) {
      if (cur === '') cur = ind();
      cur += str;
    }
    function write(str) {
      if (cur === '') {
        cur = ind();
      } else if (pendingSpace && !/[ \t]$/.test(cur)) {
        cur += ' ';
      }
      pendingSpace = false;
      cur += str;
    }
    function space() {
      if (cur !== '' && !/[ \t]$/.test(cur)) cur += ' ';
      pendingSpace = false;
    }
    function newline() {
      if (cur.replace(/[ \t]+$/, '') !== '') out.push(cur.replace(/[ \t]+$/, ''));
      cur = '';
      pendingSpace = false;
    }
    function blank() {
      if (out.length > 0 && out[out.length - 1] !== '') out.push('');
    }
    function top() {
      return stack.length ? stack[stack.length - 1] : null;
    }

    /* ---- 模板字符串：表达式内部做单行紧凑排版 ---- */

    function needSpaceInline(prev, tok) {
      if (!prev) return false;
      var pv = prev.v;
      if (prev.t === 'punct') {
        if (pv === '(' || pv === '[' || pv === '.' || pv === '?.' || pv === '...' ||
          pv === '!' || pv === '~' || pv === '++' || pv === '--' || pv === '@' || pv === '#') return false;
        return true;
      }
      if (tok.t === 'punct') {
        var tv = tok.v;
        if (tv === ')' || tv === ']' || tv === ',' || tv === ';' || tv === '.' || tv === '?.' || tv === ':') return false;
        if (tv === '(') return !!(prev.t === 'name' && PAREN_SPACE[pv]);
        if (tv === '[') return false;
        if (tv === '++' || tv === '--') return false;
        return true;
      }
      return true;
    }

    function inlineText(t) {
      if (t.t === 'template') return renderTemplate(t);
      if (t.t === 'string') return normalizeQuote(t.v, singleQuote);
      return t.v;
    }

    /* 模板 ${} 内部：单行紧凑排版 */
    function inlineFormat(src) {
      var r = tokenize(src);
      if (r.error || !r.tokens.length) return null;
      var s = '';
      var prev = null;
      var tern = 0;
      for (var k = 0; k < r.tokens.length; k++) {
        var t = r.tokens[k];
        if (t.t === 'lineComment' || t.t === 'blockComment') return null;
        if (t.t === 'punct' && t.v === '?') { tern++; }
        if (t.t === 'punct' && t.v === ':' && tern > 0) {
          if (s !== '' && !/ $/.test(s)) s += ' ';
          s += ':';
          tern--;
          prev = t;
          continue;
        }
        if (needSpaceInline(prev, t)) s += ' ';
        s += inlineText(t);
        prev = t;
      }
      return s.replace(/^[ \t]+/, '').replace(/[ \t]+$/, '');
    }

    function renderTemplate(tok) {
      if (!tok.parts) return tok.v;
      var s = '';
      for (var k = 0; k < tok.parts.length; k++) {
        var p = tok.parts[k];
        if (p.type === 'raw') { s += p.v; continue; }
        var inner = inlineFormat(p.src);
        s += (inner === null ? p.src : inner);
      }
      return s;
    }

    function tokenText(tok) {
      if (tok.t === 'template') return renderTemplate(tok);
      if (tok.t === 'string') return normalizeQuote(tok.v, singleQuote);
      return tok.v;
    }

    /* ---- 前后有意义 token 索引 ---- */
    var prevs = [];
    var nexts = [];
    var pprevs = [];
    var lastSig = null;
    var lastSig2 = null;
    var i;
    for (i = 0; i < tokens.length; i++) {
      prevs[i] = lastSig;
      pprevs[i] = lastSig2;
      if (tokens[i].t !== 'lineComment' && tokens[i].t !== 'blockComment') {
        lastSig2 = lastSig;
        lastSig = tokens[i];
      }
    }
    var nxt = null;
    for (i = tokens.length - 1; i >= 0; i--) {
      nexts[i] = nxt;
      if (tokens[i].t !== 'lineComment' && tokens[i].t !== 'blockComment') nxt = tokens[i];
    }

    /* ---- 判断辅助 ---- */

    function isBlockBrace(prev, p2, parent) {
      if (!prev) return true;
      if (prev.t === 'name') {
        if (BLOCK_AFTER_NAME[prev.v]) return true;
        /* 类成员方法体：`get x(): number {`、`foo(): void {` */
        if (parent && parent.classBody && !VALUE_KW[prev.v]) {
          if (p2 && p2.t === 'punct' && (p2.v === ':' || p2.v === ')' || p2.v === '>' || p2.v === ']')) return true;
        }
        return false;
      }
      if (prev.t === 'punct') {
        var v = prev.v;
        if (v === ')' || v === ']' || v === '}' || v === ';' || v === '=>') return true;
        return false;
      }
      return false;
    }

    function isIndexAccessP(prev) {
      if (!prev) return false;
      if (prev.t === 'number' || prev.t === 'string' || prev.t === 'template' || prev.t === 'regex') return true;
      if (prev.t === 'name') return !KEYWORDS[prev.v];
      if (prev.t === 'punct') return prev.v === ')' || prev.v === ']' || prev.v === '}';
      return false;
    }

    function isUnaryPosition(prev) {
      if (!prev) return true;
      if (prev.t === 'punct') {
        var v = prev.v;
        if (v === ')' || v === ']' || v === '}' || v === '++' || v === '--') return false;
        return true;
      }
      if (prev.t === 'name') return !!KEYWORDS[prev.v] && !VALUE_KW[prev.v];
      return false;
    }

    /* 一元 + / - 与其操作数之间不留空格 */
    function unaryOperandGap(prev) {
      if (!prev) return false;
      return !!prev.__unary;
    }

    function wantSpaceBeforeWord(prev, tok) {
      if (!prev) return false;
      if (prev.__genOpen) return false;
      if (prev.__tightName) return false;
      if (prev.t === 'punct') {
        var pv = prev.v;
        if (pv === '(' || pv === '[' || pv === '.' || pv === '?.' || pv === '...' ||
          pv === '!' || pv === '~' || pv === '++' || pv === '--' || pv === '@' || pv === '#') return false;
        if (tok && tok.__genClose) return false;
        return true;
      }
      return true;
    }

    /* ---- 无分号语句的 ASI 换行（保留源码中的语句切分） ---- */

    function endsStmt(p) {
      if (!p) return false;
      if (p.t === 'number' || p.t === 'string' || p.t === 'template' || p.t === 'regex') return true;
      if (p.t === 'name') return !KEYWORDS[p.v] || !!VALUE_KW[p.v];
      if (p.t === 'punct') {
        var pv = p.v;
        return pv === ')' || pv === ']' || pv === '++' || pv === '--';
      }
      return false;
    }

    function startsStmt(t) {
      if (!t) return false;
      if (t.t === 'name') return !CONTINUATION[t.v];
      return t.t === 'number' || t.t === 'string' || t.t === 'template' || t.t === 'regex';
    }

    function asiBreak(p, t) {
      return !!(p && t && endsStmt(p) && startsStmt(t));
    }

    function canInlineArray(idx, curLen) {
      var depth = 0;
      var len = 2;
      for (var j = idx; j < tokens.length; j++) {
        var t = tokens[j];
        if (t.t === 'lineComment' || t.t === 'blockComment') return false;
        if (j > idx) {
          if (t.nl > 0) return false;
          if (t.t === 'punct' && (t.v === '{' || t.v === '}')) return false;
          if (t.t === 'punct' && t.v === '=>') return false;
          if (t.t === 'name' && (t.v === 'function' || t.v === 'class')) return false;
        }
        var v = t.v || '';
        if (t.t === 'punct') {
          if (v === '{') return false;
          if (v === '[' || v === '(') depth++;
          else if (v === ']' || v === ')') {
            depth--;
            if (depth === 0) return (len + curLen) <= 100;
          }
        }
        len += v.length + 1;
        if (len + curLen > 110) return false;
      }
      return false;
    }

    /* 多行块注释：整体逐字保留（内部各行不再追加缩进，保证内容与幂等性不变） */
    function emitBlockComment(text) {
      var lines = text.split(/\r\n|[\r\n\u2028\u2029]/);
      if (lines.length === 1) {
        write(text);
        pendingSpace = true;
        return;
      }
      if (cur.replace(/[ \t]+$/, '') !== '') newline();
      writeRaw(lines[0]);
      for (var k = 1; k < lines.length; k++) {
        newline();
        cur += lines[k];
      }
      pendingSpace = true;
    }

    /* ---- 主循环 ---- */

    for (i = 0; i < tokens.length; i++) {
      var tok = tokens[i];
      var prev = prevs[i];
      var next = nexts[i];

      /* 保留空行：连续换行压缩为最多一个空行 */
      if (tok.nl >= 2) {
        newline();
        if (prev && prev.v !== '{' && prev.v !== '(' && prev.v !== '[' && prev.v !== ',') blank();
      }

      /* 行注释：保留在该行末尾 */
      if (tok.t === 'lineComment') {
        if (cur.replace(/[ \t]+$/, '') !== '') {
          space();
          writeRaw(tok.v);
        } else {
          writeRaw(tok.v);
        }
        newline();
        continue;
      }

      /* 块注释 */
      if (tok.t === 'blockComment') {
        emitBlockComment(tok.v);
        continue;
      }

      /* 字符串 / 模板 / 数字 / 正则 */
      if (tok.t === 'string' || tok.t === 'template' || tok.t === 'number' || tok.t === 'regex') {
        if (unaryOperandGap(prev, tok) || (prev && prev.t === 'punct' && (prev.v === '!' || prev.v === '~'))) {
          pendingSpace = false;
        } else if (wantSpaceBeforeWord(prev)) space();
        write(tokenText(tok));
        continue;
      }

      /* 标识符 / 关键字 */
      if (tok.t === 'name') {
        var nv = tok.v;
        var tk = top();

        if ((nv === 'case' || nv === 'default') && tk && tk.isSwitch) {
          if (tk.caseExtra) { level--; tk.caseExtra = false; }
          if (cur.replace(/[ \t]+$/, '') !== '') newline();
          write(nv);
          pendingSpace = true;
          caseLabelPending = true;
          continue;
        }

        if (nv === 'class' || nv === 'interface' || nv === 'enum' || nv === 'namespace' || nv === 'module' || nv === 'declare') {
          pendingDecl = true;
        }
        if (nv === 'switch') pendingSwitch = true;

        if (asiBreak(prev, tok)) {
          var cTop = top();
          var inParen = cTop && (cTop.type === 'paren' || cTop.type === 'generic' ||
            cTop.type === 'index' || (cTop.type === 'array' && cTop.inline));
          if (!inParen) newline();
        }
        /* 装饰器之后：下一个标记（声明/成员名）另起一行 */
        if (decActive && decParen === 0 && !(prev && prev.t === 'punct' && (prev.v === '.' || prev.v === '?.'))) {
          if (decFirst) {
            decFirst = false;
            pendingSpace = false;
            writeRaw(nv);
            continue;
          }
          decActive = false;
          newline();
        }
        if (wantSpaceBeforeWord(prev) && !unaryOperandGap(prev, tok) &&
          !(prev && prev.t === 'punct' && (prev.v === '!' || prev.v === '~'))) space();
        write(nv);
        if (KEYWORDS[nv] && !NO_SPACE_AFTER[nv]) pendingSpace = true;
        continue;
      }

      /* ---------------- 标点 / 运算符 ---------------- */
      var v = tok.v;
      var t2 = top();

      /* { */
      if (v === '{') {
        var isBlock = false;
        var isClass = false;
        var isImportList = !!(prev && prev.t === 'name' && (prev.v === 'import' || prev.v === 'export'));
        if (pendingDecl) { isBlock = true; isClass = true; pendingDecl = false; }
        else if (isImportList) { isBlock = false; }
        else isBlock = isBlockBrace(prev, pprevs[i], t2);
        var markSwitch = pendingSwitch;
        pendingSwitch = false;

        if (isImportList) {
          /* import/export 说明符列表：单行，逗号后加空格 */
          space();
          writeRaw('{');
          if (next && next.v === '}') {
            writeRaw('}');
            pendingSpace = true;
            continue;
          }
          stack.push({ type: 'importBrace' });
          pendingSpace = true;
          continue;
        }

        if (next && next.v === '}') {
          if (isBlock && cur !== '') space();
          write('{');
          pendingSpace = false;
          stack.push({ type: 'empty' });
          continue;
        }
        if (isBlock) {
          if (cur !== '') space();
          write('{');
          newline();
          level++;
          stack.push({ type: 'block', isSwitch: markSwitch, caseExtra: false, classBody: isClass });
        } else {
          write('{');
          newline();
          level++;
          stack.push({ type: 'object' });
        }
        pendingSpace = false;
        continue;
      }

      /* } */
      if (v === '}') {
        var tClose = stack.pop();
        var parentFrame = top();
        pendingSpace = false;
        caseLabelPending = false;
        if (tClose && tClose.type === 'empty') {
          write('}');
        } else if (tClose && tClose.type === 'importBrace') {
          if (!/[{ ]$/.test(cur)) cur += ' ';
          writeRaw('}');
          pendingSpace = true;
          continue;
        } else {
          if (!tClose) {
            return { text: '', error: '括号不配对：出现了多余的 "}"' };
          }
          level--;
          if (tClose.caseExtra) level--;
          if (tClose.type !== 'array' || !tClose.inline) newline();
          write('}');
        }
        var needBreak = true;
        if (next) {
          var nvv = next.v;
          if (nvv === 'else' || nvv === 'catch' || nvv === 'finally' || nvv === 'while') needBreak = false;
          else if (next.t === 'punct') {
            if (nvv === ';' || nvv === ',' || nvv === ')' || nvv === ']' || nvv === '}' ||
              nvv === '.' || nvv === '?.' || nvv === ':' || nvv === '=' || nvv === '=>' || nvv === '?') needBreak = false;
            else needBreak = true;
          } else if (parentFrame && parentFrame.classBody) {
            needBreak = true;
          } else if (next.t === 'name' && next.nl === 0) {
            needBreak = false;
          } else {
            needBreak = true;
          }
        }
        /* 源码中紧跟在 "}" 之后另起一行的注释，仍保持换行 */
        if (tokens[i + 1] && (tokens[i + 1].t === 'blockComment' || tokens[i + 1].t === 'lineComment') &&
          tokens[i + 1].nl > 0) {
          needBreak = true;
        }
        if (needBreak) newline();
        continue;
      }

      /* ( */
      if (v === '(') {
        write('(');
        stack.push({ type: 'paren' });
        continue;
      }
      /* ) */
      if (v === ')') {
        if (!top() || (top().type !== 'paren' && top().type !== 'index' && top().type !== 'array' &&
          top().type !== 'empty' && top().type !== 'generic')) {
          return { text: '', error: '括号不配对：出现了多余的 ")"' };
        }
        stack.pop();
        pendingSpace = false;
        writeRaw(')');
        continue;
      }

      /* [ */
      if (v === '[') {
        if (isIndexAccessP(prev) && !(prev && prev.t === 'name' && PAREN_SPACE[prev.v])) {
          pendingSpace = false;
          writeRaw('[');
          stack.push({ type: 'index' });
          continue;
        }
        write('[');
        if (next && next.v === ']') {
          stack.push({ type: 'empty' });
          pendingSpace = false;
          continue;
        }
        if (canInlineArray(i, cur.length)) {
          stack.push({ type: 'array', inline: true });
          pendingSpace = false;
        } else {
          newline();
          level++;
          stack.push({ type: 'array', inline: false });
          pendingSpace = false;
        }
        continue;
      }
      /* ] */
      if (v === ']') {
        var tBracket = stack.pop();
        pendingSpace = false;
        if (tBracket && tBracket.type === 'empty') {
          write(']');
          continue;
        }
        if (!tBracket) {
          return { text: '', error: '括号不配对：出现了多余的 "]"' };
        }
        if (tBracket.type === 'array' && !tBracket.inline) {
          level--;
          newline();
        }
        write(']');
        continue;
      }

      /* ; */
      if (v === ';') {
        pendingSpace = false;
        writeRaw(';');
        var tSemi = top();
        if (tSemi && (tSemi.type === 'paren' || tSemi.type === 'array' || tSemi.type === 'index' || tSemi.type === 'generic')) {
          if (!(next && (next.v === ';' || next.v === ')'))) pendingSpace = true;
        } else {
          pendingSwitch = false;
          pendingDecl = false;
          newline();
        }
        continue;
      }

      /* , */
      if (v === ',') {
        pendingSpace = false;
        writeRaw(',');
        var tComma = top();
        if (tComma && tComma.type === 'object') newline();
        else if (tComma && tComma.type === 'array' && !tComma.inline) newline();
        else if (next && (next.v === ')' || next.v === ']' || next.v === '}' || next.v === ';')) { /* 不加空格 */ }
        else pendingSpace = true;
        continue;
      }

      /* : */
      if (v === ':') {
        var tColon = top();
        if (caseLabelPending && tColon && tColon.isSwitch) {
          pendingSpace = false;
          writeRaw(':');
          newline();
          level++;
          tColon.caseExtra = true;
          caseLabelPending = false;
          continue;
        }
        var hasTern = (tColon && tColon.tern > 0) || (!tColon && ternCount > 0);
        if (hasTern) {
          if (tColon && tColon.tern > 0) tColon.tern--;
          else ternCount--;
          space();
          writeRaw(':');
          pendingSpace = true;
          continue;
        }
        pendingSpace = false;
        writeRaw(':');
        pendingSpace = true;
        continue;
      }

      /* ? */
      if (v === '?') {
        if (next && next.v === ':') {
          pendingSpace = false;
          writeRaw('?');
          continue;
        }
        var tQ = top();
        if (tQ) tQ.tern = (tQ.tern || 0) + 1;
        else ternCount++;
        space();
        writeRaw('?');
        pendingSpace = true;
        continue;
      }

      /* . ?. */
      if (v === '.' || v === '?.') {
        pendingSpace = false;
        writeRaw(v);
        continue;
      }

      /* ... */
      if (v === '...') {
        write('...');
        pendingSpace = false;
        continue;
      }

      /* 一元前缀 */
      if (v === '!' || v === '~') {
        write(v);
        pendingSpace = false;
        continue;
      }

      /* ++ -- */
      if (v === '++' || v === '--') {
        write(v);
        pendingSpace = false;
        continue;
      }

      /* @ # */
      if (v === '@') {
        if (decActive) { decActive = false; newline(); }
        pendingSpace = false;
        writeRaw('@');
        decActive = true;
        decFirst = true;
        decParen = 0;
        continue;
      }
      if (v === '#') {
        if (prev && (prev.t === 'name' || (prev.t === 'punct' && (prev.v === '}' || prev.v === ')' || prev.v === ']')))) space();
        else pendingSpace = false;
        writeRaw('#');
        tok.__tightName = true;
        continue;
      }

      /* 星号：生成器标记（function / async / 成员起点）或 yield 之后 */
      if (v === '*') {
        var starPrev = prev;
        if (starPrev && starPrev.t === 'name' && starPrev.v === 'async') {
          space();
          writeRaw('*');
          pendingSpace = false;
          tok.__tightName = true;
          continue;
        }
        if (starPrev && starPrev.t === 'name' && (starPrev.v === 'function' || starPrev.v === 'yield')) {
          pendingSpace = false;
          writeRaw('*');
          pendingSpace = true;
          continue;
        }
        if (starPrev && starPrev.t === 'punct' && (starPrev.v === '{' || starPrev.v === ',' || starPrev.v === ';')) {
          pendingSpace = false;
          writeRaw('*');
          pendingSpace = false;
          tok.__tightName = true;
          continue;
        }
      }

      /* 泛型尖括号 */
      if (v === '<' && tok.__genOpen) {
        pendingSpace = false;
        writeRaw('<');
        stack.push({ type: 'generic' });
        continue;
      }
      if (v === '>' && tok.__genClose) {
        if (top() && top().type === 'generic') stack.pop();
        pendingSpace = false;
        writeRaw('>');
        continue;
      }
      if ((v === '>=' ) && tok.__genClose) {
        if (top() && top().type === 'generic') stack.pop();
        pendingSpace = false;
        writeRaw('>=');
        continue;
      }
      if ((v === '>>' || v === '>>>') && tok.__genClose) {
        var times = v === '>>' ? 2 : 3;
        while (times-- > 0 && top() && top().type === 'generic') stack.pop();
        pendingSpace = false;
        writeRaw(v);
        continue;
      }

      /* + - 的一元用法：与操作数紧贴，左侧按上下文决定是否留空格 */
      if ((v === '+' || v === '-') && isUnaryPosition(prev)) {
        if (wantSpaceBeforeWord(prev)) space();
        writeRaw(v);
        pendingSpace = false;
        tok.__unary = true;
        continue;
      }

      /* 其余按二元 / 赋值 / 逻辑运算符处理 */
      space();
      writeRaw(v);
      pendingSpace = true;
      continue;
    }

    newline();

    while (out.length && out[out.length - 1] === '') out.pop();

    if (stack.length) {
      var bad = stack[0];
      var what = bad.type === 'paren' ? '(' : (bad.type === 'index' || bad.type === 'array' || bad.type === 'empty' ? '[' : '{');
      var closing = bad.type === 'paren' ? ')' : (bad.type === 'index' || bad.type === 'array' || bad.type === 'empty' ? ']' : '}');
      return { text: '', error: '括号不配对：存在未闭合的 "' + what + '"（缺少配对的 "' + closing + '"）' };
    }

    return { text: out.join(opts.newline) };
  }

  /* ------------------------------------------------------------------ *
   * 对外接口
   * ------------------------------------------------------------------ */

  function format(text, opts) {
    try {
      opts = opts || {};
      var indentStr = typeof opts.indent === 'string' ? opts.indent : '  ';
      var newlineStr = (typeof opts.newline === 'string' && opts.newline !== '') ? opts.newline : '\n';
      var singleQuote = !!opts.singleQuote;

      if (typeof text !== 'string') {
        if (text === null || text === undefined) text = '';
        else text = String(text);
      }

      if (text.replace(/\s+/g, '') === '') return { ok: true, text: '' };

      var lexed = tokenize(text);
      if (lexed.error) return { ok: false, error: lexed.error };

      var rendered = render(lexed.tokens, {
        indent: indentStr,
        newline: newlineStr,
        singleQuote: singleQuote
      });
      if (rendered.error) return { ok: false, error: rendered.error };

      return { ok: true, text: rendered.text };
    } catch (e) {
      return {
        ok: false,
        error: '格式化失败：' + (e && e.message ? e.message : String(e))
      };
    }
  }

  var api = { format: format };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.TBFormat = root.TBFormat || {};
  root.TBFormat.js = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
