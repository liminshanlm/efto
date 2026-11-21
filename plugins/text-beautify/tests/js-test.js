/* 自测：E:\dsh\text-beautify\tests\js-test.js — 运行 node E:\dsh\text-beautify\tests\js-test.js */
'use strict';

var fmt = require('../lib/fmt-js');

var failures = 0;
var checks = 0;

function ok(cond, msg, extra) {
  checks++;
  if (cond) {
    console.log('  PASS  ' + msg);
  } else {
    failures++;
    console.log('  FAIL  ' + msg);
    if (extra !== undefined) console.log('        ' + String(extra).replace(/\n/g, '\n        '));
  }
}

/* ------------------------------------------------------------------ *
 * 独立的简化 tokenizer（用于 token 等价自检）
 * 只保留 token 序列，不关心空白与换行
 * ------------------------------------------------------------------ */

function isIdStart(ch) {
  if (!ch) return false;
  if (/[A-Za-z_$]/.test(ch)) return true;
  var c = ch.charCodeAt(0);
  if (c < 0x80) return false;
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
  return /[A-Za-z0-9_$]/.test(ch) || ch === '\u200c' || ch === '\u200d' || isIdStart(ch);
}
function isDigit(ch) { return ch >= '0' && ch <= '9'; }

var PUNCS = [
  '>>>=', '...', '===', '!==', '**=', '<<=', '>>=', '>>>', '&&=', '||=', '??=',
  '=>', '==', '!=', '<=', '>=', '&&', '||', '??', '?.', '++', '--',
  '+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '<<', '>>', '**',
  '{', '}', '(', ')', '[', ']', ';', ',', '<', '>', '+', '-', '*', '/',
  '%', '&', '|', '^', '!', '~', '?', ':', '=', '.', '@', '#'
];

var REGEX_KW = {
  'return': 1, 'typeof': 1, 'instanceof': 1, 'in': 1, 'of': 1, 'new': 1,
  'delete': 1, 'void': 1, 'throw': 1, 'case': 1, 'do': 1, 'else': 1,
  'yield': 1, 'await': 1
};

