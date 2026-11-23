# macOS 同步清单 A/B 项：隐藏「打开方式」入口 + 运行时探测/文案按平台分流

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `docs/macos-parity-todo.md`（第 21/22/30 行）、`docs/verification.md`、
  `docs/architecture.md`、`docs/plugin-development.md`

## 背景

`docs/macos-parity-todo.md` 里 12 条「待同步」按性质分三类，本次处理前两类：

- 真缺失：`assoc::register` 在非 Windows 直接报错，但设置页的插件卡片**无条件**渲染
  「注册到「打开方式」」按钮（只要清单声明了 `fileTypes`），mac 用户点一下就吃错误。
- 需适配：`exec::detect_on_path` 只读进程 `PATH`；Finder / Dock 启动的 app 不执行
  shell 的 rc 文件，`/opt/homebrew/bin`、`/usr/local/bin` 常常不在 PATH 里，自动探测在
  mac 上大概率什么也找不到。安装引导文案同样是 Windows 说法（「Add python.exe to PATH」、
  `C:\...` 占位符）。

## 改动

1. `crates/app/src/commands.rs`：`platform_info.features` 增加 `file_assoc = cfg!(windows)`
   —— 前端按这份清单展示/隐藏，不自己猜平台（沿用既有约定）。
2. `frontend/settings.js`：插件卡片的注册按钮改为宿主下发 `file_assoc === false` 时才隐藏；
   `platform_info` 读取失败（fallback `features: {}`）仍按全量展示，避免降级时少一个入口。
   「可打开 .md」这类说明行不受影响。
3. `crates/core/src/exec.rs`：`detect_on_path` 的目录序列变成 `search_dirs(PATH) =
   进程 PATH + FALLBACK_DIRS`；`FALLBACK_DIRS` 只有 macOS 非空
   （`/opt/homebrew/bin`、`/usr/local/bin`），且**只补在 PATH 后面**，顺序不变。
   `PATH` 环境变量缺失时不再整体返回 None（改为按空 PATH + 兜底继续找）。
4. `frontend/locales/{zh-CN,en-US}.json` + `settings.js`：新增
   `settings.runtime.{node,python}.{guideMac,placeholderMac}`，`paintRuntimeCopy` 在 macOS 上
   换 **`data-i18n*` 属性**（不是直接写文本），切语言后 `apply()` 仍按属性重画，
   不会退回 Windows 文案。

## 判据

- 为什么是"属性换 key"而不是在 JS 里写死文案：`i18n.apply` 唯一的重画依据是
  `data-i18n*` 属性，直接改 `textContent` 会在下次 `apply()` 被覆盖。
- 为什么兜底目录硬编码而不是 `which`：`which` 依赖 shell 与 PATH，GUI 进程里同样拿不到；
  固定两个目录的行为可预测、可单测。nvm / asdf 这类动态目录不在兜底范围，用户可在
  设置页手填。

## 验证

- `scripts/cargo.sh "test --workspace"`（macOS 真机）：core 255 / jsrt 48 / app 147 全通过。
  新增单测 `search_dirs_appends_platform_fallbacks_after_path`，并在
  `platform_info_is_consistent_with_this_platform` 加了 `file_assoc == cfg!(windows)` 断言。
- 兜底目录端到端（临时用例，跑完已删）：进程 `PATH` 收成 `/usr/bin:/bin` 后
  `detect_on_path(Node)` → `/opt/homebrew/bin/node`，`detect_on_path(Python)` → `/usr/bin/python3`。
- `node --check frontend/settings.js`、`node scripts/check-i18n.js`（477 keys）、
  `node scripts/check-frontend.js` 全通过。

## 没验证

- macOS GUI：设置 → 插件（Markdown 卡片无注册按钮）、设置 → 运行时（Homebrew 文案 +
  `/opt/homebrew/bin/...` 占位符）都还没在真机窗口里看过；本机没有浏览器自动化，
  前端只有静态检查。
- GUI 启动自动填入 → 清空保存 → 再启动重新填入的边界（本机 settings.json 已配置过）。
