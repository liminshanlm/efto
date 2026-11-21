# P1：片段 / 通用动作 / 文件动作与缓冲 / 剪贴板 2.0

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `docs/todo.md` 第 3 项、`docs/architecture.md`、`docs/help.zh-CN.md`、`docs/help.en-US.md`、
  `docs/verification.md`、`docs/roadmap.md`

## 做了什么

- **片段**：`core/src/snippets.rs`（`~/.efto/snippets.json`）提供 `SnippetStore`；
  搜索只在前台输入与关键词整串相等（trim、忽略大小写）时产出 `ItemKind::Snippet`（分数 720）。
  正文支持 `{date}`/`{time}`/`{datetime}`；回车复制展开结果，有前台窗口时延迟粘贴。
  设置页新增“片段”页负责增删改。
- **通用动作**：`app/src/universal.rs` + 第二全局热键（默认 `Ctrl+Alt+Space`，设置可关/可改）。
  `capture` 记住前台窗口 → `SendInput` 发 `Ctrl+C` → 读剪贴板文本 → 打开 `actions` 常驻窗口。
  动作固定为复制/粘贴回前台/网页搜索，链接或已存在路径追加“打开”。
- **文件动作与缓冲**：文件结果 `Tab` / `⋯` 打开主窗口内动作菜单；`Ctrl+Enter` 加入缓冲。
  `Shared.file_buffer` 去重、上限 100；`files` 常驻窗口批量复制到剪贴板/复制到…/移动到…/删除/清空。
  删除统一 `SHFileOperationW` + `FOF_ALLOWUNDO`。
- **剪贴板 2.0**：`clipwatch` 轮询优先级改为 文件列表（CF_HDROP）> 图片 > 文本；
  图片落盘 `~/.efto/clipboard-images/`，缩略图按需生成；置顶条目不参与 200 条上限与 30 天清理；
  `Ctrl+Enter` 复制后隐藏主窗口并 `SendInput` 粘贴到前台。

## 关键判据

- 片段**不做**前缀/子序列匹配，避免日常输入被劫持；测试覆盖 `si`、`signature`、`sig extra` 不出片段。
- 文件去重按 Windows 路径大小写不敏感；剪贴板 v1 纯文本 JSON 仍可反序列化。
- 通用动作抓不到选中文本时显示空态提示，动作执行返回明确错误。
- 目标窗口以更高权限运行导致 `SetForegroundWindow` 失败时，明确报错并保留剪贴板内容。
- 新建窗口 `actions` / `files` 在 `tauri.conf.json` 里 `create: false`，由 `create_config_windows`
  统一 build，关闭时 `prevent_close` + `hide`。
- 文件缓冲只能从搜索结果加入，不支持资源管理器拖入；通用动作 v1 只抓取文本，不处理选中的文件
  （均已写入 `verification.md` 的"没验证/未实现"）。

## 踩坑

- `windows-sys 0.59` 的 `GlobalFree` 在 `Win32::Foundation`，不在 `Win32::System::Memory`。
- `ShellExecuteW` 需要 `Win32_UI_WindowsAndMessaging` feature；只在 app crate 开时靠 feature 合并能过，
  `cargo test -p efto-core` 单独构建会失败，因此 core 显式声明。
- `query_with_sources` 新增第 8 个参数后，CLI 通道也要加载 `SnippetStore` 并传入，否则片段在 CLI 下不可见。

## 验证

- `cargo test --workspace`：core 203 / app 100 / jsrt 48 全过。
- `node scripts/check-i18n.js`、`node scripts/check-frontend.js` 通过。
- 未手测：真实 `SendInput` 粘贴（UWP/管理员窗口）、图片/文件剪贴板落到 Word/微信、
  新窗口渲染与多显示器、网络路径批量操作。详见 `docs/verification.md`。
