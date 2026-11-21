# 搜索框命令执行（`>` 前缀 + 可配置 Shell）

- **Date**: 2026-11-19
- **Status**: done
- **相关**: `docs/todo.md` 第 1 项、`docs/help.zh-CN.md` 第 13 节、`docs/verification.md`「2026-11-19」

## 事实

- 查询以配置前缀（默认 `>`）开头时，`SearchService::query_with_sources` 在**所有搜索源之前**
  短路，只返回一条执行提示行；前缀为空/关闭开关时不触发。
- 前缀校验：去空白后最多 4 个字符，且不能含字母、数字、空白——避免 `>1+1` 被计算器抢走，
  也避免把普通查询误当命令。
- 唯一执行入口是 `core/src/exec.rs`：`run_command`（内联、超时、`AtomicBool` 取消）、
  `run_command_terminal`（`CREATE_NEW_CONSOLE`）。GUI 与 CLI 共用，不在 `commands.rs` 另写一份。
- Shell 参数固定拼装、不过二次 shell 解释：PowerShell/pwsh 用
  `-NoLogo -NoProfile -NonInteractive -Command`，cmd 用 `/D /C`，Git Bash 用 `-lc`，
  WSL 用 `wsl.exe [-d distro] -e bash -lc`；PowerShell 先设 UTF-8 输出编码。
- 默认内联输出；`Esc` 调 `cancel_command` 终止子进程。输出行走 `ItemKind::Command` +
  `HitPayload::RunCommand`，stderr 单独成组标红，不进应用条目。

## 判据

- `command_enabled` 默认开启（用户确认），但结果行必须写明「将执行什么」。
- 管道/重定向允许（Shell 自身语义），风险提示放在设置页与帮助文档，不做语法拦截。
- `custom` Shell 必须显式填路径；自动发现只覆盖已知 Shell。

## 验证

- `crates/core` 单测覆盖前缀判定、参数拼装、超时/取消、截断。
- `scripts/verify.ps1` 跑 `run-command --dry-run`（只解析不执行）与真实 `echo` 往返。
- GUI 手测（各 Shell、中文输出、Esc 取消）未做，已写入 `docs/verification.md`。
