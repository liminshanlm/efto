'use strict';
/*
 * text-beautify / tests / sql-test.js
 * 运行：node E:\dsh\text-beautify\tests\sql-test.js
 * 覆盖：多行化、子查询、JOIN/ON、GROUP BY/HAVING、INSERT 多行元组、
 *       UPDATE SET、字符串保护、块注释保留、错误处理、token 序列不变自检。
 */

var mod = require('../lib/fmt-sql');
var format = mod.format;

var passed = 0;
var failed = [];

function assert(cond, msg) {
  if (cond) {
    passed++;
  } else {
    failed.push(msg);
  }
}

function show(title, src, res) {
  console.log('=== ' + title + ' ===');
  console.log('IN : ' + JSON.stringify(src));
  if (res && res.ok) {
    console.log('OUT:\n' + res.text);
  } else {
    console.log('ERR: ' + JSON.stringify(res));
  }
  console.log('');
}

function eq(actual, expected, msg) {
  assert(actual === expected, msg + '\n--- 实际 ---\n' + actual + '\n--- 期望 ---\n' + expected);
}

/* ======================================================================
 * 独立实现的 tokenizer，只用于「token 序列不变」自检，与 lib 无共享代码
 * ==================================================================== */

var MULTI = ['<=>', '->>', '#>>', '!<', '!>', '<>', '!=', '>=', '<=', '||', '::', '->', '#>', ':=', '<<', '>>', '=>'];

