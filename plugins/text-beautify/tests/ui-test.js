'use strict';

// ui.js 的离线冒烟测试：用一个极小的假 DOM 跑通 UI 的初始化与美化路径。
// 目的不是替代 GUI 复测，而是抓住"元素 id 写错、初始化就抛异常、点美化没反应"这类问题。

const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

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

// ---------- 假 DOM ----------

function makeClassList(el) {
  return {
    add(c) { el._classes.add(c); },
    remove(c) { el._classes.delete(c); },
    contains(c) { return el._classes.has(c); }
  };
}

function makeEl(tag, id) {
  const el = {
    tagName: String(tag || 'div').toUpperCase(),
    id: id || '',
    value: '',
    textContent: '',
    hidden: false,
    checked: false,
    className: '',
    type: '',
    min: '',
    max: '',
    step: '',
    placeholder: '',
    scrollTop: 0,
    scrollLeft: 0,
    selectionStart: 0,
    selectionEnd: 0,
    _attrs: {},
    _classes: new Set(),
    _children: [],
    _listeners: {},
    _html: '',
    classList: null,
    getAttribute(n) { return Object.prototype.hasOwnProperty.call(this._attrs, n) ? this._attrs[n] : null; },
    setAttribute(n, v) { this._attrs[n] = String(v); },
    appendChild(c) { c._parent = this; this._children.push(c); return c; },
    addEventListener(t, fn) { (this._listeners[t] = this._listeners[t] || []).push(fn); },
    removeEventListener() {},
    dispatch(t, ev) {
      // 真实 DOM 会冒泡：ui.js 用容器上的事件委托处理类型切换，这里必须模拟冒泡
      const payload = Object.assign({ target: this, preventDefault() {}, key: '', ctrlKey: false, metaKey: false, altKey: false }, ev || {});
      let node = this;
      while (node) {
        const list = node._listeners[t] || [];
        for (const fn of list) fn(payload);
        node = node._parent || null;
      }
    },
    focus() { global.__focused = this; },
    select() { this.selected = true; },
    querySelectorAll(sel) {
      // ui.js 只用 .seg-item 这一种选择器
      if (sel !== '.seg-item') return [];
      return this._children.filter((c) => (c.className || '').indexOf('seg-item') >= 0);
    },
    set innerHTML(v) { this._html = v; if (v === '') this._children = []; },
    get innerHTML() { return this._html; }
  };
  el.classList = makeClassList(el);
  return el;
}

function buildDoc(ids) {
  const map = {};
  for (const id of ids) {
    map[id] = makeEl(id === 'src' || id === 'out' ? 'textarea' : 'div', id);
  }
  // types 容器里放四个类型按钮，和 index.html 一致
  const types = map.types;
  for (const t of ['json', 'xml', 'sql', 'js']) {
    const b = makeEl('button');
    b.className = 'seg-item' + (t === 'json' ? ' on' : '');
    b.setAttribute('data-type', t);
    types.appendChild(b);
  }
  const doc = {
    _els: map,
    _created: [],
    getElementById(id) { return map[id] || null; },
    createElement(tag) { const e = makeEl(tag); doc._created.push(e); return e; },
    execCommand() { return true; },
    _listeners: {},
    addEventListener(t, fn) { (doc._listeners[t] = doc._listeners[t] || []).push(fn); }
  };
  return doc;
}

// ---------- 环境 ----------

const REQUIRED_IDS = ['src', 'out', 'srcMeta', 'outMeta', 'status', 'hint', 'banner', 'toast', 'opts', 'types', 'indent', 'beautify', 'copy', 'paste', 'clear'];

// 校验 index.html 里确实有 ui.js 需要的所有 id
const html = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');
for (const id of REQUIRED_IDS) {
  ok('index.html 含 #' + id, html.indexOf('id="' + id + '"') >= 0);
}

const doc = buildDoc(REQUIRED_IDS);
doc._els.indent.value = '  ';

const messages = [];
const eftoStub = {
  ipcMessage(name, data) { messages.push({ name, data }); return Promise.resolve({ ok: true }); },
  onMessage(name, cb) { (eftoStub._handlers[name] = eftoStub._handlers[name] || []).push(cb); },
  _handlers: {},
  copyText(text) { eftoStub._copied = text; return Promise.resolve(); }
};

const sandbox = {
  document: doc,
  window: { performance: { now: () => Date.now() } },
  console,
  setTimeout,
  clearTimeout,
  Promise,
  Object,
  Array,
  String,
  Number,
  Math,
  isFinite,
  JSON,
  efto: eftoStub
};
sandbox.globalThis = sandbox;
sandbox.window.document = doc;
vm.createContext(sandbox);

// 按 index.html 的顺序加载四个格式化器 + ui.js
for (const f of ['lib/fmt-json.js', 'lib/fmt-xml.js', 'lib/fmt-sql.js', 'lib/fmt-js.js']) {
  const p = path.join(__dirname, '..', f);
  if (!fs.existsSync(p)) {
    console.log('skip ' + f + '（尚未生成）');
    continue;
  }
  vm.runInContext(fs.readFileSync(p, 'utf8'), sandbox, { filename: f });
}

vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'ui.js'), 'utf8'), sandbox, { filename: 'ui.js' });
ok('ui.js 初始化不抛异常', true);

