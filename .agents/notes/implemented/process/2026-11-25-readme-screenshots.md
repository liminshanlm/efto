# README 界面截图：用前端页面渲染生成

- **Date**: 2026-11-25
- **Status**: done
- **相关**: `scripts/gen-readme-shots.mjs`、`docs/images/`、`README.md`、`README.zh-CN.md`、`frontend/`

## 为什么

README 里只有文字表格，"长什么样"全靠读者想象。要补典型使用截图，但**宿主的窗口层拿不到**：
窗口要靠热键唤起、抓屏要屏幕录制权限、还会把桌面背景一起拍进去，且开发机是 macOS、
而产物主战场是 Windows。页面层（`frontend/*.html` 与插件 `index.html`）与二进制无关，
因此改为**渲染同一份页面 + 注入假 IPC 回包**：界面像素来自真实 CSS/JS，只有列表数据是样例。

## 做了什么

- 新增 `scripts/gen-readme-shots.mjs`：起一个只读静态服务（`file://` 下 `i18n.js` 的
  `fetch('locales/…')` 会被 CORS 拦），用 Chromium 打开页面，`addInitScript` 注入
  `__TAURI_INTERNALS__`（`ui_language` / `theme_mode` / `search` / `search_files` / `icon_for` /
  `get_settings` / `platform_info` / `index_status` / `file_search_status` / `permission_status` /
  `runtime_status` / `plugin:event|listen`），再把窗口合成到深色渐变桌面（圆角 + 投影）上截 2x 图。
- 场景 7 个：默认面板、`wx` 应用搜索、`6*7` 计算器、`2026-11-16 15:03:22` 时间戳两行、
  `date` 八种系统时间、设置页「常规」、`text-beautify` 插件窗口。
- 主窗口与设置页各出中英两套（`<场景>.zh-CN.png` / `.en-US.png`，界面语言跟随设置；
  插件窗口文案由插件自己写死中文，两套共用一张）。
- 结果行的时间戳/八种格式数值、副标题形态按后端 `SearchHit` 抄；插件行的图标直接取仓库里
  真实的 `plugins/quickweb/logo.png`。应用行 `icon` 置空 → 退回首字头像（真实图标要宿主从
  exe/.lnk 抽，这里造不出来）。
- 两份 README 在「现在能做什么 / What works today」之前各加一节截图，并注明"由前端页面渲染、
  只有数据是样例"。

## 判据 / 结果

- 13 张 PNG 落在 `docs/images/`，两份 README 里 13 条引用全部命中（逐条 `[ -f ]` 校验过）。
- 重跑幂等：`NODE_PATH=$(npm root -g) node scripts/gen-readme-shots.mjs`（需
  `npm i -g playwright` + `npx playwright install chromium`，不入库、不进 `verify`）。
- **已知差异**：字体走 macOS 的 PingFang 而非 Windows 雅黑；设置页没有原生标题栏
  （页面本身无 chrome，只有内容区）；英文界面下内置指令名、应用名与结果副标题仍是中文——
  这是后端直出文案、本来就只做了中文，截图如实反映，README 里已写明。
- **顺带发现**：`frontend/settings.css` 的 `.desc { word-break: break-all }` 在英文长句下会断词换行
  （"falls back t / o English"），已在同一天单独修掉，见
  `.agents/notes/implemented/bug-fix/2026-11-25-settings-desc-word-break.md`。
