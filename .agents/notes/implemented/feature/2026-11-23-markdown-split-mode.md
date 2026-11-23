# Markdown 插件编辑模式（左源码右实时预览）

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `docs/verification.md`（Markdown 插件编辑模式）、`docs/macos-parity-todo.md`、`docs/help.zh-CN.md`、`docs/help.en-US.md`

## 为什么

用户要边写边看：源码模式和文档模式来回切，改一句要按一次 `Ctrl+E` 才能确认效果。

## 改了什么

只动插件前端，宿主零改动：

- `index.html`：模式段加 `data-mode="split"` 的「编辑模式」；textarea 包进 `#editorPane`，
  `.workspace` 带 `data-mode` 供 CSS 分栏。
- `ui.js`：`switchMode` 接受三态（`showEditor = mode !== 'preview'`、`showPreview = mode !== 'source'`）；
  `schedulePreview` 120ms 防抖，`input` / 工具栏插入在 split 下刷新预览；`applyDoc` 在非源码模式渲染；
  `Ctrl+E` 改为 文档 → 编辑 → 源码 循环。
- `style.css`：`.workspace` 改 flex，`.editor-pane` / `.preview-wrap` 各 `flex: 1 1 50%` 且各自滚动；
  split 下收窄 paper/editor 内边距并加左分隔线；`@media print` 隐藏 `.editor-pane`。

## 验证

- `node --check plugins/markdown/ui.js`、`node plugins/markdown/tests/run-all.js`：24 项通过。
- 静态起 `plugins/markdown` 用浏览器实测：左右各 50%、独立滚动、输入即时渲染成 `<h1>`/`<p>`。
- 未验证：真 efto webview、`Ctrl+E` 三态循环手测、打印预览（详见 `docs/verification.md`）。
