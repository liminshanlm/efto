#!/usr/bin/env node
// README 截图生成器：把 frontend/ 的页面与插件 UI 在 Chromium 里渲染出来，
// 注入假 IPC 数据（结果行、设置项），再合成到一块深色渐变"桌面"上截图。
//
// 为什么这么做：宿主的窗口层要真机才有，而页面层（HTML/CSS/JS）与二进制无关。
// 这里渲染的是**同一份页面代码**，只有 IPC 回包是构造的样例数据。
//
// 依赖 Playwright（不入库、不进 verify）：
//   npm i -g playwright && npx playwright install chromium
//   NODE_PATH=/opt/homebrew/lib/node_modules node scripts/gen-readme-shots.mjs
//
// 输出：docs/images/<场景>-<语言>.png（2x DPR）

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
let chromium;
try {
  ({ chromium } = require('playwright'));
} catch {
  console.error(
    '找不到 playwright。先 `npm i -g playwright && npx playwright install chromium`，\n' +
      '再用 `NODE_PATH=$(npm root -g) node scripts/gen-readme-shots.mjs` 运行。'
  );
  process.exit(1);
}

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const OUT = path.join(ROOT, 'docs', 'images');

// ---------- 静态服务（file:// 下 i18n 的 fetch 会被拦） ----------
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml'
};

