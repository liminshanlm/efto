// hosts/ui.js —— 插件 UI（普通 webview，无 Node 权限）。
// 文本的解析/序列化在这里完成；落盘前由宿主（Rust）再校验一次。

(function () {
  'use strict';

  var pathEl = document.getElementById('path');
  var encEl = document.getElementById('enc');
  var dotEl = document.getElementById('dot');
  var bannerEl = document.getElementById('banner');
  var saveEl = document.getElementById('save');
  var reloadEl = document.getElementById('reload');
  var tabListEl = document.getElementById('tab-list');
  var tabRawEl = document.getElementById('tab-raw');
  var listEl = document.getElementById('view-list');
  var rawEl = document.getElementById('view-raw');
  var rowsEl = document.getElementById('rows');
  var emptyEl = document.getElementById('empty');
  var textareaEl = document.getElementById('raw');
  var addEl = document.getElementById('add');
  var newIpEl = document.getElementById('new-ip');
  var newNamesEl = document.getElementById('new-names');
  var newCommentEl = document.getElementById('new-comment');
  var statusEl = document.getElementById('status');

  var state = { path: '', backup: '', encoding: 'utf8', text: '', saving: false, status: '', error: '' };
  var model = [];          // [{kind:'raw',text} | {kind:'entry',enabled,ip,names,comment}]
  var loadedText = '';     // 最近一次从宿主拿到的文本，用于判断是否有本地改动
  var dirty = false;
  var saveInFlight = false;
  var tab = 'list';

  // ---------- 文本模型 ----------

  function isIpv4(t) {
    var p = t.split('.');
    if (p.length !== 4) return false;
    for (var i = 0; i < 4; i++) {
      if (!/^\d{1,3}$/.test(p[i]) || Number(p[i]) > 255) return false;
    }
    return true;
  }

  function isIpv6(t) {
    if (t.indexOf(':') < 0) return false;
    var base = t.split('%')[0];
    return base.length > 0 && base.length <= 45 && /^[0-9a-fA-F:.]+$/.test(base);
  }

  function isIp(t) { return isIpv4(t) || isIpv6(t); }
  function isName(t) { return /^[A-Za-z0-9._-]{1,253}$/.test(t); }

  function parse(text) {
    var out = [];
    var lines = String(text == null ? '' : text).split(/\r?\n/);
    // 末尾换行会多出一个空行，但保留它也无害；去掉更贴近编辑器直觉
    if (lines.length && lines[lines.length - 1] === '') lines.pop();
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      var trim = line.trim();
      if (!trim) { out.push({ kind: 'raw', text: line }); continue; }
      var enabled = trim.charAt(0) !== '#';
      var body = enabled ? trim : trim.replace(/^#+\s*/, '');
      var comment = '';
      var h = body.indexOf('#');
      if (h >= 0) {
        comment = body.slice(h + 1).trim();
        body = body.slice(0, h).trim();
      }
      var toks = body.split(/\s+/).filter(function (x) { return x; });
      var entryLike = toks.length >= 2 && isIp(toks[0]);
      if (!enabled && !isIp(toks[0] || '')) entryLike = false;
      if (entryLike) {
        var names = toks.slice(1).filter(isName);
        if (names.length === toks.length - 1) {
          out.push({ kind: 'entry', enabled: enabled, ip: toks[0], names: names, comment: comment });
          continue;
        }
      }
      out.push({ kind: 'raw', text: line });
    }
    return out;
  }

  function serialize() {
    var out = [];
    for (var i = 0; i < model.length; i++) {
      var l = model[i];
      if (l.kind === 'raw') { out.push(l.text); continue; }
      var body = l.ip + ' ' + l.names.join(' ');
      if (l.comment) body += ' # ' + l.comment;
      out.push((l.enabled ? '' : '#') + body);
    }
    return out.join('\r\n') + '\r\n';
  }

  function validateText(text) {
    var lines = String(text).split(/\r?\n/);
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i].trim();
      if (!line) continue;
      var enabled = line.charAt(0) !== '#';
      var body = enabled ? line : line.replace(/^#+\s*/, '');
      var h = body.indexOf('#');
      if (h >= 0) body = body.slice(0, h).trim();
      var toks = body.split(/\s+/).filter(function (x) { return x; });
      if (!toks.length) continue;
      if (!enabled && !isIp(toks[0])) continue;
      if (!isIp(toks[0])) return '第 ' + (i + 1) + ' 行：' + toks[0] + ' 不是合法的 IPv4/IPv6 地址';
      if (toks.length < 2) return '第 ' + (i + 1) + ' 行：IP ' + toks[0] + ' 后面缺少主机名';
      for (var j = 1; j < toks.length; j++) {
        if (!isName(toks[j])) {
          return '第 ' + (i + 1) + ' 行：主机名 ' + toks[j] + ' 含非法字符（只允许字母、数字、. - _）';
        }
      }
    }
    return '';
  }

  // ---------- 渲染 ----------

  function send(name, data) {
    if (efto && efto.ipcMessage) {
      efto.ipcMessage(name, data || {}).catch(function (e) { showError('发送失败：' + e); });
    }
  }

  function showError(text) {
    state.error = text;
    bannerEl.textContent = text;
    bannerEl.className = 'banner bad';
    bannerEl.hidden = !text;
  }

  function renderBanner() {
    if (state.error) {
      bannerEl.textContent = state.error;
      bannerEl.className = 'banner bad';
      bannerEl.hidden = false;
    } else if (state.status) {
      bannerEl.textContent = state.status;
      bannerEl.className = 'banner ok';
      bannerEl.hidden = false;
    } else {
      bannerEl.hidden = true;
    }
  }

  function rowEl(line, index) {
    var li = document.createElement('li');
    li.className = 'row' + (line.enabled ? '' : ' off');

    var check = document.createElement('input');
    check.type = 'checkbox';
    check.checked = !!line.enabled;
    check.title = line.enabled ? '取消勾选 = 写成注释（禁用）' : '勾选 = 启用';
    check.setAttribute('aria-label', '启用此映射');
    check.addEventListener('change', function () {
      line.enabled = check.checked;
      li.className = 'row' + (line.enabled ? '' : ' off');
      dirty = true;
    });
    li.appendChild(check);

    var ip = document.createElement('input');
    ip.className = 'ip';
    ip.value = line.ip;
    ip.spellcheck = false;
    ip.setAttribute('aria-label', 'IP 地址');
    ip.addEventListener('input', function () { line.ip = ip.value.trim(); dirty = true; });
    li.appendChild(ip);

    var names = document.createElement('input');
    names.className = 'names';
    names.value = line.names.join(' ');
    names.spellcheck = false;
    names.setAttribute('aria-label', '主机名');
    names.addEventListener('input', function () {
      line.names = names.value.split(/\s+/).filter(function (x) { return x; });
      dirty = true;
    });
    li.appendChild(names);

    var comment = document.createElement('input');
    comment.className = 'comment';
    comment.value = line.comment || '';
    comment.placeholder = '备注';
    comment.spellcheck = false;
    comment.setAttribute('aria-label', '备注');
    comment.addEventListener('input', function () { line.comment = comment.value; dirty = true; });
    li.appendChild(comment);

    var del = document.createElement('button');
    del.className = 'btn ghost small';
    del.textContent = '删除';
    del.setAttribute('aria-label', '删除此映射');
    del.addEventListener('click', function () {
      model.splice(index, 1);
      dirty = true;
      renderRows();
      markDirty();
    });
    li.appendChild(del);

    return li;
  }

  function renderRows() {
    rowsEl.innerHTML = '';
    var count = 0;
    for (var i = 0; i < model.length; i++) {
      if (model[i].kind !== 'entry') continue;
      rowsEl.appendChild(rowEl(model[i], i));
      count++;
    }
    emptyEl.hidden = count > 0;
  }

  function markDirty() {
    dotEl.className = 'dot' + (dirty ? ' dirty' : '');
    if (tab === 'list') statusEl.textContent = dirty ? '有未保存的修改' : '';
  }

  function loadModel(text) {
    model = parse(text);
    loadedText = text == null ? '' : String(text);
    dirty = false;
    renderRows();
    markDirty();
  }

  function render(next) {
    state = next || state;

    pathEl.textContent = state.path || '';
    pathEl.title = state.path || '';
    encEl.textContent = state.encoding === 'ansi' ? 'ANSI' : 'UTF-8';
    encEl.title = state.encoding === 'ansi'
      ? '保存时按系统 ANSI 代码页写回（与读取时一致）'
      : '保存时按 UTF-8 写回';

    saveEl.disabled = !!state.saving;
    saveEl.textContent = state.saving ? '保存中…' : '保存';
    reloadEl.disabled = !!state.saving;

    if (saveInFlight && !state.saving) {
      saveInFlight = false;
      // 保存成功或失败都已经有最终文本：失败时保留本地改动
      if (!state.error) loadModel(state.text);
    }
    if (!dirty && !saveInFlight) {
      loadModel(state.text);
    }
    if (tab === 'raw' && !dirty) textareaEl.value = loadedText;
    renderBanner();
    markDirty();
  }

  function switchTab(next) {
    if (next === tab) return;
    if (next === 'raw') {
      textareaEl.value = serialize();
    } else {
      model = parse(textareaEl.value);
      renderRows();
    }
    tab = next;
    listEl.hidden = tab !== 'list';
    rawEl.hidden = tab !== 'raw';
    tabListEl.className = 'tab' + (tab === 'list' ? ' on' : '');
    tabRawEl.className = 'tab' + (tab === 'raw' ? ' on' : '');
    tabListEl.setAttribute('aria-selected', tab === 'list' ? 'true' : 'false');
    tabRawEl.setAttribute('aria-selected', tab === 'raw' ? 'true' : 'false');
    markDirty();
  }

  function currentText() {
    return tab === 'raw' ? textareaEl.value : serialize();
  }

  // ---------- 交互 ----------

  saveEl.addEventListener('click', function () {
    var text = currentText();
    var problem = validateText(text);
    if (problem) {
      showError(problem + '。已取消保存。');
      return;
    }
    state.error = '';
    state.status = '正在请求管理员权限…';
    renderBanner();
    saveInFlight = true;
    saveEl.disabled = true;
    saveEl.textContent = '保存中…';
    send('save', { text: text });
  });

  reloadEl.addEventListener('click', function () {
    if (dirty && !window.confirm('放弃未保存的修改并重新读取 hosts？')) return;
    dirty = false;
    send('reload');
  });

  tabListEl.addEventListener('click', function () { switchTab('list'); });
  tabRawEl.addEventListener('click', function () { switchTab('raw'); });

  textareaEl.addEventListener('input', function () {
    dirty = true;
    markDirty();
  });

  addEl.addEventListener('click', function () {
    var ip = newIpEl.value.trim();
    var names = newNamesEl.value.split(/\s+/).filter(function (x) { return x; });
    var comment = newCommentEl.value.trim();

    if (!isIp(ip)) { showError('IP 地址不合法：' + (ip || '（空）')); return; }
    if (!names.length) { showError('至少填一个主机名'); return; }
    for (var i = 0; i < names.length; i++) {
      if (!isName(names[i])) { showError('主机名 ' + names[i] + ' 含非法字符（只允许字母、数字、. - _）'); return; }
    }
    model.push({ kind: 'entry', enabled: true, ip: ip, names: names, comment: comment });
    dirty = true;
    newIpEl.value = '';
    newNamesEl.value = '';
    newCommentEl.value = '';
    state.error = '';
    renderBanner();
    renderRows();
    markDirty();
    newIpEl.focus();
  });

  [newIpEl, newNamesEl, newCommentEl].forEach(function (el) {
    el.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { e.preventDefault(); addEl.click(); }
    });
  });

  efto.onMessage('state', render);
  send('init');
})();