// 宿主 SDK 不存在时（例如直接在浏览器里打开页面）也不能整个崩掉：本地美化必须照常可用
{
  const doc2 = buildDoc(REQUIRED_IDS);
  doc2._els.indent.value = '  ';
  const sb2 = {
    document: doc2,
    window: { performance: { now: () => Date.now() } },
    console,
    setTimeout: () => 0,
    clearTimeout: () => {},
    Promise,
    Object,
    Array,
    String,
    Number,
    Math,
    isFinite,
    JSON
  };
  sb2.globalThis = sb2;
  sb2.window.document = doc2;
  vm.createContext(sb2);
  for (const f of ['lib/fmt-json.js', 'lib/fmt-xml.js', 'lib/fmt-sql.js', 'lib/fmt-js.js']) {
    const p = path.join(__dirname, '..', f);
    if (fs.existsSync(p)) vm.runInContext(fs.readFileSync(p, 'utf8'), sb2, { filename: f });
  }
  let threw = null;
  try {
    vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'ui.js'), 'utf8'), sb2, { filename: 'ui.js' });
  } catch (e) {
    threw = e;
  }
  ok('无宿主 SDK 时初始化不抛异常', threw === null, threw && threw.message);
  doc2._els.src.value = '{"z":9}';
  doc2._els.beautify.dispatch('click');
  ok('无宿主 SDK 时仍可本地美化', doc2._els.out.value.indexOf('"z": 9') >= 0, doc2._els.out.value);
}

ok('初始化时向 main 发了 init', messages.some((m) => m.name === 'init'));
ok('默认类型按钮为高亮', doc._els.types._children[0].className.indexOf('on') >= 0);

// 默认选项区按 JSON 渲染出一个复选框
ok('JSON 选项已渲染', doc._els.opts._children.length === 1, 'children=' + doc._els.opts._children.length);
ok('选项文案正确', doc._els.opts._children[0]._children.some((c) => c.textContent === '标量短数组保持单行'));

// ---------- 跑一次美化 ----------

const src = doc._els.src;
const out = doc._els.out;

src.value = '{"a":1,"b":[1,2,3]}';
src.dispatch('input');
// 输入后是防抖，直接点按钮走同步路径
doc._els.beautify.dispatch('click');

ok('JSON 美化有输出', out.value.indexOf('"a": 1,') >= 0, out.value);
ok('美化后带缩进', out.value.split('\n')[1] === '  "a": 1,', JSON.stringify(out.value));
ok('状态栏显示已美化', doc._els.status.textContent.indexOf('已美化') >= 0, doc._els.status.textContent);
ok('结果字符数已更新', doc._els.outMeta.textContent.indexOf('字符') >= 0, doc._els.outMeta.textContent);

// ---------- 出错路径 ----------

doc._els.src.value = "{'a':1}";
doc._els.beautify.dispatch('click');
ok('非法 JSON 显示错误横幅', doc._els.banner.hidden === false && doc._els.banner.textContent.indexOf('双引号') >= 0, doc._els.banner.textContent);
ok('出错时结果框清空', out.value === '');
ok('出错时状态为失败', doc._els.status.textContent === '美化失败', doc._els.status.textContent);

// ---------- 复制 ----------

doc._els.src.value = '{"a":1}';
doc._els.beautify.dispatch('click');
doc._els.copy.dispatch('click');
setTimeout(() => {
  ok('复制把结果交给宿主', eftoStub._copied === out.value, JSON.stringify(eftoStub._copied));

  // ---------- 切类型 ----------

  doc._els.types._children[2].dispatch('click'); // SQL
  const sqlFmt = sandbox.TBFormat && sandbox.TBFormat.sql;
  if (sqlFmt) {
    doc._els.src.value = 'select a,b from t where a=1 and b=2';
    doc._els.beautify.dispatch('click');
    ok('SQL 美化有输出', out.value.indexOf('\n') > 0 && /SELECT/i.test(out.value), out.value);
  } else {
    console.log('skip SQL 用例（格式化器缺失）');
  }
  ok('切到 SQL 后状态栏更新', doc._els.status.textContent.indexOf('SQL') >= 0 || doc._els.status.textContent.indexOf('美化') >= 0, doc._els.status.textContent);

  doc._els.types._children[1].dispatch('click'); // XML
  const xmlFmt = sandbox.TBFormat && sandbox.TBFormat.xml;
  if (xmlFmt) {
    doc._els.src.value = '<a><b>1</b></a>';
    doc._els.beautify.dispatch('click');
    ok('XML 美化有输出', out.value.indexOf('<b>1</b>') >= 0, out.value);
  } else {
    console.log('skip XML 用例（格式化器缺失）');
  }

  doc._els.types._children[3].dispatch('click'); // JS
  const jsFmt = sandbox.TBFormat && sandbox.TBFormat.js;
  if (jsFmt) {
    doc._els.src.value = 'function f(a,b){if(a){return b}else{return a}}';
    doc._els.beautify.dispatch('click');
    ok('JS 美化有输出', out.value.indexOf('\n') > 0, out.value);
  } else {
    console.log('skip JS 用例（格式化器缺失）');
  }

  // ---------- main 推来的消息 ----------

  const seedHandlers = eftoStub._handlers.seed || [];
  if (seedHandlers.length) {
    doc._els.src.value = '';
    seedHandlers[0]({ text: '{"x":1}' });
    ok('seed 预填并直接美化', doc._els.src.value === '{"x":1}' && out.value.indexOf('"x": 1') >= 0, out.value);
  }

  const pastedHandlers = eftoStub._handlers.pasted || [];
  if (pastedHandlers.length) {
    pastedHandlers[0]({ text: '', error: 'cli 模式不提供 pasteText' });
    ok('粘贴失败如实报错', doc._els.banner.textContent.indexOf('cli 模式') >= 0, doc._els.banner.textContent);
  }

  // ---------- 清空 ----------

  doc._els.clear.dispatch('click');
  ok('清空后两侧都空', doc._els.src.value === '' && out.value === '' && doc._els.outMeta.textContent === '');

  console.log('\n通过 ' + pass + ' 项，退出码 ' + (process.exitCode || 0));
}, 0);