function serve() {
  const server = http.createServer((req, res) => {
    const rel = decodeURIComponent(new URL(req.url, 'http://x').pathname).replace(/^\/+/, '');
    const file = path.resolve(ROOT, rel);
    if (!file.startsWith(ROOT + path.sep) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404).end('not found');
      return;
    }
    res.writeHead(200, { 'content-type': MIME[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

function dataUrl(rel) {
  const file = path.join(ROOT, rel);
  if (!fs.existsSync(file)) return null;
  return 'data:image/png;base64,' + fs.readFileSync(file).toString('base64');
}

// ---------- 样例数据 ----------
// 标题/副标题的形态与后端 SearchHit 一致（kind/id/title/subtitle/score/reason/icon/source/payload）。
// 应用条目的 icon 置空：真实图标要靠宿主从 .lnk/exe 里抽，这里退回首字头像。

function appHit(title, subtitle, src, { reason = 'initials', group = null } = {}) {
  return {
    kind: 'app',
    id: title,
    title,
    subtitle,
    score: 900,
    reason,
    icon: null,
    group,
    source: src,
    payload: { type: 'launch', target: { path: title, kind: 'app' } }
  };
}

function actionHit(name, title, group) {
  return {
    kind: 'action',
    id: 'action:' + name,
    title,
    subtitle: '内置指令',
    score: 500,
    reason: 'exact',
    icon: null,
    group,
    source: 'action',
    payload: { type: 'action', name }
  };
}

function copyHit(kind, title, subtitle, source, text, echo) {
  return {
    kind,
    id: source + ':' + title,
    title,
    subtitle,
    score: 690,
    reason: 'exact',
    icon: null,
    group: null,
    source,
    payload: { type: 'copy_text', text, echo }
  };
}

const panelHits = [
  appHit('微信', 'WeChat.exe', 'start_menu', { group: 'recent' }),
  appHit('Visual Studio Code', 'Code.exe', 'start_menu', { group: 'recent' }),
  appHit('记事本', 'notepad.exe', 'start_menu', { group: 'recent' }),
  appHit('Windows Terminal', 'WindowsTerminal.exe', 'start_menu', { group: 'recent' }),
  actionHit('screenshot', '截图', 'tools'),
  actionHit('clipboard', '剪贴板', 'tools'),
  actionHit('lock_screen', '锁屏', 'tools'),
  actionHit('tile_windows', '操作窗口', 'tools'),
  actionHit('shortcut_guide', '快捷键指南', 'tools'),
  actionHit('snippets', '片段', 'tools'),
  actionHit('file_buffer', '文件缓冲', 'tools')
];

const searchHits = [
  appHit('微信', 'WeChat.exe', 'start_menu', { reason: 'initials' }),
  appHit('微信开发者工具', 'wechatdevtools.exe', 'start_menu', { reason: 'substring' }),
  {
    kind: 'app',
    id: '企业微信',
    title: '企业微信',
    subtitle: '企业微信 · 已安装程序',
    score: 226,
    reason: 'subsequence',
    icon: null,
    group: null,
    source: 'registry',
    payload: { type: 'launch', target: { path: '企业微信', kind: 'app' } }
  },
  {
    kind: 'plugin',
    id: 'plugin://quickweb#wx',
    title: '网页快开 · 微信读书',
    subtitle: '插件 · quickweb',
    score: 175,
    reason: 'initials',
    icon: 'plugins/quickweb/logo.png',
    group: null,
    source: 'plugin:quickweb',
    payload: { type: 'plugin', plugin_id: 'quickweb', cmd: '微信读书', feature: 'wx' }
  }
];

const calcHits = [
  copyHit('calc', '42', '计算结果 · 6*7 =', 'calc', '42', { expression: '6*7', value: 42 })
];

// 时间戳两行：值由本机时区（UTC+08:00）算出，与后端一致
const timestampHits = [
  copyHit('time', '1794812602', '秒级 · 2026-11-16 15:03:22 周一 · UTC+08:00 · 本机时区', 'time', '1794812602', {
    kind: 'timestamp',
    precision: 'sec',
    secs: 1794812602,
    millis: 1794812602000,
    zone: 'UTC+08:00',
    input: '2026-11-16 15:03:22',
    iso: '2026-11-16T15:03:22+08:00',
    local: '2026-11-16 15:03:22 周一'
  }),
  copyHit('time', '1794812602000', '毫秒级 · 2026-11-16 15:03:22.000 周一 · UTC+08:00 · 本机时区', 'time', '1794812602000', {
    kind: 'timestamp',
    precision: 'ms',
    secs: 1794812602,
    millis: 1794812602000,
    zone: 'UTC+08:00',
    input: '2026-11-16 15:03:22',
    iso: '2026-11-16T15:03:22+08:00',
    local: '2026-11-16 15:03:22 周一'
  })
];

const clockRows = [
  ['2026-11-16 18:59:55', 'ISO 本地'],
  ['2026/11/16 18:59', 'Windows 中文'],
  ['2026-11-16', '仅日期'],
  ['18:59:55', '仅时间'],
  ['2026-11-16T18:59:55+08:00', 'RFC3339 带时区'],
  ['Mon, 16 Nov 2026 18:59:55 +0800', 'RFC1123 / HTTP'],
  ['2026年11月16日 18时59分55秒 周一', '中文'],
  ['20261116_185955', '文件名安全']
];
const clockHits = clockRows.map(([title, label]) =>
  copyHit('time', title, label + ' · UTC+08:00', 'clock', title, {
    kind: 'clock',
    format: label,
    input: 'date',
    secs: 1794817195,
    millis: 1794817195000,
    zone: 'UTC+08:00'
  })
);

// 设置页：只覆盖"常规"页要读的几条命令，其余按默认值兜底
const settingsFixture = {
  theme: 'dark',
  language: 'auto',
  hotkey: 'Alt+Space',
  max_results: 12,
  refresh_interval_secs: 0,
  hide_on_blur: true,
  always_on_top: true,
  enable_registry: true,
  enable_dir_scan: true,
  enable_bookmarks: false,
  extra_dirs: [],
  tools: [
    'screenshot',
    'clipboard',
    'lock_screen',
    'tile_windows',
    'shortcut_guide',
    'snippets',
    'file_buffer'
  ],
  universal_actions_enabled: true,
  universal_actions_hotkey: 'Ctrl+Alt+Space',
  web_searches: [
    { keyword: 'bd', name: '百度', url: 'https://www.baidu.com/s?wd={query}' },
    { keyword: 'g', name: 'Google', url: 'https://www.google.com/search?q={query}' },
    { keyword: 'gh', name: 'GitHub', url: 'https://github.com/search?q={query}' }
  ],
  web_search_engine: 'bd',
  web_search_fallback: true,
  tool_hotkeys: {},
  node_path: '',
  python_path: '',
  command_enabled: true,
  command_prefix: '>',
  command_shell: 'powershell',
  command_shell_path: '',
  command_wsl_distro: '',
  command_custom_args: '',
  command_timeout_secs: 10,
  command_run_mode: 'inline',
  models: [],
  default_model: null
};

// ---------- 注入的假 IPC ----------
// 页面只认 __TAURI_INTERNALS__，因此这里不需要任何 Tauri 运行时。
function installIpc(config) {
  var callbacks = 0;
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () {
      callbacks += 1;
      return callbacks;
    },
    invoke: function (cmd, payload) {
      var args = payload || {};
      var value;
      switch (cmd) {
        case 'ui_language':
          value = { language: config.language };
          break;
        case 'theme_mode':
          value = config.theme || 'dark';
          break;
        case 'search':
          value = config.hits[String(args.query == null ? '' : args.query)] || [];
          break;
        case 'search_files':
          value = [];
          break;
        case 'icon_for':
          value = config.icons[args.path] || null;
          break;
        case 'get_settings':
          value = config.settings;
          break;
        case 'platform_info':
          value = config.platform;
          break;
        case 'index_status':
          value = config.index;
          break;
        case 'file_search_status':
        case 'everything_status':
          value = config.fileSearch;
          break;
        case 'permission_status':
          value = config.permission;
          break;
        case 'runtime_status':
          value = config.runtime;
          break;
        case 'list_plugins':
          value = [];
          break;
        case 'snippets_list':
          value = [];
          break;
        case 'plugin:event|listen':
        case 'hide_main':
        case 'preview_release':
          value = null;
          break;
        default:
          value = null;
      }
      return Promise.resolve(value);
    }
  };
}

const platformFixture = {
  platform: 'windows',
  version: '0.1.0',
  default_shell: 'powershell',
  tools: [
    'screenshot',
    'clipboard',
    'lock_screen',
    'tile_windows',
    'shortcut_guide',
    'snippets',
    'file_buffer',
    'color_picker',
    'sticky_note'
  ],
  shells: ['powershell', 'cmd', 'wsl'],
  features: { universal_actions: true, terminal_mode: true, file_assoc: true }
};

const indexFixture = {
  apps: 204,
  plugins: 12,
  report: {
    elapsed_ms: 168,
    start_menu: 176,
    desktop: 0,
    registry: 21,
    dir_scan: 12,
    system: 5,
    dropped_duplicates: 10
  }
};

const fileSearchFixture = {
  ready: true,
  message: 'Everything 索引就绪，文件结果排在应用条目之后',
  dirs: ['C:\\Users\\limin\\Documents', 'D:\\报告'],
  default_dirs: []
};

const permissionFixture = { accessibility: true, screen_recording: true };
const runtimeFixture = {
  node: { configured: true, path: 'C:\\Program Files\\nodejs\\node.exe', version: 'v22.11.0' },
  python: { configured: true, path: 'C:\\Users\\limin\\AppData\\Local\\Programs\\Python\\Python312\\python.exe', version: '3.12.4' }
};

// ---------- 场景 ----------
const WINDOW_CSS = `
  html { background: linear-gradient(135deg, #232936 0%, #171a22 58%, #241f2c 100%); }
  body { box-sizing: border-box; padding: 26px; }
  .shell { box-shadow: 0 20px 48px rgba(0, 0, 0, 0.55), 0 3px 10px rgba(0, 0, 0, 0.35); }
`;

const SETTINGS_CSS = `
  html { background: linear-gradient(135deg, #232936 0%, #171a22 58%, #241f2c 100%); }
  html, body { box-sizing: border-box; }
  body { padding: 28px; }
  .layout { border-radius: 12px; overflow: hidden; box-shadow: 0 24px 56px rgba(0, 0, 0, 0.55); }
`;

const PLUGIN_CSS = `
  html { background: linear-gradient(135deg, #232936 0%, #171a22 58%, #241f2c 100%); }
  body { box-sizing: border-box; padding: 26px; }
  .app { border-radius: 12px; overflow: hidden; box-shadow: 0 20px 48px rgba(0, 0, 0, 0.5); }
`;

const scenes = [
  { id: 'panel', page: 'frontend/index.html', size: [520, 320], query: '', wait: '#results .tile', hits: panelHits },
  { id: 'search-wx', page: 'frontend/index.html', size: [520, 320], query: 'wx', wait: '#results .row', hits: searchHits },
  { id: 'calculator', page: 'frontend/index.html', size: [520, 320], query: '6*7', wait: '#results .row', hits: calcHits },
  {
    id: 'timestamp',
    page: 'frontend/index.html',
    size: [520, 320],
    query: '2026-11-16 15:03:22',
    wait: '#results .row',
    hits: timestampHits
  },
  { id: 'time-formats', page: 'frontend/index.html', size: [520, 320], query: 'date', wait: '#results .row', hits: clockHits },
  { id: 'settings', page: 'frontend/settings.html', size: [960, 640], css: SETTINGS_CSS, wait: '#hotkey', settings: true }
];

// 插件 UI 单独渲染：它走 window.efto 与自己的主题脚本
const pluginScene = {
  id: 'plugin-text-beautify',
  page: 'plugins/text-beautify/index.html',
  size: [1120, 720],
  css: PLUGIN_CSS,
  src: '{"name":"efto","version":"0.1.0","cmds":[{"type":"input","label":"时间戳","cmd":"ts"}],"engines":{"efto":">=0.1.0"}}'
};

function stubPluginSdk() {
  window.efto = {
    isDarkMode: function () {
      return true;
    },
    copyText: function () {
      return Promise.resolve();
    },
    ipcMessage: function () {
      return Promise.resolve();
    },
    onMessage: function () {},
    onPluginEnter: function () {}
  };
}

async function shoot(browser, base, locale, scene) {
  const [w, h] = scene.size;
  const pad = scene.css === SETTINGS_CSS ? 28 * 2 : 26 * 2;
  const context = await browser.newContext({
    viewport: { width: w + pad, height: h + pad },
    deviceScaleFactor: 2,
    colorScheme: 'dark',
    reducedMotion: 'reduce'
  });
  const page = await context.newPage();
  await page.addInitScript((cfg) => {
    localStorage.setItem('efto.theme', 'dark');
    window.__SHOT__ = cfg;
  }, { language: locale });
  await page.addInitScript(installIpc, {
    language: locale,
    theme: 'dark',
    hits: { '': scene.hits || [], wx: searchHits, '6*7': calcHits, '2026-11-16 15:03:22': timestampHits, date: clockHits },
    icons: {
      'plugins/quickweb/logo.png': dataUrl('plugins/quickweb/logo.png'),
      'plugins/timestamp-ts/logo.png': dataUrl('plugins/timestamp-ts/logo.png')
    },
    settings: settingsFixture,
    platform: platformFixture,
    index: indexFixture,
    fileSearch: fileSearchFixture,
    permission: permissionFixture,
    runtime: runtimeFixture
  });

  await page.goto(base + '/' + scene.page, { waitUntil: 'networkidle' });
  if (scene.css) await page.addStyleTag({ content: scene.css });
  if (scene.query != null) {
    await page.fill('#q', scene.query);
    await page.waitForSelector(scene.wait, { timeout: 5000 });
  } else {
    await page.waitForSelector(scene.wait, { timeout: 5000 });
  }
  // 等 i18n 落地 + 图标/布局稳定
  await page.waitForTimeout(400);
  const name = `${scene.id}.${locale}.png`;
  await page.screenshot({ path: path.join(OUT, name) });
  await context.close();
  return name;
}

async function shootPlugin(browser, base) {
  const [w, h] = pluginScene.size;
  const context = await browser.newContext({
    viewport: { width: w + 52, height: h + 52 },
    deviceScaleFactor: 2,
    colorScheme: 'dark',
    reducedMotion: 'reduce'
  });
  const page = await context.newPage();
  await page.addInitScript(stubPluginSdk);
  await page.goto(base + '/' + pluginScene.page, { waitUntil: 'networkidle' });
  await page.addStyleTag({ content: pluginScene.css });
  await page.fill('#src', pluginScene.src);
  await page.click('#beautify');
  // wrap="off" 的 textarea 打满一行会横向滚到尾巴：截图前把两个方向都归零
  await page.evaluate(() => {
    const el = document.getElementById('src');
    el.scrollTop = 0;
    el.scrollLeft = 0;
  });
  await page.waitForTimeout(400);
  const name = `${pluginScene.id}.png`;
  await page.screenshot({ path: path.join(OUT, name) });
  await context.close();
  return name;
}

const LOCALES = ['zh-CN', 'en-US'];

async function main() {
  fs.mkdirSync(OUT, { recursive: true });
  const { server, port } = await serve();
  const base = `http://127.0.0.1:${port}`;
  const browser = await chromium.launch();
  const made = [];
  try {
    for (const locale of LOCALES) {
      for (const scene of scenes) made.push(await shoot(browser, base, locale, scene));
    }
    made.push(await shootPlugin(browser, base));
  } finally {
    await browser.close();
    server.close();
  }
  console.log(made.map((n) => 'docs/images/' + n).join('\n'));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
