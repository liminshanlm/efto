'use strict';

const assert = require('assert');
const xml = require('../lib/fmt-xml');

let passed = 0;
const failures = [];

function test(name, fn) {
  try {
    fn();
    passed++;
    console.log('  ok   ' + name);
  } catch (e) {
    failures.push({ name: name, error: e });
    console.log('  FAIL ' + name + '\n       ' + (e && e.message ? e.message.split('\n').join('\n       ') : e));
  }
}

function show(s) {
  return JSON.stringify(s);
}

/* ---------- 简易词法：与格式化无关，只用于「脱掉格式后 token 序列不变」自检 ---------- */

function isWs(c) {
  return c === ' ' || c === '\t' || c === '\n' || c === '\r';
}

function scanTokens(s) {
  const out = [];
  let i = 0;
  while (i < s.length) {
    if (s[i] === '<' && s[i + 1] !== undefined && !isWs(s[i + 1])) {
      // 注释
      if (s.startsWith('<!--', i)) {
        const e = s.indexOf('-->', i + 4);
        out.push({ t: 'comment', v: s.slice(i, e + 3) });
        i = e + 3;
        continue;
      }
      if (s.startsWith('<![CDATA[', i)) {
        const e = s.indexOf(']]>', i + 9);
        out.push({ t: 'cdata', v: s.slice(i, e + 3) });
        i = e + 3;
        continue;
      }
      if (s.startsWith('<?', i)) {
        const e = s.indexOf('?>', i + 2);
        out.push({ t: 'pi', v: s.slice(i, e + 2) });
        i = e + 2;
        continue;
      }
      if (s.slice(i, i + 9).toUpperCase() === '<!DOCTYPE') {
        let depth = 0;
        let q = '';
        let j = i + 9;
        for (; j < s.length; j++) {
          const c = s[j];
          if (q) {
            if (c === q) q = '';
            continue;
          }
          if (c === '"' || c === "'") { q = c; continue; }
          if (c === '[') depth++;
          else if (c === ']') depth--;
          else if (c === '>' && depth === 0) break;
        }
        out.push({ t: 'doctype', v: s.slice(i, j + 1) });
        i = j + 1;
        continue;
      }
      // 普通/结束标签（属性值内 '>' 不算结束）
      let q = '';
      let j = i + 1;
      for (; j < s.length; j++) {
        const c = s[j];
        if (q) {
          if (c === q) q = '';
          continue;
        }
        if (c === '"' || c === "'") { q = c; continue; }
        if (c === '>') break;
      }
      const raw = s.slice(i, j + 1);
      const closing = raw.startsWith('</');
      const selfClosing = !closing && /\/\s*>$/.test(raw);
      // 归一：标签内多空白压成一个空格，并去掉 <a > 里的多余空格
      const norm = raw.replace(/[ \t\r\n]+/g, ' ').replace(/ ?\/?>$/, function (m) {
        return m.replace(/^\s+/, '');
      }).trim();
      out.push({ t: closing ? 'close' : (selfClosing ? 'self' : 'open'), v: norm });
      i = j + 1;
      continue;
    }
    // 文本
    let j = i;
    while (j < s.length && !(s[j] === '<' && s[j + 1] !== undefined && !isWs(s[j + 1]))) j++;
    const txt = s.slice(i, j);
    if (txt.trim() !== '') {
      out.push({ t: 'text', v: txt.replace(/^[ \t\r\n]+|[ \t\r\n]+$/g, '') });
    }
    i = j;
  }
  return out;
}

function assertSameTokens(input, output, label) {
  const a = scanTokens(input);
  const b = scanTokens(output);
  assert.deepStrictEqual(
    b,
    a,
    (label || '') + ' 输出 token 序列与输入不一致\n输入: ' + JSON.stringify(a) + '\n输出: ' + JSON.stringify(b)
  );
}

/* ---------------------------- 1. 紧凑单行展开 ---------------------------- */

