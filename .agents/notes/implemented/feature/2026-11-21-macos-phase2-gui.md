# macOS 支持 Phase 2：GUI 宿主（透明窗口/图标/设置页平台化/插件平台门）

- **Date**: 2026-11-21
- **Status**: done（GUI 冒烟通过；人手验证清单见 `docs/verification.md` mac Phase 2 段）
- **相关**: `docs/macos-support-plan.md`、`crates/app/src/{icons,commands,hotkey,i18n,hosts}.rs`、
  `crates/core/src/plugins/{manifest,registry}.rs`、`frontend/{i18n.js,settings.js,settings.html}`、
  `crates/app/{tauri.conf.json,Info.plist}`

## 现象

Phase 0/1 后 CLI 主链路可用，但 GUI 在 mac 上只是"能启动"：Dock 冒图标、图标提取报
"仅支持 Windows"、设置页的工具/Shell/终端模式全是 Windows 形态、hosts 插件在 mac 上
仍可被唤起、通用动作热键注册了却什么也做不了。

## 做法

- **窗口形态**：`tauri.conf.json` 开 `macOSPrivateApi`（保持透明胶囊视觉，dmg 直发无碍，
  App Store 是 Phase 4 的事——用户定夺）；`Info.plist` 的 `LSUIElement` + 启动时
  accessory 激活策略双保险（裸 `cargo run` 没有 bundle 也能无 Dock 图标）。
- **图标**：`icons.rs` 用 NSWorkspace via objc2（tauri 依赖树本就有 objc2 0.6 /
  objc2-app-kit 0.3，零新增 C 工具链——用户定夺）。`TIFFRepresentation →
  NSBitmapImageRep → PNG`；文件不存在先报错，不让 NSWorkspace 的通用图标充数
  （对齐 SHGetFileInfoW 语义）。
- **前端平台化**：新增 `platform_info` command 作为单一事实源（tools/shells/
  default_shell/features），设置页按它显隐工具行、Shell 下拉、WSL 行、终端模式、
  通用动作行；热键展示文案在 i18n 解析出口装饰（`Alt`→`Option`、`Ctrl`→`Control`，
  Rust `decorate_keys` 与前端 `decorateKeys` 同规则、语法说明 key 豁免），录入/存储
  仍是 Tauri 语法。
- **插件平台门**：清单加插件级 `platform`（复用 cmds[].platform 的 PlatformSpec 词汇
  win32/darwin/linux，不是计划初稿的 `["windows"]`）；registry search/get 收口，
  `cli plugins` 标注、`plugin-run` 报 `unsupported_platform`；hosts 插件标 win32。
- **入口收口**：通用动作热键不再注册（`supported_on_platform` 门）；hosts 能力
  path/read/save 三入口非 Windows 报"当前平台不支持：hosts"。

## 判据

- 单测全绿（app 110 / core 224 / jsrt 48）：NSWorkspace 图标两条（Finder 真机取图 +
  缺文件报错）、hosts 平台报错、清单 platform 门、registry 平台过滤、decorate_keys
  词边界（CmdOrCtrl 不被拆成 CmdOrControl）、platform_info 形状。
- GUI 冒烟（dev-run）：热键 Alt+Space 注册成功、单项热键自动收敛为 mac 3 工具、
  托盘就绪、`通用动作 未启用`（平台门生效）。
- check-i18n 404 key 双语对齐；check-frontend 过。

## 没验证 / 留给后续

- 人手 GUI 清单（Option+Space 实际触发、设置页形态、插件 UI 的 efto://、托盘交互）——
  逐条列在 verification.md，热键触发还牵着风险 4（Carbon vs CGEventTap 免权限结论）。
- clipboard.html 的 Ctrl+Enter 未做 Cmd 适配；Intel mac 未构建；Phase 3（mdfind/
  系统设置/系统动作/文件剪贴板）未动工。
