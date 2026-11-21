// text-beautify/ui.js —— 插件 UI（普通 webview，无 Node 权限）。
// 美化全部在本地完成：lib/fmt-*.js 是纯函数，不联网、不调模型。

(function () {
  'use strict';

  var LIB = (typeof TBFormat !== 'undefined' && TBFormat) || {};

  // 宿主注入的 SDK。用 typeof 取：直接写 efto 在它不存在时是 ReferenceError，
  // 而这会让整个页面在初始化阶段就挂掉，连本地美化都跑不了。
  var host = (typeof efto !== 'undefined' && efto) ? efto : null;

  var srcEl = document.getElementById('src');
  var outEl = document.getElementById('out');
  var srcMetaEl = document.getElementById('srcMeta');
  var outMetaEl = document.getElementById('outMeta');
  var statusEl = document.getElementById('status');
  var hintEl = document.getElementById('hint');
  var bannerEl = document.getElementById('banner');
  var toastEl = document.getElementById('toast');
  var optsEl = document.getElementById('opts');
  var typesEl = document.getElementById('types');
  var indentEl = document.getElementById('indent');
  var beautifyEl = document.getElementById('beautify');
  var copyEl = document.getElementById('copy');
  var pasteEl = document.getElementById('paste');
  var clearEl = document.getElementById('clear');

  // 各类型的专属选项；state.opts 会被完整传给对应的 format()
  var TYPE_META = {
    json: {
      name: 'JSON',
      hint: '支持 // 与 /* */ 注释、尾随逗号；数字与字符串转义原样保留',
      defaults: { compactArrays: true },
      fields: [
        { key: 'compactArrays', type: 'check', label: '标量短数组保持单行' }
      ]
    },
    xml: {
      name: 'XML',
      hint: '保留注释、CDATA、DOCTYPE 与属性引号；只含文本的元素合并成一行',
      defaults: { attrWrap: 0 },
      fields: [
        { key: 'attrWrap', type: 'number', label: '属性超长换行阈值', min: 0, max: 400, step: 10, unit: '字符（0 = 不换行）' }
      ]
    },
    sql: {
      name: 'SQL',
      hint: '按子句换行；字符串与注释内容一字不改',
      defaults: { upperKeywords: true, commaBreaks: true },
      fields: [
        { key: 'upperKeywords', type: 'check', label: '关键字大写' },
        { key: 'commaBreaks', type: 'check', label: '列清单按逗号换行' }
      ]
    },
    js: {
      name: 'JS',
      hint: '支持模板字符串、正则字面量与常见 TS 语法；不改写字符串内容',
      defaults: { singleQuote: false },
      fields: [
        { key: 'singleQuote', type: 'check', label: '简单字符串统一成单引号' }
      ]
    }
  };

  var state = {
    type: 'json',
    opts: {},
    indent: '  ',
    timer: 0,
    lastInput: null,
    lastOutput: ''
  };

  // ---------- 小工具 ----------

  function toast(text, bad) {
    toastEl.textContent = text;
    toastEl.className = 'toast' + (bad ? ' bad' : '');
    toastEl.hidden = false;
    clearTimeout(toast._t);
    toast._t = setTimeout(function () { toastEl.hidden = true; }, 1800);
  }

  function setStatus(text, kind) {
    statusEl.textContent = text;
    statusEl.className = 'status' + (kind ? ' ' + kind : '');
  }

  function setBanner(text) {
    bannerEl.textContent = text || '';
    bannerEl.hidden = !text;
  }

  function countMeta(text) {    var s = text == null ? '' : String(text);
    if (!s) return '空';
    var lines = s.split(/\r\n|\r|\n/).length;
    return s.length + ' 字符 · ' + lines + ' 行';
  }

  function currentFormatter() {
    var f = LIB[state.type];
    return f && typeof f.format === 'function' ? f : null;
  }

  // 计时只用于状态栏的参考值，取不到就退回 Date（不要写裸 performance，
  // 在某些宿主/沙箱里它不是全局标识符，直接引用会抛 ReferenceError）
  function nowMs() {
    var p = (typeof window !== 'undefined' && window) ? window.performance : null;
    if (p && typeof p.now === 'function') {
      try { return p.now(); } catch (e) { /* 退回 Date */ }
    }
    return Date.now();
  }

  // ---------- 选项区 ----------

  function buildOpts() {
    var meta = TYPE_META[state.type];
    optsEl.innerHTML = '';
    if (!meta) return;

    for (var i = 0; i < meta.fields.length; i++) {
      var f = meta.fields[i];
      var label = document.createElement('label');
      label.className = 'opt';

      if (f.type === 'check') {
        var box = document.createElement('input');
        box.type = 'checkbox';
        box.checked = !!state.opts[f.key];
        box.addEventListener('change', function (key, el) {
          return function () {
            state.opts[key] = el.checked;
            schedule();
          };
        }(f.key, box));
        label.appendChild(box);
        var span = document.createElement('span');
        span.textContent = f.label;
        label.appendChild(span);
      } else {
        var name = document.createElement('span');
        name.textContent = f.label;
        label.appendChild(name);

        var num = document.createElement('input');
        num.type = 'number';
        num.className = 'num';
        if (typeof f.min === 'number') num.min = String(f.min);
        if (typeof f.max === 'number') num.max = String(f.max);
        if (typeof f.step === 'number') num.step = String(f.step);
        num.value = String(state.opts[f.key]);
        num.addEventListener('input', function (key, el, field) {
          return function () {
            var v = Number(el.value);
            if (!isFinite(v)) v = 0;
            if (typeof field.min === 'number' && v < field.min) v = field.min;
            if (typeof field.max === 'number' && v > field.max) v = field.max;
            state.opts[key] = v;
            schedule();
          };
        }(f.key, num, f));
        label.appendChild(num);

        if (f.unit) {
          var unit = document.createElement('span');
          unit.className = 'unit';
          unit.textContent = f.unit;
          label.appendChild(unit);
        }
      }

      optsEl.appendChild(label);
    }

    hintEl.textContent = meta.hint;
  }

  function switchType(next) {
    if (!TYPE_META[next] || next === state.type) return;
    state.type = next;

    var items = typesEl.querySelectorAll('.seg-item');
    for (var i = 0; i < items.length; i++) {
      items[i].className = 'seg-item' + (items[i].getAttribute('data-type') === next ? ' on' : '');
    }

    // 每类各有自己的默认值，切过去就重置为该类默认，避免把 JSON 的开关带到 SQL
    state.opts = {};
    var defaults = TYPE_META[next].defaults || {};
    for (var k in defaults) {
      if (Object.prototype.hasOwnProperty.call(defaults, k)) state.opts[k] = defaults[k];
    }

    buildOpts();
    if (srcEl.value.trim()) {
      schedule(0);
    } else {
      // 原文为空时不必跑一次"原文为空"，否则状态栏会把「已切到 X」立刻盖掉
      outEl.value = '';
      outMetaEl.textContent = '';
      setStatus('已切到 ' + TYPE_META[next].name);
    }
  }

  // ---------- 美化 ----------

  function run() {
    clearTimeout(state.timer);
    state.timer = 0;

    var input = srcEl.value;
    var meta = TYPE_META[state.type];
    setBanner('');

    if (!input.trim()) {
      outEl.value = '';
      outMetaEl.textContent = '';
      state.lastOutput = '';
      setStatus('原文为空');
      return;
    }

    var fmt = currentFormatter();
    if (!fmt) {
      setBanner('这个类型的美化器没有加载成功（lib/fmt-' + state.type + '.js 缺失或语法错误）。');
      setStatus('美化器缺失', 'bad');
      return;
    }

    var opts = {};
    var defaults = meta.defaults || {};
    for (var k in defaults) {
      if (Object.prototype.hasOwnProperty.call(defaults, k)) opts[k] = state.opts[k];
    }
    opts.indent = state.indent;

    var t0 = nowMs();
    var res;
    try {
      res = fmt.format(input, opts);
    } catch (e) {
      // 各格式化器内部都兜了 try/catch，走到这里说明是意料之外的缺陷，如实报出来
      res = { ok: false, error: '美化器内部错误：' + ((e && e.message) ? e.message : String(e)) };
    }
    var ms = Math.max(0, nowMs() - t0);

    if (!res || !res.ok) {
      outEl.value = '';
      outMetaEl.textContent = '';
      state.lastOutput = '';
      var msg = (res && res.error) ? res.error : '未能解析原文';
      setBanner(msg);
      setStatus('美化失败', 'bad');
      return;
    }

    var text = res.text == null ? '' : String(res.text);
    outEl.value = text;
    outMetaEl.textContent = countMeta(text);
    state.lastOutput = text;
    state.lastInput = input;

    setStatus('已美化 ' + TYPE_META[state.type].name + ' · ' + Math.round(ms) + ' ms · ' + countMeta(text), 'ok');

    // 结果太长时把视口拉回顶部，避免用户以为没变化
    outEl.scrollTop = 0;
    outEl.scrollLeft = 0;
  }

  function schedule(delay) {
    clearTimeout(state.timer);
    var wait = typeof delay === 'number' ? delay : 420;
    srcMetaEl.textContent = countMeta(srcEl.value);
    if (!srcEl.value.trim()) {
      run();
      return;
    }
    state.timer = setTimeout(run, wait);
  }

  // ---------- 事件 ----------

  typesEl.addEventListener('click', function (e) {
    var t = e.target;
    if (!t || !t.getAttribute) return;
    var type = t.getAttribute('data-type');
    if (type) switchType(type);
  });

  indentEl.addEventListener('change', function () {
    var v = indentEl.value;
    state.indent = v === 'tab' ? '\t' : v;
    schedule(0);
  });

  beautifyEl.addEventListener('click', run);

  copyEl.addEventListener('click', function () {
    var text = outEl.value;
    if (!text) {
      toast('还没有可复制的结果', true);
      return;
    }
    try {
      // UI 侧 copyText 是宿主能力，返回 Promise；写失败要如实说，不能假装成功
      var r = host && host.copyText ? host.copyText(text) : null;
      if (r && typeof r.then === 'function') {
        r.then(function () { toast('已复制 ' + text.length + ' 字符'); })
          .catch(function (err) { fallbackCopy(text, err); });
      } else {
        toast('已复制 ' + text.length + ' 字符');
      }
    } catch (err) {
      fallbackCopy(text, err);
    }
  });

  function fallbackCopy(text, err) {
    // 剪贴板能力不可用时退回浏览器选择复制，并把真实原因说清楚
    try {
      outEl.focus();
      outEl.select();
      var ok = document.execCommand && document.execCommand('copy');
      if (ok) {
        toast('已复制');
        return;
      }
    } catch (e) { /* 继续走下面的报错 */ }
    toast('复制失败：' + ((err && err.message) ? err.message : '宿主剪贴板不可用'), true);
  }

  pasteEl.addEventListener('click', function () {
    if (!host || !host.ipcMessage) {
      toast('当前环境没有剪贴板能力', true);
      return;
    }
    host.ipcMessage('paste', {}).catch(function (e) {
      toast('读取剪贴板失败：' + e, true);
    });
  });

  clearEl.addEventListener('click', function () {
    srcEl.value = '';
    outEl.value = '';
    srcMetaEl.textContent = '';
    outMetaEl.textContent = '';
    state.lastOutput = '';
    setBanner('');
    setStatus('已清空');
    srcEl.focus();
  });

  srcEl.addEventListener('input', function () { schedule(); });

  srcEl.addEventListener('keydown', function (e) {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      run();
    }
  });

  outEl.addEventListener('keydown', function (e) {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      run();
    }
  });

  // Tab 在原文里插缩进而不是跳焦点：这个框就是用来贴代码的
  srcEl.addEventListener('keydown', function (e) {
    if (e.key !== 'Tab' || e.ctrlKey || e.altKey) return;
    e.preventDefault();
    var start = srcEl.selectionStart;
    var end = srcEl.selectionEnd;
    var unit = state.indent || '  ';
    srcEl.value = srcEl.value.slice(0, start) + unit + srcEl.value.slice(end);
    srcEl.selectionStart = srcEl.selectionEnd = start + unit.length;
    schedule();
  });

  // ---------- 与 main 通信 ----------

  function applySeed(text) {
    if (typeof text !== 'string' || !text.trim()) return;
    // 用户已经手动打了内容就不要覆盖
    if (srcEl.value.trim() && srcEl.value !== state.lastInput) return;
    srcEl.value = text;
    srcMetaEl.textContent = countMeta(text);
    run();
  }

  if (host && host.onMessage) {
    host.onMessage('seed', function (data) {
      applySeed(data && data.text);
    });

    host.onMessage('pasted', function (data) {
      var text = data && typeof data.text === 'string' ? data.text : '';
      if (!text.trim()) {
        var why = data && data.error ? data.error : '剪贴板里没有文本';
        setBanner('没有取到原文：' + why);
        toast('剪贴板为空', true);
        return;
      }
      srcEl.value = text;
      srcMetaEl.textContent = countMeta(text);
      setStatus('已粘贴 ' + countMeta(text));
      run();
    });

    host.onMessage('ready', function () {
      setStatus('就绪');
    });
  }

  // 初始化
  state.opts = {};
  var base = TYPE_META[state.type].defaults || {};
  for (var k0 in base) {
    if (Object.prototype.hasOwnProperty.call(base, k0)) state.opts[k0] = base[k0];
  }
  buildOpts();
  srcMetaEl.textContent = '';
  outMetaEl.textContent = '';

  if (host && host.ipcMessage) {
    host.ipcMessage('init', {}).catch(function () {
      // 握手失败不影响本地美化，只是拿不到预填内容
    });
  }

  srcEl.focus();
})();
