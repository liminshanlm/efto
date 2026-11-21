# 首次唤出搜索框一闪而过

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/app/src/windows.rs`（`build_text_preview`）、`docs/verification.md`

## 现象

首次启动后打开搜索框（Alt+Space 热键、托盘菜单、二次启动转发都一样），
窗口一闪而过随即消失；第二次打开就正常。

## 根因

`show_main` 首次唤出主窗口时会顺带预热预览窗口（`warm_up_preview`，
只发生在第一次）。`build_text_preview` 虽然设置了 `focusable(false)` 和
`visible(false)`，但没有设置 `.focused(false)`——wry 建完 webview 后对
focused 的窗口调用 `ICoreWebView2Controller::MoveFocus(PROGRAMMATIC)`，
这一下会把**隐藏的预览窗口激活成前台**（`WS_EX_NOACTIVATE` 拦不住程序化
激活）。主窗口随即收到 `Focused(false)`，`hide_on_blur` 便把搜索框收起来。

第二次打开正常，是因为预热只发生一次：预览窗口已存在，不再有建窗抢前台
的动作。

## 排查过程（踩坑判据）

- 最初怀疑是 wry 宿主 `WM_SETFOCUS` → `MoveFocus` 的内部焦点转移，在
  `Focused(false)` 里加了 `GetFocus`+`IsChild` 判定——实测**没有**命中，
  该假设被证伪，探针已回退。
- 临时探针（`GetForegroundWindow` + 应用内窗口句柄枚举）抓到实锤：
  失焦瞬间前台窗口正是 `preview` 窗口句柄，且时间点与预览窗口建窗完成
  （约首次唤出后 270ms）吻合。
- wry 源码 `webview2/mod.rs`：`if attributes.focused { controller.MoveFocus(...) }`
  即为直接证据；`tauri.conf.json` 里所有常驻隐藏窗口都写了 `"focus": false`，
  运行时创建的预览窗口漏掉了对应设置。

## 修复

`build_text_preview` 补一行 `.focused(false)`（`crates/app/src/windows.rs`）。

## 验证

用 EnumWindows 检查主窗口可见性（debug 构建，RUST_LOG=info）：

- 启动后：不可见（正常）；
- 首次唤出（二次启动走 single-instance 转发）：**保持可见**（修复前此步会隐藏）；
- 启动 notepad 抢走前台（真实失焦）：窗口隐藏——`hide_on_blur` 仍正常；
- 再次唤出：正常显示。

`cargo test -p efto-core -p efto-jsrt` 272 项全部通过。热键（Alt+Space）
与托盘菜单路径走同一个 `show_main`，未单独自动化验证（SendKeys 在当前
会话里触发热键失败），需要用户日常使用确认。