function tokens(src) {
  var out = [];
  var i = 0;
  var n = src.length;
  var prev = null;

  function push(t, v) {
    out.push(v);
    prev = { t: t, v: v };
  }

  function scanStr(q) {
    var j = i + 1;
    while (j < n) {
      var c = src.charAt(j);
      if (c === '\\') { j += 2; continue; }
      if (c === q) { j++; break; }
      j++;
    }
    var s = src.slice(i, j);
    i = j;
    push('str', s);
  }

  /* 模板字符串拆成 raw / ${expr} 片段：raw 必须逐字相同，表达式内部允许重排空白 */
  function scanTmpl() {
    var start = i;
    var parts = [];
    var j = i + 1;
    var rawStart = j;
    while (j < n) {
      var c = src.charAt(j);
      if (c === '\\') { j += 2; continue; }
      if (c === '`') { j++; break; }
      if (c === '$' && src.charAt(j + 1) === '{') {
        parts.push({ raw: src.slice(rawStart, j + 2) });
        var after = scanTemplateExprEnd(j + 2);
        parts.push({ expr: src.slice(j + 2, after - 1) });
        j = after;
        rawStart = j;
        continue;
      }
      j++;
    }
    if (rawStart <= n) parts.push({ raw: src.slice(rawStart, j) });
    i = j;
    push('tmpl', 'TMPL_OPEN');
    for (var k = 0; k < parts.length; k++) {
      if (parts[k].raw !== undefined) {
        push('raw', 'RAW:' + parts[k].raw);
      } else {
        push('tmpl', 'EXPR_OPEN');
        var inner = tokens(parts[k].expr).split('\u0001');
        for (var q = 0; q < inner.length; q++) {
          if (inner[q] !== '') out.push(inner[q]);
        }
        out.push('EXPR_CLOSE');
        prev = null;
      }
    }
    push('tmpl', 'TMPL_CLOSE');
  }

  function scanTemplateExprEnd(from) {
    var j = from;
    var depth = 1;
    while (j < n) {
      var c = src.charAt(j);
      if (c === '\\') { j += 2; continue; }
      if (c === '"' || c === "'") {
        var q = c;
        j++;
        while (j < n) {
          var d = src.charAt(j);
          if (d === '\\') { j += 2; continue; }
          if (d === q) { j++; break; }
          j++;
        }
        continue;
      }
      if (c === '`') {
        j++;
        while (j < n) {
          var e = src.charAt(j);
          if (e === '\\') { j += 2; continue; }
          if (e === '`') { j++; break; }
          if (e === '$' && src.charAt(j + 1) === '{') {
            j = scanTemplateExprEnd(j + 2);
            continue;
          }
          j++;
        }
        continue;
      }
      if (c === '{') { depth++; j++; continue; }
      if (c === '}') { depth--; j++; if (depth === 0) return j; continue; }
      j++;
    }
    return j;
  }

  function regexOK() {
    if (!prev) return true;
    if (prev.t === 'num' || prev.t === 'str' || prev.t === 'tmpl' || prev.t === 're') return false;
    if (prev.t === 'id') {
      if (prev.v === 'true' || prev.v === 'false' || prev.v === 'null' || prev.v === 'this') return false;
      return !!REGEX_KW[prev.v];
    }
    if (prev.v === ')' || prev.v === ']' || prev.v === '}') return false;
    if (prev.v === '++' || prev.v === '--') return false;
    return true;
  }

  while (i < n) {
    var ch = src.charAt(i);
    if (/\s/.test(ch)) { i++; continue; }
    if (ch === '/' && src.charAt(i + 1) === '/') {
      var j0 = i + 2;
      while (j0 < n && src.charAt(j0) !== '\n' && src.charAt(j0) !== '\r') j0++;
      push('lc', src.slice(i, j0));
      i = j0;
      continue;
    }
    if (ch === '/' && src.charAt(i + 1) === '*') {
      var end = src.indexOf('*/', i + 2);
      end = end < 0 ? n : end + 2;
      push('bc', src.slice(i, end));
      i = end;
      continue;
    }
    if (ch === '"' || ch === "'") { scanStr(ch); continue; }
    if (ch === '`') { scanTmpl(); continue; }
    if (ch === '/' && regexOK()) {
      var jr = i + 1;
      var inClass = false;
      var closed = false;
      while (jr < n) {
        var cr = src.charAt(jr);
        if (cr === '\\') { jr += 2; continue; }
        if (cr === '\n') break;
        if (cr === '[') { inClass = true; jr++; continue; }
        if (cr === ']') { inClass = false; jr++; continue; }
        if (cr === '/' && !inClass) { jr++; closed = true; break; }
        jr++;
      }
      if (closed) {
        while (jr < n && /[a-zA-Z]/.test(src.charAt(jr))) jr++;
        push('re', src.slice(i, jr));
        i = jr;
        continue;
      }
      /* 落到除号处理 */
    }
    if (isDigit(ch)) {
      var jn = i;
      if (src.charAt(jn) === '0' && /[xXbBoO]/.test(src.charAt(jn + 1))) {
        jn += 2;
        while (jn < n && /[0-9a-fA-F_]/.test(src.charAt(jn))) jn++;
      } else {
        while (jn < n && /[0-9_]/.test(src.charAt(jn))) jn++;
        if (src.charAt(jn) === '.') {
          jn++;
          while (jn < n && /[0-9_]/.test(src.charAt(jn))) jn++;
        }
        if (src.charAt(jn) === 'e' || src.charAt(jn) === 'E') {
          var jk = jn + 1;
          if (src.charAt(jk) === '+' || src.charAt(jk) === '-') jk++;
          if (isDigit(src.charAt(jk))) {
            jn = jk;
            while (jn < n && /[0-9_]/.test(src.charAt(jn))) jn++;
          }
        }
      }
      if (src.charAt(jn) === 'n') jn++;
      push('num', src.slice(i, jn));
      i = jn;
      continue;
    }
    if (isIdStart(ch)) {
      var ji = i + 1;
      while (ji < n && isIdPart(src.charAt(ji))) ji++;
      push('id', src.slice(i, ji));
      i = ji;
      continue;
    }
    var matched = null;
    for (var p = 0; p < PUNCS.length; p++) {
      if (src.substr(i, PUNCS[p].length) === PUNCS[p]) { matched = PUNCS[p]; break; }
    }
    if (!matched) matched = ch;
    push('p', matched);
    i += matched.length;
  }
  return out.join('\u0001');
}

/* ------------------------------------------------------------------ *
 * 样例
 * ------------------------------------------------------------------ */

