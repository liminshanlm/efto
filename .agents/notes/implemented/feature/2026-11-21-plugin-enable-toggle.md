# 插件停用 / 启用开关

- **Date**: 2026-11-21
- **Status**: done
- **相关**: docs/plugin-development.md（停用 / 启用）、docs/architecture.md（插件启停）、docs/verification.md（2026-11-21 插件停用 / 启用）、crates/core/src/config.rs、crates/core/src/plugins/registry.rs、crates/app/src/{state,commands,cli,lib}.rs、frontend/{settings.js,settings.css,locales}

## 需求

设置页要能给每个插件单独停用 / 启用：停用后插件不再加载（搜不到、触发不了），
启用后重新加载。

## 方案

1. **持久化在设置里**：`Settings.disabled_plugins: Vec<String>`，存插件 id。
   不删插件目录、不改清单，重新启用不需要重新导入。
2. **注册表保留条目**：`PluginRegistry::set_disabled(&[...])` 只把命中项的
   `PluginDescriptor.enabled` 置 false，`plugins()` 仍列出全部（设置页要能显示"已停用"）。
   过滤点复用已有的 `enabled` 判断（`get` / `search` / `push_plugin` / 动态项），
   停用即从搜索、触发词、`efto://` 唤起里消失。
3. **只重建插件表**：新增 `AppData::reload_plugins()`，重新 discover + `set_disabled` +
   `load_dynamic_items`，并把"上次启用、这次不在启用集合里"的插件从
   `PluginRuntime` 里 `stop`（释放 isolate）。不动应用索引，启用后也不预启动，
   等用户下次唤起时按需加载。
4. **命令 `set_plugin_enabled(plugin, enabled)`**：只接受已安装的插件（避免手改
   settings.json 留下死条目），写设置并落盘 → 停用时先 `close_plugin_ui`（此时
   isolate 还活着，能收到 `ui_closed` 收尾）→ `reload_plugins()` → 返回最新的
   `PluginView` 列表。
5. **CLI 同源**：`cli.rs` 的 `discover_plugins` 也应用停用名单，
   `cli plugins --json` 输出 `enabled`，保证 `cli search` / `cli plugin-run`
   与 GUI 一致。
6. **前端**：每条插件右侧一个开关按钮（`data-plugin-toggle` / `data-next`），
   停用项加 `is-off` 压暗；badge 优先级改为"已停用 > 运行中"；成功 / 失败
   toast 文案中英同步。

## 判据与取舍

- 停用 ≠ 卸载：目录和清单保留，`app_plugin_dir` 里仍可见；只是运行时与搜索入口关闭。
- 字符串比较一律大小写不敏感（`eq_ignore_ascii_case`），与注册表里 id 去重的口径一致。
- `PluginDescriptor.enabled` 原本只由清单 `default_true` 决定、无人写过 false，
  `push_plugin` 里的 `if !d.enabled` 是死路径；本次把它变成真正的开关，不新增并列字段。

## 验证

- `plugins::registry::disabled_plugins_are_listed_but_not_searchable`：停用其一后
  列表长度不变、`get`/`search` 跳过、另一个不受影响，清空名单后恢复。
- `config::disabled_plugins_default_empty_and_roundtrip`：旧配置缺字段回落空表，
  显式列表能解析。
- `cargo build -p efto-app`、`node --check frontend/settings.js`、`check-i18n` 通过。

未验证：GUI 里点开关的实际观感；停用带 UI 插件时窗口关闭与 isolate 释放的先后在真机上
是否稳定（代码上 `close_plugin_ui` 先投递到主线程，`stop` 在命令线程同步执行）。
