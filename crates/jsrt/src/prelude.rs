//! 注入到每个插件 isolate 的 JS 垫片：efto 插件 API 的 JS 侧实现。
//!
//! 与宿主的分工：
//! - 本文件只负责 **API 形状 + 参数校验 + 回调分发 + Promise 包装**；
//! - 真正的读写/网络/窗口动作全部通过 `__host_call(module, method, argsJson)` 落到 Rust 的 `Host` trait。
//!
//! main 入口支持两种挂钩子的写法：
//! 1. `exports.onXxx = fn`（CommonJS 模块导出）
//! 2. `efto.onPluginEnter(cb)` / `efto.onMainMessage(cb)`（注册式）

/// 预编译进 isolate 的垫片源码。
pub const PRELUDE: &str = r#"(function () {
  'use strict';

  var hostCall = globalThis.__host_call;         // (module, method, argsJson) -> resultJson | throws
  var hostCallAsync = globalThis.__host_call_async; // (id, module, method, argsJson) -> undefined
  if (typeof hostCall !== 'function') {
    throw new Error('efto: 宿主桥接未注入，请检查运行时');
  }

  // ---------- 基础工具 ----------
  // 参数直接以 JS 数组传给原生侧（rusty_v8 会转成 serde_json 数组），不再 JSON.stringify：
  // 之前 JS 侧 stringify、Rust 侧按数组解析，两边不一致导致所有能力调用只拿到"一个字符串参数"。
  function call(module, method, args) {
    var out = hostCall(module, method, args === undefined ? [] : args);
    if (out === null || out === undefined) return undefined;
    return JSON.parse(out);
  }

  var pending = new Map();
  var streamHandlers = new Map();
  var callSeq = 1;
  function callAsync(module, method, args, onDelta) {
    return new Promise(function (resolve, reject) {
      var id = callSeq++;
      pending.set(id, { resolve: resolve, reject: reject });
      if (typeof onDelta === 'function') streamHandlers.set(id, onDelta);
      try {
        hostCallAsync(id, module, method, args === undefined ? [] : args);
      } catch (e) {
        pending.delete(id);
        streamHandlers.delete(id);
        reject(e);
      }
    });
  }

  function isObj(v) { return v !== null && typeof v === 'object'; }
  function mustString(v, name) {
    if (typeof v !== 'string') throw new TypeError(name + ' 必须是字符串，收到 ' + typeof v);
    return v;
  }

  // ---------- console 转发 ----------
  var fmtParts = ['log', 'info', 'warn', 'error', 'debug', 'trace'];
  fmtParts.forEach(function (lvl) {
    console[lvl] = function () {
      var parts = [];
      for (var i = 0; i < arguments.length; i++) {
        var a = arguments[i];
        if (typeof a === 'string') parts.push(a);
        else {
          try { parts.push(JSON.stringify(a)); } catch (e) { parts.push(String(a)); }
        }
      }
      // 参数必须是 JS 数组（桥接按数组解析）；早先这里 stringify 成字符串，
      // 加上宿主没实现 log 能力，插件里的 console.* 一直是被静默丢弃的
      try { hostCall('log', lvl, [parts.join(' ')]); } catch (e) { /* 忽略日志失败 */ }
    };
  });

  // ---------- 定时器 ----------
  var timers = new Map();
  var timerSeq = 1;
  function arm(fn, delay, repeat) {
    if (typeof fn !== 'function') throw new TypeError('callback 必须是函数');
    var id = timerSeq++;
    var ms = Number(delay) || 0;
    timers.set(id, { fn: fn, repeat: repeat, ms: ms });
    call('timer', 'arm', [id, ms, !!repeat]);
    return id;
  }
  globalThis.setTimeout = function (fn, ms) { return arm(fn, ms, false); };
  globalThis.setInterval = function (fn, ms) { return arm(fn, ms, true); };
  globalThis.clearTimeout = function (id) {
    if (timers.delete(id)) call('timer', 'clear', [id]);
  };
  globalThis.clearInterval = globalThis.clearTimeout;

  // ---------- 钩子注册表 ----------
  var hooks = {
    enter: [], out: [], mainMessage: [], message: [], uiClosed: [],
    windowHide: [], windowShow: [], databaseChange: []
  };
  function on(list, fn) {
    if (typeof fn !== 'function') throw new TypeError('回调必须是函数');
    list.push(fn);
    return fn;
  }
  function off(list, fn) {
    for (var i = list.length - 1; i >= 0; i--) if (list[i] === fn) list.splice(i, 1);
  }
  function fire(list, arg) {
    for (var i = 0; i < list.length; i++) {
      try { list[i](arg); } catch (e) { reportError('hook', e); }
    }
  }
  function reportError(stage, e) {
    var msg = (e && (e.message || String(e))) || String(e);
    var stack = e && e.stack ? String(e.stack) : null;
    try { hostCall('report', 'error', JSON.stringify([stage, msg, stack])); } catch (x) {}
  }

  // ---------- db: 旧版 KV ----------
  var dbLegacy = {
    get: function (key) { return call('db', 'get', [mustString(key, 'key')]); },
    set: function (key, value) { return call('db', 'set', [mustString(key, 'key'), value]); },
    assign: function (key, value) { return dbLegacy.set(key, value); },
    remove: function (key) { return call('db', 'remove', [mustString(key, 'key')]); },
    unset: function (key) { return dbLegacy.remove(key); },
    all: function () { return call('db', 'all', []); },
    keys: function () { return call('db', 'keys', []); },
    size: function () { return call('db', 'size', []); },
    read: function (key, cb) { var v = dbLegacy.get(key); if (cb) cb(v); return v; },
    write: function (key, value, cb) { var r = dbLegacy.set(key, value); if (cb) cb(r); return r; }
  };

  // ---------- db: 新版文档接口（在 KV 快照之上实现） ----------
  function snapshot() { return call('db', 'all', []) || {}; }
  function bumpRev(oldRev) {
    var n = 1;
    if (typeof oldRev === 'string') { var m = /^(\d+)-/.exec(oldRev); if (m) n = parseInt(m[1], 10) + 1; }
    return n + '-' + Math.random().toString(36).slice(2, 10);
  }
  var dbDocs = {
    put: function (doc) {
      if (!isObj(doc) || typeof doc._id !== 'string' || !doc._id) throw new TypeError('doc._id 必填');
      var cur = call('db', 'get', [doc._id]);
      var next = Object.assign({}, doc);
      next._rev = bumpRev(cur && cur._rev);
      call('db', 'set', [doc._id, next]);
      var res = { ok: true, id: doc._id, rev: next._rev };
      emitChange({ id: doc._id, doc: next, type: cur ? 'update' : 'create' });
      return res;
    },
    bulkPut: function (docs) {
      var out = [];
      (docs || []).forEach(function (d) { out.push(dbDocs.put(d)); });
      return out;
    },
    get: function (id) { return call('db', 'get', [mustString(id, 'id')]) || null; },
    bulkGet: function (ids) {
      return (ids || []).map(function (id) { return dbDocs.get(id); });
    },
    remove: function (docOrId) {
      var id = typeof docOrId === 'string' ? docOrId : (docOrId && docOrId._id);
      if (!id) throw new TypeError('缺少 _id');
      var ok = call('db', 'remove', [id]);
      emitChange({ id: id, doc: null, type: 'delete' });
      return { ok: !!ok, id: id };
    },
    allDocs: function (opts) {
      opts = opts || {};
      var all = snapshot();
      var keys = Object.keys(all).filter(function (k) { return k.indexOf('_design/') !== 0; });
      if (opts.prefix) keys = keys.filter(function (k) { return k.indexOf(opts.prefix) === 0; });
      if (opts.startkey) keys = keys.filter(function (k) { return k >= opts.startkey; });
      if (opts.endkey) keys = keys.filter(function (k) { return k <= opts.endkey; });
      if (opts.descending) keys.reverse();
      keys.sort();
      if (opts.skip) keys = keys.slice(opts.skip);
      if (opts.limit) keys = keys.slice(0, opts.limit);
      var rows = keys.map(function (k) {
        var row = { id: k, key: k, value: { rev: all[k] && all[k]._rev } };
        if (opts.include_docs !== false) row.doc = all[k];
        return row;
      });
      return { total_rows: rows.length, offset: opts.skip || 0, rows: rows };
    },
    info: function () {
      var all = snapshot();
      return { db_name: 'efto', doc_count: Object.keys(all).length, update_seq: 0 };
    },
    changes: function (opts) {
      // 只提供一次性快照式变更读取 + 本地 changeEmitter，够插件用
      var since = (opts && typeof opts.since === 'number') ? opts.since : 0;
      var all = snapshot();
      var results = Object.keys(all).map(function (k) {
        return { id: k, seq: 1, changes: [{ rev: all[k] && all[k]._rev }], doc: all[k] };
      }).filter(function (r) { return r.seq > since; });
      return { last_seq: results.length, results: results };
    }
  };

  var changeEmitter = {
    listeners: [],
    onChange: function (fn) { this.listeners.push(fn); return this; },
    removeListener: function (fn) { off(this.listeners, fn); return this; },
    start: function () { return this; },
    stop: function () { return this; }
  };
  function emitChange(evt) {
    changeEmitter.listeners.slice().forEach(function (fn) {
      try { fn(evt); } catch (e) { reportError('changeEmitter', e); }
    });
    fire(hooks.databaseChange, evt);
  }
  dbDocs.changeEmitter = function () { return changeEmitter; };

  // ---------- efto 扩展 ----------
  // 受限外部程序执行。宿主侧只认白名单（目前仅 es），且必须通过内置 sha256 校验；
  // 参数数组原样交给 CreateProcess，不经过 shell。传 path/command/shell/env 会被拒绝。

  /**
   * 用设置里配置的解释器执行一段脚本源码（node -e / python -c）。
   * 解释器路径只能来自设置 → 运行时；插件传 path/interpreter 会被宿主拒绝。
   * 返回 Promise<{ runtime, ok, code, stdout, stderr, path, truncated }>。
   */
  function runScript(kind, opts) {
    if (!isObj(opts)) throw new TypeError('efto.' + kind + '.run 需要 { code, args?, timeoutMs? }');
    if (typeof opts.code !== 'string') throw new TypeError('efto.' + kind + '.run.code 必须是字符串');
    if (opts.args !== undefined && !Array.isArray(opts.args)) {
      throw new TypeError('efto.' + kind + '.run.args 必须是字符串数组');
    }
    if (opts.args) {
      for (var i = 0; i < opts.args.length; i++) {
        if (typeof opts.args[i] !== 'string') {
          throw new TypeError('efto.' + kind + '.run.args[' + i + '] 不是字符串');
        }
      }
    }
    if (opts.timeoutMs !== undefined && typeof opts.timeoutMs !== 'number') {
      throw new TypeError('efto.' + kind + '.run.timeoutMs 必须是数字');
    }
    var o = { runtime: kind, code: opts.code };
    if (opts.args) o.args = opts.args.slice();
    if (opts.timeoutMs !== undefined) o.timeoutMs = opts.timeoutMs;
    return callAsync('runtime', 'run', [o]);
  }

  /**
   * 大模型（OpenAI 兼容 chat/completions）。
   * chat({ messages, model?, temperature?, maxTokens?, timeoutSecs?, stream?, onDelta?, webSearch?, readUrl? })
   *   -> Promise<{ content, reasoning, model, modelId, modelName, finishReason, usage, serverToolCalls, annotations }>
   * 不传 model 时用 设置 → 模型 里的默认模型；base_url / api_key 只能来自设置，
   * 插件传 path / base_url / api_key 之类字段不会被接受（宿主侧也不会读）。
   * messages[].content 可以是字符串，也可以是内容数组：
   *   [{ type: 'text', text: '...' }, { type: 'image', path: 'D:\\a.png' }]
   * 图片由宿主读盘并转成 data URL，插件拿不到文件字节。
   * 传 onDelta（或 stream: true）时走 SSE 流式接口：onDelta 收到 { text?, reasoning?, toolEvent? } 增量，
   * 最终仍以 resolve 返回完整结果。
   * webSearch / readUrl 声明 DeepSeek 的服务端联网工具（搜索 / 抓取链接），
   * 仅 DeepSeek 端点支持；需要模型名是 deepseek-flash。
   * models() 同步返回 { default, models: [{ id, name, model, base_url, has_api_key }] }（不含密钥）。
   * ocr({ path, purpose?, model? }) -> Promise<{ id, pageCount, fileName, pages: [{ index, text, finishReason }] }>
   * 调用 DeepSeek 的 /v1/tools/ocr 识别本机文件；purpose 默认 text_recognition。
   */
  function normalizeAiContent(content, mi) {
    if (typeof content === 'string') {
      if (!content.trim()) throw new TypeError('efto.ai.chat.messages[' + mi + '].content 不能为空');
      return content;
    }
    if (!Array.isArray(content) || content.length === 0) {
      throw new TypeError('efto.ai.chat.messages[' + mi + '].content 必须是字符串或内容数组');
    }
    var out = [];
    for (var j = 0; j < content.length; j++) {
      var part = content[j];
      if (!isObj(part) || typeof part.type !== 'string') {
        throw new TypeError('efto.ai.chat.messages[' + mi + '].content[' + j + '] 需要 { type, ... }');
      }
      if (part.type === 'text') {
        if (typeof part.text !== 'string' || !part.text.trim()) {
          throw new TypeError('efto.ai.chat.messages[' + mi + '].content[' + j + '].text 不能为空');
        }
        out.push({ type: 'text', text: part.text });
      } else if (part.type === 'image') {
        if (typeof part.path !== 'string' || !part.path) {
          throw new TypeError('efto.ai.chat.messages[' + mi + '].content[' + j + '].path 必须是非空字符串');
        }
        out.push({ type: 'image', path: part.path });
      } else {
        throw new TypeError('efto.ai.chat.messages[' + mi + '].content[' + j + '] 不支持的类型 ' + part.type + '（只支持 text / image）');
      }
    }
    return out;
  }

  var ai = {
    chat: function (opts) {
      if (!isObj(opts)) throw new TypeError('efto.ai.chat 需要 { messages, model?, temperature?, maxTokens?, timeoutSecs?, stream?, onDelta? }');
      if (!Array.isArray(opts.messages) || opts.messages.length === 0) {
        throw new TypeError('efto.ai.chat.messages 必须是非空数组');
      }
      if (opts.stream !== undefined && typeof opts.stream !== 'boolean') {
        throw new TypeError('efto.ai.chat.stream 必须是布尔值');
      }
      if (opts.onDelta !== undefined && typeof opts.onDelta !== 'function') {
        throw new TypeError('efto.ai.chat.onDelta 必须是函数');
      }
      if (opts.webSearch !== undefined && typeof opts.webSearch !== 'boolean') {
        throw new TypeError('efto.ai.chat.webSearch 必须是布尔值');
      }
      if (opts.readUrl !== undefined && typeof opts.readUrl !== 'boolean') {
        throw new TypeError('efto.ai.chat.readUrl 必须是布尔值');
      }
      var msgs = [];
      for (var i = 0; i < opts.messages.length; i++) {
        var m = opts.messages[i];
        if (!isObj(m) || typeof m.role !== 'string') {
          throw new TypeError('efto.ai.chat.messages[' + i + '] 需要 { role, content }');
        }
        msgs.push({ role: m.role, content: normalizeAiContent(m.content, i) });
      }
      var streaming = opts.stream === true || typeof opts.onDelta === 'function';
      var o = { messages: msgs, stream: streaming };
      if (opts.model !== undefined) o.model = String(opts.model);
      if (opts.temperature !== undefined) {
        if (typeof opts.temperature !== 'number') throw new TypeError('efto.ai.chat.temperature 必须是数字');
        o.temperature = opts.temperature;
      }
      if (opts.maxTokens !== undefined) {
        if (typeof opts.maxTokens !== 'number') throw new TypeError('efto.ai.chat.maxTokens 必须是数字');
        o.maxTokens = opts.maxTokens;
      }
      if (opts.timeoutSecs !== undefined) {
        if (typeof opts.timeoutSecs !== 'number') throw new TypeError('efto.ai.chat.timeoutSecs 必须是数字');
        o.timeoutSecs = opts.timeoutSecs;
      }
      if (opts.webSearch === true) o.webSearch = true;
      if (opts.readUrl === true) o.readUrl = true;
      return callAsync('ai', 'chat', [o], opts.onDelta);
    },
    models: function () { return call('ai', 'models', []); },
    ocr: function (opts) {
      if (!isObj(opts)) throw new TypeError('efto.ai.ocr 需要 { path, purpose?, model? }');
      if (typeof opts.path !== 'string' || !opts.path.trim()) {
        throw new TypeError('efto.ai.ocr.path 必须是非空字符串');
      }
      if (opts.purpose !== undefined
          && opts.purpose !== 'text_recognition'
          && opts.purpose !== 'rich_layout_recognition') {
        throw new TypeError('efto.ai.ocr.purpose 只能是 text_recognition / rich_layout_recognition');
      }
      var o = { path: opts.path };
      if (opts.purpose !== undefined) o.purpose = opts.purpose;
      if (opts.model !== undefined) o.model = String(opts.model);
      return callAsync('ai', 'ocr', [o]);
    }
  };

  var ext = {
    exec: function (opts) {
      if (!isObj(opts)) throw new TypeError('efto.exec 需要选项对象 { name, args, timeoutMs }');
      if (typeof opts.name !== 'string' || !opts.name) {
        throw new TypeError('efto.exec.name 必须是非空字符串');
      }
      if (opts.args !== undefined && !Array.isArray(opts.args)) {
        throw new TypeError('efto.exec.args 必须是字符串数组');
      }
      if (opts.args) {
        for (var i = 0; i < opts.args.length; i++) {
          if (typeof opts.args[i] !== 'string') {
            throw new TypeError('efto.exec.args[' + i + '] 不是字符串');
          }
        }
      }
      return callAsync('efto', 'exec', [Object.assign({}, opts)]);
    },
    /**
     * 把结果内联到搜索框下方的列表。原生插件（没有 UI）用它反馈内容；
     * 调用即表示"本插件有输出"，宿主不会收起搜索框。
     * 行格式：{ title, subtitle?, copy? , open? }
     */
    output: function (rows) {
      if (!Array.isArray(rows)) {
        throw new TypeError('efto.output(rows) 需要数组，收到 ' + typeof rows);
      }
      if (rows.length > 50) {
        throw new TypeError('efto.output 单次最多 50 条');
      }
      return call('output', 'set', [rows]);
    },
    // main -> UI 页面。原生插件看不到 plugin.*，但 UI 通信仍是必需能力。
    ipcMessage: function (name, data) {
      return callAsync('ui', 'postMessage', [mustString(name, 'name'), data === undefined ? null : data]);
    },
    /** Node.js 脚本（需在 设置 → 运行时 配置 node 解释器）。 */
    node: { run: function (opts) { return runScript('node', opts); } },
    /** Python 脚本（需在 设置 → 运行时 配置 python 解释器）。 */
    python: { run: function (opts) { return runScript('python', opts); } },
    /**
     * Windows hosts 文件（需要管理员权限写入）。
     * read() 同步返回 { path, backup, text, encoding, bytes }；
     * save({ text, encoding }) 保存时会弹一次 UAC，返回 Promise<{ ok, path, backup, bytes }>。
     */
    hosts: {
      path: function () { return call('hosts', 'path', []); },
      read: function () { return call('hosts', 'read', []); },
      save: function (opts) {
        if (!isObj(opts)) throw new TypeError('efto.hosts.save 需要 { text, encoding? }');
        if (typeof opts.text !== 'string') throw new TypeError('efto.hosts.save.text 必须是字符串');
        var o = { text: opts.text };
        if (opts.encoding !== undefined) o.encoding = String(opts.encoding);
        return callAsync('hosts', 'save', [o]);
      }
    },
    /** 大模型（需在 设置 → 模型 配置并设为默认）。 */
    ai: ai
  };

  // ---------- window / UI ----------
  function callUi(method, a, b) {
    return callAsync('ui', method, b === undefined ? [a] : [a, b]);
  }
  var windowApi = {
    showUI: function (options) { return callUi('showUI', options || {}); },
    hideUI: function () { return callUi('hideUI', null); },
    showMainWindow: function () { return callUi('showMainWindow', null); },
    hideMainWindow: function () { return callUi('hideMainWindow', null); },
    setSubTitle: function (text) { return call('ui', 'setSubTitle', [String(text == null ? '' : text)]); },
    setVersionMap: function (m) { return call('ui', 'setVersionMap', [m]); },
    redirectTo: function (label, options) {
      return callAsync('redirect', 'to', [mustString(label, 'label'), options || {}]);
    },
    closeWindow: function (winName) { return call('ui', 'closeWindow', [winName || '']); },
    setFrameRate: function (v) { return call('ui', 'setFrameRate', [Number(v) || 60]); }
  };

  // ---------- 其余系统能力 ----------
  var system = {
    copyText: function (text) { return call('system', 'copyText', [mustString(text, 'text')]); },
    pasteText: function () { return call('system', 'pasteText', []); },
    pasteImage: function () { return call('system', 'pasteImage', []); },
    imagePreview: function (p) { return call('system', 'imagePreview', [mustString(p, 'path')]); },
    shellOpenPath: function (p) { return call('shell', 'openPath', [mustString(p, 'path')]); },
    shellShowItemInFolder: function (p) { return call('shell', 'showItemInFolder', [mustString(p, 'path')]); },
    openExternal: function (url) { return call('shell', 'openExternal', [mustString(url, 'url')]); },
    showNotification: function (text, sub) { return call('notify', 'show', [String(text == null ? '' : text), sub ? String(sub) : '']); },
    hideNotification: function () { return call('notify', 'hide', []); },
    createShortCut: function (o) { return callAsync('system', 'createShortcut', [o]); },
    removeShortCut: function (o) { return call('system', 'removeShortcut', [o]); },
    relaunch: function () { return call('system', 'relaunch', []); },
    quit: function () { return call('system', 'quit', []); },
    dbPath: function () { return call('system', 'dbPath', []); },
    isMainProcess: function () { return true; },
    onPredefinedShortcut: function () { return false; }
  };

  // ---------- 启动器搜索里的动态项 ----------
  var searchExt = {
    /**
     * 把插件自己的数据注册为启动器搜索项（动态项）。
     * items: [{ title, cmd }]，回车后 cmd 会作为 info.code 回传给 onPluginEnter。
     * 传空数组即注销；宿主会落盘，重启后仍能搜到。
     */
    setItems: function (items) {
      if (!Array.isArray(items)) throw new TypeError('efto.setSearchItems 需要数组 [{ title, cmd }]');
      var list = [];
      for (var i = 0; i < items.length; i++) {
        var it = items[i];
        if (!isObj(it)) throw new TypeError('efto.setSearchItems 的每一项需要 { title, cmd }');
        var title = String(it.title == null ? '' : it.title).trim();
        var cmd = String(it.cmd == null ? '' : it.cmd).trim();
        if (!title || !cmd) throw new TypeError('efto.setSearchItems 的 title/cmd 不能为空');
        list.push({ title: title, cmd: cmd });
      }
      return call('search', 'setItems', [list]);
    }
  };

  // ---------- 组装 efto 全局 ----------
  var eftoApi = {
    // 常量
    platform: 'win32',
    version: globalThis.__HOST_VERSION__ || '0.1.0',
    locale: globalThis.__LOCALE__ || 'zh-CN',
    dbPath: globalThis.__DB_PATH__ || '',
    oneId: '',
    isDarkMode: function () { return globalThis.__THEME__ === 'dark'; },
    isLightMode: function () { return globalThis.__THEME__ !== 'dark'; },
    isWindows: function () { return true; },
    isMacOS: function () { return false; },
    isLinux: function () { return false; },

    db: dbDocs,
    kv: dbLegacy,

    onPluginEnter: function (fn) { return on(hooks.enter, fn); },
    onPluginOut: function (fn) { return on(hooks.out, fn); },
    onMainMessage: function (fn) { return on(hooks.mainMessage, fn); },
    onMessage: function (fn) { return on(hooks.message, fn); },
    onUiClosed: function (fn) { return on(hooks.uiClosed, fn); },
    onWindowHide: function (fn) { return on(hooks.windowHide, fn); },
    onWindowShow: function (fn) { return on(hooks.windowShow, fn); },
    onDatabaseChange: function (fn) { return on(hooks.databaseChange, fn); },
    removeListener: function (name, fn) {
      var list = hooks[name];
      if (!list) throw new Error('未知的事件名: ' + name);
      off(list, fn);
    },
    broadcast: function (channel, data) { return call('broadcast', 'send', [mustString(channel, 'channel'), data]); },
    sendToDefaultOutput: function (payload) { return call('broadcast', 'sendToDefaultOutput', [payload]); },
    sendToFocusWindow: function (text) { return call('system', 'sendToFocusWindow', [String(text || '')]); },

    redirect: windowApi.redirectTo,
    redirects: function (title, options) {
      if (isObj(title)) return windowApi.redirectTo(title.label || title.cmd, title);
      return windowApi.redirectTo(title, options);
    },
    showUI: windowApi.showUI,
    hideUI: windowApi.hideUI,
    hideMainWindow: windowApi.hideMainWindow,
    showMainWindow: windowApi.showMainWindow,
    setSubTitle: windowApi.setSubTitle,
    closeWindow: windowApi.closeWindow,

    copyText: system.copyText,
    pasteText: system.pasteText,
    pasteImage: system.pasteImage,
    imagePreview: system.imagePreview,
    shellOpenPath: system.shellOpenPath,
    shellShowItemInFolder: system.shellShowItemInFolder,
    openExternal: system.openExternal,
    showNotification: system.showNotification,
    hideNotification: system.hideNotification,
    createShortCut: system.createShortCut,
    relaunch: system.relaunch,
    quit: system.quit,
    removePredefinedShortcut: function () { return true; },
    getNativeWindowHandle: function () { return ''; },
    getPathByName: function (name) { return call('system', 'getPath', [mustString(name, 'name')]); },
    screen: { getAllDisplays: function () { return call('system', 'getDisplays', []); } },
    getPathForFile: function (file) {
      if (!file) return '';
      if (typeof file === 'string') return file;
      return file.path || '';
    },

    // efto 扩展能力
    exec: ext.exec,
    output: ext.output,
    ipcMessage: ext.ipcMessage,
    node: ext.node,
    python: ext.python,
    hosts: ext.hosts,
    ai: ext.ai,
    setSearchItems: searchExt.setItems
  };

  // 给 UI 侧（webview）用的占位：main 线程没有 DOM
  if (typeof globalThis.window === 'undefined') {
    Object.defineProperty(globalThis, 'window', { value: globalThis, writable: false, configurable: true });
  }
  globalThis.efto = eftoApi;
  globalThis.window.efto = eftoApi;
  globalThis.window.exports = Object.create(null);
  // main 用 exports 挂钩子
  globalThis.exports = globalThis.window.exports;

  // ---------- 分发器：宿主唯一的回调入口 ----------
  globalThis.__efto_dispatch = function (kind, payloadText) {
    var p = payloadText ? JSON.parse(payloadText) : {};
    switch (kind) {
      case 'enter':
        fire(hooks.enter, p);
        return;
      case 'out':
        fire(hooks.out, p);
        return;
      case 'main_message':
      case 'ui_message':
        fire(hooks.mainMessage, p);
        return;
      case 'ui_closed':
        fire(hooks.uiClosed, p);
        return;
      case 'window_hide':
        fire(hooks.windowHide, p);
        return;
      case 'window_show':
        fire(hooks.windowShow, p);
        return;
      case 'database_change':
        fire(hooks.databaseChange, p);
        return;
      case 'timer': {
        var t = timers.get(p.id);
        if (!t) return;
        if (!t.repeat) timers.delete(p.id);
        try { t.fn(); } catch (e) { reportError('timer', e); }
        return;
      }
      case 'reply': {
        var d = pending.get(p.id);
        if (!d) return;
        pending.delete(p.id);
        streamHandlers.delete(p.id);
        if (p.ok) d.resolve(p.result);
        else d.reject(new Error(typeof p.result === 'string' ? p.result : JSON.stringify(p.result)));
        return;
      }
      case 'stream': {
        var cb = streamHandlers.get(p.id);
        if (!cb) return;
        try { cb(p.delta); } catch (e) { reportError('stream', e); }
        return;
      }
      default:
        reportError('dispatch', new Error('未知事件类型 ' + kind));
    }
  };

  // 模块导出登记：入口脚本执行完后由宿主调用
  globalThis.__efto_set_exports = function (mod) {
    if (!isObj(mod)) return;
    Object.keys(mod).forEach(function (k) {
      var v = mod[k];
      if (typeof v !== 'function' || !/^on[A-Z]/.test(k)) return;
      var map = {
        PluginEnter: 'enter', PluginOut: 'out', MainMessage: 'mainMessage',
        Message: 'message', IpcMessage: 'message',
        WindowHide: 'windowHide', WindowShow: 'windowShow', DatabaseChange: 'databaseChange',
        UiClosed: 'uiClosed'
      }[k.slice(2)];
      if (map && hooks[map]) hooks[map].push(v);
    });
  };

  // ---------- 极简 CommonJS：宿主负责解析与转译，模块在本 isolate 内编译执行 ----------
  var moduleCache = new Map();

  function makeRequire(dir) {
    return function (request) { return loadModule(request, dir); };
  }

  function loadModule(request, fromDir) {
    if (typeof request !== 'string' || !request) throw new TypeError('require 需要模块路径');
    if (!request.startsWith('.') && !request.startsWith('/') && !/^[A-Za-z]:[\\\\]/.test(request)) {
      // 裸模块名：第一阶段不提供 node 内置/第三方包，给出明确错误
      throw new Error(
        'efto 暂不提供 node_modules 依赖解析（"' + request + '"）。' +
        '请把依赖打包进插件目录并用相对路径引用。'
      );
    }
    var resolved = call('module', 'resolve', [request, fromDir || globalThis.__PLUGIN_ROOT__]);
    if (!resolved || !resolved.path) throw new Error('无法解析模块: ' + request);
    if (moduleCache.has(resolved.path)) return moduleCache.get(resolved.path).exports;
    var mod = { id: resolved.path, filename: resolved.path, exports: {} };
    moduleCache.set(resolved.path, mod);
    var req = makeRequire(resolved.dirname || globalThis.__PLUGIN_ROOT__);
    var ok = false;
    try {
      ok = globalThis.__run_module_source(mod, resolved.path, req);
    } catch (e) {
      moduleCache.delete(resolved.path);
      throw e;
    }
    if (!ok) {
      moduleCache.delete(resolved.path);
      throw new Error('模块加载失败: ' + resolved.path);
    }
    return mod.exports;
  }

  globalThis.__require = loadModule;
  globalThis.require = loadModule;
})();
"#;

