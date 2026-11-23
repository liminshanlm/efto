# 设置页「关于」显示版本

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `frontend/settings.html`、`frontend/settings.js`、`crates/app/src/commands.rs`、`docs/help.zh-CN.md`、`docs/help.en-US.md`

## 事实

- 设置页左侧新增第 8 页「关于」，展示产品名与当前版本号。
- 版本来源是 `platform_info` 新增的 `version` 字段（`env!("CARGO_PKG_VERSION")`），
  复用设置页启动时已有的那次调用，不额外增加 IPC 往返与启动开销。
- 前端在 `applyPlatformCaps` 里把 `version` 写进 `#aboutVersion`，`platform_info`
  失败时保留占位 `—`，不阻塞设置页其它功能。
- 后续升级、版权等信息直接在这一页追加，不改导航结构。

## 验证

- `node scripts/check-i18n.js`：OK（456 keys，2 语言），中英文键一致。
- `cargo check -p efto-app`、`platform_info_is_consistent_with_this_platform` 通过。
