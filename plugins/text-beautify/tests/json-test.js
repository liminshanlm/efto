'use strict';

const assert = require('assert');
const json = require('../lib/fmt-json');

let pass = 0;
function ok(name, cond, extra) {
  if (!cond) {
    console.error('FAIL: ' + name + (extra ? '\n' + extra : ''));
    process.exitCode = 1;
  } else {
    pass++;
    console.log('ok   ' + name);
  }
}

function fmt(text, opts) {
  return json.format(text, opts);
}

// 1. 紧凑 JSON 展开
let r = fmt('{"a":1,"b":[1,2,3],"c":{"d":true}}');
ok('紧凑 JSON ok', r.ok, r.error);
ok('key 缩进两空格', r.ok && r.text.split('\n')[1] === '  "a": 1,', r.text);
ok('对象与数组都展开', r.ok && r.text.split('\n').length === 7, r.text);
console.log('--- 样例1 ---\n' + r.text + '\n');

// 2. 数字/键顺序原样保留（1.0 不能变 1，大整数不能丢精度）
r = fmt('{"f":1.0,"big":123456789012345678901234567890,"e":1e5}');
ok('数字字面量原样', r.ok && r.text.indexOf('1.0') >= 0 && r.text.indexOf('123456789012345678901234567890') >= 0 && r.text.indexOf('1e5') >= 0, r.text);

// 3. 字符串转义原样
r = fmt('{"s":"a\\"b\\\\c\\u4e2d"}');
ok('字符串转义原样', r.ok && r.text.indexOf('"a\\"b\\\\c\\u4e2d"') >= 0, r.text);

// 4. 注释与尾随逗号宽容
r = fmt('{\n // note\n "a": [1,2,], /* x */ "b": 2,\n}');
ok('注释/尾随逗号宽容', r.ok, r.error);
ok('注释被丢弃', r.ok && r.text.indexOf('note') < 0 && r.text.indexOf('/*') < 0, r.text);

// 4b. 宽容不等于把坏 JSON 原样吐回去：尾随逗号必须被去掉，结果要能被 JSON.parse
r = fmt('{"a":1,"b":[1,2,],"c":3,}');
ok('对象尾随逗号被去掉', r.ok && JSON.stringify(JSON.parse(r.text)) === JSON.stringify({ a: 1, b: [1, 2], c: 3 }), r.text);
r = fmt('[1,2,3,]');
ok('数组尾随逗号被去掉且可解析', r.ok && JSON.stringify(JSON.parse(r.text)) === '[1,2,3]', r.text);
r = fmt('[[1,2,],{"x":1,},]');
ok('嵌套尾随逗号被去掉且可解析', r.ok && JSON.stringify(JSON.parse(r.text)) === '[[1,2],{"x":1}]', r.text);

// 4c. 连续逗号是真的语法错误，必须报错而不是输出 [1, 2]
r = fmt('[1,,2]');
ok('连续逗号报错', !r.ok, r.text);

// 5. 标量短数组保持单行
r = fmt('{"ids":[1,2,3,4,5]}');
ok('短标量数组单行', r.ok && r.text.indexOf('[1, 2, 3, 4, 5]') >= 0, r.text);

// 6. 嵌套数组按行展开
r = fmt('[[1,2],[3,4]]');
ok('嵌套数组展开', r.ok && r.text.split('\n').length === 4, r.text);

// 7. 根值是数组
r = fmt('[1,2]');
ok('根数组', r.ok && r.text === '[1, 2]', r.text);

// 8. 空对象/空数组
r = fmt('{"a":{},"b":[]}');
ok('空容器不拆行', r.ok && r.text.indexOf('"a": {}') >= 0 && r.text.indexOf('"b": []') >= 0, r.text);

// 9. 错误：单引号键
r = fmt("{'a':1}");
ok('单引号键报错', !r.ok && /双引号/.test(r.error), JSON.stringify(r));

// 10. 错误：缺冒号
r = fmt('{"a" 1}');
ok('缺冒号报错带位置', !r.ok && /第 1 行/.test(r.error) && /冒号/.test(r.error), JSON.stringify(r));

// 11. 错误：未闭合字符串
r = fmt('{"a":"x}');
ok('未闭合字符串报错', !r.ok && /字符串/.test(r.error), JSON.stringify(r));

// 12. 错误：根值后面多出内容
r = fmt('{"a":1} {"b":2}');
ok('多根值报错', !r.ok && /根值/.test(r.error), JSON.stringify(r));

// 13. 错误：未闭合对象
r = fmt('{"a":1');
ok('未闭合对象报错', !r.ok && /闭合/.test(r.error), JSON.stringify(r));

// 14. 空输入
r = fmt('   ');
ok('空输入报错', !r.ok, JSON.stringify(r));

// 15. 幂等性：格式化两次结果一致
const samples = [
  '{"a":1,"b":[1,2,3],"c":{"d":true,"e":[{"f":null}]}}',
  '[{"id":1,"tags":["a","b"]},{"id":2,"tags":[]}]',
  '{"nested":[[[1,2],[3]],{"x":[]}]}',
  '{"cn":"中文","emoji":"🎯","esc":"\\n\\t"}'
];
for (const s of samples) {
  const a = fmt(s);
  const b = a.ok ? fmt(a.text) : a;
  ok('幂等 ' + s.slice(0, 24), a.ok && b.ok && a.text === b.text, a.text + '\n----\n' + b.text);
}

// 16. 内容等价：去掉空白后 token 序列一致
function tokens(text) {
  const t = [];
  let i = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (/\s/.test(c)) { i++; continue; }
    if (c === '"') {
      const s = i; i++;
      while (i < n) {
        if (text[i] === '\\') { i += 2; continue; }
        if (text[i] === '"') { i++; break; }
        i++;
      }
      t.push(text.slice(s, i));
      continue;
    }
    t.push(c);
    i++;
  }
  return t;
}
for (const s of samples) {
  const a = fmt(s);
  ok('token 等价 ' + s.slice(0, 24), a.ok && tokens(s).join('|') === tokens(a.text).join('|'), a.text);
}

// 17. 深层嵌套不崩（1500 层应被安全拦下而不是栈溢出）
let deep = '';
for (let i = 0; i < 1500; i++) deep += '[';
deep += '1';
for (let i = 0; i < 1500; i++) deep += ']';
r = fmt(deep);
ok('深嵌套不抛异常', typeof r.ok === 'boolean', JSON.stringify(r).slice(0, 120));

// 18. 缩进与换行参数
r = fmt('{"a":1}', { indent: '\t', newline: '\r\n' });
ok('自定义缩进/换行', r.ok && r.text.indexOf('\r\n\t"a": 1') >= 0, JSON.stringify(r.text));

console.log('\n通过 ' + pass + ' 项，退出码 ' + (process.exitCode || 0));
