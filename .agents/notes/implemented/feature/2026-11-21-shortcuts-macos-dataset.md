# 快捷键指南 macOS 数据集与平台分流

- **Date**: 2026-11-21
- **Status**: done
- **相关**: `crates/core/src/shortcuts/{mod.rs,data/*.macos.json,data/macos.json}`、
  `crates/core/src/{config,search/service}.rs`

## 现象

快捷键指南只有 Windows 键位数据（`windows.json` 通用 + 11 份应用集），mac 上入口被
平台门收口；`by_id("windows")` 把通用回退写死。

## 做法

- **数据模型**：`ShortcutSet` 加 `os`（`win32`/`darwin`，与插件清单同词汇；缺省=全平台，
  用户导入数据不受影响）。同一 id 允许多平台各一份，`by_id`/`by_exe`/`apps()` 查询侧只
  放行当前平台的那条；`ids_are_unique` 改为"每平台内唯一"。
- **数据集**：现有 13 份打 `os: "win32"`；新增 `macos.json`（通用 48 条：系统/窗口/截图/
  编辑/文本导航/访达/调度中心）+ 11 份 `*.macos.json`（chrome/edge/firefox/vscode/idea/
  word/excel/powerpoint/photoshop/outlook/figma/trae，共 250 条）。键位写法直接用
  mac 记法（Cmd/Option/Control），不用装饰层。**只收高置信条目**——拿不准的宁可回退
  通用集，错键比没键更糟。
- **进程名匹配**：mac 用 CGWindow ownerName（本地化，如"访达"/"Finder"双收）；
  跨版本软件（Adobe Photoshop 20xx）把多个版本名都列进 exe。
- **开关**：`ShortcutGuide` 出平台收口（现在只剩 Screenshot），`KNOWN_TOOLS` mac 6 项；
  `generic_id()` 替换写死的 "windows" 回退。

## 判据

- 单测 142/241/48 全绿（shortcuts 11 个：数据完整性、os 过滤、大小写不敏感匹配、
  平台回退、用户合并）；verify.sh ALL CHECKS PASSED；GUI 冒烟 Ctrl+Alt+4 → shortcut_guide。
- 每份新数据过 `every_builtin_json_parses`（分组/条目/keys 不重复）与
  `exe_mapping_is_unique_across_apps`（exe 全局不撞名）。

## 没验证 / 留给后续

- 指南窗口的实际展示与"识别唤起前的应用"链路（需 GUI：唤起 → 打开某应用 →
  再唤起看是否命中该应用的键位表）。
- 键位正确性来自各产品公开文档/长期使用经验；个别条目（Office/PS 细节）欢迎
  用户通过 `~/.efto/shortcuts/<exe>.json` 覆盖修正（同名分组按 keys 合并、用户优先）。
