'use strict';
// 网页快开（main 侧）：触发词 → URL → 系统默认浏览器。
//
// 这里只做三件事：
//   1. 触发词 → URL（内置表在 lib/sites.js，自定义的表存在 kv 里）
//   2. 带关键词且站点有 search 时拼站内搜索，否则打开主页
//   3. efto.openExternal 交给系统浏览器打开
//
// 运行在 efto 的内嵌 V8：没有 Node、没有 DOM、没有 fetch，只能用 efto.*。

var builtin = require('./lib/sites');

// 自定义站点：{ name, url }（name 就是触发词后的参数）
function loadCustom() {
  try {
    var v = efto.kv.get('custom_sites');
    return Array.isArray(v) ? v : [];
  } catch (e) {
    return [];
  }
}

// 被用户删掉的内置站点：存 code（没有 code 时退回 cmd），比较时忽略大小写。
// 旧版本的 code 形如 islide@web，这里统一去掉 @分组 后缀，避免升级后已删的站点又冒出来。
function normalizeKey(v) {
  return String(v == null ? '' : v).trim().toLowerCase().split('@')[0];
}

function loadHidden() {
  try {
    var v = efto.kv.get('hidden_sites');
    if (!Array.isArray(v)) return [];
    var out = [];
    for (var i = 0; i < v.length; i++) {
      var key = normalizeKey(v[i]);
      if (key && out.indexOf(key) < 0) out.push(key);
    }
    return out;
  } catch (e) {
    return [];
  }
}

function siteKey(site) {
  return normalizeKey(site.code || site.cmd);
}

function visibleBuiltin() {
  var hidden = loadHidden();
  return builtin.sites.filter(function (s) { return hidden.indexOf(siteKey(s)) < 0; });
}

// 自定义项同步到启动器搜索：回车后 name 会作为 cmd 回到 onPluginEnter。
function syncSearchItems() {
  var custom = loadCustom();
  var items = [];
  for (var i = 0; i < custom.length; i++) {
    var name = String(custom[i].name || '').trim();
    if (name) items.push({ title: name, cmd: name });
  }
  efto.setSearchItems(items);
}

function saveCustom(list) {
  efto.kv.set('custom_sites', Array.isArray(list) ? list : []);
  syncSearchItems();
}

function triggerOf(site) {
  return String(site.cmd || '').trim().toLowerCase();
}

function findByTrigger(key) {
  key = String(key || '').trim().toLowerCase();
  var visible = visibleBuiltin();
  for (var i = 0; i < visible.length; i++) {
    if (triggerOf(visible[i]) === key) return { site: visible[i], custom: false };
  }
  var custom = loadCustom();
  for (var j = 0; j < custom.length; j++) {
    if (String(custom[j].name || '').trim().toLowerCase() === key) {
      return { site: { cmd: custom[j].name, url: custom[j].url, group: 'custom' }, custom: true };
    }
  }
  return null;
}

function fillQuery(tpl, query) {
  var q = query == null ? '' : String(query);
  if (tpl.indexOf('{query}') < 0) return tpl;
  return tpl.split('{query}').join(encodeURIComponent(q.trim()));
}

/** 告诉用户该输入点什么，同时把搜索框留着别收。 */
function hint(title, subtitle, copy) {
  efto.output([{ title: title, subtitle: subtitle, copy: copy || title }]);
}

function open(url) {
  try {
    efto.openExternal(url);
    return true;
  } catch (e) {
    hint('打开失败：' + ((e && e.message) ? e.message : String(e)), url, url);
    return false;
  }
}

/** 「网页快开 xxx」/「web xxx」：按名字打开自定义站点；没有参数就开管理界面。 */
function manage(arg) {
  var name = String(arg == null ? '' : arg).trim();
  if (!name) {
    efto.showUI({ title: '网页快开 · 管理', width: 860, height: 620 });
    return;
  }
  var hit = findByTrigger(name);
  if (!hit) {
    var custom = loadCustom();
    hint('没有这个快开项：' + name,
      custom.length ? ('已自定义 ' + custom.length + ' 个，可输入「网页快开」查看') : '还没有自定义项，输入「网页快开」添加',
      name);
    return;
  }
  open(hit.site.url);
}

