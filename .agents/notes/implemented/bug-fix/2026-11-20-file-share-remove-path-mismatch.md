# file-share：条目移除不生效（路径前缀不匹配）

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/app/src/share.rs`、`plugins/file-share/main.js`、
  `docs/plugin-development.md`（`efto.share` 行为约定）、`docs/verification.md`

## 现象

file-share 列表里点「移除」没有任何变化，重新打开条目还在；「清空」正常。

## 根因

宿主 `share.start` 返回的 `items[].path` 是 `Path::canonicalize()` 的结果，Windows 上是
`\\?\D:\...` 的 verbatim 形式；而插件 KV 里存的是用户原始路径 `D:\...`。

`remove` 用 UI 回传的（带前缀的）path 去匹配 `rawItems`，永远不相等 → `rawItems` 不变 →
`apply()` 重新推送同样的列表，看起来"点不动"。清空不看 path，所以是好的。

## 修法

- `share.rs::parse_items`：`ShareItem.path` 回传调用方传入的路径文本，`root` 保留
  canonicalize 结果继续用于前缀校验；字段注释同步修正。这样返回值也不再暴露 `\\?\`。
- `file-share/main.js`：新增 `normPath()`（剥 `\\?\` / `\\?\UNC\` 前缀 + 小写），
  删除匹配与新增去重统一用它，兼容升级前 `itemsMeta` 里已经落盘的带前缀路径。
- `share.rs` 新增回归测试 `echoes_input_path_instead_of_canonical_root`。

## 验证

- `cargo test -p efto-app share`：7 passed / 0 failed。
- `node --check plugins/file-share/main.js` 通过。
- 未验证：GUI 里真实点击「移除」的完整链路。
