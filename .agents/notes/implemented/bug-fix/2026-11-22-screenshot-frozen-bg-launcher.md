# 截图底图里带着启动器：收窗口没落地 + 预览预热占住主线程

- **Date**: 2026-11-22
- **Status**: done
- **相关**: docs/verification.md「截图底图里带着启动器」、crates/app/src/windows.rs `hide_main_native`、crates/app/src/commands.rs `run_action("screenshot")`

## 现象与两轮误判

用户报「截图时搜索框自己跳出来、点复制就消失」。日志里会话期间
`appkit_visible=false`、`CGWindowList` 也没有主窗口——API 与肉眼矛盾，
前两轮一直去找"谁调了 show_main"（其实没有人调）。

## 真因（两层）

1. **启动器被拍进冻结底图**：`capture_virtual_screen` 读窗口服务器画面，
   而 `hide()` 的 `isVisible` 立刻为假，合成器却还要再画一两帧。
2. 更关键的是**收窗口本身没落地**：`park_before_hide` 用 tao 的
   `set_position`，实测出现 `nsVisible=false` 但 `cocoa=(488,276)` 没被挪走、
   `cg` 在屏列表里还在（同一窗口号持续数秒）。
3. **触发条件**：`show_main` 里顺手 `warm_up_preview`，WKWebView 初始化占用
   主线程 1~3 秒。用户"唤起启动器 → 马上点截图"时，`hide_main` 的投递排在
   预热后面，抓图时启动器还在画面上；等待超时就直接报错——表现为
   "截图界面出不来了"。

## 修复

- mac 上 `hide_main` 直接对 NSWindow 操作（`setFrameOrigin:` + `orderOut:`，
  主线程同步完成），不再经 tao 的异步投递；
- mac 上**彻底不预热预览窗口**（`warm_up_preview` 只留 Windows）：先试过
  「启动后 2 秒预热」，但用户「刚启动就点截图」仍会撞上 3000ms 超时——
  WKWebView 初始化要占主线程 1~3 秒，Shift 首次预览慢一点可以接受；
- 截图前轮询 `CGWindowList` 确认主窗口已消失（最长 3 秒），超时明确报错；
- `exclude_from_capture` 的 `setSharingType:` 改回主线程（后台线程调用会
  SIGTRAP，崩溃报告 `efto-2026-11-22-122048.ips`，长截图路径也会踩）。

## 判据（复用）

本地复刻"唤起启动器 300ms 后点截图"：修复后 `hide→server 等 25ms`、零超时，
冻结底图中心亮度 14（p99.9=131，只有压暗的终端），无启动器面板。
**不要**用 `is_visible()` 判断"画面上有没有它"——它比合成器早。
## 追加：窗口服务器表面状态卡住

用户环境里出现更顽固的形态——`orderOut` 之后 AppKit 状态正确，窗口服务器却一直
按旧位置画着（`cg` 仍列出该窗口号），连 `setAlphaValue:0` 也不生效，普通显示器
捕获照样把启动器拍进底图。本轮处置：

1. **不再把主窗口挪到 `(-32000,-32000)`** 再藏：这种极端坐标是窗口事务卡住的常见
   诱因（远程控制/窗口管理守护进程在场时更容易踩）。
2. **兜底捕获**：检测到"AppKit 已收起、服务器仍列着"就改走
   `CGWindowListCreateImage(.optionOnScreenBelowWindow)`，从主窗口之下逐屏合成。
   同一时刻实测：普通捕获启动器区域亮度 225，兜底捕获 23。
   兜底图本来缺系统菜单栏（本窗口层级之下拿不到），用一张普通捕获把每块屏顶部
   那一条拼回去：实测启动器区域 225→24、菜单栏区域 208（完整）。只在异常环境触发。
3. 判据依旧是"窗口服务器在屏列表"，但**只用来决定走哪条捕获路径**，不再拿它
   当"能不能截图"的闸门（那条路会一直等不到，用户看到的就是"点了没反应"）。

