# 插件导入：一个包支持多个插件

- **Date**: 2026-11-20
- **Status**: done
- **相关**: docs/plugin-development.md（打包与分发）、docs/verification.md（2026-11-20 插件包多插件导入）、crates/core/src/plugins/import.rs、crates/app/src/commands.rs、frontend/settings.js

## 需求

用户手上的 `plugins.zip` 与 `plugins1.zip` 各装 10 个插件，之前导入时报
"找到多个插件，请直接选择其中一个目录"。

## 两种包布局（都要认）

- `plugins1.zip`：插件目录直接在包根 —— `ai-chat/efto-plugin.json`
- `plugins.zip`：外面多套一层容器目录 —— `plugins/ai-chat/efto-plugin.json`

## 方案

1. `find_manifest_root`（只认一个）换成 `find_manifest_roots`：从包根向下深度优先找
   `efto-plugin.json`，**某个目录本身是插件就不再往下找**（插件自己的 `assets/` 里
   就算有清单也不算独立插件）；扫描上限 3 层，够覆盖"容器目录 + 插件目录"。
2. `import_path` 返回 `ImportBatch { imported, failed }`，逐个插件走原来的单插件流程
   （校验清单 → 复制 → 写 `.efto-import.json`）。
3. **部分失败不整体失败**：重名/清单坏只记进 `failed`，其它插件照常导入；
   全部失败才返回 Err（保持原来"重名导入报错"的语义）。
4. 命令 `import_plugin` 返回 `ImportBatch`；设置页提示改为"已导入 N 个插件：a、b"，
   并追加"；未导入：名字（原因）"。新增文案 `settings.plugins.importedMany` /
   `settings.plugins.importSkipped`（中英文同步）。

## 验证

- 单测 8/8：新增包根多插件、容器目录多插件、部分失败保留其它插件三个用例。
- 真实包：临时集成测试分别导入 `plugins.zip` / `plugins1.zip`，均 10/10 成功、
  `failed` 为空（验证后删除该测试）。
- 未验证：GUI 里多插件与部分失败两种提示的观感。
