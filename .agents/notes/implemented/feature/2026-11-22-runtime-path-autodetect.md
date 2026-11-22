# 运行时未配置时自动从 PATH 探测

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `docs/plugin-development.md`（`efto.node` / `efto.python`）、`docs/architecture.md`（插件脚本运行时）、`docs/verification.md`（2026-11-22 运行时自动探测）、`docs/macos-parity-todo.md`

## 需求

设置 → 运行时留空时，用户还得自己去 `where node` 抄路径。改为：未配置就自动从系统 PATH
探测 node / python，写进 settings.json，界面上仍可手动改。

## 判据与实现

- **只在空/空白时探测**：`state::autofill_runtimes` 用 `trim().is_empty()` 判断；已配置的路径
  永不覆盖（用户改成自定义解释器后不会被启动流程改回去）。
- **探测在 core**：`exec::detect_on_path(rt)` 按 `PATH` 顺序拼固定文件名
  （Windows：`node.exe` / `python.exe` / `python3.exe`；其它平台：`node` / `python3` / `python`），
  逐个走已有的 `probe_interpreter`（`--version` 退出码）判定。只看文件名不够——Windows
  商店的 `python.exe` 占位会在这一步被跳过。
- **落盘两处**：
  1. `lib.rs` 的 `setup` 里补一次并 `Settings::save`，随后 `AppData::new` 读到的就是补好的值；
  2. `commands::runtime_status` 再补一次（用户把路径清空后重新打开运行时页）。
  **不放 `AppData::new`**：单测会构造 AppData，不能让 `cargo test` 写用户的 settings.json。
- **UI**：`settings.js` 的 `loadRuntime` 在输入框为空时用返回的 `path` 回填；提示文案与
  `help.zh-CN.md` / `help.en-US.md` 同步说明"留空会自动探测"。

## 已知边界

- 清空路径 = 视为"未配置"，下次启动会被自动填回；目前没有"显式禁用运行时"的开关。
  若以后要支持，需要区分 `null`（从未配置）与显式关闭标志，而不是复用空串。
- 探测是同步的，最坏受 `probe_interpreter` 的 5s 超时约束（PATH 里有不可达网络路径时）。
  正常情况下每个解释器几十毫秒。

## 验证

- `cargo test -p efto-core`：254 通过，含新增 `detect_skips_dirs_without_a_working_interpreter`
  （同名占位文件必须被判据跳过）。
- `cargo test -p efto-app --lib`：109 通过，含 `autofill_runtimes_only_fills_blank_fields`
  （空白才填、已有值不覆盖、第二次无事可做）与 `..._keeps_fields_empty_when_nothing_detected`。
- 本机 PATH 实测：`where node` → `C:\Program Files\nodejs\node.exe`；
  `where python` → `Python312\python.exe`（另有 WindowsApps 占位）。

未验证：GUI 启动时的真实自动填写与设置页回填（单实例机制下未重启现有 GUI）、macOS 上
Finder 启动的 app 拿到的 PATH（不会继承 shell 的 nvm/Homebrew 路径）。

## 追加：未安装时的安装引导（同日）

自动探测解决的是"装了但没配"，没装的人仍然不知道去哪装。运行时页两张卡片各加一行引导：

- 结构：`settings.html` 的 `.import-row` 之后插 `#nodeGuide` / `#pythonGuide`，一行文案 +
  「前往下载」按钮（`data-url` 指向 `nodejs.org/en/download` / `python.org/downloads`）。
  注意 Node 官网的 `/download/` 是裸目录索引，必须用 `/en/download`，实测点开是目录列表。
- 显隐：`paintRuntime` 统一控制，`guide.hidden = !!(st.configured && st.ok)`；未配置、
  检测失败都保持可见，检测通过即收起。CSS 必须补 `.runtime-guide[hidden] { display:none }`
  （页面没有全局 `[hidden]` 规则，`.runtime-guide` 是 flex，不补会一直显示）。
- 打开方式：点击走宿主命令 `open_url`（内部 `launch::shell_open`），失败走 toast
  `settings.runtime.openFailed`；插件/前端不自己拉起进程。
- i18n：`settings.runtime.node.guide` / `python.guide` / `download` / `openFailed` 中英各四条。

验证：`node --check frontend/settings.js`、`node scripts/check-frontend.js` 通过
（守卫认到 2 个 `.hidden` 控制元素）。Windows GUI 实跑：两解释器可用时引导不在 UI 树里；
改错路径检测后引导与按钮出现；Node / Python 下载分别打开 `nodejs.org/en/download` 与
`python.org/downloads`；改回后引导重新隐藏，未保存时 settings.json 无变化。macOS 待办：
下载文案与链接需按平台调整。
