// efto 设置窗口。无打包器，IPC 直接走 Tauri 注入的 __TAURI_INTERNALS__。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  function $(id) { return document.getElementById(id); }

  // i18n.js 缺失时退化为"用中文兜底"，设置页仍可操作
  var i18n = window.eftoI18n || {
    t: function (k, fb) { return fb != null ? fb : k; },
    tf: function (k, vars, fb) { return fb != null ? fb : k; },
    ready: Promise.resolve(null)
  };
  function T(key, fallback) { return i18n.t(key, fallback); }
  function TF(key, vars, fallback) { return i18n.tf(key, vars, fallback); }

  function esc(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  var toastTimer = null;
  function toast(text, isError) {
    var el = $('toast');
    el.textContent = text;
    el.hidden = false;
    el.className = 'toast' + (isError ? ' error' : '');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { el.hidden = true; }, 2600);
  }

  // ---------- 导航 ----------
  document.querySelectorAll('.nav-item').forEach(function (btn) {
    btn.addEventListener('click', function () {
      document.querySelectorAll('.nav-item').forEach(function (b) {
        b.classList.toggle('active', b === btn);
      });
      var page = btn.dataset.page;
      document.querySelectorAll('.page').forEach(function (p) {
        p.hidden = p.id !== 'page-' + page;
      });
      document.querySelector('.main').scrollTop = 0;
      if (page === 'plugins') loadPlugins();
      if (page === 'runtime') loadRuntime();
      if (page === 'snippets') loadSnippets();
    });
  });

  /// 主窗口的"片段"内置动作会发 `settings:page`，直接切到对应页。
  function listen(name, handler) {
    if (!I || !I.transformCallback) return;
    var cb = I.transformCallback(function (ev) {
      handler(ev && ev.payload);
    }, false);
    invoke('plugin:event|listen', {
      event: name,
      target: { kind: 'Any' },
      handler: cb
    });
  }

  listen('settings:page', function (page) {
    if (typeof page !== 'string') return;
    var btn = document.querySelector('.nav-item[data-page="' + page + '"]');
    if (btn) btn.click();
  });

  // ---------- 常规设置 ----------
  var indexDirs = [];
  var savedExtraDirs = '[]';
  var BUILTIN_TOOLS = {
    screenshot: true,
    clipboard: true,
    lock_screen: true,
    tile_windows: true,
    shortcut_guide: true,
    snippets: true,
    file_buffer: true,
    color_picker: true,
    sticky_note: true
  };
  var BUILTIN_ORDER = [
    'screenshot',
    'clipboard',
    'lock_screen',
    'tile_windows',
    'shortcut_guide',
    'snippets',
    'file_buffer',
    'color_picker',
    'sticky_note'
  ];
  // 上一次读到的完整工具列表：保存时据此保留 app:/plugin: 条目与用户顺序
  var savedTools = [];
  // 面板顺序（拖动排序的结果）：只含"已勾选的内置工具 + 全部 app:/plugin: 条目"，
  // 保存时按这个数组原样写回 settings.tools。
  var toolOrder = [];

  // 平台适配信息（宿主 platform_info 下发）：工具/Shell/功能入口都按它展示，
  // 前端不自己猜平台。key = 工具 id，value = 该工具关联的输入控件 id。
  var platformCaps = null;
  var TOOL_INPUTS = {
    screenshot: ['toolScreenshot', 'hkScreenshot'],
    clipboard: ['toolClipboard', 'hkClipboard'],
    lock_screen: ['toolLockScreen', 'hkLockScreen'],
    tile_windows: ['toolTileWindows', 'hkTileWindows'],
    shortcut_guide: ['toolShortcutGuide', 'hkShortcutGuide'],
    snippets: ['toolSnippets', 'hkSnippets'],
    file_buffer: ['toolFileBuffer', 'hkFileBuffer'],
    color_picker: ['toolColorPicker', 'hkColorPicker'],
    sticky_note: ['toolStickyNote', 'hkStickyNote']
  };

  function applyPlatformCaps(info) {
    platformCaps = info || {};
    var tools = platformCaps.tools || [];
    Object.keys(TOOL_INPUTS).forEach(function (id) {
      if (tools.indexOf(id) >= 0) return;
      TOOL_INPUTS[id].forEach(function (inputId) {
        var el = $(inputId);
        if (el && el.closest('.row')) el.closest('.row').hidden = true;
      });
    });
    if (!platformCaps.features || !platformCaps.features.universal_actions) {
      var ua = $('uaEnabled');
      if (ua && ua.closest('.row')) ua.closest('.row').hidden = true;
    }
    var shells = platformCaps.shells || [];
    var sel = $('commandShell');
    if (sel) {
      Array.prototype.forEach.call(sel.options, function (opt) {
        opt.hidden = shells.length > 0 && shells.indexOf(opt.value) < 0;
      });
    }
    // WSL 发行版只有 wsl 可用时才有意义
    var wsl = $('commandWslDistro');
    if (wsl && wsl.closest('.row')) {
      wsl.closest('.row').hidden = shells.indexOf('wsl') < 0;
    }
    // 终端模式非 Windows 未实现（明确报错），入口不展示
    if (!platformCaps.features || !platformCaps.features.terminal_mode) {
      var term = document.querySelector('#commandModeSeg .seg-item[data-mode="terminal"]');
      if (term) term.hidden = true;
    }
  }

  // 权限区块：仅 macOS 展示（Windows 无此授权模型）
  function loadPermissions() {
    if (!platformCaps || platformCaps.platform !== 'macos') return;
    $('permGroup').hidden = false;
    invoke('permission_status')
      .then(function (p) {
        var paint = function (id, on) {
          var el = $(id);
          el.textContent = T(on ? 'settings.perm.granted' : 'settings.perm.notGranted');
          el.className = 'hint ' + (on ? 'ok' : 'error');
        };
        paint('permA11yState', !!p.accessibility);
        paint('permScreenState', !!p.screen_recording);
      })
      .catch(function (e) {
        console.warn('[settings] 权限状态读取失败: ' + e);
      });
  }

  function paintTheme(mode) {
    document.querySelectorAll('#themeSeg .seg-item').forEach(function (btn) {
      var on = btn.dataset.theme === mode;
      btn.classList.toggle('active', on);
      btn.setAttribute('aria-checked', on ? 'true' : 'false');
    });
  }

  function paintCommandMode(mode) {
    document.querySelectorAll('#commandModeSeg .seg-item').forEach(function (btn) {
      var on = btn.dataset.mode === mode;
      btn.classList.toggle('active', on);
      btn.setAttribute('aria-checked', on ? 'true' : 'false');
    });
  }

  function commandModeValue() {
    var btn = document.querySelector('#commandModeSeg .seg-item.active');
    return btn ? btn.dataset.mode : 'inline';
  }

  // 主题即时生效：先重画当前窗口，再落盘（后端会把其它窗口的原生标题栏一起切过来）
  function chooseTheme(mode) {
    if (window.eftoTheme) window.eftoTheme.set(mode);
    paintTheme(mode);
    invoke('set_settings', { patch: { theme: mode } }).catch(function (e) {
      toast(T('settings.toast.saveThemeFailed', '保存主题失败：') + e, true);
    });
  }

  function renderSettings(s) {
    paintTheme(s.theme || 'system');
    $('languageSelect').value = s.language || 'auto';
    $('hotkey').value = s.hotkey || '';
    $('maxResults').value = s.max_results;
    $('refreshInterval').value = s.refresh_interval_secs;
    $('hideOnBlur').checked = !!s.hide_on_blur;
    $('alwaysOnTop').checked = !!s.always_on_top;
    $('enableRegistry').checked = !!s.enable_registry;
    $('enableDirScan').checked = !!s.enable_dir_scan;
    $('enableBookmarks').checked = !!s.enable_bookmarks;
    indexDirs = (s.extra_dirs || []).slice();
    savedExtraDirs = JSON.stringify(indexDirs);
    renderIndexDirs();
    var tools = s.tools || [];
    savedTools = tools.slice();
    $('toolScreenshot').checked = tools.indexOf('screenshot') >= 0;
    $('toolClipboard').checked = tools.indexOf('clipboard') >= 0;
    $('toolLockScreen').checked = tools.indexOf('lock_screen') >= 0;
    $('toolTileWindows').checked = tools.indexOf('tile_windows') >= 0;
    $('toolShortcutGuide').checked = tools.indexOf('shortcut_guide') >= 0;
    $('toolSnippets').checked = tools.indexOf('snippets') >= 0;
    $('toolFileBuffer').checked = tools.indexOf('file_buffer') >= 0;
    $('toolColorPicker').checked = tools.indexOf('color_picker') >= 0;
    $('toolStickyNote').checked = tools.indexOf('sticky_note') >= 0;
    // 顺序列表必须在勾选框赋值之后算：reconcileToolOrder 依据勾选状态取舍，
    // 先算会把"还没勾上"的内置工具全删掉（实测只剩 app: 条目）。
    toolOrder = tools.slice();
    reconcileToolOrder();
    renderToolOrder();
    $('uaEnabled').checked = s.universal_actions_enabled !== false;
    $('uaHotkey').value = s.universal_actions_hotkey || 'Ctrl+Alt+Space';
    engines = (s.web_searches || []).slice();
    renderEngines();
    renderEngineSelect(s.web_search_engine || '');
    $('webSearchFallback').checked = s.web_search_fallback !== false;
    var hotkeys = s.tool_hotkeys || {};
    $('hkScreenshot').value = hotkeys.screenshot || '';
    $('hkClipboard').value = hotkeys.clipboard || '';
    $('hkLockScreen').value = hotkeys.lock_screen || '';
    $('hkTileWindows').value = hotkeys.tile_windows || '';
    $('hkShortcutGuide').value = hotkeys.shortcut_guide || '';
    $('hkSnippets').value = hotkeys.snippets || '';
    $('hkFileBuffer').value = hotkeys.file_buffer || '';
    $('nodePath').value = s.node_path || '';
    $('pythonPath').value = s.python_path || '';
    $('commandEnabled').checked = s.command_enabled !== false;
    $('commandPrefix').value = s.command_prefix || '>';
    $('commandShell').value =
      s.command_shell || (platformCaps && platformCaps.default_shell) || 'powershell';
    $('commandShellPath').value = s.command_shell_path || '';
    $('commandWslDistro').value = s.command_wsl_distro || '';
    $('commandCustomArgs').value = s.command_custom_args || '';
    $('commandTimeout').value = s.command_timeout_secs || 10;
    paintCommandMode(s.command_run_mode || 'inline');
    renderModels(s);
  }

  function dirKey(p) {
    return String(p || '').replace(/[\\/]+$/, '').toLowerCase();
  }

  // ---------- 常用工具：面板顺序（拖动排序） ----------
  // 内置工具的中文名（顺序列表要显示可读名字；勾选框那侧用的是静态 HTML 文案）
  var TOOL_LABELS = {
    screenshot: ['settings.tool.screenshot.label', '截图'],
    clipboard: ['settings.tool.clipboard.label', '剪贴板'],
    lock_screen: ['settings.tool.lock.label', '锁屏'],
    tile_windows: ['settings.tool.tile.label', '操作窗口'],
    shortcut_guide: ['settings.tool.shortcut.label', '快捷键指南'],
    snippets: ['settings.tool.snippets.label', '片段'],
    file_buffer: ['settings.tool.fileBuffer.label', '文件缓冲'],
    color_picker: ['settings.tool.colorPicker.label', '颜色提取器'],
    sticky_note: ['settings.tool.stickyNote.label', '便签纸']
  };

  /// 把 toolOrder 对齐到"当前勾选状态"：去掉未勾选的内置工具，
  /// 把新勾选但还不在列表里的内置工具按 BUILTIN_ORDER 追加到末尾。
  /// app:/plugin: 条目原样保留（它们的增删不在这里做）。
  function reconcileToolOrder() {
    var kept = toolOrder.filter(function (t) {
      if (BUILTIN_TOOLS[t]) return toolCheckbox(t) && toolCheckbox(t).checked;
      return true;
    });
    BUILTIN_ORDER.forEach(function (t) {
      var box = toolCheckbox(t);
      if (box && box.checked && kept.indexOf(t) < 0) kept.push(t);
    });
    toolOrder = kept;
  }

  /// 保存前的对齐：以拖动结果 `order` 为准，但把"设置页打开期间主面板新加的
  /// app:/plugin: 条目"追加到末尾（不静默丢弃）。未勾选的内置工具已被
  /// reconcileToolOrder 摘掉，这里再过滤一次防勾选框被改动后未同步。
  function reconcileOrderAgainstFresh(order, fresh) {
    var out = order.filter(function (t) {
      if (BUILTIN_TOOLS[t]) {
        var box = toolCheckbox(t);
        return !!box && box.checked;
      }
      return true;
    });
    fresh.forEach(function (t) {
      if (BUILTIN_TOOLS[t]) return; // 内置工具的顺序与取舍由 toolOrder 决定
      if (out.indexOf(t) < 0) out.push(t);
    });
    return out;
  }

  function toolCheckbox(id) {
    var boxId = TOOL_INPUTS[id] && TOOL_INPUTS[id][0];
    return boxId ? $(boxId) : null;
  }

  function toolDisplayName(id) {
    if (BUILTIN_TOOLS[id]) {
      var l = TOOL_LABELS[id];
      return l ? T(l[0], l[1]) : id;
    }
    if (id.indexOf('app:') === 0) return id.slice(4);
    if (id.indexOf('plugin:') === 0) return id.slice(7);
    return id;
  }

  function toolKindTag(id) {
    if (BUILTIN_TOOLS[id]) return T('settings.tools.kind.builtin', '内置');
    if (id.indexOf('app:') === 0) return T('settings.tools.kind.app', '应用');
    return T('settings.tools.kind.plugin', '插件');
  }

  // 渲染顺序列表。空列表时给一句提示，避免看起来像"坏了"。
  function renderToolOrder() {
    var box = $('toolOrderList');
    if (!box) return;
    if (!toolOrder.length) {
      box.innerHTML = '';
      var empty = document.createElement('div');
      empty.className = 'tool-order-empty';
      empty.textContent = T('settings.tools.order.empty', '还没有加入任何常用工具');
      box.appendChild(empty);
      return;
    }
    box.innerHTML = '';
    toolOrder.forEach(function (id, i) {
      var row = document.createElement('div');
      row.className = 'tool-order-item';
      row.dataset.tool = id;
      row.draggable = false;

      var grip = document.createElement('span');
      grip.className = 'tool-grip';
      grip.textContent = '⠿';
      grip.title = T('settings.tools.order.drag', '拖动调整顺序');
      row.appendChild(grip);

      var no = document.createElement('span');
      no.className = 'tool-order-no';
      no.textContent = String(i + 1);
      row.appendChild(no);

      var name = document.createElement('span');
      name.className = 'tool-order-name';
      name.textContent = toolDisplayName(id);
      row.appendChild(name);

      var tag = document.createElement('span');
      tag.className = 'dir-tag';
      tag.textContent = toolKindTag(id);
      row.appendChild(tag);

      // app:/plugin: 条目给一个移除入口（内置工具的增删走上面的勾选框）
      if (!BUILTIN_TOOLS[id]) {
        var rm = document.createElement('button');
        rm.type = 'button';
        rm.className = 'btn ghost dir-remove';
        rm.textContent = T('settings.tools.order.remove', '移除');
        rm.addEventListener('click', function () {
          toolOrder = toolOrder.filter(function (x) { return x !== id; });
          savedTools = savedTools.filter(function (x) { return x !== id; });
          renderToolOrder();
          markToolsDirty();
        });
        row.appendChild(rm);
      }

      attachDragHandlers(row, grip);
      box.appendChild(row);
    });
  }

  /// 指针拖动排序：WKWebView 里 HTML5 dnd 的 dragover 不稳，这里用 pointer 事件。
  /// 拖动中只在"越过相邻项中线"时交换 DOM 位置，松手后按 DOM 顺序回写 toolOrder。
  function attachDragHandlers(row, grip) {
    grip.addEventListener('pointerdown', function (down) {
      down.preventDefault();
      var list = row.parentNode;
      if (!list) return;
      grip.setPointerCapture(down.pointerId);
      row.classList.add('dragging');
      var prevY = down.clientY;

      function onMove(e) {
        var items = Array.prototype.slice.call(list.querySelectorAll('.tool-order-item'));
        var cy = e.clientY;
        var goingDown = cy > prevY;
        prevY = cy;
        // 找与当前拖动项最近的兄弟，比较中线决定是否换位
        for (var i = 0; i < items.length; i++) {
          var other = items[i];
          if (other === row) continue;
          var r = other.getBoundingClientRect();
          var mid = r.top + r.height / 2;
          if (goingDown && cy > mid && row.compareDocumentPosition(other) & Node.DOCUMENT_POSITION_FOLLOWING) {
            list.insertBefore(row, other.nextSibling);
            break;
          }
          if (!goingDown && cy < mid && row.compareDocumentPosition(other) & Node.DOCUMENT_POSITION_PRECEDING) {
            list.insertBefore(row, other);
            break;
          }
        }
      }

      function onUp() {
        grip.removeEventListener('pointermove', onMove);
        grip.removeEventListener('pointerup', onUp);
        grip.removeEventListener('pointercancel', onUp);
        row.classList.remove('dragging');
        // 用 DOM 顺序回写（拖动只改视觉顺序，数据以此为准）
        var order = Array.prototype.slice
          .call(list.querySelectorAll('.tool-order-item'))
          .map(function (el) { return el.dataset.tool; });
        toolOrder = order;
        renderToolOrder();
        markToolsDirty();
      }

      grip.addEventListener('pointermove', onMove);
      grip.addEventListener('pointerup', onUp);
      grip.addEventListener('pointercancel', onUp);
    });
  }

  // 顺序/成员变了就提示"未保存"，与勾选框的提示保持一致
  function markToolsDirty() {
    var hint = $('toolsDirtyHint');
    if (!hint) return;
    hint.hidden = false;
    hint.textContent = T('settings.tools.order.dirty', '顺序已调整，点“保存设置”生效');
  }

  function renderIndexDirs() {
    var box = $('indexDirList');
    if (!indexDirs.length) {
      box.innerHTML = '';
      return;
    }
    box.innerHTML = indexDirs.map(function (d, i) {
      return '<div class="dir-item">' +
        '<span class="dir-path mono" title="' + esc(d) + '">' + esc(d) + '</span>' +
        '<button class="btn ghost dir-remove" data-i="' + i + '">' + esc(T('settings.index.removeDir', '移除')) + '</button>' +
        '</div>';
    }).join('');
  }

  function addIndexDir() {
    invoke('pick_index_dir')
      .then(function (path) {
        if (!path) return;
        var key = dirKey(path);
        var dup = indexDirs.some(function (d) { return dirKey(d) === key; });
        if (dup) {
          toast(T('settings.toast.dirExists', '该目录已在列表里'));
          return;
        }
        indexDirs.push(path);
        renderIndexDirs();
      })
      .catch(function (e) { toast(T('settings.toast.pickFailed', '打开选择框失败：') + e, true); });
  }

  function loadSettings() {
    invoke('get_settings')
      .then(renderSettings)
      .catch(function (e) { toast(T('settings.toast.loadFailed', '读取设置失败：') + e, true); });
  }

  function saveSettings() {
    var hint = $('saveHint');
    var dirsChanged = JSON.stringify(indexDirs) !== savedExtraDirs;
    hint.className = 'hint';
    hint.textContent = T('settings.common.saving', '保存中…');
    // 工具顺序即面板顺序与 Alt+编号：以拖动后的 toolOrder 为准。
    // 重新读一次设置，避免覆盖主面板在设置页打开期间新加的 app:/plugin: 条目：
    // 它们不在 toolOrder 里时追加到末尾（用户没拖过就不该丢）。
    invoke('get_settings')
      .then(function (cur) {
        var fresh = (cur && cur.tools) || savedTools;
        var tools = reconcileOrderAgainstFresh(toolOrder, fresh);
        var patch = {
          hotkey: $('hotkey').value.trim(),
          max_results: Number($('maxResults').value) || 12,
          refresh_interval_secs: Number($('refreshInterval').value) || 0,
          hide_on_blur: $('hideOnBlur').checked,
          always_on_top: $('alwaysOnTop').checked,
          enable_registry: $('enableRegistry').checked,
          enable_dir_scan: $('enableDirScan').checked,
          enable_bookmarks: $('enableBookmarks').checked,
          extra_dirs: indexDirs,
          tools: tools,
          command_enabled: $('commandEnabled').checked,
          command_prefix: $('commandPrefix').value.trim(),
          command_shell: $('commandShell').value,
          command_shell_path: $('commandShellPath').value.trim(),
          command_wsl_distro: $('commandWslDistro').value.trim(),
          command_custom_args: $('commandCustomArgs').value.trim(),
          command_timeout_secs: Number($('commandTimeout').value) || 10,
          command_run_mode: commandModeValue(),
          universal_actions_enabled: $('uaEnabled').checked,
          universal_actions_hotkey: $('uaHotkey').value.trim()
        };
        return invoke('set_settings', { patch: patch });
      })
      .then(function () {
        hint.className = 'hint ok';
        if (!dirsChanged) {
          hint.textContent = T('settings.common.saved', '已保存');
          setTimeout(function () { hint.textContent = ''; }, 2200);
          return;
        }
        savedExtraDirs = JSON.stringify(indexDirs);
        hint.textContent = T('settings.index.savedReindexing', '已保存，正在重建索引…');
        return invoke('refresh_index').then(function () {
          hint.textContent = T('settings.index.savedReindexed', '已保存，索引已重建');
          setTimeout(function () { hint.textContent = ''; }, 2200);
          loadIndex();
        });
      })
      .catch(function (e) {
        hint.className = 'hint error';
        hint.textContent = T('settings.common.saveFailed', '保存失败：') + e;
      });
  }

  // ---------- 索引 ----------
  function loadIndex() {
    invoke('index_status')
      .then(function (st) {
        var r = st.report || {};
        $('indexCount').textContent = TF('settings.index.counts', {
          apps: st.apps || 0,
          plugins: st.plugins || 0
        }, (st.apps || 0) + ' 应用 / ' + (st.plugins || 0) + ' 插件');
        $('indexMeta').textContent = r.elapsed_ms
          ? TF('settings.index.lastElapsed', { ms: r.elapsed_ms }, '上次重建耗时 ' + r.elapsed_ms + ' ms')
          : T('settings.index.never', '尚未重建');
        $('indexSources').textContent = TF('settings.index.sources', {
          start: r.start_menu || 0,
          desktop: r.desktop || 0,
          registry: r.registry || 0,
          dir: r.dir_scan || 0,
          system: r.system || 0,
          dup: r.dropped_duplicates || 0
        }, '开始菜单 ' + (r.start_menu || 0) + ' · 桌面 ' + (r.desktop || 0) +
          ' · 注册表 ' + (r.registry || 0) + ' · 目录 ' + (r.dir_scan || 0) +
          ' · 系统 ' + (r.system || 0) + ' · 去重 ' + (r.dropped_duplicates || 0));
      })
      .catch(function (e) { toast(T('settings.index.loadFailed', '读取索引状态失败：') + e, true); });
  }

  function reindex() {
    var btn = $('reindex');
    btn.disabled = true;
    btn.textContent = T('settings.index.rebuilding', '重建中…');
    invoke('refresh_index')
      .then(function (r) {
        toast((r && r.merged != null)
          ? TF('settings.index.rebuilt', { n: r.merged }, '索引已重建：' + r.merged + ' 条')
          : T('settings.index.rebuiltDone', '完成'));
        loadIndex();
        loadPlugins();
      })
      .catch(function (e) { toast(T('settings.index.rebuildFailed', '重建失败：') + e, true); })
      .then(function () {
        btn.disabled = false;
        btn.textContent = T('settings.index.reindex', '重建索引');
      });
  }

  // ---------- 文件检索（平台分流） ----------
  // macOS 走 Spotlight（系统自带，无需安装）；其它平台走 Everything/es.exe。
  // 两块互斥展示：mac 上不显示 es.exe 下载按钮，避免用户以为"文件检索坏了"。
  function loadFileSearch() {
    var isMac = platformCaps && platformCaps.platform === 'macos';
    var spotlightGroup = $('spotlightGroup');
    var everythingGroup = $('everythingGroup');
    if (spotlightGroup) spotlightGroup.hidden = !isMac;
    if (everythingGroup) everythingGroup.hidden = !!isMac;
    if (isMac) {
      loadSpotlight();
    } else {
      loadEverything();
    }
  }

  function loadSpotlight() {
    invoke('file_search_status')
      .then(renderSpotlight)
      .catch(function (e) {
        var pill = $('spotlightPill');
        if (!pill) return;
        pill.className = 'pill warn';
        pill.textContent = T('settings.spotlight.notReady', '不可用');
        $('spotlightMessage').textContent = T('settings.spotlight.checkFailed', '检测失败：') + e;
      });
  }

  function renderSpotlight(st) {
    var msg = $('spotlightMessage');
    var pill = $('spotlightPill');
    var scope = $('spotlightScope');
    if (!msg || !pill) return;
    pill.className = 'pill';
    if (st && st.ready) {
      pill.classList.add('ok');
      pill.textContent = T('settings.spotlight.ready', '可用');
      msg.textContent = T('settings.spotlight.readyDesc', 'Spotlight 索引就绪，系统自带无需安装');
    } else {
      pill.classList.add('warn');
      pill.textContent = T('settings.spotlight.notReady', '不可用');
      msg.textContent = (st && st.message) || T('settings.spotlight.notReadyDesc', '未找到 mdfind 或索引未启用');
    }
    renderSpotlightDirs(st);
  }

  // 目录列表：默认目录只读展示，用户追加的目录带「移除」按钮。
  // 默认与追加的区分由后端下发（default_dirs），前端不自己猜。
  function renderSpotlightDirs(st) {
    var scope = $('spotlightScope');
    if (!scope) return;
    scope.innerHTML = '';
    var dirs = (st && st.dirs) || [];
    var defaults = (st && st.default_dirs) || [];
    if (!dirs.length) {
      scope.textContent = T('settings.spotlight.noDirs', '（没有可检索的目录，文件检索已停用）');
      return;
    }
    dirs.forEach(function (d) {
      var isDefault = defaults.indexOf(d) >= 0;
      var line = document.createElement('div');
      line.className = 'dir-line';
      var text = document.createElement('span');
      text.className = 'mono dir-path';
      text.textContent = d;
      line.appendChild(text);
      if (isDefault) {
        var tag = document.createElement('span');
        tag.className = 'dir-tag';
        tag.textContent = T('settings.spotlight.defaultTag', '默认');
        line.appendChild(tag);
      } else {
        var rm = document.createElement('button');
        rm.type = 'button';
        rm.className = 'btn ghost dir-remove';
        rm.textContent = T('settings.spotlight.removeDir', '移除');
        rm.addEventListener('click', function () { removeSpotlightDir(d); });
        line.appendChild(rm);
      }
      scope.appendChild(line);
    });
  }

  function addSpotlightDir() {
    invoke('pick_plugin_source', { kind: 'dir' })
      .then(function (picked) {
        if (!picked) return;
        return invoke('add_file_search_dir', { path: picked })
          .then(function (st) {
            toast(T('settings.spotlight.added', '已加入检索范围：') + picked);
            renderSpotlight(st);
          });
      })
      .catch(function (e) { toast(String(e), true); });
  }

  function removeSpotlightDir(dir) {
    invoke('remove_file_search_dir', { path: dir })
      .then(function (st) {
        toast(T('settings.spotlight.removed', '已移出检索范围：') + dir);
        renderSpotlight(st);
      })
      .catch(function (e) { toast(String(e), true); });
  }

  // ---------- Everything ----------
  function loadEverything() {
    invoke('everything_status')
      .then(renderEverything)
      .catch(function (e) { toast(T('settings.everything.checkFailed', '检测 Everything 失败：') + e, true); });
  }

  function renderEverything(st) {
    var pill = $('esPill');
    var btn = $('installEs');
    pill.className = 'pill';
    if (st.state === 'ready') {
      pill.classList.add('ok');
      pill.textContent = T('settings.everything.ready', '就绪');
      btn.textContent = T('settings.everything.reinstall', '重新安装 es.exe');
    } else if (st.state === 'mismatch') {
      pill.classList.add('danger');
      pill.textContent = T('settings.everything.mismatch', '校验失败');
      btn.textContent = T('settings.everything.redownload', '重新下载 es.exe');
    } else {
      pill.classList.add('warn');
      pill.textContent = T('settings.everything.notReady', '未就绪');
      btn.textContent = T('settings.everything.install', '下载并安装 es.exe');
    }
    $('esMessage').textContent = st.message;
    $('esPath').textContent = st.es_path || T('settings.everything.notFound', '未找到');
    $('everythingInstalled').textContent = st.everything_installed
      ? T('settings.everything.installed', '已安装 Everything')
      : T('settings.everything.notInstalled', '未检测到 Everything 本体');
    btn.disabled = false;
  }

  function installEs() {
    var btn = $('installEs');
    btn.disabled = true;
    btn.textContent = T('settings.everything.downloading', '下载中…');
    invoke('install_es')
      .then(function (path) {
        toast(T('settings.everything.installedToast', 'es.exe 已安装：') + path);
        loadEverything();
      })
      .catch(function (e) {
        toast(T('settings.everything.installFailed', '安装 es.exe 失败：') + e, true);
        loadEverything();
      });
  }

  // ---------- 插件 ----------
  function loadPlugins() {
    invoke('list_plugins')
      .then(renderPlugins)
      .catch(function (e) { toast(T('settings.plugins.loadFailed', '读取插件列表失败：') + e, true); });
  }

  function renderPlugins(rows) {
    rows = rows || [];
    $('pluginCount').textContent = TF('settings.plugin.count', { n: rows.length }, '插件（' + rows.length + '）');
    var box = $('pluginList');
    if (!rows.length) {
      box.innerHTML = '<div class="empty-state">' + esc(T('settings.plugins.empty', '还没有插件。导入插件目录或 .zip 包开始使用。')) + '</div>';
      return;
    }
    box.innerHTML = rows.map(function (p) {
      var badges = [];
      if (!p.enabled) badges.push('<span class="pill">' + esc(T('settings.plugins.disabled', '已停用')) + '</span>');
      else if (p.running) badges.push('<span class="pill ok">' + esc(T('settings.plugins.running', '运行中')) + '</span>');
      if (!p.has_backend && !p.has_ui) badges.push('<span class="pill danger">' + esc(T('settings.plugins.broken', '不可执行')) + '</span>');
      var entry = (p.has_backend ? 'main' : '') +
        (p.has_backend && p.has_ui ? ' + ' : '') +
        (p.has_ui ? 'UI' : '');
      var cmdRows = (p.cmds || []).map(function (c) {
        return '<div class="plugin-cmd">' +
          '<span class="plugin-cmd-name" title="' + esc(c.label) + '">' + esc(c.cmd) + '</span>' +
          '<input class="plugin-cmd-alias" data-plugin="' + esc(p.id) + '"' +
          ' data-cmd="' + esc(c.cmd) + '" value="' + esc(c.alias || '') + '"' +
          ' placeholder="' + esc(T('settings.plugins.cmdAliasPlaceholder', '自定义触发词（可选）')) + '" spellcheck="false" />' +
          '</div>';
      }).join('');
      var cmdBlock = cmdRows
        ? '<details class="plugin-cmds"><summary>' + esc(TF('settings.plugins.cmds', { n: p.cmds.length }, '触发词（' + p.cmds.length + '）')) + '</summary>' +
          cmdRows + '</details>'
        : '';
      var problems = (p.problems || [])
        .map(function (x) { return '<div class="plugin-problem">' + esc(x) + '</div>'; })
        .join('');
      var toggle = '<button class="btn ghost plugin-toggle" data-plugin-toggle="' + esc(p.id) +
        '" data-next="' + (p.enabled ? '0' : '1') + '">' +
        esc(p.enabled ? T('settings.plugins.disable', '停用') : T('settings.plugins.enable', '启用')) +
        '</button>';
      return '<div class="plugin-item' + (p.enabled ? '' : ' is-off') + '">' +
        '<div class="plugin-avatar">' +
        esc((p.name || '?').charAt(0)) + '</div>' +
        '<div class="plugin-main">' +
        '<div class="plugin-name">' + esc(p.name) +
        '<span class="plugin-version">' + esc(p.version || '') + '</span>' + badges.join('') +
        '</div>' +
        (p.description ? '<div class="plugin-desc">' + esc(p.description) + '</div>' : '') +
        '<div class="plugin-meta">' + esc(T('settings.plugins.entry', '入口')) + ' ' + esc(entry || '—') + '</div>' +
        cmdBlock +
        problems +
        '</div>' + toggle + '</div>';
    }).join('');
  }

  function importPlugin() {
    var input = $('importPath');
    var hint = $('importHint');
    var path = input.value.trim();
    if (!path) {
      hint.className = 'hint error';
      hint.textContent = T('settings.plugins.importEmptyPath', '请填写插件目录或 .zip 文件路径');
      return;
    }
    var btn = $('importBtn');
    btn.disabled = true;
    btn.textContent = T('settings.plugins.importing', '导入中…');
    hint.className = 'hint';
    hint.textContent = T('settings.plugins.copying', '正在复制并校验清单…');
    invoke('import_plugin', { path: path })
      .then(function (r) {
        var list = r.imported || [];
        var names = list.map(function (p) { return p.name; });
        hint.className = 'hint ok';
        if (list.length === 1) {
          hint.textContent = TF('settings.plugins.imported', { name: list[0].name, version: list[0].version }, '已导入 ' + list[0].name + '（' + list[0].version + '）');
        } else {
          hint.textContent = TF('settings.plugins.importedMany', { n: list.length, names: names.join('、') }, '已导入 ' + list.length + ' 个插件：' + names.join('、'));
        }
        input.value = '';
        var problems = [];
        list.forEach(function (p) {
          (p.problems || []).forEach(function (x) { problems.push(p.name + '：' + x); });
        });
        if (problems.length) {
          hint.textContent += T('settings.plugins.importProblems', '；清单提示：') + problems.join('；');
        }
        if (r.failed && r.failed.length) {
          var skipped = r.failed.map(function (f) { return f.plugin + '（' + f.reason + '）'; });
          hint.textContent += T('settings.plugins.importSkipped', '；未导入：') + skipped.join('；');
        }
        loadPlugins();
        loadIndex();
      })
      .catch(function (e) {
        hint.className = 'hint error';
        hint.textContent = T('settings.plugins.importFailed', '导入失败：') + e;
      })
      .then(function () {
        btn.disabled = false;
        btn.textContent = T('settings.import.button', '导入');
      });
  }

  function pickPluginSource(kind) {
    var hint = $('importHint');
    invoke('pick_plugin_source', { kind: kind })
      .then(function (path) {
        if (!path) return;
        $('importPath').value = path;
        hint.className = 'hint';
        hint.textContent = T('settings.plugins.selected', '已选择：') + path;
      })
      .catch(function (e) { toast(T('settings.toast.pickFailed', '打开选择框失败：') + e, true); });
  }

  function openPluginDir() {
    invoke('open_plugin_dir').then(function (msg) {
      if (msg) toast(msg);
    }).catch(function (e) { toast(T('settings.plugins.openFailed', '打开插件目录失败：') + e, true); });
  }

  // ---------- 运行时 ----------
  function paintRuntime(kind, st) {
    st = st || {};
    var pill = $(kind + 'Pill');
    var desc = $(kind + 'Status');
    // 已配置且检测通过就把安装引导收起来，避免误导用户以为还得再装一次
    var guide = $(kind + 'Guide');
    if (guide) guide.hidden = !!(st.configured && st.ok);
    if (!st.configured) {
      pill.className = 'pill';
      pill.textContent = T('settings.runtime.notConfigured', '未配置');
      desc.className = 'desc mono';
      desc.textContent = T('settings.runtime.notDetected', '未检测');
      return;
    }
    if (st.ok) {
      pill.className = 'pill ok';
      pill.textContent = T('settings.runtime.available', '可用');
      desc.className = 'desc mono';
      desc.textContent = st.version || T('settings.runtime.checkPassed', '检测通过');
    } else {
      pill.className = 'pill danger';
      pill.textContent = T('settings.runtime.unavailable', '不可用');
      desc.className = 'desc mono';
      desc.textContent = st.error || T('settings.runtime.checkFailed', '检测失败');
    }
  }

  function loadRuntime() {
    invoke('runtime_status')
      .then(function (st) {
        paintRuntime('node', st.node);
        paintRuntime('python', st.python);
        // 后端在未配置时会从 PATH 自动补一个；输入框还没填就同步过来，仍可改
        if (st.node && st.node.path && !$('nodePath').value) $('nodePath').value = st.node.path;
        if (st.python && st.python.path && !$('pythonPath').value) $('pythonPath').value = st.python.path;
      })
      .catch(function (e) { toast(T('settings.runtime.loadFailed', '读取运行时状态失败：') + e, true); });
  }

  function checkRuntime(kind) {
    var path = $(kind + 'Path').value.trim();
    if (!path) {
      paintRuntime(kind, { configured: false });
      return;
    }
    var desc = $(kind + 'Status');
    desc.className = 'desc mono';
    desc.textContent = T('settings.runtime.checking', '检测中…');
    invoke('runtime_check', { runtime: kind, path: path })
      .then(function (st) { paintRuntime(kind, st); })
      .catch(function (e) {
        paintRuntime(kind, { configured: true, ok: false, error: String(e) });
      });
  }

  function saveRuntime() {
    var hint = $('runtimeHint');
    hint.className = 'hint';
    hint.textContent = T('settings.common.saving', '保存中…');
    invoke('set_settings', {
      patch: {
        node_path: $('nodePath').value.trim(),
        python_path: $('pythonPath').value.trim()
      }
    })
      .then(function () {
        hint.className = 'hint ok';
        hint.textContent = T('settings.common.saved', '已保存');
        setTimeout(function () { hint.textContent = ''; }, 2200);
        loadRuntime();
      })
      .catch(function (e) {
        hint.className = 'hint error';
        hint.textContent = T('settings.common.saveFailed', '保存失败：') + e;
      });
  }

  // ---------- 模型 ----------
  var modelSeq = 0;

  function newModelId() {
    modelSeq++;
    return 'model-' + Date.now().toString(36) + '-' + modelSeq;
  }

  // 从 DOM 读回当前表单：默认模型用单选按钮表示，只有保存时才落盘。
  function collectModels() {
    var rows = document.querySelectorAll('#modelList .model-item');
    var list = [];
    var def = '';
    for (var i = 0; i < rows.length; i++) {
      var row = rows[i];
      var m = {
        id: row.dataset.id,
        name: row.querySelector('.model-name').value.trim(),
        base_url: row.querySelector('.model-base').value.trim(),
        api_key: row.querySelector('.model-key').value,
        model: row.querySelector('.model-model').value.trim()
      };
      // 刚点“添加”还没填的空行：静默忽略，避免强迫用户删掉它才能保存
      if (!m.name && !m.base_url && !m.api_key && !m.model) continue;
      list.push(m);
      if (row.querySelector('.model-default').checked) def = m.id;
    }
    return { models: list, default_model: def };
  }

  function renderModels(s) {
    s = s || {};
    var list = s.models || [];
    var def = s.default_model || '';
    var box = $('modelList');
    if (!list.length) {
      box.innerHTML = '<div class="empty-state">' + esc(T('settings.models.empty', '还没有模型。点下方“添加”配置一个 OpenAI 兼容接口。')) + '</div>';
      return;
    }
    box.innerHTML = list.map(function (m) {
      var checked = m.id === def ? ' checked' : '';
      return '<div class="model-item" data-id="' + esc(m.id) + '">' +
        '<div class="model-head">' +
        '<input class="text-input model-name" placeholder="' + esc(T('settings.models.name.placeholder', '名称，例如 DeepSeek')) + '" value="' + esc(m.name) + '" spellcheck="false" />' +
        '<label class="model-default-label"><input type="radio" class="model-default" name="defaultModel"' + checked + ' />' + esc(T('settings.models.default', '默认')) + '</label>' +
        '<span class="model-status"></span>' +
        '<button class="btn model-test">' + esc(T('settings.models.test', '测试')) + '</button>' +
        '<button class="btn ghost model-remove">' + esc(T('settings.models.remove', '删除')) + '</button>' +
        '</div>' +
        '<div class="model-fields">' +
        '<label class="field"><span>Base URL</span><input class="text-input model-base" placeholder="' + esc(T('settings.models.url.placeholder', 'https://api.openai.com/v1')) + '" value="' + esc(m.base_url) + '" spellcheck="false" /></label>' +
        '<label class="field"><span>' + esc(T('settings.models.model.label', '模型')) + '</span><input class="text-input model-model" placeholder="gpt-4o-mini" value="' + esc(m.model) + '" spellcheck="false" /></label>' +
        '<label class="field"><span>API Key</span><input class="text-input model-key" type="password" placeholder="' + esc(T('settings.models.key.placeholder', '本地服务可留空')) + '" value="' + esc(m.api_key) + '" spellcheck="false" /></label>' +
        '</div></div>';
    }).join('');
  }

  function addModel() {
    var state = collectModels();
    state.models.push({ id: newModelId(), name: '', base_url: '', api_key: '', model: '' });
    renderModels(state);
  }

  function removeModel(btn) {
    var row = btn.closest('.model-item');
    if (!row) return;
    var state = collectModels();
    state.models = state.models.filter(function (m) { return m.id !== row.dataset.id; });
    renderModels(state);
  }

  function testModel(btn) {
    var row = btn.closest('.model-item');
    if (!row) return;
    var status = row.querySelector('.model-status');
    var cfg = {
      id: row.dataset.id,
      name: row.querySelector('.model-name').value.trim() || T('settings.models.unnamed', '未命名'),
      base_url: row.querySelector('.model-base').value.trim(),
      api_key: row.querySelector('.model-key').value.trim(),
      model: row.querySelector('.model-model').value.trim()
    };
    if (!cfg.base_url || !cfg.model) {
      status.className = 'model-status error';
      status.textContent = T('settings.models.missing', '缺少 Base URL 或模型名');
      return;
    }
    btn.disabled = true;
    status.className = 'model-status';
    status.textContent = T('settings.models.testing', '测试中…');
    invoke('model_check', { model: cfg })
      .then(function (r) {
        status.className = 'model-status ok';
        status.textContent = TF('settings.models.available', { ms: r.latency_ms || 0 }, '可用 · ' + (r.latency_ms || 0) + ' ms');
      })
      .catch(function (e) {
        status.className = 'model-status error';
        status.textContent = String(e);
      })
      .then(function () { btn.disabled = false; });
  }

  function saveModels() {
    var state = collectModels();
    if (state.models.length && !state.default_model) state.default_model = state.models[0].id;
    var hint = $('modelsHint');
    hint.className = 'hint';
    hint.textContent = T('settings.common.saving', '保存中…');
    invoke('set_settings', { patch: { models: state.models, default_model: state.default_model } })
      .then(function () {
        hint.className = 'hint ok';
        hint.textContent = T('settings.common.saved', '已保存');
        setTimeout(function () { hint.textContent = ''; }, 2200);
        // 后端会补齐空 id / 纠正默认模型，重新读一次让界面与磁盘一致
        loadSettings();
      })
      .catch(function (e) {
        hint.className = 'hint error';
        hint.textContent = T('settings.common.saveFailed', '保存失败：') + e;
      });
  }

  // ---------- 绑定 ----------
  $('save').addEventListener('click', saveSettings);

  // 勾选/取消内置工具时同步"面板顺序"列表：勾上追加到末尾、取消移除。
  // 用事件委托，省掉 9 个 checkbox 各自的监听（新增工具时也不用再改这里）。
  Object.keys(BUILTIN_TOOLS).forEach(function (id) {
    var box = toolCheckbox(id);
    if (!box) return;
    box.addEventListener('change', function () {
      reconcileToolOrder();
      renderToolOrder();
      markToolsDirty();
    });
  });
  $('addIndexDir').addEventListener('click', addIndexDir);
  $('indexDirList').addEventListener('click', function (e) {
    var btn = e.target.closest('.dir-remove');
    if (!btn) return;
    indexDirs.splice(Number(btn.dataset.i), 1);
    renderIndexDirs();
  });
  $('reindex').addEventListener('click', reindex);
  $('recheckEs').addEventListener('click', loadEverything);
  var recheckSpotlight = $('recheckSpotlight');
  if (recheckSpotlight) recheckSpotlight.addEventListener('click', loadSpotlight);
  var addSpotlightDirBtn = $('addSpotlightDir');
  if (addSpotlightDirBtn) addSpotlightDirBtn.addEventListener('click', addSpotlightDir);
  $('installEs').addEventListener('click', installEs);
  $('downloadEverything').addEventListener('click', function () {
    invoke('everything_status').then(function (st) {
      return invoke('open_url', { url: st.download_everything_url });
    }).catch(function (e) { toast(T('settings.toast.openDownloadFailed', '打开下载页失败：') + e, true); });
  });
  $('importBtn').addEventListener('click', importPlugin);
  $('pickFileBtn').addEventListener('click', function () { pickPluginSource('file'); });
  $('pickDirBtn').addEventListener('click', function () { pickPluginSource('dir'); });
  $('importPath').addEventListener('keydown', function (e) {
    if (e.key === 'Enter') importPlugin();
  });
  $('openPluginDir').addEventListener('click', openPluginDir);
  $('reloadPlugins').addEventListener('click', function () {
    loadPlugins();
    loadIndex();
  });
  // 启用 / 停用：停用后不再加载，立即从搜索结果消失
  $('pluginList').addEventListener('click', function (e) {
    var btn = e.target.closest('[data-plugin-toggle]');
    if (!btn) return;
    var enabling = btn.getAttribute('data-next') === '1';
    btn.disabled = true;
    invoke('set_plugin_enabled', { plugin: btn.getAttribute('data-plugin-toggle'), enabled: enabling })
      .then(function (rows) {
        renderPlugins(rows);
        toast(enabling ? T('settings.toast.pluginEnabled', '已启用') : T('settings.toast.pluginDisabled', '已停用'));
      })
      .catch(function (err) {
        btn.disabled = false;
        toast(T('settings.toast.pluginToggleFailed', '切换插件状态失败：') + err, true);
      });
  });
  // 改完失焦即保存：空值表示恢复清单里的原触发词
  $('pluginList').addEventListener('change', function (e) {
    var el = e.target;
    if (!el || !el.classList.contains('plugin-cmd-alias')) return;
    var alias = el.value.trim();
    invoke('set_plugin_cmd_alias', {
      plugin: el.dataset.plugin,
      cmd: el.dataset.cmd,
      alias: alias
    })
      .then(function (saved) {
        el.value = saved || '';
        toast(saved ? T('settings.toast.aliasSaved', '已保存触发词：') + saved : T('settings.toast.aliasReset', '已恢复原触发词'));
      })
      .catch(function (err) {
        toast(String(err), true);
      });
  });
  $('checkNode').addEventListener('click', function () { checkRuntime('node'); });
  $('checkPython').addEventListener('click', function () { checkRuntime('python'); });
  $('nodePath').addEventListener('keydown', function (e) {
    if (e.key === 'Enter') checkRuntime('node');
  });
  $('pythonPath').addEventListener('keydown', function (e) {
    if (e.key === 'Enter') checkRuntime('python');
  });
  $('saveRuntime').addEventListener('click', saveRuntime);
  document.querySelectorAll('.runtime-guide .link-btn').forEach(function (btn) {
    btn.addEventListener('click', function () {
      invoke('open_url', { url: btn.dataset.url }).catch(function (e) {
        toast(T('settings.runtime.openFailed', '打开下载页失败：') + e, true);
      });
    });
  });
  $('addModel').addEventListener('click', addModel);
  $('saveModels').addEventListener('click', saveModels);
  $('modelList').addEventListener('click', function (e) {
    var t = e.target;
    if (t.classList.contains('model-remove')) removeModel(t);
    else if (t.classList.contains('model-test')) testModel(t);
  });

  $('themeSeg').addEventListener('click', function (e) {
    var btn = e.target.closest('.seg-item');
    if (!btn) return;
    chooseTheme(btn.dataset.theme);
  });

  $('commandModeSeg').addEventListener('click', function (e) {
    var btn = e.target.closest('.seg-item');
    if (!btn) return;
    paintCommandMode(btn.dataset.mode);
  });

  // 检测命令分组：用表单里尚未保存的值真跑一条 echo，通过后给出 Shell 实际路径
  $('checkCommand').addEventListener('click', function () {
    var btn = $('checkCommand');
    btn.disabled = true;
    invoke('command_check', {
      shell: $('commandShell').value,
      path: $('commandShellPath').value.trim(),
      distro: $('commandWslDistro').value.trim(),
      custom_args: $('commandCustomArgs').value.trim()
    })
      .then(function (r) {
        toast(T('settings.command.checkPassed', '检测通过：') + ((r && (r.sample || r.path)) || $('commandShell').value));
      })
      .catch(function (e) {
        toast(T('settings.command.checkFailed', '检测失败：') + e, true);
      })
      .then(function () { btn.disabled = false; });
  });

  // “检索收藏夹”即时生效：勾选/取消立刻写回，不需要再滚到底部点“保存”。
  $('enableBookmarks').addEventListener('change', function () {
    var on = $('enableBookmarks').checked;
    invoke('set_settings', { patch: { enable_bookmarks: on } })
      .then(function () { toast(on ? T('settings.bookmarks.on', '已开启收藏夹检索') : T('settings.bookmarks.off', '已关闭收藏夹检索')); })
      .catch(function (e) { toast(T('settings.common.saveFailed', '保存失败：') + e, true); });
  });

  // 语言切换：后端会重建托盘/窗口标题并广播给其它窗口；本窗口直接重载，重新加载词典
  $('languageSelect').addEventListener('change', function () {
    var lang = $('languageSelect').value;
    invoke('set_settings', { patch: { language: lang } })
      .then(function () { location.reload(); })
      .catch(function (e) { toast(T('settings.common.saveFailed', '保存失败：') + e, true); });
  });

  // ---------- 片段 ----------
  var snippets = [];
  var engines = [];

  function escHtml(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function renderSnippets() {
    var box = $('snippetList');
    if (!snippets.length) {
      box.innerHTML = '<div class="snippet-empty">' +
        escHtml(T('settings.snippets.empty', '还没有片段。点下方“添加”创建第一条。')) + '</div>';
      return;
    }
    box.innerHTML = snippets.map(function (s, i) {
      return '<div class="snippet-row" data-i="' + i + '">' +
        '<div class="snippet-head">' +
        '<input class="text-input tiny" data-k="keyword" spellcheck="false" value="' + escHtml(s.keyword) + '" placeholder="' + escHtml(T('settings.snippets.keyword', '关键词')) + '" />' +
        '<input class="text-input" data-k="title" value="' + escHtml(s.title) + '" placeholder="' + escHtml(T('settings.snippets.title', '标题（可选）')) + '" />' +
        '<button class="btn ghost snippet-remove" data-i="' + i + '">' + escHtml(T('settings.snippets.remove', '移除')) + '</button>' +
        '</div>' +
        '<textarea class="text-input snippet-body" data-k="body" rows="3" placeholder="' + escHtml(T('settings.snippets.body', '展开后的正文')) + '">' + escHtml(s.body) + '</textarea>' +
        '</div>';
    }).join('');
  }

  function loadSnippets() {
    invoke('snippets_list')
      .then(function (list) {
        snippets = list || [];
        renderSnippets();
      })
      .catch(function (e) {
        $('snippetsHint').textContent = String(e);
      });
  }

  function saveSnippets() {
    var hint = $('snippetsHint');
    hint.className = 'hint';
    hint.textContent = T('settings.common.saving', '保存中…');
    invoke('snippets_save', { items: snippets })
      .then(function (n) {
        hint.className = 'hint ok';
        hint.textContent = TF('settings.snippets.saved', { n: n }, '已保存 ' + n + ' 条片段');
        setTimeout(function () { hint.textContent = ''; }, 2200);
        loadSnippets();
      })
      .catch(function (e) {
        hint.className = 'hint error';
        hint.textContent = T('settings.common.saveFailed', '保存失败：') + e;
      });
  }

  $('addSnippet').addEventListener('click', function () {
    snippets.push({ id: '', keyword: '', title: '', body: '' });
    renderSnippets();
    var rows = $('snippetList').querySelectorAll('.snippet-row');
    var last = rows[rows.length - 1];
    if (last) last.querySelector('input[data-k="keyword"]').focus();
  });

  $('snippetList').addEventListener('input', function (e) {
    var row = e.target.closest('.snippet-row');
    var key = e.target.dataset.k;
    if (!row || !key) return;
    var item = snippets[Number(row.dataset.i)];
    if (item) item[key] = e.target.value;
  });

  $('snippetList').addEventListener('click', function (e) {
    var btn = e.target.closest('.snippet-remove');
    if (!btn) return;
    snippets.splice(Number(btn.dataset.i), 1);
    renderSnippets();
  });

  $('saveSnippets').addEventListener('click', saveSnippets);

  // ---------- 网页搜索 ----------
  function renderEngineSelect(active) {
    var box = $('webSearchEngine');
    if (!engines.length) {
      box.innerHTML = '<option value="">' + escHtml(T('settings.search.engine.empty', '没有可用的搜索引擎')) + '</option>';
      return;
    }
    box.innerHTML = engines.map(function (e) {
      return '<option value="' + escHtml(e.keyword) + '">' +
        escHtml(e.name) + '（' + escHtml(e.keyword) + '）</option>';
    }).join('');
    box.value = active || engines[0].keyword;
  }

  function renderEngines() {
    var box = $('engineList');
    if (!engines.length) {
      box.innerHTML = '<div class="snippet-empty">' +
        escHtml(T('settings.search.empty', '还没有搜索引擎，点下方“添加”创建。')) + '</div>';
      return;
    }
    box.innerHTML = engines.map(function (e, i) {
      return '<div class="snippet-row" data-i="' + i + '">' +
        '<div class="snippet-head">' +
        '<input class="text-input tiny" data-k="keyword" spellcheck="false" value="' + escHtml(e.keyword) + '" placeholder="' + escHtml(T('settings.search.keyword', '触发词')) + '" />' +
        '<input class="text-input" data-k="name" value="' + escHtml(e.name) + '" placeholder="' + escHtml(T('settings.search.name', '名称')) + '" />' +
        '<button class="btn ghost snippet-remove" data-i="' + i + '">' + escHtml(T('settings.snippets.remove', '移除')) + '</button>' +
        '</div>' +
        '<input class="text-input engine-url" data-k="url" spellcheck="false" value="' + escHtml(e.url) + '" placeholder="https://example.com/search?q={q}" />' +
        '</div>';
    }).join('');
  }

  function saveSearch() {
    var hint = $('searchHint');
    hint.className = 'hint';
    hint.textContent = T('settings.common.saving', '保存中…');
    invoke('set_settings', {
      patch: {
        web_searches: engines,
        web_search_fallback: $('webSearchFallback').checked,
        web_search_engine: $('webSearchEngine').value
      }
    })
      .then(function () { return invoke('get_settings'); })
      .then(function (s) {
        engines = (s.web_searches || []).slice();
        renderEngines();
        renderEngineSelect(s.web_search_engine || '');
        hint.className = 'hint ok';
        hint.textContent = T('settings.search.saved', '已保存（无效项已忽略）');
        setTimeout(function () { hint.textContent = ''; }, 2200);
      })
      .catch(function (e) {
        hint.className = 'hint error';
        hint.textContent = T('settings.common.saveFailed', '保存失败：') + e;
      });
  }

  $('addEngine').addEventListener('click', function () {
    engines.push({ keyword: '', name: '', url: '' });
    renderEngines();
    var rows = $('engineList').querySelectorAll('.snippet-row');
    var last = rows[rows.length - 1];
    if (last) last.querySelector('input[data-k="keyword"]').focus();
  });

  $('engineList').addEventListener('input', function (e) {
    var row = e.target.closest('.snippet-row');
    var key = e.target.dataset.k;
    if (!row || !key) return;
    var item = engines[Number(row.dataset.i)];
    if (item) item[key] = e.target.value;
  });

  $('engineList').addEventListener('click', function (e) {
    var btn = e.target.closest('.snippet-remove');
    if (!btn) return;
    engines.splice(Number(btn.dataset.i), 1);
    renderEngines();
    renderEngineSelect('');
  });

  $('saveSearch').addEventListener('click', saveSearch);

  // ---------- 单项热键 ----------
  var HOTKEY_TOOLS = [
    ['screenshot', 'hkScreenshot'],
    ['clipboard', 'hkClipboard'],
    ['lock_screen', 'hkLockScreen'],
    ['tile_windows', 'hkTileWindows'],
    ['shortcut_guide', 'hkShortcutGuide'],
    ['snippets', 'hkSnippets'],
    ['file_buffer', 'hkFileBuffer'],
    ['color_picker', 'hkColorPicker'],
    ['sticky_note', 'hkStickyNote']
  ];

  // 把 KeyboardEvent 转成 Tauri global-shortcut 的写法（Ctrl+Alt+1、Alt+Space）。
  // e.code 本身就是 global-hotkey 认的键名，只有字母/数字要剥掉 Key/Digit 前缀；
  // 单按修饰键不产出结果，避免录成 "Ctrl+ControlLeft"。
  function accelFromEvent(e) {
    var code = e.code || '';
    if (/^(Control|Alt|Shift|Meta|OS)(Left|Right)$/.test(code)) return '';
    var key = '';
    if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
    else if (/^Digit[0-9]$/.test(code)) key = code.slice(5);
    else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code) || code === 'Space' ||
      code === 'Enter' || code === 'Tab' || code === 'Backspace' ||
      code === 'Delete' || code === 'Escape' || /^Arrow(Up|Down|Left|Right)$/.test(code)) key = code;
    if (!key) return '';
    var mods = [];
    if (e.ctrlKey) mods.push('Ctrl');
    if (e.altKey) mods.push('Alt');
    if (e.shiftKey) mods.push('Shift');
    if (e.metaKey) mods.push('Super');
    // 裸字母/数字当全局热键会吞掉正常输入，必须带修饰键；功能键例外。
    if (!mods.length && !/^F\d+$/.test(key)) return '';
    return mods.concat(key).join('+');
  }

  function bindHotkeyCapture(id) {
    var el = $(id);
    if (!el) return;
    el.addEventListener('keydown', function (e) {
      // Tab / Shift+Tab 留给浏览器切焦点，其余按键一律吞掉，避免往框里敲字符
      if (e.key === 'Tab' && !e.ctrlKey && !e.altKey && !e.metaKey) return;
      e.preventDefault();
      if ((e.key === 'Backspace' || e.key === 'Delete' || e.key === 'Escape') &&
        !e.ctrlKey && !e.altKey && !e.metaKey) {
        el.value = '';
        return;
      }
      var accel = accelFromEvent(e);
      if (accel) el.value = accel;
    });
  }

  HOTKEY_TOOLS.forEach(function (pair) { bindHotkeyCapture(pair[1]); });

  function saveHotkeys() {
    var hint = $('hotkeysHint');
    var map = {};
    HOTKEY_TOOLS.forEach(function (pair) {
      var v = $(pair[1]).value.trim();
      if (v) map[pair[0]] = v;
    });
    hint.className = 'hint';
    hint.textContent = T('settings.common.saving', '保存中…');
    invoke('set_settings', { patch: { tool_hotkeys: map } })
      .then(function () {
        hint.className = 'hint ok';
        hint.textContent = T('settings.hotkeys.saved', '已保存；重复或无效的键位会被忽略');
        setTimeout(function () { hint.textContent = ''; }, 2600);
      })
      .catch(function (e) {
        hint.className = 'hint error';
        hint.textContent = T('settings.common.saveFailed', '保存失败：') + e;
      });
  }

  $('saveHotkeys').addEventListener('click', saveHotkeys);

  // 词典加载完再读设置，避免动态文案先渲染成中文
  i18n.ready.then(function () {
    // 先拿平台清单再渲染：工具行/Shell 下拉/终端模式的展示都依赖它
    invoke('platform_info')
      .then(applyPlatformCaps)
      .catch(function (e) {
        console.warn('[settings] platform_info 失败，按全量展示: ' + e);
        applyPlatformCaps({ tools: Object.keys(BUILTIN_TOOLS), shells: [], features: {} });
      })
      .then(function () {
        loadSettings();
        loadIndex();
        loadFileSearch();
        loadPlugins();
        loadPermissions();
        // 热键占位符是展示文案，按平台装饰（录入/存储仍是 Tauri 语法）
        if (window.eftoI18n && window.eftoI18n.decorateKeys) {
          var dk = window.eftoI18n.decorateKeys;
          document.querySelectorAll('input[id^="hk"]').forEach(function (el) {
            if (el.placeholder) el.placeholder = dk(el.placeholder);
          });
        }
      });
  });

  $('permA11yOpen').addEventListener('click', function () {
    // 辅助功能只能手动勾选；打开对应设置页并刷新状态
    invoke('open_permission_settings', { kind: 'accessibility' })
      .then(function () { setTimeout(loadPermissions, 800); })
      .catch(function (e) { toast(String(e), true); });
  });

  $('permScreenOpen').addEventListener('click', function () {
    // 先触发系统权限引导（CGRequestScreenCaptureAccess），再打开设置页兜底
    invoke('request_permission', { kind: 'screen_recording' })
      .then(function () { return invoke('open_permission_settings', { kind: 'screen_recording' }); })
      .then(function () { setTimeout(loadPermissions, 800); })
      .catch(function (e) { toast(String(e), true); });
  });
})();