var samples = [
  {
    name: '01 压缩单行 function',
    code: 'function f(a,b){if(a){return b}else{return a}}'
  },
  {
    name: '02 对象/数组/模板字符串',
    code: [
      'const cfg={name:"demo",list:[1,2,3],nested:{a:1,b:[4,5]}};',
      'const s=`hello ${a+b} and ${`inner ${c}`} done`;',
      'const arr=[1,2,3,4,5];',
      'const obj2={x:{y:{z:1}}};'
    ].join('\n')
  },
  {
    name: '03 正则与除号混用',
    code: [
      'const re=/a\\/b[/]/g;',
      'const q=a / b / c;',
      'const r2=x.replace(/\\d+/g,"N");',
      'let z=(a+b)/2;',
      'const cls=/[a-z]+/i.test(s);'
    ].join('\n')
  },
  {
    name: '04 注释 async/await for',
    code: [
      '// 顶部注释',
      'async function run(list){',
      '/* 块注释：遍历',
      '   第二行内容 */',
      'const out=[];',
      'for (let i=0;i<3;i++){',
      'out.push(await fn(list[i])); // 行尾注释',
      '}',
      'return out;',
      '}'
    ].join('\n')
  },
  {
    name: '05 箭头/可选链/空值合并/可选赋值',
    code: [
      'const g=(x)=>({a:1});',
      'const val=a?.b?.(c);',
      'const d=a ?? b;',
      'a ??= b;',
      'const h=async (p)=>{const v=await p?.q?.();return v??0;};',
      'const n=obj?.deep?.value ?? "def";'
    ].join('\n')
  },
  {
    name: '06 switch / try-catch-finally',
    code: [
      'function go(k){',
      'switch(k){',
      'case 1:',
      'return "one";',
      'case 2:',
      'return "two";',
      'default:',
      'return "other";',
      '}',
      '}',
      'try{',
      'risky();',
      '}catch(e){',
      'console.log(e);',
      '}finally{',
      'clean();',
      '}'
    ].join('\n')
  },
  {
    name: '07 TypeScript 片段',
    code: [
      'interface Point<T>{x:number;y:T;}',
      'type Pair<A,B>=[A,B];',
      'function map<T,U>(xs:Array<T>,f:(x:T)=>U):U[]{',
      'const out:U[]=[];',
      'for (const x of xs){out.push(f(x));}',
      'return out;',
      '}',
      'const p:Point<string>={x:1,y:"a"};',
      'class Box<T>{private v:T;constructor(v:T){this.v=v;}}'
    ].join('\n')
  },
  {
    name: '08 中文标识符与字符串',
    code: [
      'const 用户名="张三";',
      'const 设置={标题:"报表",项目:[1,2,3]};',
      'function 计算(甲,乙){return 甲+乙;}',
      'console.log(`结果：${用户名} 共 ${计算(1,2)} 项`);'
    ].join('\n')
  }
];

/* ------------------------------------------------------------------ *
 * 1. token 等价 + 4. 幂等性
 * ------------------------------------------------------------------ */

console.log('=== 用例：token 等价 & 幂等 ===');
samples.forEach(function (s) {
  var f1 = fmt.format(s.code, {});
  if (!f1.ok) {
    ok(false, s.name + ' → 格式化成功', f1.error);
    return;
  }
  var tin = tokens(s.code);
  var tout = tokens(f1.text);
  ok(tin === tout, s.name + ' → token 序列等价',
    tin === tout ? undefined : '输入:\n' + tin.replace(/\u0001/g, ' ') + '\n输出:\n' + tout.replace(/\u0001/g, ' '));

  var f2 = fmt.format(f1.text, {});
  ok(f2.ok && f2.text === f1.text, s.name + ' → 幂等（二次格式化相同）',
    f2.ok ? (f2.text === f1.text ? undefined : 'diff:\n--- 1 ---\n' + f1.text + '\n--- 2 ---\n' + f2.text) : f2.error);
});

/* ------------------------------------------------------------------ *
 * 2. 缩进/换行
 * ------------------------------------------------------------------ */

console.log('');
console.log('=== 用例：缩进与换行 ===');
(function () {
  var r = fmt.format('function f(){return 1}', {});
  ok(r.ok, 'function f(){return 1} 成功', r.error);
  ok(r.ok && r.text.indexOf('\n') >= 0, '输出包含换行');
  ok(r.ok && r.text.indexOf('\n' + '  ') >= 0, '输出包含两个空格缩进');
  console.log('        ---- 实际输出 ----');
  console.log(r.text.split('\n').map(function (l) { return '        ' + l; }).join('\n'));

  var r2 = fmt.format('function f(){return 1}', { indent: '\t', newline: '\r\n' });
  ok(r2.ok && r2.text.indexOf('\r\n\t') >= 0, '自定义 indent/newline 生效');
})();

