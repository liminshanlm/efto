# 文件动作菜单透明且被顶到窗口顶部

- **Date**: 2026-11-22
- **Status**: done
- **相关**: frontend/style.css、frontend/app.js

## 现象

搜索结果文件行点右侧「⋯」后，菜单背景透明、下层文件名与菜单文字重叠，且菜单出现在窗口左上角覆盖整个列表。

## 根因

1. `style.css` 的主窗口令牌里没有 `--panel`（只有 clipboard.css / files.css 定义过），`.file-menu { background: var(--panel) }` 解析失败 → 背景透明。
2. 主窗口只有 520×320，9 个菜单项约 290px 高。`openFileMenu` 把锚点传成整行 `li`，`top = rect.bottom + 4` 又被 `innerHeight - offsetHeight - 8` 夹到 22px，于是菜单贴着顶部；同时左对齐行首，进一步盖住标题。

## 修复

- `:root` / 浅色主题补上不透明的 `--panel`（`#1a1d26` / `#ffffff`）。
- `.file-menu` 增加 `max-height: calc(100vh - 16px)` + `overflow-y: auto`，窗口放不下时可滚动，末项不被裁掉。
- 锚点改为「⋯」按钮：菜单右对齐按钮，下方放不下时翻到按钮上方；Tab 打开时同样取该行的 `.row-file`。

## 验证

- `node --check frontend/app.js`、`scripts/check-frontend.js`、`scripts/check-i18n.js` 通过。
- GUI 需重新构建后人工确认（前端资源打进二进制，改 CSS/JS 必须重编译）。
