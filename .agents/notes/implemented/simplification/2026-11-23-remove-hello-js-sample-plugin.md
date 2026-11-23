# 删掉 hello-js 示例插件

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `plugins/hello-js/`（已删）、`README.md` / `README.zh-CN.md`（内置插件表）、
  `AGENTS.md`（目录说明）、`docs/plugin-development.md`（跟随主题的一方插件清单）

## 为什么

`hello-js` 是仓库里唯一纯教学用的插件：演示 `onPluginEnter`、`db`、通知与 UI 页面往返。
但它的教学职责已经被两处覆盖——`README.md`「写一个插件」一节内联了完整的最简 JS 插件
（清单 + `main.js`），`plugins/timestamp-ts` 是可运行示例，且 `docs/plugin-development.md`
本身就有完整 API 表。留一份独立目录只是让 12 个功能插件里混进一个不提供功能的条目，
安装包里也多一个「打了招呼就没别的用」的插件。

## 做法

- `git rm -r plugins/hello-js`（含 `efto-plugin.json` / `main.js` / `index.html` / `logo.png`
  / `package.json`；它是仓库里唯一带 `package.json` 的插件，该文件无外部引用）。
- 同步仍描述「当前状态」的文档：两份 README 的插件表删掉该行并把计数 13 改成 12、
  「最简 JS 示例」的指代改成「上面那段内联代码」；`AGENTS.md` 目录说明去掉 `hello-js、`；
  `docs/plugin-development.md` 跟随主题的一方插件清单去掉它。

## 有意不改的地方

- `docs/verification.md` 里所有带日期的冒烟/重写记录：那是历史证据，删插件不使当年跑过的
  结果变假。
- `.agents/notes/**` 里提到 hello-js 的历史 note（主题跟随、mac 语言探测、smoke 夹具改造）。
- `scripts/verify.ps1` 第 40 行注释（过去式，解释为什么冒烟不用仓库插件而用自造夹具）与
  `crates/app/src/windows.rs` 里「消息排队」那段代码注释（记录该 bug 是靠 hello-js 发现的）。

## 验证

- 删除前 `rg --hidden 'hello-js'` 全仓核对引用面：无构建脚本、CI（`ci.yml` / `release.yml`）、
  单元测试或 `scripts/package-plugins.ps1` 依赖该目录；两侧 verify 的 plugin-run 冒烟用的是
  自造夹具 `verify-smoke`，不是仓库插件。
- 删除后 `ls plugins/` 为 12 个目录；两份 README 表格列数、代码围栏数与标题数比对仍一致。
- 未跑完整 `scripts/verify.sh` / `verify.ps1`（本次改动只删插件目录与文档，不触碰构建产物与
  代码路径）。
