# macOS 语言探测缺失：GUI 从 Finder 启动显示英文

- **Date**: 2026-11-21
- **Status**: done
- **相关**: `crates/app/src/i18n.rs`（`system_is_chinese`）

## 现象

系统是简体中文，应用界面却是英文。CLI 下正常（shell 里 `LANG` 是 zh），从 Finder/
`open` 启动的 GUI 是英文。

## 根因

`system_is_chinese()` 的环境变量检查（LC_ALL/LC_MESSAGES/LANG）之后，非 Windows
分支直接 `false`——注释写的"其它平台没有可靠信号时按英文处理"。但 **macOS GUI
应用不继承 shell 的 LANG**，Finder 启动时环境变量检查必然落空，于是 `auto` 语言
一律回退英文。mac 上其实有可靠信号：CFLocale 语言首选项（`zh-Hans-CN`）。

## 做法

macOS 分支改走 `CFLocaleCopyPreferredLanguages`（CoreFoundation 裸 extern），
首个首选项以 `zh` 开头即中文；环境变量优先级保持不变（CLI/测试可覆盖）。
前端经 `ui_language` 命令走同一 resolve，一处修全覆盖。

## 判据

- `env -i`（清空 LANG 等，模拟 Finder 启动）下 `cli plugin-run hello-js greet x`
  输出 `"locale":"zh-CN"`（修复前是 `en-US`）。
- 新增 `mac_locale_detection_matches_cfl` 测试；verify.sh 全绿。
- GUI 界面实际显示中文留人工确认。
