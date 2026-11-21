# 内置 Trae 快捷键 + CLI 快捷键导入

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/core/src/shortcuts/{mod.rs,data/trae.json}`、`crates/app/src/cli.rs`、
  `docs/help.zh-CN.md` / `help.en-US.md`、`docs/architecture.md`、`docs/verification.md`

## 为什么

快捷键指南的内置数据里没有 Trae（用户主力 IDE），且补数据只能手工往
`~/.efto/shortcuts/<exe>.json` 写文件。后者对 agent 不友好：没有校验、没有路径约定、
写错就整份覆盖用户数据。

## 做了什么

1. 新增 `data/trae.json`，接入 `BUILTINS`。数据取 https://docs.trae.cn/ide_keyboard-shortcuts
   的 Windows 列，分「文本编辑 / 全局设置 / AI 对话」三组共 20 条。
2. `shortcuts::import_set(set, target_exe, dry_run)`：与用户覆盖文件同构的 JSON，
   校验后按同名分组 + `keys`（忽略大小写）合并，导入数据优先，`write_atomic` 落盘。
3. CLI `import-shortcuts [--file <路径>] [--exe <文件名>] [--json] [--dry-run]`：
   省略 `--file` 从 stdin 读；`capabilities --json` 内嵌 `shortcuts_import.format` 示例。

## 判据

- `app.exe` 用实测进程名 `trae cn.exe`，同时保留国际版 `trae.exe`；
  `exe_mapping_is_unique_across_apps` 保证不与既有应用抢。
- 复用用户覆盖文件的格式而不是新造一套：导入结果就是用户手改的同一种文件，
  出问题时用户可以直接编辑排查。
- 合并而非覆盖：重复导入幂等，agent 循环跑不会清掉手工数据；
  目标文件解析失败时拒绝覆盖（坏文件不能因为一次导入变成"被冲掉"）。
- `--exe` 只接受单个文件名（拒绝路径分隔符与控制字符），防止写到目录外。
- `--dry-run` 仍做完整校验与统计，只是不写盘——agent 可以先看将改什么。

## 验证

- `cargo test -p efto-core shortcuts -- --test-threads=1`：10/10，含导入幂等与
  "脏数据拒绝 + dry-run 不写盘"两个新测试（`tempfile` 隔离，不碰真实 `~/.efto`）。
- `cargo check -p efto-core -p efto-app`、`cargo build -p efto-app --bin efto` 通过。
- CLI：`import-shortcuts --file … --dry-run --json` → `added:2, updated:0, groups:1`，退出码 0；
  stdin 模式成功；缺文件 `io_error`（1），坏 JSON（2）；`capabilities --json` 含新命令与格式示例。
- 未验证：真实写盘导入 Trae 数据（避免污染用户目录）、GUI 里打开指南看到 Trae 卡片。
