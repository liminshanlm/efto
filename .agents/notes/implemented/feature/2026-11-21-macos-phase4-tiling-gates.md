# macOS 支持 Phase 4：窗口平铺、通用动作放出、权限引导与发布工程（截图跳过）

- **Date**: 2026-11-21
- **Status**: done（截图按用户决定跳过，单独留待后续；人手验证清单见 `docs/verification.md`）
- **相关**: `docs/macos-support-plan.md`、`crates/app/src/{tile,ax,permissions,universal,commands}.rs`、
  `frontend/settings.{html,js}`、`crates/core/src/{config,search/service}.rs`

## 现象

Phase 3 后 mac 的系统集成底层就绪（winclip/CGEvent/CGWindowList），但窗口平铺是 Win32 整块、
权限（辅助功能/屏幕录制）没有任何引导入口、file_buffer/通用动作的开关仍收口。

## 做法

- **窗口平铺**（并行子代理）：`ax.rs` 裸 extern FFI（AXUIElement*/AXValue/`_AXUIElementGetWindow`
  /CGDisplay*/CGRectMakeWithDictionaryRepresentation）+ `tile.rs` mac 版 run_op（Grid/半屏/
  最小化/Undo 全实现，Report 文案与 Windows 逐字对齐）。布局纯函数复用，Undo 全局记录
  两平台同位置同语义。
- **权限引导**：新 `permissions.rs`（AXIsProcessTrusted / CGPreflightScreenCaptureAccess 单点）
  + `permission_status`/`request_permission`/`open_permission_settings` 命令 + 设置页"权限"
  区块（仅 mac 展示，去授权按钮直开对应系统设置页——pane id 复用 `sys_settings::pane_uri`）。
- **开关放出**：`BuiltinAction::supported_on_platform` 放出 TileWindows / UniversalActions /
  FileBuffer（通用动作链路 = send_copy + 剪贴板 + 动作窗，Phase 3 底层已通）；
  `KNOWN_TOOLS` mac 增到 5 项。Screenshot / ShortcutGuide 仍收口（截图跳过、指南缺数据集）。
- **发布工程**：`bundle.macOS.minimumSystemVersion` + 计划文档附签名/公证/双架构步骤。
- **截图**：两个子代理（整面 + 最小面）均被取消且未落盘，按用户决定跳过，screenshot.rs 保持原样。

## 判据

- 单测全绿：app 142 / core 240 / jsrt 48（tile+ax 22 个：布局、Cocoa 坐标换算、显示器归属、
  错误文案分类；permissions 形状测试）。
- 实机：`cli tile-windows --dry-run` 产出 16 窗 4×4 计划；`--mode left` 精确报
  `缺少辅助功能（Accessibility）权限：…`（本机终端无授权）；`--mode undo` 空记录不误要权限。
- **本机（终端）无辅助功能/屏幕录制授权**：真实移动窗口与实抓屏幕均未验证，缺权限报错
  路径已实测；授权后 tile 的 `tile_then_undo` 测试会自动跑。

## 没验证 / 留给后续

- 真实平铺+Undo（需辅助功能授权）、CGEvent 实键（通用动作/粘贴）、实抓屏幕——人手清单。
- **截图捕获层**（ScreenCaptureKit/CG）：用户决定跳过，独立立项；screenshot 入口保持收口。
- 快捷键指南 macOS 数据集（shortcuts/data）；`_AXUIElementGetWindow` 是长期稳定的私有导出
  （yabai/Hammerspoon 同用），极端情况下是链接期失败。
