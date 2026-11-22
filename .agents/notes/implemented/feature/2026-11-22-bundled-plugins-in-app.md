# 安装包内置插件（换机器开箱可用）

- **Date**: 2026-11-22
- **Status**: done
- **相关**: crates/app/tauri.conf.json、crates/core/src/config.rs、crates/core/src/plugins/registry.rs、docs/architecture.md、docs/plugin-development.md、docs/verification.md

## 需求

用户原话：

> 需要，帮我重新打包，把 markdown 插件一起打进去
> 要，把插件内嵌进 .app，换机器也能直接用

在此之前插件只从目录发现（`EFTO_PLUGIN_DIRS` → `settings.plugin_dirs` → `~/.efto/plugins`），
`.app` 里不带任何插件 ⇒ 换一台机器装上，搜索框里一个插件都没有，必须手工拷 `plugins/`。

## 实现

1. **打包**：`crates/app/tauri.conf.json` 的 `bundle.resources` 加 `"../../plugins": "plugins"`。
   tauri 会按目录映射（实测子目录完整保留），mac 落在
   `efto.app/Contents/Resources/plugins/<id>/`。
2. **解析**：`Settings::bundled_plugin_dir()` 按可执行文件位置探两个候选——`Contents/Resources/plugins`
   （mac .app）与 `exe 同级/plugins`（Windows / Linux / 绿色版）。两个都探而不是按平台写死：
   开发态 `cargo run` 与将来的绿色版布局落在"同级"这条上。另支持 `EFTO_BUNDLED_PLUGINS`
   显式指定（测试用）。
3. **优先级**：`effective_plugin_dirs()` 里内置目录排在**最后**（env → settings → `~/.efto/plugins` → 内置）。
4. **id 级去重**：`PluginRegistry::discover` 的 `push_plugin` 里，同 id 只保留**先发现**的那份。
   没有这一步，用户目录和内置目录里的同名插件会同时出现在结果里（同一插件显示两遍、
   `get(id)` 抢到哪份不确定）。

## 判据

- **目录顺序即优先级，不靠目录名或时间戳猜**：用户改过/装过的插件放在更靠前的目录，
  自然胜出。测试 `duplicate_id_keeps_the_first_directory` 双向验证（用户在前保留用户版、
  内置在前保留内置版）。
- **"换机器可用"必须用空数据目录实测**，不能只看 `.app` 里有文件：临时 `HOME`
  + `EFTO_PLUGIN_DIRS` 清空后跑 `cli plugins`，13 个插件全部从 `.app` 内置目录加载。

## 验证

- 空 HOME（模拟新机器）：`发现 13 个插件`，全部来自 `Contents/Resources/plugins`。
- 用户目录优先：`~/.efto/plugins` 与内置目录同 id 时，`cli plugins` 只显示一份且取自用户目录。
- `cargo test --workspace` 443 passed；`check-i18n` 451 keys / `check-frontend` / `scripts/verify.sh` 全过。
- 产物：`.app` 54.41 → 68 MiB（多出 13 个插件），已装到 `/Applications/efto.app`，
  `codesign --verify --deep --strict` OK、`allow-jit` 在。

## 代价（明确的）

- 安装包变大（+13.6 MiB），且内置副本是**只读**的：用户要改插件必须放到更靠前的目录
  （`~/.efto/plugins`），直接改 `.app` 里的那份会在下次安装时被覆盖。