/* ------------------------------------------------------------------ *
 * 3. 未闭合字符串
 * ------------------------------------------------------------------ */

console.log('');
console.log('=== 用例：错误处理 ===');
(function () {
  var bad = fmt.format('const s = "abc;\nconst t = 1;', {});
  ok(bad.ok === false, '未闭合双引号返回 ok:false', JSON.stringify(bad));
  ok(bad.ok === false && /未闭合/.test(bad.error), '错误信息为中文说明', bad.error);

  var bad2 = fmt.format('const s = `abc ${a', {});
  ok(bad2.ok === false, '未闭合模板字符串返回 ok:false', JSON.stringify(bad2));

  var bad3 = fmt.format('const c = /* abc', {});
  ok(bad3.ok === false, '未闭合块注释返回 ok:false', JSON.stringify(bad3));

  var bad4 = fmt.format('function f(){ if (x) { }', {});
  ok(bad4.ok === false, '括号不配对返回 ok:false', JSON.stringify(bad4));

  /* 不应抛异常 */
  var weird = [null, undefined, 123, {}, 'a\r\n\r\n\r\nb', ';;;', ')', '}', '`', '/', '"'];
  var threw = false;
  for (var i = 0; i < weird.length; i++) {
    try { fmt.format(weird[i], undefined); } catch (e) { threw = true; }
    try { fmt.format(weird[i], { indent: 1, newline: 2, singleQuote: 'x' }); } catch (e2) { threw = true; }
  }
  ok(!threw, '异常输入与非法 opts 均不抛异常');
})();

/* ------------------------------------------------------------------ *
 * 附加：排版规则抽查
 * ------------------------------------------------------------------ */

