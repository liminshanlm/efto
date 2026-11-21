# 托盘「帮助文档」窗口

- **Date**: 2026-11-19
- **Status**: done
- **相关**: `docs/todo.md` 第 2 项、`docs/help.zh-CN.md`、`docs/help.en-US.md`

## 事实

- 托盘菜单顺序：显示搜索框 / 设置… / 帮助文档 / 分隔线 / 退出 efto。
- 帮助窗口 label `help`，`tauri.conf.json` 里 `create: false`，由 `windows::show_help`
  按需创建；重复点击走 `get_webview_window` 分支只聚焦，不新建。
- 帮助内容来自 `docs/help.zh-CN.md` / `docs/help.en-US.md`，`include_str!` 嵌入二进制，
  断网可用，不读外部文件、不允许用户覆盖（保持单一事实源）。
- 前端 `help.html` + `help.js` 自带一个最小 Markdown 渲染器（标题/列表/表格/代码块/链接），
  不引入 npm 或 CDN。目录由渲染后的标题生成，顶部输入框按标题过滤。
- 托盘每次打开都会重新拉 `help_document`，语言设置改了以后下次打开就是新语言。

## 判据

- 帮助文档章节覆盖：快速开始、搜索与启动、直接打开、内置搜索源、预览、常用工具、
  插件、设置项、托盘、CLI、数据目录、限制与排障、命令执行。
- 关闭帮助窗口不影响主搜索框热键与焦点（窗口关闭事件里只做隐藏/销毁，不重建主窗口）。

## 验证

- `cargo check --workspace`、前端静态守卫通过。
- 帮助窗口的实际排版/表格/代码块未在真实 webview 里逐项核对，已写入
  `docs/verification.md` 的「没验证过的」。
