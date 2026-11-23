# 插件独立 zip 打包脚本

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `scripts/package-plugins.ps1`、`docs/plugin-development.md`（打包与分发）、`README.md`

## 为什么

插件要与宿主解耦发布：宿主已支持从 `~/.efto/plugins`、`EFTO_PLUGIN_DIRS` 发现，设置页也能
导入 zip，但仓库里没有"把某个插件打成可分发包"的固定入口，每次都靠手工压缩，容易压出
`plugins/<name>/` 外层目录。

## 做法

- 新增 `scripts/package-plugins.ps1`（纯 ASCII）：遍历 `plugins/*/efto-plugin.json`，
  先复制到临时目录再 `Compress-Archive`，保证 **zip 根是插件目录**；输出
  `dist/plugins/<name>-<version>.zip`（`dist/` 已在 .gitignore）。`-Only a,b` 支持逗号分隔
  （`powershell -File` 会把参数当单个字符串）。
- 两个 PS 5.1 坑已处理：清单按 UTF-8 读取（默认 ANSI 会把中文描述读坏并让 JSON 解析失败）；
  `-Only` 需要再按逗号拆分。

## 验证

`powershell -ExecutionPolicy Bypass -File scripts/package-plugins.ps1 -Only markdown,hello-js`
实跑成功，`markdown-0.1.0.zip` 内顶层即 `efto-plugin.json` / `main.js` / `ui.js` 等，
无 `plugins/` 外层。未验证：在设置页真实导入该 zip（GUI 手测）。
