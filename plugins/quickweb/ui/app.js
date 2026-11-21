'use strict';
// 网页快开的管理界面：列出内置与自定义站点、按关键词筛选、点击即用系统浏览器打开。
// UI 侧的 efto.* 全部走 IPC 并返回 Promise。
(function () {
  var state = { builtin: [], custom: [], groups: [], groupLabels: {}, filter: '' };

  var elList = document.getElementById('list');
  var elFilter = document.getElementById('filter');
  var elRefresh = document.getElementById('refresh');
  var elToast = document.getElementById('toast');
  var toastTimer = 0;

  function host(url) {
    try { return new URL(url.replace('{query}', '')).host; }
    catch (e) { return url; }
  }

  function matches(text) {
    if (!state.filter) return true;
    return String(text).toLowerCase().indexOf(state.filter) >= 0;
  }

  function openUrl(url) {
    efto.ipcMessage('open', { url: url }).catch(function (e) {
      console.warn('[quickweb] 打开失败', e);
    });
  }

  function toast(text, bad) {
    if (!elToast) return;
    elToast.textContent = text;
    elToast.className = 'toast' + (bad ? ' bad' : '');
    elToast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { elToast.hidden = true; }, 2400);
  }

  function trashIcon() {
    return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="m6 6 1 14h10l1-14"/></svg>';
  }

  function siteIcon(site) {
    var box = document.createElement('span');
    box.className = 'site-icon';
    box.textContent = String(site.cmd || '?').slice(0, 1);
    if (!site.icon) return box;
    var img = document.createElement('img');
    img.src = site.icon;
    img.alt = '';
    img.loading = 'lazy';
    img.onerror = function () { img.remove(); };
    img.onload = function () { box.textContent = ''; };
    box.appendChild(img);
    return box;
  }

  function renderBuiltin(frag) {
    var shown = 0;

    for (var g = 0; g < state.groups.length; g++) {
      var group = state.groups[g];
      var items = state.builtin.filter(function (s) {
        return s.group === group && (matches(s.cmd) || matches(s.url));
      });
      if (!items.length) continue;

      var section = document.createElement('section');
      section.className = 'group';

      var title = document.createElement('h2');
      title.className = 'group-title';
      title.textContent = (state.groupLabels[group] || group) + '（' + items.length + '）';
      section.appendChild(title);

      var grid = document.createElement('div');
      grid.className = 'grid';

      items.forEach(function (s) {
        var card = document.createElement('div');
        card.className = 'item';
        card.title = s.url;
        card.tabIndex = 0;
        card.setAttribute('role', 'button');
        card.setAttribute('aria-label', s.cmd + (s.needsQuery ? '，需要输入关键词' : ''));

        var main = document.createElement('div');
        main.className = 'item-main';

        var name = document.createElement('div');
        name.className = 'name';
        name.textContent = s.cmd;

        var where = document.createElement('div');
        where.className = 'host';
        var hostText = document.createElement('span');
        hostText.textContent = host(s.url);
        where.appendChild(hostText);
        if (s.needsQuery) {
          var need = document.createElement('span');
          need.className = 'need';
          need.textContent = '关键词';
          where.appendChild(need);
        }

        main.appendChild(name);
        main.appendChild(where);
        card.appendChild(siteIcon(s));
        card.appendChild(main);

        // 内置项也能删：记进 hidden_sites，之后既不显示也打不开。
        var btnDel = document.createElement('button');
        btnDel.type = 'button';
        btnDel.className = 'item-del';
        btnDel.title = '从列表删除';
        btnDel.setAttribute('aria-label', '删除 ' + s.cmd);
        btnDel.innerHTML = trashIcon();
        btnDel.addEventListener('click', function (ev) {
          ev.stopPropagation();
          deleteBuiltin(s.code || s.cmd);
        });
        card.appendChild(btnDel);

        function activate() {
          if (s.needsQuery) {
            // 带 {query} 的站点必须有关键词，这里只把触发词写进筛选框提示用法
            elFilter.value = s.cmd + ' ';
            elFilter.focus();
            elFilter.select();
            return;
          }
          openUrl(s.url);
        }

        card.addEventListener('click', activate);
        card.addEventListener('keydown', function (ev) {
          if (ev.key === 'Enter' || ev.key === ' ') {
            ev.preventDefault();
            activate();
          }
        });

        grid.appendChild(card);
        shown++;
      });

      section.appendChild(grid);
      frag.appendChild(section);
    }

    if (!shown) {
      var empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = state.builtin.length ? '没有匹配的内置项' : '内置项为空';
      frag.appendChild(empty);
    }
  }

  function renderCustom(frag) {
    var visible = state.custom.filter(function (item) {
      return matches(item.name) || matches(item.url);
    });

    var section = document.createElement('section');
    section.className = 'custom-card';

    var head = document.createElement('div');
    head.className = 'custom-head';
    var title = document.createElement('h2');
    title.className = 'group-title';
    title.textContent = state.filter
      ? '自定义（' + visible.length + ' / ' + state.custom.length + '）'
      : '自定义（' + state.custom.length + '）';
    head.appendChild(title);
    section.appendChild(head);

    var list = document.createElement('div');
    list.className = 'custom-list';

    if (state.filter && !visible.length) {
      var none = document.createElement('div');
      none.className = 'empty';
      none.textContent = '没有匹配的自定义项';
      list.appendChild(none);
    }

    visible.forEach(function (item) {
      var index = state.custom.indexOf(item);
      var row = document.createElement('div');
      row.className = 'custom-item';

      var avatar = document.createElement('span');
      avatar.className = 'custom-av';
      avatar.setAttribute('aria-hidden', 'true');
      avatar.textContent = String(item.name || '?').slice(0, 1);

      var who = document.createElement('div');
      who.className = 'who';
      who.textContent = item.name;

      var where = document.createElement('div');
      where.className = 'where';
      where.textContent = item.url;
      where.title = item.url;

      var actions = document.createElement('div');
      actions.className = 'custom-actions';

      var btnOpen = document.createElement('button');
      btnOpen.type = 'button';
      btnOpen.className = 'btn small';
      btnOpen.textContent = '打开';
      btnOpen.addEventListener('click', function () { openUrl(item.url); });

      var btnDel = document.createElement('button');
      btnDel.type = 'button';
      btnDel.className = 'btn small danger';
      btnDel.textContent = '删除';
      btnDel.setAttribute('aria-label', '删除 ' + item.name);
      btnDel.addEventListener('click', function () {
        var next = state.custom.slice();
        next.splice(index, 1);
        save(next);
      });

      actions.appendChild(btnOpen);
      actions.appendChild(btnDel);
      row.appendChild(avatar);
      row.appendChild(who);
      row.appendChild(where);
      row.appendChild(actions);
      list.appendChild(row);
    });

    section.appendChild(list);

    // 新增行
    var add = document.createElement('div');
    add.className = 'add-row';

    var inName = document.createElement('input');
    inName.type = 'text';
    inName.placeholder = '触发词，例如 我的博客';
    inName.setAttribute('aria-label', '触发词');
    inName.autocomplete = 'off';
    inName.spellcheck = false;

    var inUrl = document.createElement('input');
    inUrl.type = 'url';
    inUrl.placeholder = 'https://…（支持 {query} 占位）';
    inUrl.setAttribute('aria-label', '网址');
    inUrl.autocomplete = 'off';
    inUrl.spellcheck = false;

    var btnAdd = document.createElement('button');
    btnAdd.type = 'button';
    btnAdd.className = 'btn primary';
    btnAdd.textContent = '添加';

    var err = document.createElement('div');
    err.className = 'err';
    err.setAttribute('role', 'alert');

    function addSite() {
      var name = inName.value.trim();
      var url = inUrl.value.trim();
      err.textContent = '';
      if (!name) { err.textContent = '触发词不能为空'; inName.focus(); return; }
      if (!/^https?:\/\//i.test(url)) { err.textContent = '网址必须以 http:// 或 https:// 开头'; inUrl.focus(); return; }
      var dup = state.custom.some(function (c) { return c.name.toLowerCase() === name.toLowerCase(); });
      if (dup) { err.textContent = '这个触发词已经有了'; inName.focus(); return; }
      var next = state.custom.slice();
      next.push({ name: name, url: url });
      save(next);
    }

    btnAdd.addEventListener('click', addSite);
    inName.addEventListener('keydown', function (ev) { if (ev.key === 'Enter') { ev.preventDefault(); addSite(); } });
    inUrl.addEventListener('keydown', function (ev) { if (ev.key === 'Enter') { ev.preventDefault(); addSite(); } });

    add.appendChild(inName);
    add.appendChild(inUrl);
    add.appendChild(btnAdd);
    section.appendChild(add);
    section.appendChild(err);

    var tip = document.createElement('div');
    tip.className = 'hint';
    tip.textContent = '提示：自定义项的触发词在启动器里直接输入即可打开；网址里的 {query} 会被替换成触发词后的内容。';
    section.appendChild(tip);

    frag.appendChild(section);
  }

  function render() {
    elList.innerHTML = '';
    var frag = document.createDocumentFragment();
    renderBuiltin(frag);
    renderCustom(frag);
    elList.appendChild(frag);
  }

  function save(list) {
    efto.ipcMessage('custom-save', { list: list }).then(function () {
      pull();
    }, function (e) {
      console.warn('[quickweb] 保存失败', e);
      toast('保存失败', true);
    });
  }

  function deleteBuiltin(code) {
    efto.ipcMessage('builtin-delete', { code: code }).then(function () {
      pull();
    }, function (e) {
      console.warn('[quickweb] 删除内置项失败', e);
      toast('删除失败', true);
    });
  }

  function pull() {
    efto.ipcMessage('sites', {});
  }

  efto.onMessage('sites', function (data) {
    if (!data) return;
    state.builtin = data.builtin || [];
    state.custom = data.custom || [];
    state.groups = data.groups || [];
    state.groupLabels = data.groupLabels || {};
    render();
  });

  efto.onMessage('saved', function (data) {
    if (data && data.ok === false) toast(data.error || '保存失败', true);
  });

  elFilter.addEventListener('input', function () {
    state.filter = elFilter.value.trim().toLowerCase();
    render();
  });

  elFilter.addEventListener('keydown', function (ev) {
    if (ev.key === 'Escape') {
      ev.preventDefault();
      elFilter.value = '';
      state.filter = '';
      render();
      elFilter.blur();
    }
  });

  elRefresh.addEventListener('click', pull);

  pull();
})();
