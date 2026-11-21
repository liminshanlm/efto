# 一键平铺窗口

- **Date**: 2026-11-19
- **Status**: done
- **相关**: `docs/todo.md` 第 6 项、`docs/architecture.md` 内置工具、`docs/help.zh-CN.md` / `help.en-US.md`

## 做了什么

- `core` 的 `KNOWN_TOOLS` / `BuiltinAction` 增加 `tile_windows`，默认面板与搜索都能进入。
- 新增 `crates/app/src/tile.rs`：
  - `grid_layout(work, n, gap, margin)` 纯函数，`cols = ceil(sqrt(n))`、`rows = ceil(n/cols)`，
    最后一行不足时按实际个数均分整行宽度；间距与四周留白固定 8px。
  - Win32 层 `EnumWindows` 过滤可见、有标题、非最小化、非 `WS_EX_TOOLWINDOW`、无 owner、
    非 cloaked（`DwmGetWindowAttribute(DWMWA_CLOAKED)`）、非 efto 自身进程、非
    `Progman` / `WorkerW` / `Shell_TrayWnd`，并且只保留目标显示器上的窗口。
  - 最大化窗口先 `ShowWindow(SW_RESTORE)`，再 `BeginDeferWindowPos` 批量移动；
    批量失败时逐个 `SetWindowPos` 重试并把失败计入报告。
- `run_action("tile_windows")` 用主窗口句柄定位显示器，先 `hide_main` 再平铺。
- CLI 新增 `tile-windows [--json] [--dry-run]`，dry-run 输出候选窗口与目标矩形。

## 边界与判据

- 最小化窗口只计入 `skipped`，不还原、不弹出。
- 平铺不改变 z-order、不抢焦点（`SWP_NOZORDER | SWP_NOACTIVATE`）。
- 桌面/任务栏/工具窗口/owned 窗口不参与，避免"平铺完桌面图标也动了"。
- 只动搜索框所在显示器，副屏位置不变。

## 验证

- `cargo test -p efto-app`：`grid_layout` 覆盖 n=0/1/2/3/5..9、最后一行均分、窄工作区、矩形不重叠。
- `cargo check -p efto-app` 通过。
- 未做 GUI 手测的部分：多显示器与真实 UWP 窗口的失败计数（代码路径已按窗口统计）。