function tokenizeForCompare(src) {
  var out = [];
  var i = 0;
  var n = src.length;

  function isSpace(c) {
    return c === ' ' || c === '\t' || c === '\n' || c === '\r' || c === '\f' || c === '\v' || c === '\u00a0' || c === '\ufeff';
  }
  function isDigit(c) { return c >= '0' && c <= '9'; }
  function isIdStart(c) { return /[\p{L}\p{M}_$#]/u.test(c); }
  function isIdPart(c) { return /[\p{L}\p{M}\p{N}_$#\u00b7]/u.test(c); }
  /* a.5 这类「点号后跟数字」不是数字字面量，应与 lib 的切分保持一致 */
  function atomBefore() {
    var last = out.length ? out[out.length - 1] : null;
    if (!last) return false;
    return last.t === 'ident' || last.t === 'num' || last.t === 'qident' ||
      (last.t === 'op' && (last.v === ')' || last.v === ']'));
  }

  while (i < n) {
    var c = src.charAt(i);
    if (isSpace(c)) { i++; continue; }

    if (c === '-' && src.charAt(i + 1) === '-') {
      var jc = i;
      while (jc < n && src.charAt(jc) !== '\n' && src.charAt(jc) !== '\r') jc++;
      out.push({ t: 'line-comment', v: src.slice(i, jc) });
      i = jc;
      continue;
    }

    if (c === '/' && src.charAt(i + 1) === '*') {
      var jb = i, depth = 0, closed = false;
      while (jb < n) {
        if (src.charAt(jb) === '/' && src.charAt(jb + 1) === '*') { depth++; jb += 2; continue; }
        if (src.charAt(jb) === '*' && src.charAt(jb + 1) === '/') {
          depth--; jb += 2;
          if (depth === 0) { closed = true; break; }
          continue;
        }
        jb++;
      }
      if (!closed) throw new Error('test tokenizer: 块注释未闭合');
      out.push({ t: 'block-comment', v: src.slice(i, jb) });
      i = jb;
      continue;
    }

    if (c === "'" || c === '"' || c === '`') {
      var j = i + 1, done = false;
      while (j < n) {
        if (src.charAt(j) === c) {
          if (src.charAt(j + 1) === c) { j += 2; continue; }
          j++; done = true; break;
        }
        j++;
      }
      if (!done) throw new Error('test tokenizer: 引号未闭合');
      out.push({ t: c === "'" ? 'str' : 'qident', v: src.slice(i, j) });
      i = j;
      continue;
    }

    if (c === '[') {
      var jq = i + 1, closedQ = false;
      while (jq < n) {
        if (src.charAt(jq) === ']') {
          if (src.charAt(jq + 1) === ']') { jq += 2; continue; }
          jq++; closedQ = true; break;
        }
        jq++;
      }
      if (closedQ) { out.push({ t: 'qident', v: src.slice(i, jq) }); i = jq; continue; }
    }

    if (isDigit(c) || (c === '.' && isDigit(src.charAt(i + 1)) && !atomBefore())) {
      var jn = i;
      if (c === '0' && (src.charAt(i + 1) === 'x' || src.charAt(i + 1) === 'X')) {
        jn = i + 2;
        while (jn < n && /[0-9a-fA-F]/.test(src.charAt(jn))) jn++;
      } else {
        while (jn < n && isDigit(src.charAt(jn))) jn++;
        if (src.charAt(jn) === '.') {
          jn++;
          while (jn < n && isDigit(src.charAt(jn))) jn++;
        }
        if (src.charAt(jn) === 'e' || src.charAt(jn) === 'E') {
          var je = jn + 1;
          if (src.charAt(je) === '+' || src.charAt(je) === '-') je++;
          if (isDigit(src.charAt(je))) {
            while (je < n && isDigit(src.charAt(je))) je++;
            jn = je;
          }
        }
      }
      out.push({ t: 'num', v: src.slice(i, jn) });
      i = jn;
      continue;
    }

    var cp = src.codePointAt(i);
    var ch = String.fromCodePoint(cp);
    if (isIdStart(ch)) {
      var ji = i + ch.length;
      while (ji < n) {
        var cp2 = src.codePointAt(ji);
        var s2 = String.fromCodePoint(cp2);
        if (!isIdPart(s2)) break;
        ji += s2.length;
      }
      out.push({ t: 'ident', v: src.slice(i, ji).toUpperCase() });
      i = ji;
      continue;
    }

    var op = null;
    for (var m = 0; m < MULTI.length; m++) {
      if (src.startsWith(MULTI[m], i)) { op = MULTI[m]; break; }
    }
    if (op) { out.push({ t: 'op', v: op }); i += op.length; continue; }
    out.push({ t: 'op', v: c });
    i++;
  }
  return out;
}

function sig(toks) {
  return toks.map(function (t) { return t.t + ':' + t.v; });
}

function assertTokensUnchanged(label, src, out) {
  var a = sig(tokenizeForCompare(src));
  var b = sig(tokenizeForCompare(out));
  var ok = a.length === b.length;
  var diffAt = -1;
  if (ok) {
    for (var k = 0; k < a.length; k++) {
      if (a[k] !== b[k]) { ok = false; diffAt = k; break; }
    }
  } else {
    diffAt = Math.min(a.length, b.length);
  }
  if (!ok) {
    failed.push('token 序列被改变：' + label + ' @' + diffAt +
      '\n输入 token 数 ' + a.length + '，输出 token 数 ' + b.length +
      '\n输入片段 ' + JSON.stringify(a.slice(Math.max(0, diffAt - 3), diffAt + 4)) +
      '\n输出片段 ' + JSON.stringify(b.slice(Math.max(0, diffAt - 3), diffAt + 4)));
  } else {
    passed++;
  }
}

function run(label, src, opts) {
  var r = format(src, opts);
  show(label, src, r);
  return r;
}

/* ======================================================================
 * 1. 单行 SELECT 的多行化
 * ==================================================================== */

var c1 = 'SELECT a,b FROM t WHERE a=1 AND b IN (1,2) ORDER BY a DESC';
var r1 = run('1. 单行 SELECT', c1);
assert(r1.ok === true, '1. 应成功');
eq(r1.text,
  'SELECT a,\n' +
  '       b\n' +
  'FROM t\n' +
  'WHERE a = 1\n' +
  '  AND b IN (1, 2)\n' +
  'ORDER BY a DESC',
  '1. 单行 SELECT 的多行化输出不符');
assertTokensUnchanged('1', c1, r1.text);
assert(r1.text.indexOf('SELECT a,') === 0, '1. SELECT 后列清单应按逗号换行');
assert(r1.text.split('\n').length === 6, '1. 应为 6 行');

/* ======================================================================
 * 2. 子查询 + JOIN ... ON + GROUP BY / HAVING
 * ==================================================================== */

var c2 = "SELECT t.a, count(*) AS n FROM t LEFT JOIN u ON t.id = u.id AND u.x > 0 " +
  "WHERE t.a IN (SELECT id FROM z WHERE z.k = 1) GROUP BY t.a HAVING count(*) > 2 ORDER BY n DESC";
var r2 = run('2. JOIN/ON + 子查询 + GROUP BY/HAVING', c2);
assert(r2.ok === true, '2. 应成功');
eq(r2.text,
  'SELECT t.a,\n' +
  '       count(*) AS n\n' +
  'FROM t\n' +
  'LEFT JOIN u\n' +
  '  ON t.id = u.id\n' +
  '    AND u.x > 0\n' +
  'WHERE t.a IN (\n' +
  '  SELECT id\n' +
  '  FROM z\n' +
  '  WHERE z.k = 1\n' +
  ')\n' +
  'GROUP BY t.a\n' +
  'HAVING count(*) > 2\n' +
  'ORDER BY n DESC',
  '2. JOIN/子查询/GROUP BY/HAVING 输出不符');
assertTokensUnchanged('2', c2, r2.text);

var c2b = 'SELECT * FROM a INNER JOIN b ON a.id = b.id';
var r2b = run('2b. INNER JOIN 另起一行', c2b);
eq(r2b.text, 'SELECT *\nFROM a\nINNER JOIN b\n  ON a.id = b.id', '2b. INNER JOIN 输出不符');

var c2c = 'SELECT * FROM (SELECT x FROM y) t WHERE t.x > 1';
var r2c = run('2c. FROM 子查询缩进', c2c);
eq(r2c.text, 'SELECT *\nFROM (\n  SELECT x\n  FROM y\n) t\nWHERE t.x > 1', '2c. FROM 子查询输出不符');

/* ======================================================================
 * 3. INSERT INTO ... VALUES (...),(...)
 * ==================================================================== */

var c3 = "INSERT INTO t (a,b) VALUES (1,'x'),(2,'y')";
var r3 = run('3. INSERT 多行元组', c3);
assert(r3.ok === true, '3. 应成功');
eq(r3.text,
  'INSERT INTO t (\n' +
  '  a,\n' +
  '  b\n' +
  ')\n' +
  'VALUES (1, \'x\'),\n' +
  '       (2, \'y\')',
  '3. INSERT 输出不符');
assertTokensUnchanged('3', c3, r3.text);

var c3b = "INSERT INTO t (a, b) VALUES (1, 'x'), (2, 'y'), (3, 'z');";
var r3b = run('3b. INSERT 三个元组 + 分号', c3b);
assert(r3b.ok === true, '3b. 应成功');
assert(r3b.text.indexOf("VALUES (1, 'x'),") >= 0, '3b. 第一元组应在 VALUES 行');
assert(r3b.text.split('\n').length === 7, '3b. 应为 7 行（含分号收尾换行）');
assertTokensUnchanged('3b', c3b, r3b.text);

/* ======================================================================
 * 4. UPDATE ... SET a=1,b=2
 * ==================================================================== */

var c4 = 'UPDATE t SET a=1,b=2 WHERE id=3';
var r4 = run('4. UPDATE SET', c4);
assert(r4.ok === true, '4. 应成功');
eq(r4.text,
  'UPDATE t\n' +
  'SET a = 1,\n' +
  '    b = 2\n' +
  'WHERE id = 3',
  '4. UPDATE SET 输出不符');
assertTokensUnchanged('4', c4, r4.text);

/* ======================================================================
 * 5. 字符串里的 -- ; ( 必须原样保留
 * ==================================================================== */

var c5 = "SELECT 'It''s -- not a comment; (x)' AS s FROM t WHERE a = 1";
var r5 = run('5. 字符串保护', c5);
assert(r5.ok === true, "5. 含特殊字符的字符串应成功");
assert(r5.text.indexOf("'It''s -- not a comment; (x)'") >= 0, '5. 字符串内容被破坏');
eq(r5.text,
  "SELECT 'It''s -- not a comment; (x)' AS s\n" +
  "FROM t\n" +
  "WHERE a = 1",
  '5. 字符串场景输出不符');
assertTokensUnchanged('5', c5, r5.text);

var c5b = 'SELECT a FROM t WHERE s = \'a;b(c)--d\' AND z = 2';
var r5b = run('5b. 字符串内括号不参与配对', c5b);
assert(r5b.ok === true, '5b. 应成功');
assertTokensUnchanged('5b', c5b, r5b.text);
assert(r5b.text.indexOf("'a;b(c)--d'") >= 0, '5b. 字符串内容被破坏');

/* ======================================================================
 * 6. 块注释跨行保留 / 行注释
 * ==================================================================== */

var c6 = 'SELECT /* line1\n   line2 -- not a comment\n   line3 */ x, y\nFROM t';
var r6 = run('6. 块注释跨行', c6);
assert(r6.ok === true, '6. 应成功');
assert(r6.text.indexOf('/* line1\n   line2 -- not a comment\n   line3 */') >= 0,
  '6. 块注释内部内容未原样保留');
assertTokensUnchanged('6', c6, r6.text);

var c6b = 'SELECT a -- 注释里的 ; (x) 不应生效\nFROM t';
var r6b = run('6b. 行注释到行尾', c6b);
assert(r6b.ok === true, '6b. 应成功');
assert(r6b.text.indexOf('-- 注释里的 ; (x) 不应生效') >= 0, '6b. 行注释内容被破坏');
assert(r6b.text.indexOf('\nFROM t') >= 0, '6b. 行注释后应换行且 FROM 顶格');
assertTokensUnchanged('6b', c6b, r6b.text);

/* ======================================================================
 * 7. CASE / WHEN / ELSE / END 缩进
 * ==================================================================== */

var c7 = "SELECT CASE WHEN a=1 THEN 'x' WHEN a=2 THEN 'y' ELSE 'z' END AS c FROM t";
var r7 = run('7. CASE 缩进', c7);
assert(r7.ok === true, '7. 应成功');
eq(r7.text,
  "SELECT CASE\n" +
  "  WHEN a = 1 THEN 'x'\n" +
  "  WHEN a = 2 THEN 'y'\n" +
  "  ELSE 'z'\n" +
  "END AS c\n" +
  "FROM t",
  '7. CASE 输出不符（WHEN/ELSE 应相对 CASE 所在行缩进一层，END 回到 CASE 的缩进）');
assert(r7.text.indexOf('\n  WHEN') > 0, '7. WHEN 应各占一行并缩进');
assert(r7.text.indexOf('\nEND AS c') > 0, '7. END 应回到 CASE 的缩进');
assertTokensUnchanged('7', c7, r7.text);

/* ======================================================================
 * 8. UNION / UNION ALL、DELETE FROM、分号换行
 * ==================================================================== */

var c8 = 'SELECT a FROM t UNION ALL SELECT b FROM u';
var r8 = run('8. UNION ALL', c8);
eq(r8.text, 'SELECT a\nFROM t\nUNION ALL\nSELECT b\nFROM u', '8. UNION ALL 输出不符');

var c8b = 'DELETE FROM t WHERE a=1;SELECT 1';
var r8b = run('8b. DELETE FROM + 分号', c8b);
assert(r8b.ok === true, '8b. 应成功');
eq(r8b.text, 'DELETE FROM t\nWHERE a = 1;\nSELECT 1', '8b. 分号后换行不符');
assertTokensUnchanged('8b', c8b, r8b.text);

/* ======================================================================
 * 9. 限定名、操作符、括号内不换行、BETWEEN ... AND 不拆行
 * ==================================================================== */

var c9 = 'SELECT s.t.col, a::text AS x FROM s.t WHERE a <> 1 AND b >= 2 AND c BETWEEN 1 AND 9 AND d != 3';
var r9 = run('9. 限定名/运算符/BETWEEN', c9);
assert(r9.ok === true, '9. 应成功');
assert(r9.text.indexOf('s.t.col') >= 0, '9. 点号限定名输出错误');
assert(r9.text.indexOf('a::text') >= 0, '9. :: 转换输出错误');
assert(r9.text.indexOf('a <> 1') >= 0, '9. <> 两侧应各一个空格');
assert(r9.text.indexOf('b >= 2') >= 0, '9. >= 两侧应各一个空格');
assert(r9.text.indexOf('c BETWEEN 1 AND 9') >= 0, '9. BETWEEN ... AND 不应被拆行');
assert(r9.text.indexOf('d != 3') >= 0, '9. != 两侧应各一个空格');
assertTokensUnchanged('9', c9, r9.text);

var c9b = "SELECT f(a,b,c) FROM t WHERE x IN (1,2,3)";
var r9b = run('9b. 普通函数调用内不换行', c9b);
eq(r9b.text,
  'SELECT f(a, b, c)\n' +
  'FROM t\n' +
  'WHERE x IN (1, 2, 3)',
  '9b. 函数/IN 输出不符');

var c9c = 'SELECT [my col], [a]]b] FROM [dbo].[t 1]';
var r9c = run('9c. 方括号标识符', c9c);
assert(r9c.ok === true, '9c. 应成功');
assert(r9c.text.indexOf('[my col],') >= 0, '9c. 含空格方括号标识符被破坏');
assert(r9c.text.indexOf('[a]]b]') >= 0, '9c. 方括号转义被破坏');
assert(r9c.text.indexOf('[dbo].[t 1]') >= 0, '9c. 方括号限定名被破坏');
assertTokensUnchanged('9c', c9c, r9c.text);

var c9d = 'SELECT 1.5e-3 AS a, 0x1F AS b, -1 AS c';
var r9d = run('9d. 数字与负号', c9d);
assert(r9d.ok === true, '9d. 应成功');
assert(r9d.text.indexOf('1.5e-3') >= 0, '9d. 科学计数法被破坏');
assert(r9d.text.indexOf('0x1F') >= 0, '9d. 十六进制被破坏');
assertTokensUnchanged('9d', c9d, r9d.text);

/* ======================================================================
 * 10. 选项行为
 * ==================================================================== */

var c10 = 'select a,b from t where a=1';
var r10a = format(c10, { upperKeywords: false });
assert(r10a.ok === true, '10. upperKeywords:false 应成功');
assert(r10a.text.indexOf('select a,') === 0, '10. upperKeywords:false 不应转大写');
assert(r10a.text.indexOf('from t') >= 0, '10. upperKeywords:false 应保持原样（不转小写也不转大写）');
assertTokensUnchanged('10-upper-false', c10, r10a.text);

var r10b = format(c10, { commaBreaks: false });
assert(r10b.ok === true, '10. commaBreaks:false 应成功');
eq(r10b.text, 'SELECT a, b\nFROM t\nWHERE a = 1', '10. commaBreaks:false 输出不符');

var r10c = format(c10, { indent: '    ', newline: '\r\n' });
assert(r10c.ok === true, '10. 自定义 indent/newline 应成功');
assert(r10c.text.indexOf('\r\n') > 0, '10. 自定义换行符未生效');
eq(r10c.text, 'SELECT a,\r\n       b\r\nFROM t\r\nWHERE a = 1',
  '10. 自定义换行符输出不符（列清单对齐到 SELECT 之后）');

/* 子句内层缩进用 opts.indent */
var r10c2 = format('SELECT a FROM t WHERE a = 1 AND b = 2', { indent: '    ' });
eq(r10c2.text, 'SELECT a\nFROM t\nWHERE a = 1\n    AND b = 2', '10. 自定义 indent 未作用于 AND 行');

var r10d = format(c10, undefined);
assert(r10d.ok === true, '10. opts 为 undefined 应使用默认值');
assert(r10d.text.indexOf('SELECT a,') === 0, '10. 默认 opts 输出异常');

var r10e = format(c10, {});
assert(r10e.ok === true, '10. opts 为空对象应使用默认值');

var r10f = format('   \n\t  ');
assert(r10f.ok === true && r10f.text === '', '10. 纯空白应返回空串');

var r10g = format('');
assert(r10g.ok === true && r10g.text === '', '10. 空串应返回空串');

var r10h = format(null);
assert(r10h.ok === true && r10h.text === '', '10. null 应返回空串而不是抛异常');

var r10i = format(123);
assert(r10i.ok === false && typeof r10i.error === 'string', '10. 非字符串应返回 ok:false');

/* ======================================================================
 * 11. 错误处理
 * ==================================================================== */

var e1 = format("SELECT 'abc FROM t");
assert(e1.ok === false, "11. 未闭合单引号字符串应返回 ok:false");
assert(/第 1 行/.test(e1.error), '11. 未闭合字符串的错误信息应含行号');
assert(typeof e1.line === 'number' && e1.line === 1, '11. 未闭合字符串应给出行号');
console.log('未闭合字符串 → ' + JSON.stringify(e1));
console.log('');

var e2 = format('SELECT a FROM t\nWHERE b = 1 /* 没结束\n  still');
assert(e2.ok === false, '11. 未闭合块注释应返回 ok:false');
assert(e2.line === 2, '11. 未闭合块注释应给出行号 2');
console.log('未闭合块注释 → ' + JSON.stringify(e2));
console.log('');

var e3 = format('SELECT a,\n  b FROM t WHERE (a = 1');
assert(e3.ok === false, '11. 括号不配对（少右括号）应返回 ok:false');
assert(typeof e3.line === 'number', '11. 括号错误应给出行号');
console.log('少右括号 → ' + JSON.stringify(e3));
console.log('');

var e4 = format('SELECT a FROM t) WHERE x = 1');
assert(e4.ok === false, '11. 多余右括号应返回 ok:false');
console.log('多右括号 → ' + JSON.stringify(e4));
console.log('');

var e5 = format('SELECT CASE WHEN a = 1 THEN 2 FROM t');
assert(e5.ok === false, '11. CASE 缺 END 应返回 ok:false');
console.log('CASE 缺 END → ' + JSON.stringify(e5));
console.log('');

/* ======================================================================
 * 12. 「不抛异常」总检查 + 批量 token 不变检查
 * ==================================================================== */

var fuzz = [
  'SELECT',
  'SELECT 1',
  ';;;',
  'SELECT (a) FROM t',
  'SELECT * FROM t WHERE a IN (1,2,3) AND b NOT IN (SELECT id FROM u)',
  'SELECT a FROM t WHERE (a = 1 OR b = 2) AND c = 3',
  'WITH c AS (SELECT 1 AS x) SELECT x FROM c',
  'SELECT N\'中文\' AS s, "双引号""列" FROM `表`',
  'SELECT a\n-- 行注释\nFROM t',
  'SELECT 1 /* a */ + /* b */ 2',
  'SELECT * FROM t ORDER BY 1 LIMIT 10 OFFSET 5',
  'SELECT * FROM t GROUP BY a, b, c HAVING count(*) > 1',
  'SELECT a||b AS ab FROM t',
  'SELECT CAST(a AS VARCHAR(10)) FROM t',
  'SELECT * FROM t1 CROSS JOIN t2',
  'SELECT * FROM t1 FULL OUTER JOIN t2 ON t1.a = t2.a',
  'SELECT @var, :param FROM t',
  'SELECT -1, - 2, a - b FROM t',
  'SELECT t.a, x.5 FROM t',
];

for (var fi = 0; fi < fuzz.length; fi++) {
  var src = fuzz[fi];
  var res;
  var threw = false;
  try {
    res = format(src);
  } catch (err) {
    threw = true;
  }
  assert(!threw, '12. format 不应抛异常：' + JSON.stringify(src));
  assert(res && typeof res === 'object' && typeof res.ok === 'boolean',
    '12. 返回值结构不正确：' + JSON.stringify(src));
  if (res && res.ok) {
    assertTokensUnchanged('fuzz[' + fi + ']', src, res.text);
  } else {
    assert(typeof res.error === 'string' && res.error.length > 0,
      '12. 失败时应返回非空中文说明：' + JSON.stringify(src));
  }
}

/* ======================================================================
 * 13. 全局挂载检查（浏览器侧 root.TBFormat.sql）
 * ==================================================================== */

assert(typeof globalThis.TBFormat === 'object' && globalThis.TBFormat !== null,
  '13. 应挂载 globalThis.TBFormat');
assert(globalThis.TBFormat && typeof globalThis.TBFormat.sql.format === 'function',
  '13. 应挂载 globalThis.TBFormat.sql.format');

/* ======================================================================
 * 14. token 不粘连（- - 不能变成注释符、+ > 不能变成 -> 等）
 * ==================================================================== */

var glue = [
  'SELECT - - 1 AS a',
  'SELECT a - -b FROM t',
  'SELECT a+>b FROM t',
  'SELECT a < > b FROM t',
  'SELECT 1 : ::int FROM t',
  'SELECT a| |b FROM t',
  'SELECT N\'中文\' AS s',
  'SELECT a - /* c */ - b FROM t'
];
for (var gi = 0; gi < glue.length; gi++) {
  var gsrc = glue[gi];
  var gres = format(gsrc);
  assert(gres.ok === true, '14. 应成功：' + gsrc + ' → ' + (gres && gres.error));
  if (gres.ok) assertTokensUnchanged('glue[' + gi + ']', gsrc, gres.text);
}
var g1 = format('SELECT - - 1 AS a');
assert(g1.ok && g1.text.indexOf('--') < 0, '14. "- -" 不能被粘成注释符 "--"');
console.log('"- -" → ' + JSON.stringify(g1.text));
console.log('');

/* ======================================================================
 * 15. Unicode 标识符 / 大输入性能
 * ==================================================================== */

var c15 = "SELECT 中文字段, ünïcode_名 FROM 表1 WHERE 名 = '值'";
var r15 = run('15. Unicode 标识符', c15);
assert(r15.ok === true, '15. Unicode 标识符应成功');
assert(r15.text.indexOf('中文字段') >= 0 && r15.text.indexOf('表1') >= 0, '15. Unicode 标识符被破坏');
assertTokensUnchanged('15', c15, r15.text);

var bigCols = [];
for (var bi = 0; bi < 800; bi++) bigCols.push('col_' + bi);
var bigSrc = 'SELECT ' + bigCols.join(',') + ' FROM big_table WHERE a = 1';
var t0 = Date.now();
var rBig = format(bigSrc);
var bigMs = Date.now() - t0;
assert(rBig.ok === true, '15. 大输入应成功');
assert(bigMs < 2000, '15. 大输入耗时过长：' + bigMs + 'ms');
assertTokensUnchanged('15-big', bigSrc, rBig.text);
console.log('800 列输入：' + bigSrc.length + ' 字符 → ' + bigMs + ' ms，' +
  rBig.text.split('\n').length + ' 行');
console.log('');

/* ======================================================================
 * 结果汇总
 * ==================================================================== */

console.log('----------------------------------------');
if (failed.length) {
  console.log('失败 ' + failed.length + ' 项，通过 ' + passed + ' 项');
  for (var k = 0; k < failed.length; k++) {
    console.log('✗ ' + failed[k]);
    console.log('');
  }
  process.exit(1);
} else {
  console.log('全部断言通过：' + passed + ' 项');
  process.exit(0);
}