test('紧凑单行 XML 展开成多行', function () {
  const r = xml.format('<root><a><b>1</b></a><c/></root>');
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  const expected = '<root>\n  <a>\n    <b>1</b>\n  </a>\n  <c/>\n</root>\n';
  assert.strictEqual(r.text, expected, '\n实际: ' + show(r.text));
});

/* ------------------------ 2. 声明 + 注释 + CDATA + 自闭合 ------------------------ */

test('XML 声明 + 注释 + CDATA + 自闭合标签', function () {
  const input =
    '<?xml version="1.0" encoding="utf-8"?><root>' +
    '<!-- 多行\n  注释  保留\n-->' +
    '<script><![CDATA[ if (a<b && c>d) { x(); }  保留  空白 ]]></script>' +
    '<img src="a.png" alt="图片"/>' +
    '</root>';
  const r = xml.format(input);
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  const lines = r.text.split('\n');
  assert.strictEqual(lines[0], '<?xml version="1.0" encoding="utf-8"?>');
  assert.strictEqual(lines[1], '<root>');
  assert.strictEqual(lines[2], '  <!-- 多行');
  assert.strictEqual(lines[3], '  注释  保留');
  assert.strictEqual(lines[4], '-->');
  // CDATA 内容原样保留，独占一行（CDATA 不是文本节点，不参与单行合并）
  assert.strictEqual(lines[5], '  <script>');
  assert.strictEqual(lines[6], '    <![CDATA[ if (a<b && c>d) { x(); }  保留  空白 ]]>');
  assert.strictEqual(lines[7], '  </script>');
  assert.strictEqual(lines[8], '  <img src="a.png" alt="图片"/>');
  assert.strictEqual(lines[9], '</root>');
  assert.strictEqual(r.text.slice(-1), '\n');
  assert.strictEqual(r.text.slice(-2), '>\n', '结尾不应多出空行');
  assertSameTokens(input, r.text, 'CDATA/注释样例');
});

/* ------------------------- 3. 带内部子集的 DOCTYPE ------------------------- */

test('带内部子集的 DOCTYPE', function () {
  const input =
    '<?xml version="1.0"?>\n' +
    '<!DOCTYPE note [\n' +
    '  <!ELEMENT note (to,from)>\n' +
    '  <!ENTITY copy "&#169;">\n' +
    ']>\n' +
    '<note><to>A&amp;B</to></note>';
  const r = xml.format(input);
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  const lines = r.text.split('\n');
  assert.strictEqual(lines[0], '<?xml version="1.0"?>');
  assert.strictEqual(lines[1], '<!DOCTYPE note [');
  assert.strictEqual(lines[2], '  <!ELEMENT note (to,from)>');
  assert.strictEqual(lines[3], '  <!ENTITY copy "&#169;">');
  assert.strictEqual(lines[4], ']>');
  assert.strictEqual(lines[5], '<note>');
  assert.strictEqual(lines[6], '  <to>A&amp;B</to>');
  assert.strictEqual(lines[7], '</note>');
  assertSameTokens(input, r.text, 'DOCTYPE 样例');
});

/* ------------------------ 4. 属性值里含 '>' 的标签 ------------------------ */

test("属性值里含 '>' 的标签", function () {
  const input = '<root><a cond="x &gt; 1" other=\'y > 2\'>v</a></root>';
  const r = xml.format(input);
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(r.text, '<root>\n  <a cond="x &gt; 1" other=\'y > 2\'>v</a>\n</root>\n', show(r.text));
  assertSameTokens(input, r.text, '属性含 > 样例');
});

/* ---------------------------- 5. 只含文本的元素 ---------------------------- */

test('只含文本的元素合并成一行', function () {
  const r = xml.format('<r>\n  <title>  Hello  World  </title>\n  <empty></empty>\n</r>');
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(r.text, '<r>\n  <title>Hello  World</title>\n  <empty></empty>\n</r>\n', show(r.text));
});

/* ------------------------------ 6. 畸形输入 ------------------------------ */

