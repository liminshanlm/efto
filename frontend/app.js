// efto 主窗口逻辑。不使用任何打包器与 npm 依赖：
// IPC 直接走 Tauri 注入到每个 webview 的 __TAURI_INTERNALS__。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var q = document.getElementById('q');
  var list = document.getElementById('results');
  var empty = document.getElementById('empty');
  var status = document.getElementById('status');
  var toastEl = document.getElementById('toast');

  // i18n.js 可能因静态资源缺失而没加载：用兜底实现保证界面仍可用
  var i18n = window.eftoI18n || {
    t: function (k, fb) { return fb != null ? fb : k; },
    tf: function (k, vars, fb) { return fb != null ? fb : k; },
    ready: Promise.resolve(null)
  };
  function T(key, fallback) { return i18n.t(key, fallback); }
  function TF(key, vars, fallback) { return i18n.tf(key, vars, fallback); }

  var hits = [];
  var active = 0;
  var timer = null;
  var busy = false;
  // 窗口操作二级菜单：点「平铺窗口」后，结果列表换成窗口操作项
  var windowOpsActive = false;
  // 文件检索单独防抖/作废：es 是子进程，晚到的结果只补在应用条目后面
  var fileTimer = null;
  var fileSeq = 0;
  // 默认面板分组标题（后端只给 group id）
  function groupLabel(g) {
    return T('main.group.' + g, g);
  }
  // 常用工具的图标：内联 SVG，离线可用且跟随当前主题色
  var TOOL_ICONS = {
    screenshot:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<path d="M4 8.5V6a2 2 0 0 1 2-2h2.5M15.5 4H18a2 2 0 0 1 2 2v2.5M20 15.5V18a2 2 0 0 1-2 2h-2.5M8.5 20H6a2 2 0 0 1-2-2v-2.5"/>' +
      '<circle cx="12" cy="12" r="3.2"/></svg>',
    clipboard:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<rect x="6" y="5" width="12" height="15.5" rx="2"/>' +
      '<path d="M9.5 5V3.6h5V5"/><path d="M9.5 10.5h5M9.5 14h5M9.5 17.5h3"/></svg>',
    lock_screen:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<rect x="4.5" y="10.5" width="15" height="9.5" rx="2"/>' +
      '<path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/><circle cx="12" cy="15.2" r="1.2"/></svg>',
    tile_windows:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<rect x="3.5" y="4" width="7.5" height="7" rx="1.4"/>' +
      '<rect x="13" y="4" width="7.5" height="7" rx="1.4"/>' +
      '<rect x="3.5" y="13" width="7.5" height="7" rx="1.4"/>' +
      '<rect x="13" y="13" width="7.5" height="7" rx="1.4"/></svg>',
    shortcut_guide:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<rect x="3" y="6" width="18" height="12" rx="2.2"/>' +
      '<path d="M6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M7.5 13.5h9"/></svg>',
    snippets:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<path d="M7 4.5h7.5L19 9v10.5a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 6 19.5v-14A1 1 0 0 1 7 4.5Z"/>' +
      '<path d="M14 4.5V9h4.5M9 13h6M9 16.5h4"/></svg>',
    file_buffer:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<path d="M4 7.5h6l1.6 2H20v9a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-11Z"/>' +
      '<path d="M12 12.5v5M9.6 15h4.8"/></svg>',
    color_picker:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<path d="M14.8 4.4l4.8 4.8M12.9 6.3l4.8 4.8M4.2 19.8l3.2-.7 9.1-9.1a2.1 2.1 0 0 0 0-3l-1.4-1.4a2.1 2.1 0 0 0-3 0l-9.1 9.1-.8 5.1Z"/></svg>',
    sticky_note:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">' +
      '<path d="M5 4.5h14v10.2L13.7 20H5V4.5Z"/>' +
      '<path d="M19 14.7h-5.3V20"/><path d="M8 9h8M8 12.5h5"/></svg>'
  };
  // 系统图标缓存：path -> data URL，null 表示这次没取到（未到上限时下次重试）
  var iconCache = {};
  var iconPending = {};
  var iconFailures = {};
  var ICON_MAX_TRIES = 3;
  // 无 UI 插件：先等插件输出，超时没等到才收起搜索框。
  var outputSeq = 0;
  var outputTimer = null;
  // Shift 预览：按住 Shift 显示当前选中项的预览，松手不关；
  // 预览已打开时再按一次 Shift 则关闭它（本次按住期间不再重开）。
  var shiftHeld = false;
  var previewKey = null;
  // 预览窗口当前是否处于打开状态
  var previewOpen = false;
  // 本次按住 Shift 已经用来关闭预览了：松手前不要再打开
  var previewSuppressed = false;
  // Shift 也可能只是组合键的修饰键（Shift+A 输入大写、Shift+方向键选词）：
  // 按下后先等一小会儿，确认没有别的键参与再弹预览。
  var previewTimer = null;
  var PREVIEW_HOLD_DELAY_MS = 180;
  // 搜索框命令执行中：Esc 优先取消命令而不是清空输入
  var commandRunning = false;
  var commandCancelled = false;

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  // Rust 侧 emit 的 payload 可能是字符串，需要兜底解析
  function parsePayload(p) {
    if (typeof p !== 'string') return p;
    try { return JSON.parse(p); } catch (e) { return p; }
  }

  function listen(name, handler) {
    var cb = I.transformCallback(function (ev) {
      handler(parsePayload(ev && ev.payload));
    }, false);
    // target 必须给：Tauri 2 的 EventTarget 是 `{kind:"Any"}` 结构，漏了订阅直接失败
    // （报错只在 webview 控制台，宿主日志里看不到 → 表现为"事件永远收不到"）
    return invoke('plugin:event|listen', {
      event: name,
      target: { kind: 'Any' },
      handler: cb
    });
  }

  // ---------- Shift 预览 ----------
  // 选中项变化时同步预览窗口；同一条目不重复请求（避免每帧重读文件）。
  function syncPreview() {
    if (!shiftHeld || previewSuppressed) return;
    var hit = hits[active];
    if (!hit) {
      if (previewKey) {
        previewKey = null;
        invoke('preview_hide').catch(function () {});
      }
      return;
    }
    if (hit.id === previewKey && previewOpen) return;
    previewKey = hit.id;
    invoke('preview_show', { hit: hit })
      .then(function (ok) {
        // 不支持预览（exe / 目录 / 插件动作…）：静默，下次切换重新判定
        if (!ok) {
          if (previewKey === hit.id) previewKey = null;
          return;
        }
        // 弹出期间用户又按了别的键（Shift 组合）或关掉了预览：把窗口收回去
        if (previewSuppressed) {
          if (previewKey === hit.id) previewKey = null;
          invoke('preview_hide').catch(function () {});
          return;
        }
        if (previewKey === hit.id) previewOpen = true;
      })
      .catch(function () {
        if (previewKey === hit.id) previewKey = null;
      });
  }

  function stopPreview() {
    // 松手不关窗：预览留在屏幕上，由预览窗口自己的"关闭"按钮收起。
    // 但要告诉宿主松开 Shift 了，否则主窗口的失焦隐藏会一直被豁免。
    if (!shiftHeld) return;
    shiftHeld = false;
    previewSuppressed = false;
    clearTimeout(previewTimer);
    previewTimer = null;
    invoke('preview_release').catch(function () {});
  }

  var toastTimer = null;
  function toast(text, isError) {
    toastEl.textContent = text;
    toastEl.hidden = false;
    toastEl.className = 'toast' + (isError ? ' error' : '');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { toastEl.hidden = true; }, 2600);
  }

  // ---------- 渲染 ----------
  function sourceLabel(hit) {
    if (hit.source === 'start_menu') return T('main.source.start_menu', '开始菜单');
    if (hit.source === 'desktop') return T('main.source.desktop', '桌面');
    if (hit.source === 'custom') return T('main.source.custom', '自定义目录');
    if (hit.source === 'registry') return T('main.source.registry', '已安装');
    if (hit.source === 'dir_scan') return T('main.source.dir_scan', '程序目录');
    if (hit.source === 'manual') return T('main.source.manual', '系统工具');
    if (hit.source === 'action') return T('main.source.action', '指令');
    if (hit.source === 'winop') return T('main.source.winop', '窗口操作');
    if (hit.source === 'calc') return T('main.source.calc', '计算 · 单击复制');
    if (hit.source === 'clock') return T('main.source.clock', '系统时间 · 单击复制');
    if (hit.source === 'everything') return T('main.source.everything', 'Everything · 双击打开');
    if (hit.source === 'bookmark:chrome') return T('main.source.bookmark.chrome', 'Chrome 收藏夹');
    if (hit.source === 'bookmark:edge') return T('main.source.bookmark.edge', 'Edge 收藏夹');
    if (hit.source === 'time') return T('main.source.time', '时间戳 · 单击复制');
    if (hit.source === 'command') return T('main.source.command', '命令 · Enter 执行');
    if (hit.source === 'command:out') return T('main.source.command.out', '输出 · 单击复制');
    if (hit.source === 'command:err') return T('main.source.command.err', '错误 · 单击复制');
    if (hit.source === 'snippet') return T('main.source.snippet', '片段 · Enter 粘贴');
    if (hit.source === 'websearch') return T('main.source.websearch', '网页搜索 · Enter 打开');
    if (hit.source === 'system') return T('main.source.system', '系统命令 · Enter 执行');
    if (hit.kind === 'output') return T('main.source.output', '插件输出 · 单击复制');
    if (hit.source === 'url') return T('main.source.url', 'URL');
    if (hit.source === 'path') return T('main.source.path', '路径');
    if (hit.source === 'uri') return T('main.source.uri', '系统');
    if (hit.source && hit.source.indexOf('plugin:') === 0) return T('main.source.plugin', '插件');
    return hit.source || '';
  }

  function highlight(title, query) {
    var idx = title.toLowerCase().indexOf(query.toLowerCase().trim());
    if (idx < 0 || !query.trim()) return escapeHtml(title);
    return (
      escapeHtml(title.slice(0, idx)) +
      '<mark>' + escapeHtml(title.slice(idx, idx + query.trim().length)) + '</mark>' +
      escapeHtml(title.slice(idx + query.trim().length))
    );
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function avatarText(title) {
    var t = (title || '?').trim();
    // 中文名取首字，英文名取首字母大写
    var c = t.charAt(0);
    return /[a-z]/.test(c) ? c.toUpperCase() : c;
  }

  // 只有三个常用工具带内联图标；其他 action / 应用仍用系统图标或首字。
  function toolIcon(hit) {
    var p = hit.payload;
    if (hit.kind !== 'action' || !p || p.type !== 'action') return null;
    return TOOL_ICONS[p.name] || null;
  }

  function avatarHtml(hit) {
    var svg = toolIcon(hit);
    if (svg) return '<span class="avatar tool">' + svg + '</span>';
    return '<span class="avatar">' + escapeHtml(avatarText(hit.title)) + '</span>';
  }

  // 把列表里所有指向同一路径的头像换成系统图标
  function paintIcon(path, url) {
    if (!url) return;
    var rows = list.querySelectorAll('[data-icon]');
    for (var i = 0; i < rows.length; i++) {
      if (rows[i].dataset.icon !== path) continue;
      var avatar = rows[i].querySelector('.avatar');
      if (!avatar || avatar.classList.contains('tool')) continue;
      avatar.textContent = '';
      var img = document.createElement('img');
      img.src = url;
      img.alt = '';
      avatar.appendChild(img);
    }
  }

  function ensureIcon(path) {
    if (!path) return;
    if (Object.prototype.hasOwnProperty.call(iconCache, path)) {
      if (iconCache[path]) { paintIcon(path, iconCache[path]); return; }
      // 失败未到上限：丢掉 null 记录再问一次，避免一次失败永久停在首字头像
      if ((iconFailures[path] || 0) >= ICON_MAX_TRIES) return;
      delete iconCache[path];
    }
    if (iconPending[path]) return;
    iconPending[path] = true;
    invoke('icon_for', { path: path })
      .then(function (url) {
        iconCache[path] = url || null;
        if (url) delete iconFailures[path];
        else iconFailures[path] = (iconFailures[path] || 0) + 1;
        delete iconPending[path];
        paintIcon(path, iconCache[path]);
      })
      .catch(function () {
        iconCache[path] = null;
        iconFailures[path] = (iconFailures[path] || 0) + 1;
        delete iconPending[path];
      });
  }

  // 默认面板（空查询）：每组一排横向卡片，第一排最近使用，第二排常用工具。
  function renderPanel() {
    var order = [];
    var byGroup = {};
    hits.forEach(function (hit, i) {
      var g = hit.group || '';
      if (!byGroup[g]) { byGroup[g] = []; order.push(g); }
      byGroup[g].push({ hit: hit, index: i });
    });
    order.forEach(function (g) {
      var sec = document.createElement('li');
      sec.className = 'panel-section';
      var title = document.createElement('div');
      title.className = 'section-title';
      var label = document.createElement('span');
      label.textContent = groupLabel(g);
      title.appendChild(label);
      // 最近使用是历史数据，给一个就地清空的入口。
      if (g === 'recent') {
        var clearBtn = document.createElement('button');
        clearBtn.type = 'button';
        clearBtn.className = 'section-clear';
        clearBtn.textContent = T('main.recent.clear', '清除');
        clearBtn.title = T('main.recent.clearTitle', '清空最近使用');
        clearBtn.addEventListener('click', clearRecent);
        title.appendChild(clearBtn);
      }
      sec.appendChild(title);
      var tiles = document.createElement('div');
      tiles.className = 'tiles';
      // 常用工具的序号就是 Alt+数字 的编号（1 起），只算本组
      var toolNo = 0;
      byGroup[g].forEach(function (entry) {
        if (g === 'tools') toolNo++;
        tiles.appendChild(makeTile(entry.hit, entry.index, g === 'tools' ? toolNo : 0));
      });
      sec.appendChild(tiles);
      list.appendChild(sec);
    });
  }

  // 清空"最近使用"，再重走一次搜索把默认面板刷成最新状态。
  function clearRecent(e) {
    if (e) e.stopPropagation();
    status.textContent = T('main.recent.clearing', '清除中…');
    invoke('clear_recent')
      .then(function () {
        toast(T('main.recent.cleared', '已清空最近使用'));
        search();
      })
      .catch(function (err) {
        status.textContent = T('main.recent.clearFailed', '清除失败');
        toast(T('main.recent.clearFailedDetail', '清除失败：') + err, true);
      });
  }

  // ---------- 常用工具 ----------
  // settings.tools 的当前值（原始条目：内置 id / app:... / plugin:...）
  var pinnedTools = [];

  function loadPinnedTools() {
    invoke('get_settings')
      .then(function (s) { pinnedTools = (s && s.tools) || []; })
      .catch(function () {});
  }

  // 搜索结果里哪些行能加进常用工具：应用与插件。
  // 插件行的 id 是 plugin://name#slug，必须用 payload 里的 plugin_id。
  function toolValue(hit) {
    if (hit.group === 'tools') return null;
    if (hit.kind === 'plugin' && hit.payload && hit.payload.plugin_id) {
      return 'plugin:' + hit.payload.plugin_id;
    }
    if (hit.kind === 'app') return 'app:' + hit.id;
    return null;
  }

  function setAddState(btn, added) {
    btn.textContent = added ? T('main.tile.added', '已加入') : T('main.tile.addLabel', '加入常用工具');
    btn.disabled = added;
  }

  // 追加到末尾：顺序即卡片顺序与 Alt+数字
  function addTool(value, btn) {
    if (pinnedTools.indexOf(value) >= 0) return;
    invoke('get_settings')
      .then(function (s) {
        var tools = (s && s.tools) || [];
        if (tools.indexOf(value) >= 0) {
          pinnedTools = tools;
          if (btn) setAddState(btn, true);
          return null;
        }
        if (tools.length >= 9) {
          toast(T('main.tile.max', '常用工具最多 9 个'), true);
          return null;
        }
        var next = tools.concat([value]);
        return invoke('set_settings', { patch: { tools: next } }).then(function () {
          pinnedTools = next;
          if (btn) setAddState(btn, true);
          toast(T('main.tile.addedToast', '已加入常用工具'));
        });
      })
      .catch(function (e) { toast(T('main.tile.addFailed', '添加失败：') + e, true); });
  }

  function removeTool(value) {
    invoke('get_settings')
      .then(function (s) {
        var tools = (s && s.tools) || [];
        var next = tools.filter(function (v) { return v !== value; });
        if (next.length === tools.length) return null;
        return invoke('set_settings', { patch: { tools: next } }).then(function () {
          pinnedTools = next;
          toast(T('main.tile.removedToast', '已从常用工具移除'));
          search();
        });
      })
      .catch(function (e) { toast(T('main.tile.removeFailed', '移除失败：') + e, true); });
  }

  function makeTile(hit, i, toolNo) {
    var isTool = hit.group === 'tools';
    var btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'tile' + (i === active ? ' active' : '');
    btn.dataset.index = String(i);
    btn.dataset.kind = hit.kind;
    if (hit.icon) btn.dataset.icon = hit.icon;
    var svg = toolIcon(hit);
    if (svg) btn.dataset.tool = hit.payload.name;
    btn.innerHTML = avatarHtml(hit) +
      '<span class="tile-title">' + escapeHtml(hit.title) + '</span>';
    // 常用工具单击即执行；最近使用单击只选中，双击才执行（与列表态一致）。
    btn.title = isTool
      ? (toolNo ? TF('main.tile.runWithKey', { n: toolNo }, '单击执行（Alt+' + toolNo + '）') : T('main.tile.run', '单击执行'))
      : T('main.tile.dblclick', '双击执行（Enter 也可）');
    if (isTool && toolNo && toolNo <= 9) {
      var key = document.createElement('span');
      key.className = 'tile-key';
      key.textContent = String(toolNo);
      btn.appendChild(key);
    }
    if (isTool && hit.pin) {
      var rm = document.createElement('span');
      rm.className = 'tile-remove';
      rm.setAttribute('role', 'button');
      rm.title = T('main.tile.remove', '从常用工具移除');
      rm.textContent = '×';
      rm.addEventListener('click', function (e) {
        e.stopPropagation();
        e.preventDefault();
        removeTool(hit.pin);
      });
      rm.addEventListener('dblclick', function (e) { e.stopPropagation(); });
      btn.appendChild(rm);
    }
    btn.addEventListener('click', function () {
      active = i;
      updateActive();
      if (isTool) run(i);
    });
    btn.addEventListener('dblclick', function () {
      if (!isTool) run(i);
    });
    btn.addEventListener('mousemove', function () {
      if (active !== i) { active = i; updateActive(); }
    });
    return btn;
  }

  // ---------- 文件动作菜单 ----------
  function isFileHit(hit) {
    return !!hit && hit.kind === 'file' && hit.payload && hit.payload.type === 'open' && hit.payload.path;
  }

  var fileMenuEl = null;

  function closeFileMenu() {
    if (fileMenuEl) {
      fileMenuEl.remove();
      fileMenuEl = null;
    }
  }

  function fileAction(path, action) {
    closeFileMenu();
    if (action === 'transfer_copy' || action === 'transfer_move') {
      invoke('file_transfer', { paths: [path], moveit: action === 'transfer_move' })
        .then(function (msg) { toast(msg); search(); })
        .catch(function (e) {
          if (String(e).indexOf('取消') < 0) toast(String(e), true);
        });
      return;
    }
    invoke('file_action', { path: path, action: action })
      .then(function (msg) {
        if (msg) toast(msg);
        if (action === 'delete') search();
      })
      .catch(function (e) { toast(String(e), true); });
  }

  var FILE_ACTIONS = [
    ['open', 'main.file.open', '打开'],
    ['open_with', 'main.file.openWith', '打开方式…'],
    ['reveal', 'main.file.reveal', '在资源管理器中定位'],
    ['copy_path', 'main.file.copyPath', '复制路径'],
    ['copy_file', 'main.file.copyFile', '复制文件'],
    ['add_buffer', 'main.file.addBuffer', '加入文件缓冲'],
    ['transfer_copy', 'main.file.copyTo', '复制到…'],
    ['transfer_move', 'main.file.moveTo', '移动到…'],
    ['delete', 'main.file.delete', '删除到回收站']
  ];

  function openFileMenu(hit, anchor) {
    closeFileMenu();
    var path = hit.payload.path;
    var menu = document.createElement('div');
    menu.className = 'file-menu';
    FILE_ACTIONS.forEach(function (entry) {
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'file-menu-item' + (entry[0] === 'delete' ? ' danger' : '');
      btn.textContent = T(entry[1], entry[2]);
      btn.addEventListener('click', function (e) {
        e.stopPropagation();
        fileAction(path, entry[0]);
      });
      menu.appendChild(btn);
    });
    document.body.appendChild(menu);
    // 菜单右对齐「⋯」按钮，避免从行首盖住标题；下方放不下时翻到按钮上方。
    var rect = anchor ? anchor.getBoundingClientRect() : null;
    var left = rect ? rect.right - menu.offsetWidth : 8;
    var top = rect ? rect.bottom + 4 : 44;
    if (rect && top + menu.offsetHeight > window.innerHeight - 8) {
      top = rect.top - menu.offsetHeight - 4;
    }
    menu.style.left = Math.max(8, Math.min(left, window.innerWidth - menu.offsetWidth - 8)) + 'px';
    menu.style.top = Math.max(8, Math.min(top, window.innerHeight - menu.offsetHeight - 8)) + 'px';
    fileMenuEl = menu;
  }

  document.addEventListener('mousedown', function (e) {
    if (fileMenuEl && !fileMenuEl.contains(e.target)) closeFileMenu();
  });

  function render(query) {
    list.innerHTML = '';
    empty.hidden = hits.length > 0 || !!query.trim();
    // 默认面板的分组标题是"展示用"行：不写 data-index，不参与 active 索引
    var panelMode = isPanel();
    list.className = 'results' + (panelMode ? ' panel-mode' : '');
    if (panelMode) {
      renderPanel();
      // 必须在整块面板挂到 list 之后才补图标：paintIcon 依赖 DOM 查询，
      // 在 makeTile 里调会因节点还没入树而静默丢图（第二次渲染起必现）。
      hits.forEach(function (hit) { if (hit.icon) ensureIcon(hit.icon); });
      syncPreview();
      return;
    }
    var lastGroup = null;
    hits.forEach(function (hit, i) {
      if (hit.group && hit.group !== lastGroup) {
        var head = document.createElement('li');
        head.className = 'group-head';
        head.textContent = hit.group ? groupLabel(hit.group) : '';
        list.appendChild(head);
      }
      lastGroup = hit.group || null;
      var li = document.createElement('li');
      li.className = 'row' + (i === active ? ' active' : '');
      li.dataset.kind = hit.kind;
      li.dataset.source = hit.source || '';
      li.dataset.index = String(i);
      if (hit.icon) li.dataset.icon = hit.icon;
      var svg = toolIcon(hit);
      if (svg) li.dataset.tool = hit.payload.name;
      li.innerHTML =
        avatarHtml(hit) +
        '<span class="meta">' +
        '<span class="title">' + highlight(hit.title, query) + '</span>' +
        '<span class="subtitle">' + escapeHtml(hit.subtitle || '') + '</span>' +
        '</span>' +
        '<span class="badge">' + escapeHtml(sourceLabel(hit)) + '</span>';
      // - 应用 / 插件行右侧给一个"加入常用工具"小按钮，点它不会触发行本身的执行。
      var pinValue = toolValue(hit);
      if (pinValue) {
        var addBtn = document.createElement('button');
        addBtn.type = 'button';
        addBtn.className = 'row-add';
        addBtn.title = T('main.tile.add', '添加到常用工具');
        setAddState(addBtn, pinnedTools.indexOf(pinValue) >= 0);
        addBtn.addEventListener('click', function (e) {
          e.stopPropagation();
          addTool(pinValue, addBtn);
        });
        addBtn.addEventListener('dblclick', function (e) { e.stopPropagation(); });
        li.appendChild(addBtn);
      }
      // 文件行：右侧"⋯"打开动作菜单（Tab 同效），Ctrl+Enter 快速加入文件缓冲。
      if (isFileHit(hit)) {
        var fileBtn = document.createElement('button');
        fileBtn.type = 'button';
        fileBtn.className = 'row-file';
        fileBtn.textContent = '⋯';
        fileBtn.title = T('main.file.menu', '文件动作（Tab）');
        fileBtn.addEventListener('click', function (e) {
          e.stopPropagation();
          active = i;
          updateActive();
          openFileMenu(hit, fileBtn);
        });
        fileBtn.addEventListener('dblclick', function (e) { e.stopPropagation(); });
        li.appendChild(fileBtn);
      }
      // - copy_text 行（计算结果 / 时间戳 / 插件输出）：单击即"选中 + 执行"，因为复制就是动作；
      //   插件输出里 open 类的行不是 copy_text，仍然只选中、双击执行；
      // - 默认面板的常用工具行（group=tools）：单击即执行（用户明确要求"直接执行"）；
      // - 其他条目：单击只选中，双击才执行（避免手滑把程序开起来）；
      // - Enter 始终执行当前选中项，与鼠标行为无关。
      var isCopy = hit.payload && hit.payload.type === 'copy_text' &&
        (hit.kind === 'calc' || hit.kind === 'time' || hit.kind === 'output');
      var isTool = hit.group === 'tools' || windowOpsActive;
      li.addEventListener('click', function () {
        active = i;
        updateActive();
        if (isCopy || isTool) run(i);
      });
      li.addEventListener('dblclick', function () {
        if (!isCopy && !isTool) run(i);
      });
      var keyHint = (!panelMode && i < 9) ? i + 1 : 0;
      li.title = isCopy
        ? (keyHint
          ? TF('main.row.copyWithKey', { n: keyHint }, '单击复制（Alt+' + keyHint + '）')
          : T('main.row.copy', '单击复制'))
        : (isTool
          ? (keyHint
            ? TF('main.tile.runWithKey', { n: keyHint }, '单击执行（Alt+' + keyHint + '）')
            : T('main.tile.run', '单击执行'))
          : (keyHint
            ? TF('main.row.runWithKey', { n: keyHint }, '双击执行（Alt+' + keyHint + ' 或 Enter）')
            : T('main.tile.dblclick', '双击执行（Enter 也可）')));
      li.addEventListener('mousemove', function () {
        if (active !== i) { active = i; updateActive(); }
      });
      list.appendChild(li);
      if (hit.icon) ensureIcon(hit.icon);
    });
    syncPreview();
  }

  function updateActive() {
    var rows = list.querySelectorAll('[data-index]');
    for (var i = 0; i < rows.length; i++) {
      var idx = Number(rows[i].dataset.index);
      var base = rows[i].classList.contains('tile') ? 'tile' : 'row';
      rows[i].className = base + (idx === active ? ' active' : '');
    }
    var cur = list.querySelector('[data-index="' + active + '"]');
    if (cur && cur.scrollIntoView) {
      if (cur.offsetTop < list.scrollTop) {
        list.scrollTop = cur.offsetTop;
      } else if (cur.offsetTop + cur.offsetHeight > list.scrollTop + list.clientHeight) {
        list.scrollTop = cur.offsetTop + cur.offsetHeight - list.clientHeight;
      }
    }
    syncPreview();
  }

  // ---------- 搜索 ----------
  function search() {
    var text = q.value;
    busy = true;
    status.textContent = T('main.status.searching', '搜索中…');
    invoke('search', { query: text })
      .then(function (res) {
        if (windowOpsActive) return;
        hits = res || [];
        active = 0;
        render(text);
        status.textContent = text.trim()
          ? TF('main.status.results', { n: hits.length }, hits.length + ' 条结果')
          : T('main.status.ready', '就绪');
        scheduleFiles(text);
      })
      .catch(function (e) {
        if (windowOpsActive) return;
        hits = [];
        render(text);
        status.textContent = T('main.status.searchFailed', '搜索失败');
        toast(T('main.status.searchFailedDetail', '搜索失败：') + e, true);
      })
      .then(function () { busy = false; });
  }

  // 文件检索走独立异步命令：es 是子进程，不能卡在 UI 线程上；
  // 晚到的文件行只追加在应用结果后面（分数由后端压到应用条目之下）。
  function scheduleFiles(text) {
    var seq = ++fileSeq;
    clearTimeout(fileTimer);
    if (!text.trim()) return;
    // 命令前缀命中时后端已短路；这里再挡一层，省掉一次 es 调用
    if (hits.length && hits[0].kind === 'command') return;
    fileTimer = setTimeout(function () {
      invoke('search_files', { query: text })
        .then(function (res) {
          if (windowOpsActive) return;
          if (seq !== fileSeq || q.value !== text) return;
          var extra = (res || []).filter(function (h) {
            return !hits.some(function (x) { return x.id === h.id; });
          });
          if (!extra.length) return;
          hits = hits.concat(extra);
          if (active >= hits.length) active = hits.length - 1;
          render(text);
          if (!busy) status.textContent = TF('main.status.results', { n: hits.length }, hits.length + ' 条结果');
        })
        .catch(function () {});
    }, 180);
  }

  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(search, 40);
  }

  // ---------- 窗口操作二级菜单 ----------
  function isWindowTool(hit) {
    return !!hit && hit.kind === 'action' && hit.payload &&
      hit.payload.type === 'action' && hit.payload.name === 'tile_windows';
  }

  function winOp(name, titleKey, titleFb, subKey, subFb) {
    return {
      kind: 'action',
      id: 'winop:' + name,
      title: T(titleKey, titleFb),
      subtitle: T(subKey, subFb),
      score: 0,
      reason: '',
      icon: null,
      group: null,
      source: 'winop',
      payload: { type: 'action', name: name }
    };
  }

  // 「平铺窗口」不再直接执行，而是在结果列表里展开窗口操作。
  // 单项热键仍走后端直连，直接网格平铺。
  function openWindowOps() {
    windowOpsActive = true;
    clearTimeout(timer);
    clearTimeout(fileTimer);
    fileSeq++;
    hits = [
      winOp('tile_windows', 'main.winop.grid', '平铺窗口',
        'main.winop.gridSub', '按网格铺满当前显示器'),
      winOp('window_min_others', 'main.winop.minOthers', '最小化其他窗口',
        'main.winop.minOthersSub', '保留搜索框下方的窗口，其余最小化'),
      winOp('window_min_all', 'main.winop.minAll', '最小化全部窗口',
        'main.winop.minAllSub', '所有可见窗口一起收起'),
      winOp('window_half_left', 'main.winop.halfLeft', '左半屏',
        'main.winop.halfLeftSub', '把搜索框下方的窗口放到左半屏'),
      winOp('window_half_right', 'main.winop.halfRight', '右半屏',
        'main.winop.halfRightSub', '把搜索框下方的窗口放到右半屏'),
      winOp('window_undo_tile', 'main.winop.undo', '还原上次平铺',
        'main.winop.undoSub', '恢复上一次平铺前的窗口位置')
    ];
    active = 0;
    render('');
    status.textContent = T('main.winop.title', '窗口操作');
    syncPreview();
  }

  function run(i) {
    var hit = hits[i];
    if (!hit) return;
    var fromMenu = windowOpsActive;
    if (fromMenu) windowOpsActive = false;
    if (!fromMenu && isWindowTool(hit)) {
      openWindowOps();
      return;
    }
    var isCommand = hit.kind === 'command' && hit.payload &&
      hit.payload.type === 'run_command' && hit.payload.command;
    if (isCommand) {
      commandRunning = true;
      commandCancelled = false;
    }
    var seqBefore = outputSeq;
    status.textContent = T('main.status.running', '执行中…');
    invoke('invoke_hit', { hit: hit })
      .then(function (res) {
        var msg = res && typeof res === 'object' ? res.message : res;
        // 后端用这个固定字符串表示"命令已交给后台线程"，不是最终状态
        var running = msg === '执行中…';
        if (msg && !running) toast(msg);
        status.textContent = running
          ? T('main.status.running', '执行中…')
          : (msg || T('main.status.ready', '就绪'));
        // 命令只有内联模式会异步回传输出；终端模式/失败到这里就结束了
        if (isCommand && !running) commandRunning = false;
        // 无 UI 的后端插件：给输出留一个窗口期。输出到了就保留搜索框，
        // 没到（showNotification 这类）才收起。
        if (res && res.wait_output) {
          clearTimeout(outputTimer);
          if (outputSeq !== seqBefore) return;
          outputTimer = setTimeout(function () {
            if (outputSeq !== seqBefore) return;
            invoke('hide_main').catch(function () {});
          }, 700);
        }
      })
      .catch(function (e) {
        if (isCommand) {
          commandRunning = false;
          commandCancelled = false;
        }
        status.textContent = T('main.status.runFailed', '执行失败');
        toast(T('main.status.runFailedDetail', '执行失败：') + e, true);
        // 失败时保持窗口可见，方便看到原因
      });
  }

  function openSettings() {
    invoke('open_settings').catch(function (e) {
      toast(T('main.status.settingsFailed', '打开设置失败：') + e, true);
    });
  }

  // 插件把结果内联到输入框下方：替换当前列表，保持搜索框可见。
  function showPluginOutput(payload) {
    var items = (payload && payload.items) || [];
    hits = items;
    active = 0;
    fileSeq++; // 作废在途的文件检索，别把它混进插件输出
    clearTimeout(fileTimer);
    clearTimeout(outputTimer);
    render('');
    empty.hidden = items.length > 0;
    status.textContent = items.length
      ? TF('main.plugin.outputCount', { plugin: payload.plugin || T('main.source.plugin', '插件'), n: items.length },
          (payload.plugin || '插件') + ' · ' + items.length + ' 条输出')
      : T('main.plugin.noOutput', '插件没有输出');
  }

  // 搜索框命令的结果：stdout / stderr 逐行变成可复制的行，错误行单独标红。
  function showCommandOutput(payload) {
    var p = payload || {};
    if (p.terminal) {
      status.textContent = T('main.command.terminal', '已在终端窗口中执行');
      return;
    }
    var items = [];
    function addLine(line, isError) {
      var text = String(line);
      if (!text.trim()) return;
      items.push({
        kind: 'output',
        id: 'command:' + (isError ? 'err' : 'out') + ':' + items.length,
        title: text,
        subtitle: isError ? T('main.source.command.err', '错误 · 单击复制') : T('main.source.command.out', '输出 · 单击复制'),
        score: 0,
        reason: '',
        icon: null,
        group: null,
        source: isError ? 'command:err' : 'command:out',
        payload: { type: 'copy_text', text: text, echo: { text: text } }
      });
    }
    if (p.ok === false && p.error) {
      addLine(p.error, true);
    } else {
      String(p.stdout || '').split(/\r?\n/).forEach(function (l) { addLine(l, false); });
      String(p.stderr || '').split(/\r?\n/).forEach(function (l) { addLine(l, true); });
    }
    hits = items;
    active = 0;
    fileSeq++; // 作废在途的文件检索
    clearTimeout(fileTimer);
    render('');
    empty.hidden = items.length > 0;
    var head = (p.shell ? p.shell + ' · ' : '') +
      (p.ok === false ? T('main.command.failed', '执行失败') : TF('main.command.exitCode', { code: p.code }, '退出码 ' + p.code));
    status.textContent = items.length
      ? head + ' · ' + TF('main.command.lineCount', { n: items.length }, items.length + ' 行')
      : head;
    if (p.truncated) toast(T('main.command.truncated', '输出过长，已截断'), true);
  }

  function reindex() {
    status.textContent = T('main.status.reindexing', '重建索引中…');
    invoke('refresh_index')
      .then(function (r) {
        toast(TF('main.status.reindexToast', { n: r.merged, ms: r.elapsed_ms },
          '索引已重建：' + r.merged + ' 条（' + r.elapsed_ms + 'ms）'));
        status.textContent = T('main.status.reindexed', '索引已更新');
        search();
      })
      .catch(function (e) {
        status.textContent = T('main.status.reindexFailed', '重建失败');
        toast(T('main.status.reindexFailedDetail', '重建索引失败：') + e, true);
      });
  }

  // ---------- 键盘 ----------
  function isPanel() {
    return hits.some(function (hit) { return !!hit.group; });
  }

  // 默认面板按 group 分排；每排的索引顺序与 renderPanel 的渲染顺序一致。
  function panelRows() {
    var rows = [];
    var byGroup = {};
    hits.forEach(function (hit, i) {
      var g = hit.group || '';
      if (!byGroup[g]) { byGroup[g] = []; rows.push(byGroup[g]); }
      byGroup[g].push(i);
    });
    return rows;
  }

  // 左右在本排内移动（到边界停住），上下换排并尽量停在原来的列。
  function movePanel(dx, dy) {
    var rows = panelRows();
    if (!rows.length) return;
    var row = 0;
    var col = 0;
    for (var r = 0; r < rows.length; r++) {
      var c = rows[r].indexOf(active);
      if (c >= 0) { row = r; col = c; break; }
    }
    if (dy) {
      row = (row + dy + rows.length) % rows.length;
      col = Math.min(col, rows[row].length - 1);
    } else {
      col = Math.max(0, Math.min(rows[row].length - 1, col + dx));
    }
    active = rows[row][col];
    updateActive();
  }

  function scrollList(delta) {
    if (!hits.length) return;
    active = (active + delta + hits.length) % hits.length;
    updateActive();
  }

  // Alt+N 对应常用工具第 N 张卡片（1 起）；没有就返回 -1。
  function toolHitIndex(n) {
    var seen = 0;
    for (var i = 0; i < hits.length; i++) {
      if (hits[i].group !== 'tools') continue;
      seen++;
      if (seen === n) return i;
    }
    return -1;
  }

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Shift') {
      if (e.repeat) return;
      // Shift 是开关：预览没开就打开当前选中项；已经开着就关掉
      // （不阻止默认行为，不影响大写输入）
      shiftHeld = true;
      if (previewOpen) {
        previewOpen = false;
        // 本次按住继续按住时（比如接着按方向键）不要再自动打开
        previewSuppressed = true;
        previewKey = null;
        clearTimeout(previewTimer);
        previewTimer = null;
        invoke('preview_hide').catch(function () {});
      } else {
        // Shift 也常被当修饰键用（Shift+A 输入大写、Shift+方向键选词）：
        // 先等一小会儿，期间没有别的键按下才弹预览。
        previewSuppressed = false;
        clearTimeout(previewTimer);
        previewTimer = setTimeout(function () {
          previewTimer = null;
          if (!shiftHeld || previewSuppressed) return;
          syncPreview();
        }, PREVIEW_HOLD_DELAY_MS);
      }
      return;
    }
    if (shiftHeld) {
      // Shift 和其他键一起输入：本次按住期间不弹预览，已经弹出的也收掉
      clearTimeout(previewTimer);
      previewTimer = null;
      previewSuppressed = true;
      if (previewOpen) {
        previewOpen = false;
        previewKey = null;
        invoke('preview_hide').catch(function () {});
      }
    }
    // 文件动作：Tab 打开菜单，Ctrl+Enter 快速加入文件缓冲。
    if (e.key === 'Tab') {
      var fileHit = hits[active];
      if (isFileHit(fileHit)) {
        e.preventDefault();
        openFileMenu(
          fileHit,
          list.querySelector('[data-index="' + active + '"] .row-file')
        );
        return;
      }
    }
    if (e.key === 'Enter' && e.ctrlKey) {
      var target = hits[active];
      if (isFileHit(target)) {
        e.preventDefault();
        fileAction(target.payload.path, 'add_buffer');
        return;
      }
    }
    if (e.key === 'Escape' && fileMenuEl) {
      e.preventDefault();
      closeFileMenu();
      return;
    }
    if (e.altKey && /^[1-9]$/.test(e.key)) {
      // 默认面板：Alt+N 对应第 N 张常用工具卡片（保持原语义）；
      // 搜索结果：Alt+N 执行第 N 行，列表顺序就是 hits 顺序。
      var altIndex = isPanel() ? toolHitIndex(Number(e.key)) : Number(e.key) - 1;
      if (altIndex >= 0 && altIndex < hits.length) {
        e.preventDefault();
        run(altIndex);
        return;
      }
    }
    if (e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n')) {
      e.preventDefault();
      if (isPanel()) movePanel(0, 1); else scrollList(1);
    } else if (e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p')) {
      e.preventDefault();
      if (isPanel()) movePanel(0, -1); else scrollList(-1);
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      // 列表态没有"行"的概念，不拦截，保留浏览器默认行为
      if (!isPanel()) return;
      e.preventDefault();
      movePanel(e.key === 'ArrowRight' ? 1 : -1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (hits.length) run(active);
      else if (q.value.trim()) {
        // 无结果时，把输入本身当路径/URL 再试一次
        invoke('invoke_hit', {
          hit: {
            kind: 'external', id: 'q:' + q.value, title: q.value, subtitle: '', score: 0,
            reason: '', icon: null, source: 'path',
            payload: { type: 'open', path: q.value }
          }
        }).catch(function (err) { toast(T('main.status.openFailed', '无法打开：') + err, true); });
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      // 命令执行中：Esc 的语义是取消命令，不是清空输入
      if (commandRunning) {
        commandCancelled = true;
        invoke('cancel_command').catch(function () {});
        status.textContent = T('main.status.cancelling', '正在取消…');
        setTimeout(function () {
          if (commandCancelled) {
            commandRunning = false;
            commandCancelled = false;
            status.textContent = T('main.status.cancelled', '已取消');
          }
        }, 1500);
        return;
      }
      if (windowOpsActive) { windowOpsActive = false; q.value = ''; search(); return; }
      if (q.value) { q.value = ''; schedule(); return; }
      invoke('hide_main').catch(function () {});
    } else if (e.altKey && (e.key === 'r' || e.key === 'R')) {
      e.preventDefault(); reindex();
    } else if (e.altKey && (e.key === 'p' || e.key === 'P')) {
      e.preventDefault(); openSettings();
    }
  });

  document.addEventListener('keyup', function (e) {
    if (e.key === 'Shift') stopPreview();
  });

  q.addEventListener('input', function () {
    closeFileMenu();
    clearTimeout(outputTimer);
    windowOpsActive = false;
    // 清空输入就回到默认面板（最近使用 + 常用工具）
    if (!q.value.trim()) { hits = []; active = 0; render(''); empty.hidden = false; status.textContent = T('main.status.ready', '就绪'); search(); return; }
    schedule();
  });

  document.getElementById('btn-settings').addEventListener('click', openSettings);

  // ---------- 与 Rust 的事件 ----------
  function focusInput() {
    q.focus();
    q.select();
  }

  if (I && I.transformCallback) {
    listen('main:show', function () {
      // 主窗口重新出现（Esc/热键）时清掉预览状态，避免 Shift 卡在按下态
      closeFileMenu();
      windowOpsActive = false;
      clearTimeout(previewTimer);
      previewTimer = null;
      shiftHeld = false;
      previewKey = null;
      previewOpen = false;
      previewSuppressed = false;
      focusInput();
      if (!q.value.trim()) search();
    });
    listen('preview:closed', function () {
      // 用户在预览窗口点了"关闭"：清掉缓存，下次按 Shift 才会重新打开
      clearTimeout(previewTimer);
      previewTimer = null;
      shiftHeld = false;
      previewKey = null;
      previewOpen = false;
    });
    listen('plugin:subtitle', function (text) {
      status.textContent = String(text || '');
    });
    listen('plugin:output', function (payload) {
      outputSeq++;
      showPluginOutput(payload);
    });
    listen('command:result', function (payload) {
      var wasCancelled = commandCancelled;
      commandRunning = false;
      commandCancelled = false;
      if (wasCancelled && payload && payload.error) {
        status.textContent = T('main.status.cancelled', '已取消');
        return;
      }
      showCommandOutput(payload);
    });
    listen('plugin:error', function (p) {
      if (!p) return;
      toast(TF('main.plugin.error', { plugin: p.plugin || '', message: p.message || '' },
        '插件 ' + (p.plugin || '') + ' 出错：' + (p.message || '')), true);
    });
    listen('main:toast', function (p) {
      toast(typeof p === 'string' ? p : (p && p.text) || '', p && p.error);
    });
    // 语言在设置窗口切换后由后端广播：整页重载，让 i18n 重新加载词典
    listen('settings:language', function () { location.reload(); });
  } else {
    status.textContent = T('main.toast.ipc', 'IPC 未就绪：请通过 tauri 构建运行');
  }

  // 首次进入展示"最近使用"面板；等词典就绪再渲染，避免动态文案停留在中文
  i18n.ready.then(function () {
    loadPinnedTools();
    search();
  });
})();
