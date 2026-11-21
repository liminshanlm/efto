// efto 前端多语言。词典是 frontend/locales/<tag>.json，构建时随 frontendDist 一起打包。
// 语言由后端解析（settings.language + 系统语言），这里只负责按 tag 加载并替换 DOM。
// 没有打包器：只暴露 window.eftoI18n，页面脚本按需调用。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var FALLBACK = 'zh-CN';
  var lang = FALLBACK;
  var dict = {};
  var fallbackDict = {};
  var warned = {};
  var listeners = [];

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') return Promise.reject(new Error('no tauri'));
    return I.invoke(cmd, payload || {});
  }

  function loadDict(tag) {
    return fetch('locales/' + tag + '.json', { cache: 'no-cache' })
      .then(function (r) {
        if (!r.ok) throw new Error('HTTP ' + r.status);
        return r.json();
      })
      .catch(function (e) {
        console.warn('[i18n] 词典加载失败 ' + tag + ': ' + e);
        return null;
      });
  }

  function t(key, fb) {
    var v = dict[key];
    if (v == null) v = fallbackDict[key];
    if (v == null) {
      if (!warned[key]) {
        warned[key] = true;
        console.warn('[i18n] 缺少 key: ' + key);
      }
      return fb != null ? fb : key;
    }
    return v;
  }

  // 带占位符的文案：t('a.b', { n: 3 }) → "3 条结果"
  function tf(key, vars, fb) {
    return String(t(key, fb)).replace(/\{(\w+)\}/g, function (m, name) {
      return vars && vars[name] != null ? String(vars[name]) : m;
    });
  }

  function applyEl(el, attr, prop) {
    var key = el.getAttribute(attr);
    if (!key) return;
    var value = t(key, prop === 'innerHTML' ? el.innerHTML : el[prop]);
    if (prop === 'innerHTML') {
      if (el.innerHTML !== value) el.innerHTML = value;
    } else if (el[prop] !== value) {
      el[prop] = value;
    }
  }

  function apply(root) {
    root = root || document;
    root.querySelectorAll('[data-i18n]').forEach(function (el) { applyEl(el, 'data-i18n', 'textContent'); });
    root.querySelectorAll('[data-i18n-html]').forEach(function (el) { applyEl(el, 'data-i18n-html', 'innerHTML'); });
    root.querySelectorAll('[data-i18n-placeholder]').forEach(function (el) { applyEl(el, 'data-i18n-placeholder', 'placeholder'); });
    root.querySelectorAll('[data-i18n-title]').forEach(function (el) { applyEl(el, 'data-i18n-title', 'title'); });
    root.querySelectorAll('[data-i18n-aria-label]').forEach(function (el) { applyEl(el, 'data-i18n-aria-label', 'aria-label'); });
  }

  var ready = Promise.resolve(null);

  function start() {
    return invoke('ui_language', {})
      .then(function (r) { if (r && r.language) lang = r.language; })
      .catch(function () { lang = FALLBACK; })
      .then(function () { return loadDict(FALLBACK); })
      .then(function (d) {
        if (d) fallbackDict = d;
        if (lang === FALLBACK) { dict = fallbackDict; return null; }
        return loadDict(lang);
      })
      .then(function (d) {
        if (d) dict = d;
        document.documentElement.lang = lang;
        apply(document);
        listeners.forEach(function (fn) {
          try { fn(lang); } catch (e) { console.warn('[i18n] 监听器异常: ' + e); }
        });
        return lang;
      });
  }

  window.eftoI18n = {
    t: t,
    tf: tf,
    apply: apply,
    ready: ready,
    lang: function () { return lang; },
    onChange: function (fn) { listeners.push(fn); }
  };
  ready = start();
  window.eftoI18n.ready = ready;
})();