test('标签不匹配的畸形输入返回 ok:false 且带行号', function () {
  const input = '<root>\n  <a>\n  </b>\n</root>';
  const r = xml.format(input);
  assert.strictEqual(r.ok, false, JSON.stringify(r));
  assert.strictEqual(r.line, 3, 'line 应为 3，实际 ' + r.line);
  assert.ok(/第 3 行/.test(r.error), r.error);
  assert.ok(/<\/b>/.test(r.error) && /<a>/.test(r.error), r.error);
  assert.strictEqual(r.text, undefined, '失败时不应输出半成品文本');
  console.log('       错误信息: ' + r.error);
});

test('未闭合标签返回 ok:false', function () {
  const r = xml.format('<root><a></root>');
  assert.strictEqual(r.ok, false, JSON.stringify(r));
  assert.ok(/第 1 行/.test(r.error), r.error);
  console.log('       错误信息: ' + r.error);
});

test('根元素之后还有文本返回 ok:false', function () {
  const r = xml.format('<root/>\n多余内容');
  assert.strictEqual(r.ok, false, JSON.stringify(r));
  assert.ok(/第 2 行/.test(r.error), r.error);
  console.log('       错误信息: ' + r.error);
});

test('注释未闭合 / CDATA 未闭合都不抛异常', function () {
  for (const bad of ['<a><!-- x</a>', '<a><![CDATA[ x</a>', '<a', '</a>', '', null]) {
    const r = xml.format(bad);
    assert.strictEqual(typeof r, 'object');
    assert.ok(r.ok === true || r.ok === false, JSON.stringify(r));
  }
  assert.strictEqual(xml.format('').ok, true);
  assert.strictEqual(xml.format(null).ok, true);
});

/* --------------------------- 7. 中文内容与实体引用 --------------------------- */

