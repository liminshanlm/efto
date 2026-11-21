'use strict';

// main.js 的离线测试：用假的 efto 全局跑后端逻辑。
// 覆盖：进入插件时开窗口、种子文本的推送（含窗口复用与页面刚加载两种情况）、
// 剪贴板失败要如实回报。

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

function makeHost(opts) {
  const host = {
    platform: 'win32',
    version: '0.1.0',
    _events: {},
    _sent: [],
    _ui: [],
    _paste: (opts && opts.paste) || null,
    onPluginEnter(cb) { this._events.enter = cb; },
    onMainMessage(cb) { this._events.main = cb; },
    onUiClosed(cb) { this._events.closed = cb; },
    showUI(o) { this._ui.push(o); return Promise.resolve({ ok: true }); },
    ipcMessage(name, data) { this._sent.push({ name, data }); return Promise.resolve(); },
    pasteText() {
      if (this._paste === 'throw') throw new Error('cli 模式不提供 pasteText');
      return this._paste === undefined ? '' : this._paste;
    }
  };
  return host;
}

function load(host) {
  const sandbox = { efto: host, console, String, Object, Array, JSON, Error };
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'main.js'), 'utf8'), sandbox, { filename: 'main.js' });
  return sandbox;
}

function enter(host, payload) {
  host._events.enter({ code: '美化', type: 'input', payload: payload === undefined ? '' : payload, msg: null });
}

// 1. 无 payload：开窗口，不推种子
{
  const host = makeHost();
  load(host);
  enter(host, '');
  ok('进入时打开 UI 窗口', host._ui.length === 1 && host._ui[0].title === '文本美化');
  ok('窗口尺寸来自清单式常量', host._ui[0].width === 1120 && host._ui[0].height === 720);
  ok('没有 payload 时不推种子', host._sent.filter((m) => m.name === 'seed').length === 0, JSON.stringify(host._sent));
}

// 2. 带 payload：立即推一次（窗口复用时页面不会发 init）
{
  const host = makeHost();
  load(host);
  enter(host, '{"a":1}');
  const seeds = host._sent.filter((m) => m.name === 'seed');
  ok('payload 立即推给页面', seeds.length === 1 && seeds[0].data.text === '{"a":1}', JSON.stringify(host._sent));
}

// 3. 新页面加载后握手：不重复推同一次种子，但会推 ready
{
  const host = makeHost();
  load(host);
  enter(host, '{"a":1}');
  host._events.main({ name: 'init', data: {} });
  const seeds = host._sent.filter((m) => m.name === 'seed');
  ok('握手后不重复推种子', seeds.length === 1, JSON.stringify(host._sent));
  ok('握手后推 ready', host._sent.some((m) => m.name === 'ready'));
}

// 4. 再进一次带新 payload：要推新种子
{
  const host = makeHost();
  load(host);
  enter(host, '{"a":1}');
  enter(host, '{"b":2}');
  const seeds = host._sent.filter((m) => m.name === 'seed');
  ok('二次进入推新种子', seeds.length === 2 && seeds[1].data.text === '{"b":2}', JSON.stringify(seeds));
}

// 5. 空白 payload 视为没有 payload
{
  const host = makeHost();
  load(host);
  enter(host, '   \n  ');
  ok('纯空白 payload 不推种子', host._sent.filter((m) => m.name === 'seed').length === 0, JSON.stringify(host._sent));
}

// 6. 关窗后清空种子
{
  const host = makeHost();
  load(host);
  enter(host, '{"a":1}');
  host._events.closed();
  host._sent.length = 0;
  enter(host, '');
  host._events.main({ name: 'init', data: {} });
  ok('关窗后不再推送旧种子', host._sent.filter((m) => m.name === 'seed').length === 0, JSON.stringify(host._sent));
}

// 7. 剪贴板：正常返回文本
{
  const host = makeHost({ paste: '{"x":1}' });
  load(host);
  enter(host, '');
  host._events.main({ name: 'paste', data: {} });
  const m = host._sent.find((x) => x.name === 'pasted');
  ok('粘贴成功回传文本', m && m.data.text === '{"x":1}' && !m.data.error, JSON.stringify(m));
}

// 8. 剪贴板：宿主拒绝时 error 必须透传，不能伪装成空剪贴板
{
  const host = makeHost({ paste: 'throw' });
  load(host);
  enter(host, '');
  host._events.main({ name: 'paste', data: {} });
  const m = host._sent.find((x) => x.name === 'pasted');
  ok('粘贴失败带真实原因', m && m.data.text === '' && m.data.error === 'cli 模式不提供 pasteText', JSON.stringify(m));
}

// 9. 未知消息不抛异常也不乱推
{
  const host = makeHost();
  load(host);
  enter(host, '');
  const before = host._sent.length;
  let threw = null;
  try { host._events.main({ name: 'nope', data: null }); } catch (e) { threw = e; }
  ok('未知消息不抛异常', threw === null, threw && threw.message);
  ok('未知消息不产生推送', host._sent.length === before);
}

// 10. ipcMessage 抛异常时被吞掉（窗口正在销毁）
{
  const host = makeHost();
  host.ipcMessage = () => { throw new Error('ui gone'); };
  load(host);
  let threw = null;
  try { enter(host, '{"a":1}'); } catch (e) { threw = e; }
  ok('推送失败不影响进入插件', threw === null, threw && threw.message);
  ok('窗口照样打开', host._ui.length === 1);
}

console.log('\n通过 ' + pass + ' 项，退出码 ' + (process.exitCode || 0));
