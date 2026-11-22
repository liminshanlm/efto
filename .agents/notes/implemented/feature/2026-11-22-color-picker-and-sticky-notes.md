# 新增颜色提取器与便签纸两个内置工具

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `docs/architecture.md`、`docs/help.zh-CN.md`、`docs/help.en-US.md`、`docs/verification.md`、`docs/macos-parity-todo.md`

## 背景

用户要两个新工具，都**默认不进常用工具**：

1. 颜色提取器：参考 PowerToys Color Picker，拾取屏幕任意位置的颜色。
2. 便签纸：输入一段文本，以便签纸的形式钉在屏幕上。

## 决策

- 两者都做**内置 Rust 工具**，不做插件。插件 SDK 没有屏幕像素读取能力，且便签需要
  无边框置顶窗口与独立存储，宿主实现更直接。
- 颜色提取器采用**冻结画面**取色（用户选择）：复用截图「收起搜索框 → 抓整屏」的
  `capture_after_hiding_main`，把 `Shot` 放进 `AppData.shot`，覆盖层 `color-picker.html`
  自己按 `view_w / innerWidth` 换算物理像素并用 canvas 采样。好处是完全复用截图的多屏/
  DPI 口径，不引入持续抓屏的性能成本；代价是动画/视频以冻结帧为准。
- 便签纸按用户选择做**多条**：每次触发新建一张，`~/.efto/notes.json` 存
  `id/text/x/y/width/height`，启动时恢复。窗口关闭即删除该条。
- 默认工具与已知工具拆成两个常量：`KNOWN_TOOLS`（9 个，设置页可勾选/可绑热键）与
  `DEFAULT_TOOLS`（原 7 个，默认面板与 `Ctrl+Alt+1..7` 只认这组）。这样新工具可以被用户
  手动加入常用工具，但不会默认出现，也不会静默占用新的全局热键。

## 坑与判据

- **冻结画面 + 双 label 坐标映射**：macOS 截图是每屏一个覆盖层，`display_mapping` 原来只认
  `screenshot-*`；颜色提取器用 `color-picker-*`，必须同时识别两个前缀，否则每屏坐标会退回
  窗口几何，取色位置整体偏移。
- **便签关闭语义与退出保护**：便签窗口 `Destroyed` 时删除该条；但宿主退出也会销毁窗口。
  在 `RunEvent::ExitRequested` 置 `QUITTING`，`Destroyed` 见到标记只跳过删除，避免退出时
  把 `notes.json` 清空。文本在输入去抖 300ms 后落盘，退出前最后几个字符可能丢失（未做同步
  flush）。
- **默认值迁移**：`Settings` 的 `tools` / `tool_hotkeys` 由 `#[serde(default)]` 只补缺失字段，
  老配置文件不会自动获得新工具；新用户默认只有原 7 个。`default_tool_hotkeys` 改成遍历
  `DEFAULT_TOOLS`，测试断言同步改为 `DEFAULT_TOOLS.len()`。
- **便签窗口**：`decorations(false)` + `always_on_top(true)` + `skip_taskbar(true)`，标题栏
  用 `start_dragging` 拖动；`resizable(true)` 在 Windows 无边框窗口上的实际手感未 GUI 手测。
- 颜色提取器关闭时先清 `AppData.shot` 再关窗；与截图互相打开前先关掉对方的覆盖层，避免
  两个全屏置顶窗口叠在一起。

## 验证程度

- `cargo test -p efto-core` 通过（新增关键词命中与默认工具断言）。
- `cargo check -p efto-app` 通过。
- 前端 `node --check`、`check-frontend.js`、`check-i18n.js` 通过。
- **未验证**：真实 GUI 取色像素/DWM/高 DPI/多屏、剪贴板落地；便签拖动/缩放/置顶/重启恢复/
  退出保留/并发保存；macOS 未构建未实测。详见 `docs/verification.md` 同日章节。