test('中文内容与实体引用原样保留', function () {
  const input = '<配置><名称>温度 &amp; 压力 &lt;上限&gt;</名称><备注>中文 内容</备注></配置>';
  const r = xml.format(input);
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(
    r.text,
    '<配置>\n  <名称>温度 &amp; 压力 &lt;上限&gt;</名称>\n  <备注>中文 内容</备注>\n</配置>\n',
    show(r.text)
  );
  assert.ok(!/&#/.test(r.text), '不应做额外转义');
});

/* ---------------------------- 8. 混合内容与命名空间 ---------------------------- */

test('混合内容保留文本行，命名空间前缀原样保留', function () {
  const input = '<ns:a>第一段 <ns:b>加粗</ns:b> 第二段</ns:a>';
  const r = xml.format(input);
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(
    r.text,
    '<ns:a>\n  第一段\n  <ns:b>加粗</ns:b>\n  第二段\n</ns:a>\n',
    show(r.text)
  );
  const round = scanTokens(input);
  assert.deepStrictEqual(round.filter(t => t.t === 'text').map(t => t.v), ['第一段', '加粗', '第二段']);
  // 输出里 第一段/第二段 各自独占一行且未被吃掉首尾有意义的空白
  assert.ok(/\n  第一段\n/.test(r.text), show(r.text));
  assert.ok(/\n  第二段\n/.test(r.text), show(r.text));
});

/* ------------------------------ 9. attrWrap ------------------------------ */

test('attrWrap 生效时属性逐个换行并对齐', function () {
  const input = '<root><item id="1" name="abcdef" type="x" desc="yyyy"/></root>';
  const r = xml.format(input, { attrWrap: 40 });
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(
    r.text,
    '<root>\n  <item\n    id="1"\n    name="abcdef"\n    type="x"\n    desc="yyyy"\n  />\n</root>\n',
    show(r.text)
  );
  assertSameTokens(input, r.text, 'attrWrap 样例');
});

test('attrWrap=0（默认）从不换行，多余空白被压平', function () {
  const input = '<root>\n  <item\n    id="1"     name="ab"\n    type="x"/>\n</root>';
  const r = xml.format(input);
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(r.text, '<root>\n  <item id="1" name="ab" type="x"/>\n</root>\n', show(r.text));
});

test('attrWrap 对开始标签（非自闭合）同样生效', function () {
  const r = xml.format('<r><a id="1" name="abcdefghij">t</a></r>', { attrWrap: 12 });
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(r.text, '<r>\n  <a id="1" name="abcdefghij">t</a>\n</r>\n', show(r.text));
  const r2 = xml.format('<r><a id="1" name="abcdefghij"><c/></a></r>', { attrWrap: 12 });
  assert.strictEqual(
    r2.text,
    '<r>\n  <a\n    id="1"\n    name="abcdefghij"\n  >\n    <c/>\n  </a>\n</r>\n',
    show(r2.text)
  );
});

/* -------------------------- 10. indent / newline 选项 -------------------------- */

test('indent 与 newline 选项', function () {
  const r = xml.format('<root><a><b>1</b></a></root>', { indent: '\t' });
  assert.strictEqual(r.text, '<root>\n\t<a>\n\t\t<b>1</b>\n\t</a>\n</root>\n', show(r.text));
  const r2 = xml.format('<root><a><b>1</b></a></root>', { indent: '    ', newline: '\r\n' });
  assert.strictEqual(r2.text, '<root>\r\n    <a>\r\n        <b>1</b>\r\n    </a>\r\n</root>\r\n', show(r2.text));
  const r3 = xml.format('<root><a><b>1</b></a></root>', undefined);
  assert.strictEqual(r3.text, '<root>\n  <a>\n    <b>1</b>\n  </a>\n</root>\n', show(r3.text));
});

/* ---------------------------- 11. token 序列不变自检 ---------------------------- */

test('token 序列不变自检（综合样例）', function () {
  const input =
    '<?xml version="1.0" encoding="utf-8"?>' +
    '<!DOCTYPE cfg [<!ENTITY e "v>1">]>' +
    '<!-- head -->' +
    '<配置 xmlns:x="urn:x"  x:id="1" >' +
    '<节点 a="1"> 中文 &amp; 字符 </节点>' +
    '<x:空/>' +
    '<脚本><![CDATA[  var a = 1 < 2;  ]]></脚本>' +
    '<列表><项>一</项><项>二</项></列表>' +
    '</配置>';
  const r = xml.format(input, { attrWrap: 30 });
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assertSameTokens(input, r.text, '综合样例');
  assert.ok(!/\uFEFF/.test(r.text), '输出不应含 BOM');
  // 再格式化一次应为幂等（结构相同）
  const again = xml.format(r.text, { attrWrap: 30 });
  assert.strictEqual(again.ok, true);
  assert.strictEqual(again.text, r.text, '二次格式化应幂等');
});

/* ---------------------------- 12. BOM 输入与空输入 ---------------------------- */

test('输入带 BOM 时不报错，输出不带 BOM', function () {
  const r = xml.format('\uFEFF<?xml version="1.0"?><r><a/></r>');
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(r.text, '<?xml version="1.0"?>\n<r>\n  <a/>\n</r>\n', JSON.stringify(r.text));
  assert.ok(r.text.charCodeAt(0) !== 0xFEFF, '输出不应带 BOM');
});

test('空输入与纯空白输入返回空字符串', function () {
  for (const v of ['', '   ', '\n\n', null, undefined]) {
    const r = xml.format(v);
    assert.strictEqual(r.ok, true, JSON.stringify(v) + ' -> ' + JSON.stringify(r));
    assert.strictEqual(r.text, '');
  }
});

test('异常输入统一返回对象而不抛异常', function () {
  const r = xml.format({ toString() { throw new Error('boom'); } });
  assert.strictEqual(typeof r, 'object');
  assert.ok(r.ok === false || r.ok === true, JSON.stringify(r));
  if (r.ok === false) console.log('       错误信息: ' + r.error);
});

/* -------------------------------- 汇总 -------------------------------- */

console.log('');
if (failures.length) {
  console.log('失败 ' + failures.length + ' 项，通过 ' + passed + ' 项');
  process.exit(1);
}
console.log('全部通过：' + passed + ' 项断言用例');
process.exit(0);
