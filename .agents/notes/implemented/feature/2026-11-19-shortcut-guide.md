# 应用感知的快捷键指南

- **Date**: 2026-11-19
- **Status**: done
- **相关**: `docs/todo.md` 第 5 项、`docs/architecture.md` 内置工具、`docs/help.zh-CN.md` / `help.en-US.md`

## 做了什么

- 新增 `crates/core/src/shortcuts/`：serde 数据模型、`resolve(exe)`、`by_exe` / `by_id` / `apps`。
  内置 JSON 用 `include_str!` 嵌入（windows/word/excel/powerpoint/outlook/chrome/edge/
  firefox/vscode/photoshop/figma/idea），解析结果用 `once_cell::Lazy` 缓存。
- 用户覆盖：`~/.efto/shortcuts/<exe>.json`。同名分组按 `keys` 不区分大小写合并，用户条目覆盖；
  新分组/新条目追加；文件损坏打日志并忽略，不影响内置数据。
- 新增 `crates/app/src/guide.rs`：热键按下、`toggle_main` 之前调 `winproc::foreground()` 记录前台窗口，
  跳过 efto 自身；连续按热键时保留上一次记录。
- `winproc` 增加 `foreground()` / `info(hwnd)`；`shortcut_guide_data` 执行时复核窗口仍存在。
- `run_action("shortcut_guide")` 打开常驻隐藏窗口 `shortcuts`，关闭只隐藏；
  `tauri.conf.json` 以 `create: false` 延迟创建。
- 新增 `frontend/shortcuts.html/js/css`：顶部显示识别到的应用名与 exe、下拉切换应用、
  窗口内过滤、分组点击折叠、`Esc` 关闭、`Ctrl+F` 聚焦筛选。
- `KNOWN_TOOLS` / `BuiltinAction` 增加 `shortcut_guide`，设置页与默认面板同步。

## 边界与判据

- 未收录应用不报错：回退 Windows 通用快捷键并提示"未收录 {exe}"；没有前台窗口时也回退。
- 应用识别只读进程名与窗口标题，不监听键盘、不注入其他进程。
- 数据错误在单测暴露：JSON 可解析、exe 映射唯一、组内 `keys` 唯一、`action` 非空。
- 不在搜索管线新增任何搜索源，不改变打分与触发判定。

## 验证

- `cargo test -p efto-core shortcuts` 通过（12 份内置 JSON 的解析/唯一性/合并测试）。
- `cargo check -p efto-app` 通过。
- 未做 GUI 手测：真实应用切换后的识别与下拉切换需要在桌面环境里点一遍。
