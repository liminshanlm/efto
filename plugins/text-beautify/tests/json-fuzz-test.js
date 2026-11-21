'use strict';

// JSON 属性测试：随机生成合法 JSON，美化后必须
//   1) 能被 JSON.parse 解析
//   2) 解析出来的值与原始对象深度相等（美化不能改数据）
//   3) 再美化一次结果不变（幂等）
// 另外对随机合法 JSON 注入注释/尾随逗号，宽容路径同样要满足上面三条。

const assert = require('assert');
const json = require('../lib/fmt-json.js');

let seed = 20261117;
function rnd() {
  // xorshift32：固定种子，失败可复现
  seed ^= seed << 13; seed >>>= 0;
  seed ^= seed >>> 17;
  seed ^= seed << 5; seed >>>= 0;
  return seed / 0x100000000;
}
function pick(list) { return list[Math.floor(rnd() * list.length) % list.length]; }

function randValue(depth) {
  const r = rnd();
  if (depth <= 0 || r < 0.35) {
    const k = Math.floor(rnd() * 7);
    switch (k) {
      case 0: return null;
      case 1: return true;
      case 2: return false;
      case 3: return Math.floor(rnd() * 2000) - 1000;
      case 4: return Math.round(rnd() * 100000) / 1000;
      case 5: return pick(['', 'a', '中文', 'quote"in', 'back\\slash', 'line\nbreak', 'tab\there', '🎯']);
      default: return 'x'.repeat(Math.floor(rnd() * 12));
    }
  }
  if (r < 0.68) {
    const n = Math.floor(rnd() * 4);
    const arr = [];
    for (let i = 0; i < n; i++) arr.push(randValue(depth - 1));
    return arr;
  }
  const n = Math.floor(rnd() * 4);
  const obj = {};
  for (let i = 0; i < n; i++) obj['k' + i + '_' + Math.floor(rnd() * 100)] = randValue(depth - 1);
  return obj;
}

function injectNoise(text) {
  // 在 token 之间随机插入注释与尾随逗号（只加在对象/数组末尾的逗号位置）
  let out = '';
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      out += c;
      i++;
      while (i < text.length) {
        out += text[i];
        if (text[i] === '\\') { i++; out += text[i]; }
        else if (text[i] === '"') break;
        i++;
      }
      continue;
    }
    if (c === '{' || c === '[') depth++;
    if (c === '}' || c === ']') {
      depth--;
      // 把 "[1,2]" 变成 "[1,2,]"（合法 JSON 里 ] 前一定是值或空，空容器不加）
      if (rnd() < 0.5 && out.length && /[\]}"0-9a-z]/.test(out[out.length - 1]) && out[out.length - 1] !== '[' && out[out.length - 1] !== '{') {
        out += ',';
      }
    }
    out += c;
    if (c === ',' && rnd() < 0.3) out += ' /* c */ ';
  }
  return out;
}

let pass = 0;
function ok(name, cond, extra) {
  if (!cond) {
    console.error('FAIL: ' + name + (extra ? '\n' + extra : ''));
    process.exitCode = 1;
  } else {
    pass++;
  }
}

const N = 400;
for (let i = 0; i < N; i++) {
  const value = randValue(4);
  const compact = JSON.stringify(value);

  const r = json.format(compact, {});
  if (!r.ok) {
    ok('随机样例 #' + i + ' 应美化成功', false, compact + '\n错误：' + r.error);
    continue;
  }

  let parsed;
  let parseErr = null;
  try { parsed = JSON.parse(r.text); } catch (e) { parseErr = e; }
  ok('#' + i + ' 输出可被 JSON.parse', parseErr === null, r.text + '\n' + (parseErr && parseErr.message));
  if (parseErr) continue;

  ok('#' + i + ' 值等价', JSON.stringify(parsed) === compact, compact + '\n---\n' + r.text);

  const again = json.format(r.text, {});
  ok('#' + i + ' 幂等', again.ok && again.text === r.text, r.text + '\n====\n' + (again.ok ? again.text : again.error));

  // 带注释与尾随逗号的宽容路径
  const noisy = injectNoise(compact);
  const r2 = json.format(noisy, {});
  if (!r2.ok) {
    ok('#' + i + ' 噪声输入应美化成功', false, noisy + '\n错误：' + r2.error);
    continue;
  }
  let parsed2 = null;
  let err2 = null;
  try { parsed2 = JSON.parse(r2.text); } catch (e) { err2 = e; }
  ok('#' + i + ' 噪声输出可解析', err2 === null, noisy + '\n---\n' + r2.text);
  if (err2) continue;
  ok('#' + i + ' 噪声值等价', JSON.stringify(parsed2) === compact, noisy + '\n---\n' + r2.text);
}

// 缩进选项不影响可解析性与等价性
for (const indent of ['  ', '    ', '\t']) {
  const r = json.format('{"a":[1,{"b":2}],"c":"中"}', { indent });
  ok('indent=' + JSON.stringify(indent) + ' 可解析', r.ok && JSON.stringify(JSON.parse(r.text)) === '{"a":[1,{"b":2}],"c":"中"}', r.text);
}

console.log('JSON 属性测试通过 ' + pass + ' 项（' + N + ' 轮随机样例）');
