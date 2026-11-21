# 窗口操作二级菜单（最小化 / 半屏 / 撤销）

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `docs/architecture.md` 内置工具、`docs/help.zh-CN.md` / `help.en-US.md`、`docs/verification.md`「窗口操作二级菜单」、`docs/todo.md` 第 6 项

## 为什么

原来「平铺窗口」只有一个动作。用户要的是"把除了搜索框下方的窗口，其他窗口最小化"，
同类需求还有全部最小化、左右半屏、撤销平铺。设计上选择**在平铺工具内展开二级菜单**，
而不是再拆几个独立工具，也不是每次点击循环切模式——入口保持一个，操作在菜单里一次选清。

## 做了什么

- `crates/app/src/tile.rs` 增加 `Op` 枚举：`Grid` / `HalfLeft` / `HalfRight` /
  `MinimizeOthers` / `MinimizeAll` / `Undo`，`Op::from_name` 同时服务 CLI `--mode`。
- 新增纯函数 `half_layout(work, right, gap, margin)`；`grid_layout` 保持原判定。
- `run_op(main_hwnd, op, dry_run)` 统一入口：
  - 网格：按主窗口定位显示器，`enumerate(Some(monitor), None)`，只动本屏窗口。
  - 半屏：`target_window()` 取一个窗口，套 `half_layout`。
  - 最小化：`enumerate(None, keep)`，**不限显示器**；`keep` 既不进结果也不计 `skipped`。
- `target_window()` 的"搜索框下方的窗口"：前台可见且不是 efto 自己时直接用前台
  （单项热键路径）；否则回落到 `guide::last()`（launcher 路径）；标题为空或
  `IsWindow == 0` 明确报错，不静默成功。
- 撤销：`LAST_TILE: Mutex<Vec<SavedWindow>>` 在移动前快照窗口矩形与最大化状态，
  成功后写入；`Undo` 还原后清空。原最大化窗口用 `SW_MAXIMIZE` 还原，其余 `SetWindowPos`。
- 移动后逐个复核实际矩形（偏差 >2px 计失败），批量 `BeginDeferWindowPos` 失败时逐个重试。
- `commands.rs` 新增 `window_min_others` / `window_min_all` / `window_half_left` /
  `window_half_right` / `window_undo_tile`，统一走 `run_window_op`：先执行、成功后再隐藏搜索框。
- `cli.rs` 的 `tile-windows` 增加 `--mode grid|left|right|min-others|min-all|undo`，
  `--json` 输出带 `work/candidates/layout/tiled/skipped/failed/message`。
- 前端 `app.js`：`isWindowTool` 命中 `tile_windows` 时不再直接执行，而是用合成的
  `source: 'winop'` 行替换结果列表；`windowOpsActive` 控制"单击即执行"、`Esc` 退出、
  输入即退出，并挡住晚到的搜索/文件检索结果。菜单行不带 `group`，不会触发工具面板网格。
- 文案：`zh-CN.json` / `en-US.json` 新增 `main.source.winop` 与 `main.winop.*`，
  设置页与帮助文档同步改为「窗口操作」。

## 边界与判据

- 网格/半屏只处理搜索框所在显示器；最小化作用于所有显示器（用户预期"其他窗口都收起来"）。
- 最小化/关闭/工具窗口分类：最小化窗口计 `skipped`；efto 自身、桌面、任务栏、
  工具窗口、owned 窗口、cloaked 窗口一律不参与。
- `LAST_TILE` 只在进程内有效，不落盘；最小化不写入撤销记录，所以撤销不会恢复被最小化的窗口。
- 单项热键仍由其后端回调直接执行 `Grid`，不展开菜单——热键的语义是"按一下就做完"。

## 验证

- `cargo check -p efto-app` 通过（0 告警）；`cargo test -p efto-app tile` 9 passed。
- CLI `tile-windows` 各模式 `--dry-run` 实测：grid 10 候选 / 跳过 11；left 948x1024；
  min-others 正确保留目标窗口；min-all 12 个（含副屏）；undo 无记录返回退出码 3；
  未知模式退出码 2。
- `node --check frontend/app.js`、`check-i18n.js`（394 keys × 2）、`check-frontend.js` 通过。
- 未验证：真实 GUI 点击菜单 / `Esc` 返回、实际移动/最小化/还原窗口、多屏不同 DPI 的坐标换算。
