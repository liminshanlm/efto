# 插件窗口主题不跟主应用（file-share 浅/深不一致）

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/app/assets/plugin-sdk.js`、`crates/app/src/windows.rs`、`plugins/file-share/index.html`、
  `docs/plugin-development.md`、`docs/verification.md`

## 现象

主窗口浅色、file-share 插件窗口深色，同时出现。插件页面里写的是
`efto.isDarkMode() ? 'dark' : 'light'`，异常时回退 `dark`。

## 根因（两个叠加）

1. UI 侧 SDK 把 `isDarkMode` 实现成**布尔属性**（`isDarkMode: false`，随后赋布尔值），
   而 `plugin-sdk/efto.d.ts` 与文档约定它是**函数**。页面调用 `efto.isDarkMode()` 抛
   `TypeError`，被 `<head>` 里的 `catch` 吞掉 → 永远 `data-theme="dark"`。
   之前的插件 UI 重写只修了插件侧"当属性用"的写法，SDK 侧这个错实现没被发现。
2. 即使函数正常，插件窗口也没被套主题：`apply_theme` 只遍历 7 个常驻窗口，
   `open_plugin_ui` 建窗口时也没传 theme。`prefers-color-scheme` 只跟 Windows 系统设置，
   应用里固定"浅色/深色"时必然与主窗口分叉。

## 修法

- `plugin-sdk.js`：`isDarkMode()` / `isLightMode()` 改为函数，读窗口的
  `prefers-color-scheme`；取不到时回退深色（与插件样式表默认令牌一致）。
- `windows.rs`：新增 `theme_setting(app)`；`open_plugin_ui` 用
  `WebviewWindowBuilder::theme(Some(..))` 按设置锁定窗口主题（`System` 不设，跟随系统）；
  `apply_theme` 额外遍历 `UI_PREFIX`（`plugin-ui-*`）窗口，切换设置时同步已打开插件窗口。
- `file-share/index.html`：主题改为 `applyTheme()`，并监听 `matchMedia` 的 `change`
  在窗口打开期间跟随切换重画；其余 6 个一方插件（`ai-chat`、`hosts`、`ocr`、`quickweb`、
  `text-beautify`、`hello-js`）随后批量补上同一段监听。

## 验证

- `cargo check -p efto-app` 通过；`cargo test -p efto-app windows` 5 passed。
- `node --check` 通过 `plugin-sdk.js`、`file-share/ui.js`。
- 7 个插件 `index.html` 的内联脚本用 Node 逐段解析检查通过。
- 未验证：真实 GUI 下打开各插件切三态主题目视核对。