/// 入口脚本的包装模板：给插件 main 提供 CJS 作用域。
pub const ENTRY_WRAPPER: &str = r#"(function (module, exports, require, __filename, __dirname) {
%SOURCE%
})"#;

/// 每个宿主调用在 JS 里的同步返回约定：payload 一律 JSON 字符串。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelude_is_not_empty_and_balanced() {
        assert!(PRELUDE.len() > 2_000);
        // 粗查括号配平，防止误删收尾
        let open = PRELUDE.matches('{').count();
        let close = PRELUDE.matches('}').count();
        assert!(
            (open as isize - close as isize).abs() <= 2,
            "braces {open} vs {close}"
        );
        assert!(PRELUDE.contains("__efto_dispatch"));
        assert!(PRELUDE.contains("globalThis.efto = eftoApi"));
    }

    #[test]
    fn entry_wrapper_has_placeholder() {
        assert!(ENTRY_WRAPPER.contains("%SOURCE%"));
    }

    #[test]
    fn prelude_exposes_native_api_only() {
        assert!(PRELUDE.contains("'output', 'set'"));
        // 模型能力改名/漏挂会让插件在运行时才炸，这里钉住名字
        assert!(PRELUDE.contains("'ai', 'chat'"));
        assert!(PRELUDE.contains("kv: dbLegacy"));
    }
}
