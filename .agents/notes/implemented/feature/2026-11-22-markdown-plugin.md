# 新增 Markdown 预览编辑插件与插件文本文件 API

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `plugins/markdown/`、`crates/app/src/fileapi.rs`、`docs/architecture.md`、`docs/plugin-development.md`、`docs/verification.md`、`docs/macos-parity-todo.md`、`plugin-sdk/efto.d.ts`

## 背景

用户要一个参考夸克 Qdoc 的 Markdown 插件：文档 / 源码双模式、编辑并保存回原文件、
快捷插入工具栏、导出 / 打印。插件 main 没有 `fs`，UI 只拿得到路径，读不到内容，因此
先补宿主能力，再写插件。

## 决策

- **宿主新增受限文本文件 API**（用户选择），而不是要求本机 Node：
  - `plugin_file_read/write/save_as` 放在 `crates/app/src/fileapi.rs`，UI 侧暴露为
    `efto.file.read/write/saveAs`。
  - 授权模型 `FileGrants`（插件 id → canonical 路径集合）只接受两个来源：`plugin_pick_files`
    返回值与落在 `plugin-ui-*` 窗口上的 `WindowEvent::DragDrop`；`saveAs` 成功后把用户选中
    的路径加入授权。插件不能自行拼路径读任意文件。
  - 只处理普通文件 + UTF-8，上限 8 MiB；读取去 BOM；写入走 `fsutil::write_atomic`。
  - `saveAs` 取消返回 `null`，不做“静默成功”。
- **打开入口在插件内**（用户选择）：输入 `md` / `markdown` 进插件，用「打开文件」或拖入
  载入，可新建空白文档；不改搜索结果行为。
- **渲染器为纯函数** `plugins/markdown/render.js`：先转义 HTML 再生成标签，不解析原始 HTML；
  Node 测试与浏览器共用同一份。
- 触发词后的输入作为新文档初始内容（`seed`），方便 `cli plugin-run` 验证，不当作文件路径
  （路径没有授权来源，读了也会被拒绝）。

## 坑与判据

- **拖拽授权必须挂窗口事件**：HTML5 `drop` 拿不到绝对路径；宿主在 `WindowEvent::DragDrop`
  里按 `plugin-ui-` 前缀反查插件 id 再 `grant`。插件页面同时要 `preventDefault` 掉
  `dragover/drop`，否则 webview 会直接导航到被拖入的文件。
- **`plugin_of` 复用**：`ui.rs` 里拆出 `plugin_of` / `plugin_id_by_label`，都走同一个
  `resolve_plugin_id`（label 去前缀 → 插件列表里按 `sanitize_key` 匹配），避免两处判定漂移。
- **未闭合代码围栏**：按常见实现延伸到文末；测试一开始写反了预期，已改为断言“后续内容仍在
  代码块内”。渲染循环的兜底消费一行保证任何输入都不会死循环。
- **预览里的图片**：`efto://` 页面加载不了相对路径 / `file://` 图片，只认 http(s) 绝对地址；
  这是当前限制，导出 HTML 后不受影响，已在 verification 里写明。
- **打印**：走 `window.print()`，打印前强制切到文档模式；打印 CSS 只保留纸面内容。

## 验证

- `cargo test -p efto-app fileapi`：4 项通过。
- `node plugins/markdown/tests/run-all.js`：24 项通过；`node --check` 三个 JS 通过。
- `cli plugins` 发现 markdown；`cli search md` 首条为「Markdown 编辑器 [exact]」；
  `cli plugin-run markdown md "# hi" --json` 返回 `ok: true` 且收到 `[ui→page:seed]`。

## 没验证

GUI 实跑（模式切换观感、拖入授权、打开/保存/另存/导出的系统对话框、打印预览、主题、
快捷键、未保存确认）与 macOS 全部未实测。
