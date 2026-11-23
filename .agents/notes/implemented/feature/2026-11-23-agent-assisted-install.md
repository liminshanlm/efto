# 设置页「让 Agent 帮我安装」（Everything / Node.js / Python）

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `docs/architecture.md`（内联文件检索、插件脚本运行时）、`docs/verification.md`、`docs/macos-parity-todo.md`

## 为什么

Everything / es.exe 与 Node/Python 都属于"没有它功能就不可用，但宿主不该替用户静默装环境"的依赖。
原有入口只有自动下载 / 手动下载 / 官网链接；用户还得自己找命令、校验版本、把文件放到正确位置。

## 做法

不新增安装执行路径，只生成一段**提示词**复制到剪贴板，由用户粘贴到能执行本机命令的 Agent 对话里完成安装：

- Everything：按 `everything_status` 的 `state` / `everything_installed` 生成步骤。固定官方
  `https://www.voidtools.com/es.zip`、锁定的 sha256 `224550eddd6f09fbae8ffe9d5e1d4bab07563a96c895fe1fc43fe60df48c0d12`、
  目标 `%USERPROFILE%\.efto\tools\es\es.exe`；`mismatch` 时要求先删除旧文件。
- 运行时：Windows 用 `winget`（`OpenJS.NodeJS.LTS` / `Python.Python.3.13`），macOS 用 Homebrew，
  并要求 Agent 回报解释器绝对路径，用户回设置页「检测」。
- 语言跟随界面（`i18n.lang()`），中英两套提示词。
- 新增 Tauri 命令 `copy_to_clipboard(text)`，内部复用 `crate::host::copy_text`（arboard），
  不引入前端剪贴板权限或新依赖。

## 边界

- 按钮只在未就绪时出现：Everything 用 `state === 'ready'` 隐藏；运行时沿用引导的
  `configured && ok` 显隐。
- 宿主仍然不代装、不改 PATH：安装动作发生在用户确认的 Agent 会话里；Agent 装完后仍回到
  `everything_status` / `runtime_check` 的同一套校验。
- `runtime-guide` 的既有「前往下载」绑定收窄为 `.link-btn[data-url]`，避免把新按钮误当下载链接。

## 验证

- `node --check frontend/settings.js`
- `node scripts/check-i18n.js`（473 keys，双语一致）
- `node scripts/check-frontend.js`
- `scripts/cargo.ps1 -Cmd "check -p efto-app"`（仅既有 4 条告警）

未验证：GUI 实跑复制与提示词内容；macOS 未构建未实测（已登记 parity）。
