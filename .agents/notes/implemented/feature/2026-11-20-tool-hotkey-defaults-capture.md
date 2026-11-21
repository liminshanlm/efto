# 单项热键默认值与按键录入 / 平铺窗口改名为操作窗口

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/core/src/config.rs`、`crates/app/src/commands.rs`、`frontend/settings.{js,html}`、
  `frontend/locales/*.json`、`crates/core/src/search/service.rs`、`docs/{help.zh-CN,help.en-US,architecture,verification}.md`

## 为什么

1. 单项热键输入框是普通文本框，加组合键时 Ctrl 会先被浏览器/系统吃掉，只能一个字符一个字符敲
   `Ctrl+Alt+1`，且敲错没有提示。
2. 七个内置工具默认全空，用户不知道能绑，也感受不到"按一下直接执行"的价值。
3. 面板里叫「平铺窗口」，但点开是五个窗口操作，名字和内容对不上。

## 做了什么

- 默认值：`default_tool_hotkeys()` = `KNOWN_TOOLS` 顺序的 `Ctrl+Alt+1..7`，挂在 `Settings::default()`；
  新增 `tool_hotkeys_seeded` 标记——老配置（字段缺失或表为空）加载时补默认值，
  设置页保存过（哪怕空表）就再也不补，"主动全部解绑"不会被悄悄装回去。
- 录入：`bindHotkeyCapture` 拦截 `keydown`，用 `e.code` 归一成 Tauri 写法
  （`KeyA`→`A`、`Digit1`→`1`，其余沿用 Code 名），裸字母/数字必须带修饰键；
  Backspace/Delete/Esc 清空；Tab/Shift+Tab 保留焦点切换。
- 改名：「平铺窗口」→「操作窗口」（中英文 locale + 设置页静态文案 + 面板标题）；
  搜索关键词保留 `平铺窗口`/`平铺`/`排列窗口`，老输入仍能命中；二级菜单的「网格平铺」行不变。

## 判据

- 默认值必须可解绑：只按"字段缺失"补，不按"表为空"补，否则用户清空后重启又回来。
- 录入用 `e.code` 而不是 `e.key`：后者受键盘布局与 Shift 影响（`Shift+1` 会变成 `!`），
  而 `e.code` 就是 global-hotkey 认的 Code 名，只有字母/数字需要剥前缀。
- 改名只改工具项，不改二级菜单里的动作名——「网格平铺」那一行仍叫「平铺窗口」，
  否则菜单里会出现"操作窗口 → 操作窗口"。

## 验证

- `cargo test -p efto-core` 216 passed（新增 `tool_hotkeys_seed_defaults_only_once`：
  补默认、清空后不回填、已有自定义不覆盖）。
- `cargo check -p efto-app` 通过；`node --check frontend/settings.js`、`check-i18n.js` 通过。
- 未做：真机按 `Ctrl+Alt+1..7`（是否被占用）、真实 WebView2 里试录组合键。
