# 插件清单 `engines.efto` 宿主版本闸门

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `crates/core/src/plugins/manifest.rs`、`crates/core/src/plugins/registry.rs`、`crates/app/src/cli.rs`、`docs/plugin-development.md`、`docs/architecture.md`、`docs/macos-parity-todo.md`

## 为什么

主程序与插件准备分开演进（独立 zip 发布）。此前清单只有插件自己的 `version`，宿主 API 还在
加能力（最近才加 `fileTypes`、`efto.file.*`），跨版本安装时没有任何协商机制，旧宿主会静默加载
按新 API 写的插件并随后报奇怪的错。

## 契约

- 清单可写 `"engines": { "efto": ">=0.1.0" }`；只支持单条比较 `>=` `>` `<=` `<` `=`，
  省略操作符等价 `=`，`x.y` 缺省段按 0。`^` / `~` / `||` / 非数字**明确判为不兼容**并写进
  `problems`（不静默放行）。
- 不匹配 = 插件仍出现在 `cli plugins` / 设置页（附原因），但 `PluginDescriptor::usable()` 为
  false：`registry.get()` 与 `registry.search()` 都过滤掉，`cli plugin-run` 报
  `host_version_mismatch`。`HOST_VERSION` 取 `efto-core` 的 `CARGO_PKG_VERSION`。
- 内置 13 个插件清单统一加 `>=0.1.0`，作为写法示例与最低契约。

## 验证

- `cargo test -p efto-core`：266 通过 / 0 失败，新增 `host_requirement_comparisons`、
  `incompatible_host_marks_descriptor_unusable`、`host_mismatch_plugin_is_listed_but_not_usable`
  （最后一条是"搜索不该出结果"的反例）。
- 未验证：低版本宿主二进制的真实 `cli plugins` / `plugin-run` 输出（需改版本号重构建）；
  设置页停用原因未 GUI 手测。