/** 「添加快开 名称 网址」：新增或更新一条自定义快开。 */
function addCustom(arg) {
  var text = String(arg == null ? '' : arg).trim();
  var m = text.match(/^(\S+)\s+(https?:\/\/\S+)$/i);
  if (!m) {
    hint('添加快开：需要「名称 网址」两段',
      '例如：添加快开 我的博客 https://example.com',
      '添加快开 ');
    return;
  }
  var name = m[1].trim();
  var url = m[2].trim();
  var custom = loadCustom();
  for (var i = 0; i < custom.length; i++) {
    if (String(custom[i].name).toLowerCase() === name.toLowerCase()) {
      custom[i].url = url;
      saveCustom(custom);
      hint('已更新快开项：' + name, url, name);
      return;
    }
  }
  custom.push({ name: name, url: url });
  saveCustom(custom);
  hint('已添加快开项：' + name, url + '（之后输入「' + name + '」即可打开）', name);
}

efto.onPluginEnter(function (info) {
  var cmd = info && info.code ? String(info.code) : '';
  var payload = info && info.payload;
  var key = cmd.trim().toLowerCase();

  if (key === '添加快开' || key === 'addweb') {
    addCustom(payload);
    return;
  }

  if (key === '网页快开' || key === 'web' || key === 'quickweb') {
    manage(payload);
    return;
  }

  var hit = findByTrigger(cmd);
  if (!hit) {
    // 清单里的内置触发词是静态的：在管理界面删掉后，启动器仍可能搜到这条，
    // 这里给出准确提示，而不是笼统地说"没有这个快开项"。
    for (var i = 0; i < builtin.sites.length; i++) {
      if (triggerOf(builtin.sites[i]) === key) {
        hint('「' + builtin.sites[i].cmd + '」已从网页快开中删除',
          '可在「网页快开」管理界面重新添加为自定义快开',
          builtin.sites[i].cmd);
        return;
      }
    }
    // 触发词可能来自旧清单或已删除的自定义项：明确告知，不要静默无动作。
    hint('没有这个快开项：' + (cmd || '（空）'),
      '输入「网页快开」查看全部，或用「添加快开 名称 网址」新增',
      cmd || '网页快开');
    return;
  }

  var site = hit.site;
  var query = typeof payload === 'string' ? payload.trim() : '';
  // 带关键词且有搜索地址时走站内搜索，否则打开主页。
  var target = query && site.search ? fillQuery(site.search, query) : site.url;

  if (!target) {
    hint(site.cmd + '：请在该触发词后输入关键词',
      '例如：' + site.cmd + ' 关键词',
      site.cmd + ' ');
    return;
  }
  open(target);
});

// UI 页面用：拉站点数据、增删改自定义项
efto.onMainMessage(function (msg) {
  if (!msg) return;
  var data = (msg && msg.data) || {};

  if (msg.name === 'sites') {
    efto.ipcMessage('sites', {
      builtin: visibleBuiltin(),
      groups: builtin.groups,
      groupLabels: builtin.groupLabels,
      custom: loadCustom()
    });
    return;
  }

  if (msg.name === 'builtin-delete') {
    var code = String(data.code == null ? '' : data.code).trim().toLowerCase();
    var target = null;
    for (var i = 0; i < builtin.sites.length; i++) {
      if (siteKey(builtin.sites[i]) === code) { target = builtin.sites[i]; break; }
    }
    if (!target) {
      efto.ipcMessage('saved', { ok: false, error: '没有这个内置项' });
      return;
    }
    var hidden = loadHidden();
    var key = siteKey(target);
    if (hidden.indexOf(key) < 0) hidden.push(key);
    efto.kv.set('hidden_sites', hidden);
    efto.ipcMessage('saved', { ok: true, count: hidden.length });
    return;
  }

  if (msg.name === 'custom-save') {
    var list = Array.isArray(data.list) ? data.list : [];
    // 清洗：只接受非空 name 与 http(s) url，其余丢掉（不静默保留脏数据）
    var clean = [];
    for (var i = 0; i < list.length; i++) {
      var it = list[i] || {};
      var name = String(it.name == null ? '' : it.name).trim();
      var url = String(it.url == null ? '' : it.url).trim();
      if (!name || !/^https?:\/\//i.test(url)) continue;
      clean.push({ name: name, url: url });
    }
    saveCustom(clean);
    efto.ipcMessage('saved', { ok: true, count: clean.length });
    return;
  }

  if (msg.name === 'open') {
    var u = String(data.url || '');
    if (/^https?:\/\//i.test(u)) efto.openExternal(u);
  }
});

// 启动时把已保存的自定义项重新挂进搜索（数据恢复 / 重启宿主后也能搜到）。
syncSearchItems();
