// efto 快捷键指南：识别唤起搜索框前的应用，展示内置快捷键与用户覆盖。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var groupsEl = document.getElementById('groups');
  var nameEl = document.getElementById('app-name');
  var exeEl = document.getElementById('app-exe');
  var selectEl = document.getElementById('app-select');
  var filterEl = document.getElementById('filter');
  var hintEl = document.getElementById('hint');
  var closeEl = document.getElementById('close');
  var state = { apps: [], data: null, query: '' };
  var collapsed = {};

  function invoke(cmd, args) {
    if (!I || typeof I.invoke !== 'function') return Promise.reject(new Error('no tauri'));
    return I.invoke(cmd, args || {});
  }

  function t(key, vars) {
    if (window.eftoI18n && typeof window.eftoI18n.tf === 'function') {
      return window.eftoI18n.tf(key, vars || {});
    }
    return key;
  }

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  }

  function fillApps(apps) {
    if (!Array.isArray(apps)) return;
    state.apps = apps;
    selectEl.textContent = '';
    apps.forEach(function (app) {
      var opt = document.createElement('option');
      opt.value = app.id;
      opt.textContent = app.name;
      selectEl.appendChild(opt);
    });
  }

  function currentName(data) {
    if (data.name) return data.name;
    if (data.fallback) {
      return data.exe ? t('shortcuts.unknownApp') : t('shortcuts.noForeground');
    }
    if (data.set && data.set.app && data.set.app.name) return data.set.app.name;
    return t('shortcuts.generic');
  }

  function apply(data) {
    if (!data) return;
    state.data = data;
    nameEl.textContent = currentName(data);
    var exe = data.exe || (data.set && data.set.app && data.set.app.exe[0]) || '';
    var meta = [exe, data.window_title].filter(function (s) { return !!s; }).join(' · ');
    exeEl.textContent = meta;
    if (data.id) {
      // 用户自建数据可能没有对应的下拉项，补一个再选中
      var has = Array.prototype.some.call(selectEl.options, function (o) {
        return o.value === data.id;
      });
      if (!has) {
        var extra = document.createElement('option');
        extra.value = data.id;
        extra.textContent = currentName(data);
        selectEl.appendChild(extra);
      }
      selectEl.value = data.id;
    }
    if (data.fallback) {
      hintEl.textContent = data.exe
        ? t('shortcuts.fallback', { exe: data.exe })
        : t('shortcuts.fallbackNone');
      hintEl.hidden = false;
    } else {
      hintEl.hidden = true;
      hintEl.textContent = '';
    }
    render();
  }

  function matches(item) {
    var q = state.query;
    if (!q) return true;
    return (
      String(item.keys || '').toLowerCase().indexOf(q) >= 0 ||
      String(item.action || '').toLowerCase().indexOf(q) >= 0 ||
      String(item.note || '').toLowerCase().indexOf(q) >= 0
    );
  }

  function render() {
    var set = state.data && state.data.set;
    groupsEl.textContent = '';
    if (!set || !Array.isArray(set.groups) || set.groups.length === 0) {
      groupsEl.appendChild(el('p', 'loading', t('shortcuts.empty')));
      return;
    }
    var shown = 0;
    set.groups.forEach(function (group) {
      var groupHit = !state.query || String(group.name).toLowerCase().indexOf(state.query) >= 0;
      var items = (group.items || []).filter(function (item) {
        return groupHit || matches(item);
      });
      if (items.length === 0) return;
      shown += items.length;
      var section = el('section', 'group' + (collapsed[group.name] ? ' collapsed' : ''));
      var title = el('h2', null, group.name);
      title.addEventListener('click', function () {
        collapsed[group.name] = !collapsed[group.name];
        section.classList.toggle('collapsed', collapsed[group.name]);
      });
      section.appendChild(title);
      var list = el('div', 'items');
      items.forEach(function (item) {
        var row = el('div', 'item');
        row.appendChild(el('kbd', 'keys', item.keys));
        var text = el('div', 'action');
        text.appendChild(el('span', 'action-text', item.action));
        if (item.note) text.appendChild(el('span', 'note', item.note));
        row.appendChild(text);
        list.appendChild(row);
      });
      section.appendChild(list);
      groupsEl.appendChild(section);
    });
    if (shown === 0) groupsEl.appendChild(el('p', 'loading', t('shortcuts.empty')));
  }

  function load() {
    return invoke('shortcut_guide_data')
      .then(function (data) {
        fillApps(data.apps);
        apply(data);
      })
      .catch(function (e) {
        groupsEl.textContent = '';
        groupsEl.appendChild(el('p', 'loading', t('shortcuts.failed') + ': ' + e));
      });
  }

  selectEl.addEventListener('change', function () {
    var id = selectEl.value;
    if (!id) return;
    // 用户自建数据没有内置 id，重新按前台应用解析一次
    var req = id === 'custom' ? invoke('shortcut_guide_data') : invoke('shortcut_guide_lookup', { id: id });
    req
      .then(function (data) {
        data.apps = state.apps;
        apply(data);
      })
      .catch(function (e) {
        hintEl.textContent = String(e);
        hintEl.hidden = false;
      });
  });

  filterEl.addEventListener('input', function () {
    state.query = String(filterEl.value || '').trim().toLowerCase();
    render();
  });

  closeEl.addEventListener('click', function () {
    invoke('hide_shortcuts').catch(function () {});
  });

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      invoke('hide_shortcuts').catch(function () {});
    } else if ((e.ctrlKey || e.metaKey) && (e.key === 'f' || e.key === 'F')) {
      e.preventDefault();
      filterEl.focus();
      filterEl.select();
    }
  });

  // 每次从搜索框打开都重新识别：语言/主题可能刚改过，前台应用也变了
  if (I && typeof I.transformCallback === 'function') {
    try {
      var cb = I.transformCallback(function () { load(); }, false);
      I.invoke('plugin:event|listen', {
        event: 'shortcuts:show',
        target: { kind: 'Any' },
        handler: cb
      }).catch(function () {});
    } catch (e) {
      // 非 Tauri 环境忽略
    }
  }

  if (window.eftoI18n && window.eftoI18n.ready) {
    window.eftoI18n.ready.then(function () { load(); });
  } else {
    load();
  }
})();
