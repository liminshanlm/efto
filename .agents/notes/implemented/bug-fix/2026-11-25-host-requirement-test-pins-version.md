# `host_requirement_comparisons` 把宿主版本写死，跟着版本号一起红

- **Date**: 2026-11-25
- **Status**: done
- **相关**: `crates/core/src/plugins/manifest.rs`、`docs/verification.md`（v0.1.1 段）、
  `.agents/notes/implemented/process/2026-11-23-github-release-workflow.md`

## 为什么

发 v0.1.1 时把 `Cargo.toml` / `tauri.conf.json` 的版本从 0.1.0 改成 0.1.1，`release` 工作流
全绿并发布了 15 个资产，但同一个提交上的 `ci` 红了（run `45640253323`，`unit tests (windows)`）。

根因：`plugins::manifest::tests::host_requirement_comparisons` 用**字面量** `0.1.0` 当宿主版本
来断言 `engines.efto` 的比较语义。`HOST_VERSION` 是 `env!("CARGO_PKG_VERSION")`，版本一改，
`<=0.1.0`、`=0.1.0`、`>0.1.0`、`^0.1.0`、`>=0.1.0 || <0.3.0` 这几条期望必然翻转——测试测的
其实是"当时恰好是 0.1.0"，而不是比较规则。

## 做了什么

断言一律相对 `HOST_VERSION` 表达（`format!("<={host}")` 等），并把注释写在测试头上说明原因。
两段式写法（`>=0.1`，缺省段按 0）改成不会随 patch 号翻转的形式；`^` / `~` / 多条件 / `latest`
这类"不支持必须报错"的用例改成拼进 `host`，仍与版本无关。

`release` 只 build 不 test，所以产物本身没有受影响（只有测试断言）。按发布流程 note 里的
约定「发布后不再移动 tag」，`v0.1.1` 保持不动，修复随下一个提交进 main。

## 判据 / 结果

- 本地 `scripts/cargo.sh "test --workspace"`：efto-app 148 passed / 1 ignored、efto-core 255
  passed、efto-jsrt 48 passed，0 failed。
- `release` run `45640273364` 三个 job success，Release `efto v0.1.1` 15 个资产正常。
- 未验证：修复推上去后 `ci` 的 Windows / macOS 两个 gate 是否转绿（含 macos 的 `verify.sh`）；
  本机只能在 macOS 上跑同一套测试，Windows 侧差异仅靠 CI。