console.log('');
console.log('=== 用例：排版规则抽查 ===');
(function () {
  var r = fmt.format('if (a) { b(); } else { c(); }', {});
  ok(r.ok && r.text.indexOf('} else {') >= 0, '} 后接 else 同行', r.text);

  var r2 = fmt.format('const o = {}; const a = [];', {});
  ok(r2.ok && r2.text.indexOf('{}') >= 0 && r2.text.indexOf('[]') >= 0, '空对象/空数组不拆行', r2.text);

  var r3 = fmt.format('a ??= b; c &&= d; e >>>= f;', {});
  ok(r3.ok && /a \?\?= b/.test(r3.text), '??= 两侧空格', r3.text);

  var r4 = fmt.format('const s = "it\'s ok"; const t = "plain";', { singleQuote: true });
  ok(r4.ok && r4.text.indexOf("'plain'") >= 0, 'singleQuote 归一化生效', r4.text);
  ok(r4.ok && r4.text.indexOf('"it\'s ok"') >= 0, '含单引号的字符串保持不变', r4.text);

  var r5 = fmt.format('const s = "plain";', {});
  ok(r5.ok && r5.text.indexOf('"plain"') >= 0, '默认不改引号', r5.text);

  var r6 = fmt.format('a\r\n\r\n\r\n\r\nb', {});
  ok(r6.ok && r6.text === 'a' + '\n' + '\n' + 'b', '连续空行压缩为一个', JSON.stringify(r6.text));

  var r7 = fmt.format('for (let i=0;i<3;i++){x(i);}', {});
  ok(r7.ok && /for \(let i = 0; i < 3; i\+\+\) \{/.test(r7.text), 'for 头分号留在同一行', r7.text);

  var r8 = fmt.format('class Box<T>{private v:T;get x():number{return 1;}}', {});
  ok(r8.ok && /class Box<T> \{/.test(r8.text) && /get x\(\): number \{/.test(r8.text),
    '类体与方法体识别、泛型无空格', r8.ok ? r8.text : r8.error);
  ok(r8.ok && r8.text.indexOf('} get x') < 0, '类成员之间正确换行');

  var r9 = fmt.format('import{a as b,c}from"m";', {});
  ok(r9.ok && r9.text === 'import { a as b, c } from "m";', 'import 说明符单行且间距正确', r9.ok ? r9.text : r9.error);

  var r10 = fmt.format('const v = -1 + +2 - -a; const w = !x && ~y;', {});
  ok(r10.ok && /-1 \+ \+2 - -a/.test(r10.text) && /!x && ~y/.test(r10.text), '一元运算符紧贴操作数', r10.ok ? r10.text : r10.error);

  var r11 = fmt.format('const s=`a${b?1:2}c`;', {});
  ok(r11.ok && r11.text.indexOf('`a${b ? 1 : 2}c`') >= 0, '模板 ${} 内三元空格正确', r11.ok ? r11.text : r11.error);

  var r12 = fmt.format('let a=1\nlet b=2\nfoo()', {});
  ok(r12.ok && r12.text.split('\n').length === 3, '无分号语句按 ASI 换行', r12.ok ? r12.text : r12.error);

  var r13 = fmt.format('a(b)c)', {});
  ok(r13.ok === false, '多余右括号返回 ok:false', JSON.stringify(r13));

  var r14 = fmt.format('const n=a?b?(c?d:e):f:g;', {});
  ok(r14.ok && r14.text === 'const n = a ? b ? (c ? d : e) : f : g;', '嵌套三元每个冒号都留空格', r14.ok ? r14.text : r14.error);

  var r15 = fmt.format('const o={...a,b:1}; const arr=[...b];', {});
  ok(r15.ok && /\.\.\.a/.test(r15.text) && /\[\.\.\.b\]/.test(r15.text), '展开运算符紧贴操作数', r15.ok ? r15.text : r15.error);

  var r16 = fmt.format('class A{#p=1;static #q=2;}', {});
  ok(r16.ok && /static #q = 2;/.test(r16.text) && /#p = 1;/.test(r16.text), '私有字段 # 与名字紧贴', r16.ok ? r16.text : r16.error);

  var r17 = fmt.format('function*g(){yield* h();}', {});
  ok(r17.ok && /function\* g\(\)/.test(r17.text), 'function* 紧贴、名字后空格', r17.ok ? r17.text : r17.error);

  var r18 = fmt.format('function f(a, b) { return a - -b; }', {});
  ok(r18.ok && /a - -b/.test(r18.text), '二元减与一元负号组合正确', r18.ok ? r18.text : r18.error);

  var r19 = fmt.format('export{a,b};', {});
  ok(r19.ok && r19.text === 'export { a, b };', 'export 说明符单行', r19.ok ? r19.text : r19.error);

  var r20 = fmt.format('switch(x){case 1:case 2:a();break;default:b();}', {});
  ok(r20.ok && /case 1:\n  case 2:\n    a\(\);/.test(r20.text), 'switch 连续 case 与体缩进正确', r20.ok ? r20.text : r20.error);
})();

/* ------------------------------------------------------------------ *
 * 5. 真实源码回归：项目自身的 .js 文件应能格式化、幂等、且产物仍可被引擎解析
 * ------------------------------------------------------------------ */

console.log('');
console.log('=== 用例：真实源码回归（幂等 + 可解析） ===');
(function () {
  var fs = require('fs');
  var path = require('path');
  var vm = require('vm');
  var root = path.join(__dirname, '..');
  var targets = [
    path.join(root, 'ui.js'),
    path.join(root, 'main.js'),
    path.join(root, 'lib', 'fmt-json.js'),
    path.join(root, 'lib', 'fmt-js.js'),
    path.join(root, 'tests', 'run-all.js')
  ];
  targets.forEach(function (p) {
    if (!fs.existsSync(p)) return;
    var name = path.basename(p);
    var src = fs.readFileSync(p, 'utf8');
    var r1 = fmt.format(src, {});
    if (!r1.ok) { ok(false, name + ' 格式化成功', r1.error); return; }
    var r2 = fmt.format(r1.text, {});
    var r3 = fmt.format(r2.ok ? r2.text : '', {});
    ok(r2.ok && r2.text === r1.text, name + ' 第一次幂等');
    ok(r3.ok && r3.text === r2.text, name + ' 第二次幂等（无渐进漂移）');
    var parsed = true;
    try { new vm.Script(r1.text, { filename: name }); } catch (e) { parsed = false; }
    ok(parsed, name + ' 输出仍可被 JS 引擎解析');
    ok(tokens(src) === tokens(r1.text), name + ' token 序列等价');
  });
})();

/* ------------------------------------------------------------------ *
 * 汇总
 * ------------------------------------------------------------------ */

console.log('');
if (failures === 0) {
  console.log('全部 ' + checks + ' 项断言通过。');
  process.exit(0);
} else {
  console.log('失败 ' + failures + ' / ' + checks + ' 项断言。');
  process.exit(1);
}
