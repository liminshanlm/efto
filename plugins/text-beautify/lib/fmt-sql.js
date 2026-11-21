/*
 * text-beautify / fmt-sql
 * 零依赖 SQL 美化（pretty print）模块。
 * 对外仅暴露 format(text, opts)，任何情况下都不抛异常。
 */
(function (root) {
  'use strict';

  /* ======================================================================
   * 基础工具
   * ==================================================================== */

  function makeSet(list) {
    var s = Object.create(null);
    for (var i = 0; i < list.length; i++) s[list[i]] = true;
    return s;
  }

  function rep(str, times) {
    var out = '';
    for (var i = 0; i < times; i++) out += str;
    return out;
  }

  function isDigit(c) {
    return c >= '0' && c <= '9';
  }

  function isSpaceChar(c) {
    return c === ' ' || c === '\t' || c === '\n' || c === '\r' ||
      c === '\f' || c === '\v' || c === '\u00a0' || c === '\ufeff';
  }

  var RE_ID_START = /[\p{L}\p{M}_$#]/u;
  var RE_ID_PART = /[\p{L}\p{M}\p{N}_$#\u00b7]/u;

  /* 这些关键字后面紧跟 "(" 时要留一个空格（如 IN ( / VALUES ( / EXISTS (） */
  var BEFORE_PAREN = makeSet([
    'IN', 'VALUES', 'EXISTS', 'ANY', 'ALL', 'SOME', 'FROM', 'WHERE', 'HAVING',
    'AND', 'OR', 'NOT', 'ON', 'WHEN', 'THEN', 'ELSE', 'SELECT', 'RETURNING',
    'SET', 'BY', 'AS', 'USING', 'INTO', 'DISTINCT', 'BETWEEN', 'LIKE', 'IS',
    'JOIN', 'UNION', 'INTERSECT', 'EXCEPT', 'OFFSET', 'LIMIT', 'CASE',
    'INSERT', 'UPDATE', 'DELETE', 'GROUP', 'ORDER', 'WITH', 'OVER', 'PARTITION'
  ]);

  /* 允许紧贴字符串的前缀（T-SQL 的 N'...'、MySQL 的 X'..' / B'..'） */
  var STR_PREFIX = makeSet(['N', 'E', 'X', 'B']);

  /* 参与格式化的 SQL 关键字（upperKeywords 为 true 时统一转大写） */  var KEYWORDS = makeSet([
    'SELECT', 'FROM', 'WHERE', 'GROUP', 'BY', 'HAVING', 'ORDER', 'LIMIT',
    'OFFSET', 'FETCH', 'NEXT', 'ONLY', 'UNION', 'ALL', 'DISTINCT',
    'INTERSECT', 'EXCEPT', 'MINUS', 'INSERT', 'INTO', 'VALUES', 'UPDATE',
    'SET', 'DELETE', 'WITH', 'RECURSIVE', 'RETURNING', 'ON', 'USING',
    'CONFLICT', 'DO', 'NOTHING', 'JOIN', 'INNER', 'LEFT', 'RIGHT', 'FULL',
    'OUTER', 'CROSS', 'NATURAL', 'LATERAL', 'AND', 'OR', 'NOT', 'IS', 'NULL',
    'IN', 'LIKE', 'ILIKE', 'BETWEEN', 'EXISTS', 'SOME', 'ESCAPE', 'COLLATE',
    'CASE', 'WHEN', 'THEN', 'ELSE', 'END', 'AS', 'ASC', 'DESC', 'NULLS',
    'FIRST', 'LAST', 'TRUE', 'FALSE', 'UNKNOWN', 'OVER', 'PARTITION',
    'WINDOW', 'FILTER', 'PRECEDING', 'FOLLOWING', 'CREATE', 'TABLE', 'VIEW',
    'INDEX', 'DROP', 'ALTER', 'TRUNCATE', 'PRIMARY', 'FOREIGN', 'KEY',
    'REFERENCES', 'DEFAULT', 'CONSTRAINT', 'UNIQUE', 'CHECK', 'CASCADE',
    'RESTRICT', 'TEMP', 'TEMPORARY', 'BEGIN', 'COMMIT', 'ROLLBACK',
    'TRANSACTION', 'SAVEPOINT', 'START', 'GRANT', 'REVOKE', 'EXPLAIN',
    'ANALYZE', 'PRAGMA', 'MERGE', 'MATCHED', 'DECLARE', 'EXEC', 'EXECUTE',
    'PROCEDURE', 'FUNCTION', 'TRIGGER', 'RETURNS', 'LANGUAGE', 'CAST',
    'INTERVAL', 'TOP', 'PERCENT', 'TIES', 'ROLLUP', 'CUBE', 'GROUPING',
    'SETS', 'CURRENT_DATE', 'CURRENT_TIME', 'CURRENT_TIMESTAMP', 'IF'
  ]);

  /* 多字符运算符，按长度倒序匹配 */
  var MULTI_OPS = [
    '<=>', '->>', '#>>', '!<', '!>', '<>', '!=', '>=', '<=', '||', '::',
    '->', '#>', ':=', '<<', '>>', '=>'
  ].sort(function (a, b) { return b.length - a.length; });

  /* 相邻两个字符不加空格就会被词法器认成新 token 的组合（首2字符切分），
     排版时据此强制补一个空格，保证 token 序列与原文一致 */
  var MERGE_PAIRS = (function () {
    var s = Object.create(null);
    var bad = ['<=', '->', '#>', '!<', '!>', '<>', '!=', '>=', '||', '::',
      ':=', '<<', '>>', '=>', '--', '/*'];
    for (var i = 0; i < bad.length; i++) s[bad[i]] = true;
    return s;
  })();

  /* ======================================================================
   * 词法分析
   * ==================================================================== */

  function lexError(message, line, col) {
    var e = new Error(message);
    e.__sqlLex = true;
    e.line = line;
    e.col = col;
    return e;
  }

  function tokenize(src) {
    var toks = [];
    var i = 0;
    var len = src.length;
    var line = 1;
    var col = 1;
    var last = null;

    function adv(k) {
      for (var s = 0; s < k; s++) {
        var ch = src.charAt(i);
        i++;
        if (ch === '\n') {
          line++;
          col = 1;
        } else {
          col++;
        }
      }
    }

    function push(type, value, startLine, startCol) {
      var t = { type: type, value: value, line: startLine, col: startCol };
      if (type === 'ident') {
        t.quoted = false;
        t.up = value.toUpperCase();
      } else if (type === 'qident') {
        t.quoted = true;
        t.up = value.toUpperCase();
      }
      toks.push(t);
      last = t;
      return t;
    }

    function atomBefore() {
      if (!last) return false;
      if (last.type === 'ident' || last.type === 'num' || last.type === 'qident') return true;
      if (last.type === 'op' && last.value === ')') return true;
      if (last.type === 'op' && last.value === ']') return true;
      return false;
    }

    while (i < len) {
      var c = src.charAt(i);

      if (isSpaceChar(c)) {
        adv(1);
        continue;
      }

      var startLine = line;
      var startCol = col;

      /* 行注释 -- ... */
      if (c === '-' && src.charAt(i + 1) === '-') {
        var jc = i;
        while (jc < len && src.charAt(jc) !== '\n' && src.charAt(jc) !== '\r') jc++;
        var lcText = src.slice(i, jc);
        push('line-comment', lcText, startLine, startCol);
        adv(jc - i);
        continue;
      }

      /* 块注释 /* ... *\/ （支持嵌套） */
      if (c === '/' && src.charAt(i + 1) === '*') {
        var jb = i;
        var depth = 0;
        var closedB = false;
        while (jb < len) {
          if (src.charAt(jb) === '/' && src.charAt(jb + 1) === '*') {
            depth++;
            jb += 2;
            continue;
          }
          if (src.charAt(jb) === '*' && src.charAt(jb + 1) === '/') {
            depth--;
            jb += 2;
            if (depth === 0) {
              closedB = true;
              break;
            }
            continue;
          }
          jb++;
        }
        if (!closedB) {
          throw lexError('第 ' + startLine + ' 行第 ' + startCol + ' 列：块注释未闭合（缺少配对的 "*/"）', startLine, startCol);
        }
        push('block-comment', src.slice(i, jb), startLine, startCol);
        adv(jb - i);
        continue;
      }

      /* 单引号字符串，转义用 '' */
      if (c === "'") {
        var js = i + 1;
        var closedS = false;
        while (js < len) {
          if (src.charAt(js) === "'") {
            if (src.charAt(js + 1) === "'") {
              js += 2;
              continue;
            }
            js++;
            closedS = true;
            break;
          }
          js++;
        }
        if (!closedS) {
          throw lexError('第 ' + startLine + ' 行第 ' + startCol + ' 列：单引号字符串未闭合（缺少结束的 "\'"）', startLine, startCol);
        }
        push('str', src.slice(i, js), startLine, startCol);
        adv(js - i);
        continue;
      }

      /* 双引号标识符，转义用 "" */
      if (c === '"') {
        var jd = i + 1;
        var closedD = false;
        while (jd < len) {
          if (src.charAt(jd) === '"') {
            if (src.charAt(jd + 1) === '"') {
              jd += 2;
              continue;
            }
            jd++;
            closedD = true;
            break;
          }
          jd++;
        }
        if (!closedD) {
          throw lexError('第 ' + startLine + ' 行第 ' + startCol + ' 列：双引号标识符未闭合（缺少结束的 \'"\'）', startLine, startCol);
        }
        push('qident', src.slice(i, jd), startLine, startCol);
        adv(jd - i);
        continue;
      }

      /* 反引号标识符，转义用 `` */
      if (c === '`') {
        var jk = i + 1;
        var closedK = false;
        while (jk < len) {
          if (src.charAt(jk) === '`') {
            if (src.charAt(jk + 1) === '`') {
              jk += 2;
              continue;
            }
            jk++;
            closedK = true;
            break;
          }
          jk++;
        }
        if (!closedK) {
          throw lexError('第 ' + startLine + ' 行第 ' + startCol + ' 列：反引号标识符未闭合（缺少结束的 "`"）', startLine, startCol);
        }
        push('qident', src.slice(i, jk), startLine, startCol);
        adv(jk - i);
        continue;
      }

      /* SQL Server 方括号标识符 [a b]，转义用 ]] ；找不到收尾时退化为普通符号 */
      if (c === '[') {
        var jq = i + 1;
        var closedQ = false;
        while (jq < len) {
          if (src.charAt(jq) === ']') {
            if (src.charAt(jq + 1) === ']') {
              jq += 2;
              continue;
            }
            jq++;
            closedQ = true;
            break;
          }
          jq++;
        }
        if (closedQ) {
          push('qident', src.slice(i, jq), startLine, startCol);
          adv(jq - i);
          continue;
        }
        /* 不闭合则按普通符号处理，交给后续分支 */
      }

      /* 数字 */
      if (isDigit(c) || (c === '.' && isDigit(src.charAt(i + 1)) && !atomBefore())) {
        var jn = i;
        if (c === '0' && (src.charAt(i + 1) === 'x' || src.charAt(i + 1) === 'X')) {
          jn = i + 2;
          while (jn < len && /[0-9a-fA-F]/.test(src.charAt(jn))) jn++;
        } else {
          while (jn < len && isDigit(src.charAt(jn))) jn++;
          if (src.charAt(jn) === '.') {
            jn++;
            while (jn < len && isDigit(src.charAt(jn))) jn++;
          }
          if (src.charAt(jn) === 'e' || src.charAt(jn) === 'E') {
            var je = jn + 1;
            if (src.charAt(je) === '+' || src.charAt(je) === '-') je++;
            if (isDigit(src.charAt(je))) {
              while (je < len && isDigit(src.charAt(je))) je++;
              jn = je;
            }
          }
        }
        push('num', src.slice(i, jn), startLine, startCol);
        adv(jn - i);
        continue;
      }

      /* 标识符 / 关键字（支持 Unicode 词） */
      var cp = src.codePointAt(i);
      var chStr = String.fromCodePoint(cp);
      if (RE_ID_START.test(chStr)) {
        var ji = i + chStr.length;
        while (ji < len) {
          var cp2 = src.codePointAt(ji);
          var s2 = String.fromCodePoint(cp2);
          if (!RE_ID_PART.test(s2)) break;
          ji += s2.length;
        }
        push('ident', src.slice(i, ji), startLine, startCol);
        adv(ji - i);
        continue;
      }

      /* 运算符 / 标点 */
      var matchedOp = null;
      for (var mo = 0; mo < MULTI_OPS.length; mo++) {
        if (src.startsWith(MULTI_OPS[mo], i)) {
          matchedOp = MULTI_OPS[mo];
          break;
        }
      }
      if (matchedOp) {
        push('op', matchedOp, startLine, startCol);
        adv(matchedOp.length);
        continue;
      }
      push('op', c, startLine, startCol);
      adv(1);
    }

    return toks;
  }

  /* ======================================================================
   * 排版
   * ==================================================================== */

  function layoutError(message, tok) {
    var e = new Error(message);
    e.__sqlLayout = true;
    if (tok) {
      e.line = tok.line;
      e.col = tok.col;
    }
    return e;
  }

  /* 子句 / 连接 / 其它关键字序列，按优先级从长到短排列 */
  var SEQS = [
    { w: ['UNION', 'ALL'], kind: 'clause', meta: { kind: 'setop' } },
    { w: ['UNION'], kind: 'clause', meta: { kind: 'setop' } },
    { w: ['INSERT', 'INTO'], kind: 'clause', meta: { kind: 'insert' } },
    { w: ['INSERT'], kind: 'clause', meta: { kind: 'insert' } },
    { w: ['DELETE', 'FROM'], kind: 'clause', meta: { kind: 'delete' } },
    { w: ['DELETE'], kind: 'clause', meta: { kind: 'delete' } },
    { w: ['GROUP', 'BY'], kind: 'clause', meta: { kind: 'group' } },
    { w: ['ORDER', 'BY'], kind: 'clause', meta: { kind: 'order' } },
    { w: ['WITH', 'RECURSIVE'], kind: 'clause', meta: { kind: 'with' } },
    { w: ['WITH'], kind: 'clause', meta: { kind: 'with' } },
    { w: ['SELECT'], kind: 'clause', meta: { kind: 'select' } },
    { w: ['FROM'], kind: 'clause', meta: { kind: 'from' } },
    { w: ['WHERE'], kind: 'clause', meta: { kind: 'pred' } },
    { w: ['HAVING'], kind: 'clause', meta: { kind: 'pred' } },
    { w: ['LIMIT'], kind: 'clause', meta: { kind: 'limit' } },
    { w: ['OFFSET'], kind: 'clause', meta: { kind: 'limit' } },
    { w: ['INTERSECT'], kind: 'clause', meta: { kind: 'setop' } },
    { w: ['EXCEPT'], kind: 'clause', meta: { kind: 'setop' } },
    { w: ['VALUES'], kind: 'clause', meta: { kind: 'values' } },
    { w: ['SET'], kind: 'clause', meta: { kind: 'set' } },
    { w: ['RETURNING'], kind: 'clause', meta: { kind: 'select' } },
    { w: ['UPDATE'], kind: 'clause', meta: { kind: 'update' } },
    { w: ['LEFT', 'OUTER', 'JOIN'], kind: 'join' },
    { w: ['RIGHT', 'OUTER', 'JOIN'], kind: 'join' },
    { w: ['FULL', 'OUTER', 'JOIN'], kind: 'join' },
    { w: ['LEFT', 'JOIN'], kind: 'join' },
    { w: ['RIGHT', 'JOIN'], kind: 'join' },
    { w: ['FULL', 'JOIN'], kind: 'join' },
    { w: ['INNER', 'JOIN'], kind: 'join' },
    { w: ['CROSS', 'JOIN'], kind: 'join' },
    { w: ['NATURAL', 'JOIN'], kind: 'join' },
    { w: ['JOIN'], kind: 'join' },
    { w: ['ON'], kind: 'on' },
    { w: ['CASE'], kind: 'case' },
    { w: ['WHEN'], kind: 'when' },
    { w: ['ELSE'], kind: 'else' },
    { w: ['END'], kind: 'end' },
    { w: ['BETWEEN'], kind: 'between' },
    { w: ['AND'], kind: 'andor' },
    { w: ['OR'], kind: 'andor' }
  ];

  function layout(toks, o) {
    var indentStr = o.indent;
    var NL = o.newline;
    var commaBreaks = o.commaBreaks;
    var upper = o.upperKeywords;
    var n = toks.length;

    var lines = [];
    var cur = { prefix: '', text: '' };
    var last1 = null;
    var last2 = null;
    var glueNext = false;
    var pendingBetween = null;

    var frames = [{
      clausePrefix: '',
      colPrefix: '',
      clause: null,
      clauseDepth: 0,
      predIndent: null,
      joinSeen: false
    }];
    var parenStack = [];
    var parenDepth = 0;
    var caseStack = [];

    function frame() {
      return frames[frames.length - 1];
    }

    function flush() {
      if (cur.text) {
        lines.push({ prefix: cur.prefix, text: cur.text });
      }
      cur = { prefix: cur.prefix, text: '' };
    }

    function newline(prefix) {
      flush();
      cur.prefix = prefix || '';
    }

    function spacingFor(tok) {
      if (!last1 || !cur.text) return false;
      if (glueNext) return false;
      var pv = last1.value;
      var pt = last1.type;

      if (tok.type === 'op') {
        if (tok.value === ',' || tok.value === ';' || tok.value === ')' ||
          tok.value === '.' || tok.value === '::') {
          return false;
        }
        if (tok.value === '(') {
          if (pt === 'op') return !(pv === '(' || pv === '.' || pv === '::');
          if (pt === 'ident' && !last1.quoted) return !!BEFORE_PAREN[last1.up];
          if (pt === 'qident' || pt === 'num' || pt === 'str') return false;
          return true;
        }
        if (pt === 'op' && (pv === '(' || pv === '.' || pv === '::')) return false;
        return true;
      }

      if (pt === 'op') {
        if (pv === '(' || pv === '.' || pv === '::') return false;
        return true;
      }
      if (tok.type === 'str' && pt === 'ident' && !last1.quoted && STR_PREFIX[last1.up]) return false;
      return true;
    }

    /* 判断不加空格会不会把两个 token 粘连成另一个 token（如 "-" + "-" 变成注释符、"+" + ">" 变成运算符） */
    var RE_WORD = /[\p{L}\p{M}\p{N}_$#]/u;
    function wouldMerge(head, next) {
      if (!head || !next) return false;
      var p = head.charAt(head.length - 1);
      var c = next.charAt(0);
      if (MERGE_PAIRS[p + c]) return true;
      if ((p === "'" || p === '"' || p === '`') && c === p) return true;
      if (p === '.' && c >= '0' && c <= '9') return true;
      if (p >= '0' && p <= '9' && c === '.') return true;
      if (RE_WORD.test(p) && RE_WORD.test(c)) return true;
      return false;
    }

    function appendToken(text, tok) {
      if (text === null || text === undefined || text === '') return;
      var sp = tok ? spacingFor(tok) : false;
      if (!sp && cur.text && wouldMerge(cur.text, text)) sp = true;
      cur.text += (sp ? ' ' : '') + text;
      glueNext = false;
      if (tok) {
        last2 = last1;
        last1 = tok;
      }
    }

    function appendIdent(tok) {
      var text = (upper && KEYWORDS[tok.up]) ? tok.up : tok.value;
      appendToken(text, tok);
    }

    function emitOp(tok) {
      appendToken(tok.value, tok);
      var v = tok.value;
      if (v === '@' || v === ':') {
        glueNext = true;
        return;
      }
      if (v === '-' || v === '+') {
        var unary = !last2 ||
          (last2.type === 'op' && last2.value !== ')') ||
          (last2.type === 'ident' && !last2.quoted && !!KEYWORDS[last2.up]);
        if (unary) glueNext = true;
      }
    }

    function matchSeq(idx, words) {
      for (var k = 0; k < words.length; k++) {
        var tk = toks[idx + k];
        if (!tk || tk.type !== 'ident' || tk.quoted) return false;
        if (tk.up !== words[k]) return false;
      }
      return true;
    }

    function emitWords(idx, words) {
      var text = '';
      for (var k = 0; k < words.length; k++) {
        var tk = toks[idx + k];
        var w = (upper && tk.type === 'ident' && !tk.quoted) ? tk.up : tk.value;
        appendToken(w, tk);
        text += (k ? ' ' : '') + w;
      }
      return text;
    }

    function emitClause(idx, words, meta) {
      var f = frame();
      if (cur.text) {
        newline(f.clausePrefix);
      } else {
        cur.prefix = f.clausePrefix;
      }
      var text = emitWords(idx, words);
      f.clause = meta.kind;
      f.clauseDepth = parenDepth;
      f.predIndent = meta.kind === 'pred' ? (cur.prefix + indentStr) : null;
      f.colPrefix = cur.prefix + rep(' ', text.length + 1);
      f.joinSeen = false;
    }

    function emitJoin(idx, words) {
      var f = frame();
      if (cur.text) {
        newline(f.clausePrefix);
      } else {
        cur.prefix = f.clausePrefix;
      }
      emitWords(idx, words);
      f.clause = 'from';
      f.clauseDepth = parenDepth;
      f.predIndent = null;
      f.joinSeen = true;
    }

    function emitOn(idx) {
      var f = frame();
      var onPrefix = f.clausePrefix + indentStr;
      newline(onPrefix);
      emitWords(idx, ['ON']);
      f.predIndent = onPrefix + indentStr;
      f.clauseDepth = parenDepth;
    }

    function emitAndOr(idx, up) {
      if (up === 'AND' && pendingBetween !== null && pendingBetween === parenDepth) {
        pendingBetween = null;
        appendIdent(toks[idx]);
        return;
      }
      var f = frame();
      if (f.predIndent && f.clauseDepth === parenDepth) {
        newline(f.predIndent);
        emitWords(idx, [up]);
        return;
      }
      appendIdent(toks[idx]);
    }

    function emitCase(idx) {
      var t = toks[idx];
      caseStack.push({ prefix: cur.prefix, tok: t });
      appendIdent(t);
    }

    function emitWhenElse(idx) {
      var t = toks[idx];
      if (caseStack.length) {
        var c = caseStack[caseStack.length - 1];
        newline(c.prefix + indentStr);
        appendIdent(t);
        return;
      }
      appendIdent(t);
    }

    function emitEnd(idx) {
      var t = toks[idx];
      if (caseStack.length) {
        var c = caseStack.pop();
        newline(c.prefix);
        appendIdent(t);
        return;
      }
      appendIdent(t);
    }

    function keywordStep(idx) {
      var t = toks[idx];
      var seq = null;
      for (var s = 0; s < SEQS.length; s++) {
        var cand = SEQS[s];
        if (!matchSeq(idx, cand.w)) continue;
        if (cand.kind === 'on') {
          var f0 = frame();
          if (!(f0.joinSeen && f0.clause === 'from')) continue;
        }
        seq = cand;
        break;
      }
      if (!seq) return 0;

      switch (seq.kind) {
        case 'clause':
          emitClause(idx, seq.w, seq.meta);
          return seq.w.length;
        case 'join':
          emitJoin(idx, seq.w);
          return seq.w.length;
        case 'on':
          emitOn(idx);
          return 1;
        case 'andor':
          emitAndOr(idx, t.up);
          return 1;
        case 'between':
          pendingBetween = parenDepth;
          appendIdent(t);
          return 1;
        case 'case':
          emitCase(idx);
          return 1;
        case 'when':
        case 'else':
          emitWhenElse(idx);
          return 1;
        case 'end':
          emitEnd(idx);
          return 1;
        default:
          return 0;
      }
    }

    function nextIsSubqueryStart(idx) {
      var k = idx + 1;
      while (k < n && (toks[k].type === 'line-comment' || toks[k].type === 'block-comment')) k++;
      var t2 = toks[k];
      if (!t2 || t2.type !== 'ident' || t2.quoted) return false;
      return t2.up === 'SELECT' || t2.up === 'WITH';
    }

    function openParen(idx) {
      var t = toks[idx];
      var f = frame();

      if (nextIsSubqueryStart(idx)) {
        appendToken('(', t);
        var preSub = cur.prefix;
        parenStack.push({ kind: 'sub', prefix: preSub, depth: parenDepth + 1, tok: t });
        parenDepth++;
        frames.push({
          clausePrefix: preSub + indentStr,
          colPrefix: preSub + indentStr,
          clause: null,
          clauseDepth: parenDepth,
          predIndent: null,
          joinSeen: false
        });
        newline(preSub + indentStr);
        return;
      }

      if (commaBreaks && f.clause === 'insert' && f.clauseDepth === parenDepth) {
        /* 展开成多行的列清单不是函数调用，括号前留一个空格：INSERT INTO t ( */
        if (cur.text && cur.text.charAt(cur.text.length - 1) !== ' ') cur.text += ' ';
        appendToken('(', t);
        var preCol = cur.prefix;
        parenStack.push({
          kind: 'collist',
          prefix: preCol,
          depth: parenDepth + 1,
          itemPrefix: preCol + indentStr,
          tok: t
        });
        parenDepth++;
        newline(preCol + indentStr);
        return;
      }

      appendToken('(', t);
      parenStack.push({ kind: 'plain', prefix: cur.prefix, depth: parenDepth + 1, tok: t });
      parenDepth++;
    }

    function closeParen(t) {
      var e = parenStack.pop();
      if (!e) {
        throw layoutError('第 ' + t.line + ' 行第 ' + t.col + ' 列：括号不匹配，多出一个右括号 ")"', t);
      }
      parenDepth--;
      if (e.kind === 'sub') {
        frames.pop();
        newline(e.prefix);
        appendToken(')', t);
        return;
      }
      if (e.kind === 'collist') {
        newline(e.prefix);
        appendToken(')', t);
        return;
      }
      appendToken(')', t);
    }

    function commaStep(t) {
      var f = frame();
      if (commaBreaks) {
        var top = parenStack.length ? parenStack[parenStack.length - 1] : null;
        if (top && top.kind === 'collist' && top.depth === parenDepth) {
          appendToken(',', t);
          newline(top.itemPrefix);
          return;
        }
        if (f.clause && f.clauseDepth === parenDepth && f.colPrefix &&
          (f.clause === 'select' || f.clause === 'set' ||
            f.clause === 'values' || f.clause === 'returning')) {
          appendToken(',', t);
          newline(f.colPrefix);
          return;
        }
      }
      appendToken(',', t);
    }

    /* ---------------------------- 主循环 ---------------------------- */

    var i = 0;
    while (i < n) {
      var t = toks[i];

      if (t.type === 'line-comment') {
        appendToken(t.value, t);
        i++;
        newline(cur.prefix);
        continue;
      }

      if (t.type === 'block-comment') {
        appendToken(t.value, t);
        i++;
        continue;
      }

      if (t.type === 'op') {
        if (t.value === '(') {
          openParen(i);
          i++;
          continue;
        }
        if (t.value === ')') {
          closeParen(t);
          i++;
          continue;
        }
        if (t.value === ',') {
          commaStep(t);
          i++;
          continue;
        }
        if (t.value === ';') {
          appendToken(';', t);
          i++;
          if (frames.length === 1) {
            var f0 = frames[0];
            f0.clause = null;
            f0.clauseDepth = 0;
            f0.predIndent = null;
            f0.joinSeen = false;
            f0.colPrefix = '';
            pendingBetween = null;
            newline('');
          } else {
            newline(cur.prefix);
          }
          continue;
        }
        emitOp(t);
        i++;
        continue;
      }

      if (t.type === 'qident') {
        appendToken(t.value, t);
        i++;
        continue;
      }

      if (t.type === 'ident') {
        if (!t.quoted) {
          var used = keywordStep(i);
          if (used > 0) {
            i += used;
            continue;
          }
          appendIdent(t);
          i++;
          continue;
        }
        appendToken(t.value, t);
        i++;
        continue;
      }

      appendToken(t.value, t);
      i++;
    }

    flush();

    /* ---------------------------- 收尾校验 ---------------------------- */

    if (parenStack.length) {
      var unclosed = parenStack[0];
      throw layoutError(
        '第 ' + unclosed.tok.line + ' 行第 ' + unclosed.tok.col +
        ' 列：括号 "(" 未闭合，缺少配对的 ")"',
        unclosed.tok
      );
    }
    if (caseStack.length) {
      var uc = caseStack[caseStack.length - 1];
      throw layoutError(
        '第 ' + uc.tok.line + ' 行第 ' + uc.tok.col + ' 列：CASE 缺少配对的 END',
        uc.tok
      );
    }

    var outLines = [];
    for (var li = 0; li < lines.length; li++) {
      var body = lines[li].prefix + lines[li].text;
      body = body.replace(/[ \t]+$/g, '');
      outLines.push(body);
    }
    return outLines.join(NL);
  }

  /* ======================================================================
   * 对外接口
   * ==================================================================== */

  function normalizeOpts(opts) {
    opts = opts || {};
    var indent = typeof opts.indent === 'string' ? opts.indent : '  ';
    var nl = (typeof opts.newline === 'string' && opts.newline.length) ? opts.newline : '\n';
    var upper = opts.upperKeywords === undefined || opts.upperKeywords === null
      ? true
      : !!opts.upperKeywords;
    var breaks = opts.commaBreaks === undefined || opts.commaBreaks === null
      ? true
      : !!opts.commaBreaks;
    return {
      indent: indent,
      newline: nl,
      upperKeywords: upper,
      commaBreaks: breaks
    };
  }

  function format(text, opts) {
    try {
      var o = normalizeOpts(opts);

      if (typeof text !== 'string') {
        if (text === null || text === undefined) {
          return { ok: true, text: '' };
        }
        return {
          ok: false,
          error: '输入不是字符串（实际类型：' + typeof text + '）'
        };
      }

      if (!text.trim()) {
        return { ok: true, text: '' };
      }

      var toks;
      try {
        toks = tokenize(text);
      } catch (e) {
        if (e && e.__sqlLex) {
          return {
            ok: false,
            error: e.message,
            line: e.line,
            column: e.col
          };
        }
        return {
          ok: false,
          error: '词法分析失败：' + (e && e.message ? e.message : String(e))
        };
      }

      try {
        var pretty = layout(toks, o);
        return { ok: true, text: pretty };
      } catch (e2) {
        if (e2 && e2.__sqlLayout) {
          var r = { ok: false, error: e2.message };
          if (typeof e2.line === 'number') r.line = e2.line;
          if (typeof e2.col === 'number') r.column = e2.col;
          return r;
        }
        return {
          ok: false,
          error: '排版失败：' + (e2 && e2.message ? e2.message : String(e2))
        };
      }
    } catch (e3) {
      return {
        ok: false,
        error: 'SQL 美化过程中发生未预期错误：' + (e3 && e3.message ? e3.message : String(e3))
      };
    }
  }

  var api = { format: format };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.TBFormat = root.TBFormat || {};
  root.TBFormat.sql = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
