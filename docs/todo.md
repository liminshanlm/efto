# 待办清单（TODO）

- **Date**: 2026-11-19
- **状态**: 第 1、2、4、5、6 项已完成；第 3 项的 4 个 P1 能力与 3 个可闭环 P2 能力（#5/#7/#8）已完成（2026-11-20），其余 P2/P3 待排期
- **范围**: 用户新提出的功能需求与缺口分析。阶段性的路线规划仍在 [`roadmap.md`](roadmap.md)；本文件只记录"要做什么、做到什么程度算完成"。

优先级口径：

| 级别 | 含义 |
| --- | --- |
| P0 | 阻塞日常使用，优先做 |
| P1 | 高频、差异化明显，排进当前阶段 |
| P2 | 有价值但可延后，或需要先做技术验证 |
| P3 | 低优先级/仅记录 |

---

## 1. 搜索框命令执行（`>` 前缀 + 可配置 Shell）

- **状态**: 已完成（2026-11-19）
- **优先级**: P1
- **目标**: 搜索框输入 `>命令` 时，把 `命令` 交给本机命令行工具执行。Windows 默认 PowerShell；前缀符号与命令行工具都可在设置中配置（如 Git Bash、WSL、cmd、PowerShell 7）。

### 交互约定（草案）

1. 查询以配置的前缀（默认 `>`）开头时，**在一切搜索源之前短路**，不再走应用/计算器/时间戳/URL/路径判定，避免 `>1+1` 被计算器抢走。
2. 去掉前缀与左侧空白后作为命令正文；正文为空时不触发，只展示一条"输入要执行的命令"的提示行。
3. 结果行显示解析后的完整命令（`在 PowerShell 中执行: ipconfig`），单击只选中、`Enter` 执行，沿用现有"防手滑"规则。
4. 执行结果默认**内联展示**：stdout 按行成为可选行（`Enter` 复制该行），stderr 单独成组并标红；另提供"在终端中打开"模式，供交互式/长命令使用。
5. 执行中按 `Esc` 取消：需要保留子进程句柄并支持终止，而不是只关掉面板。

### 配置项（`~/.efto/settings.json`）

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `command_enabled` | `true`（待定，见开放问题） | 命令执行总开关 |
| `command_prefix` | `">"` | 触发前缀，保存时校验不与现有搜索源冲突 |
| `command_shell` | `"powershell"` | `powershell` / `pwsh` / `cmd` / `git-bash` / `wsl` / `custom` |
| `command_shell_path` | 空 | 自定义或非标准安装路径；留空时按 PATH + 常见安装位置发现 |
| `command_wsl_distro` | 空 | WSL 发行版；留空用默认发行版 |
| `command_timeout_secs` | `10` | 内联执行超时，上限 60s |
| `command_run_mode` | `"inline"` | `inline`（内联输出）/ `terminal`（开终端窗口） |

设置页新增"命令"分组：前缀输入框（实时提示是否冲突）、Shell 下拉、路径与"检测"按钮（复用 Node/Python 解释器的探测方式）、超时、运行模式。

### 实现要点

- **唯一执行入口**：按 `AGENTS.md` 硬性约定，在 `crates/core/src/exec.rs` 增加 `run_shell`，GUI 与 CLI 共用；不允许在 `commands.rs` 另写一份进程启动逻辑。
- Shell 参数固定拼装，不经过二次 shell 解释：
  - PowerShell / pwsh：`-NoLogo -NoProfile -NonInteractive -Command <cmd>`
  - cmd：`/D /C <cmd>`
  - Git Bash：`<bash.exe> -lc <cmd>`
  - WSL：`wsl.exe [-d <distro>] -e bash -lc <cmd>`
  - custom：路径 + 用户给定的参数模板（`{cmd}` 占位），**放到最后实现**
- 输出解码沿用 `exec::decode_os_bytes`（Windows 控制台代码页可能是 GBK）；PowerShell 侧显式把输出编码设为 UTF-8，避免中文乱码。
- 超时、stdout/stderr 截断复用现有 `spawn_and_collect` 的上限；长命令引导用户切到终端模式。
- 结果渲染复用现有下拉结构；若现有 `HitPayload` 不够表达"命令输出行"，再新增 `ItemKind::Command` / 输出行类型，不要污染应用条目。
- CLI 增补脚本化入口（供 `verify.ps1` 与 agent 使用），例如 `efto.exe cli run-command "git status" --json`，支持 `--dry-run` 与 `--timeout`。
- 安全边界：命令只由用户主动输入触发，不暴露给插件；日志不记录命令正文中的敏感参数；`custom` 模式必须要求路径存在且可执行。

### 涉及文件

- `crates/core/src/config.rs`（设置字段）
- `crates/core/src/exec.rs`（`run_shell`）
- `crates/core/src/search/mod.rs` / `service.rs`（前缀短路与结果模型）
- `crates/app/src/commands.rs`、`state.rs`、`cli.rs`
- `frontend/app.js`（前缀提示、输出渲染、Esc 取消）
- `frontend/settings.html`、`frontend/settings.js`、`frontend/settings.css`

### 验收标准

- 单元测试：前缀解析（正常/空命令/关闭开关/与 URL、路径、计算器、时间戳不冲突）、各 shell 的参数拼装、超时与截断。
- CLI 端到端：`run-command` 返回正确的退出码与 stdout；`--dry-run` 不执行；超时能被终止。
- GUI 手测：`>ipconfig`、`>git status`（Git Bash）、`>ls`（WSL）、`>Get-Date`（PowerShell）各跑通；中文输出不乱码；`Esc` 能取消正在执行的命令。

### 开放问题（实现前确认）

1. 命令执行默认开启还是默认关闭？（建议默认开启，但结果行必须明确显示"将执行什么"）
2. 内联输出与终端窗口，哪个作为默认？交互式命令如何自动切换？
3. `custom` Shell 是否允许任意可执行文件，还是只允许从已检测到的 Shell 中选择？
4. 前缀允许改成哪些字符？与现有搜索源冲突时是拒绝保存还是自动回退？
5. 是否允许管道/重定向（`|`、`>`、`&&`）？这是 Shell 自身的语义，建议允许并在帮助文档中提示风险。

---

## 2. 托盘"帮助"菜单 + Markdown 帮助文档

- **状态**: 已完成（2026-11-19）
- **优先级**: P1
- **目标**: 系统托盘菜单新增"帮助文档"，点击打开一份 Markdown 用户手册；手册覆盖 efto 当前所有可操作、可配置的功能点。

### 交互约定（草案）

1. 托盘菜单顺序：显示搜索框 / 设置… / **帮助文档** / 分隔线 / 退出 efto。
2. 点击"帮助文档"打开独立帮助窗口（label `help`，约 900×700），再次点击时聚焦已有窗口，不重复创建。
3. 帮助窗口内渲染 Markdown（标题、列表、表格、代码块、链接），样式与设置窗口一致；断网可用。
4. 帮助内容唯一事实源为仓库 `docs/help.md`，构建时 `include_str!` 嵌入二进制，便携版 exe 不依赖外部文件。
5. 渲染使用随仓库 vendor 的轻量 Markdown 渲染器（不引入 npm/打包器）；若坚持零依赖，也可在构建期预渲染为 HTML，但源文件仍保持 Markdown。

### 帮助文档必须覆盖的章节

1. 快速开始：`Alt+Space` 唤起、输入、`Enter`/`Esc`、鼠标单击/双击规则。
2. 应用搜索与启动：拼音/首字母/子序列匹配、索引来源、重建索引、自定义索引目录、忽略规则。
3. 直接打开：URL、`ms-settings:`、已存在的文件/文件夹路径、`~` 展开。
4. 内置搜索源与严格判定：计算器、时间戳、八种系统时间格式、Everything 文件检索、Chrome/Edge 收藏夹；各自"不触发"的边界。
5. Shift 预览：文本、图片、视频、网页、PDF、本地 HTML、文件属性面板；打开/关闭按钮与焦点规则。
6. 常用工具：截图、剪贴板历史、锁屏、自定义应用/插件、`Alt+1..9`、面板内方向键操作。
7. 插件：安装/导入、触发词与别名、插件 UI、动态搜索项、能力 API 概览、TS 限制。
8. 设置项逐条说明：常规（热键、结果数、失焦隐藏、置顶、工具、索引、收藏夹、Everything）、插件、运行时、模型。
9. 托盘菜单：显示/设置/帮助/退出。
10. CLI 通道：`index`、`search`、`launch`、`plugins`、`plugin-run`、`files`、`action`、`ps`、`focus`、`capabilities`，退出码与 `--json`。
11. 数据与配置：`~/.efto` 下的 `settings.json`、索引缓存、使用记录、插件数据、`EFTO_PLUGIN_DIRS`、`RUST_LOG`。
12. 已知限制与故障排查：无 Node 运行时、CommonJS、TS 仅类型剥离、Everything 依赖、热键冲突、UWP 未索引。
13. 命令执行（功能 1 完成后补充）：前缀、Shell 配置、内联/终端模式、安全提示。

### 实现要点

- `crates/app/src/tray.rs`：新增 `tray.help` 菜单项与事件分支；同步更新该文件底部的菜单 id 单元测试（当前断言 3 个 id）。
- `crates/app/src/windows.rs`：新增 `show_help`，复用 `show_settings` 的窗口创建方式；`tauri.conf.json` 新增 `help` 窗口并设置 `create: false`（避免状态未就绪）。
- 新增 `frontend/help.html`、`frontend/help.js`、`frontend/help.css`；vendor 一份带许可证头的 Markdown 渲染器。
- 新增 `docs/help.md`；维护约定：新增/修改功能时同步更新该文件，README 继续面向开发者，帮助文档面向用户，二者不互相复制。
- 前端静态检查接入 `scripts/check-frontend.js`（若现有脚本覆盖新文件）。

### 验收标准

- 托盘出现"帮助文档"，点击打开窗口并正确渲染标题/表格/代码块/链接；重复点击不产生第二个窗口。
- 断开网络后仍可查看完整帮助。
- 对照检查表逐条核对：设置页每个可配置项、托盘每个菜单、搜索框每种输入形态，在帮助文档中都有说明。
- 帮助窗口关闭后不影响搜索框热键与焦点行为。

### 开放问题（实现前确认）

1. 是否允许用户用 `~/.efto/help.md` 覆盖内置帮助？（建议先不做，保持单一事实源）
2. 帮助窗口是否需要目录/搜索/打印？（建议先只做渲染与目录锚点）
3. 帮助文档的语言：先中文，是否同步英文版？

---

## 3. Alfred 对标：efto 缺失的高人气功能

- **状态**: P1 与 P2 可闭环三项（#5/#7/#8）已完成（2026-11-20）；其余 P2/P3 待排期
- **优先级**: P2（其中"片段"与"通用动作"已提前到 P1 完成）

### 已对齐或部分对齐的能力

| Alfred 能力 | efto 现状 |
| --- | --- |
| 应用启动、模糊匹配、使用频次排序 | 已具备，另有中文拼音/首字母匹配 |
| 计算器 | 已具备，且严格避免劫持应用搜索 |
| 剪贴板历史 | 已具备文本/图片/文件列表记录、置顶与粘贴到前台窗口 |
| 文件搜索 | 已具备，但依赖 Everything + `es.exe` |
| 收藏夹搜索 | 已具备（Chrome/Edge） |
| Quick Look 式预览 | 已具备 Shift 预览（文本/图片/视频/网页/PDF/属性） |
| 工作流（Workflows） | 以 JS/TS 插件实现，具备脚本能力，但没有可视化编排 |
| 自定义网页搜索 | `quickweb` 插件覆盖常见站点与搜索跳转；无统一配置与无结果兜底 |
| 系统命令 | 仅锁屏；睡眠/重启/清空回收站等缺失 |

### 缺失功能清单（按"受欢迎程度 × 与 efto 的适配度"排序）

| # | Alfred 能力 | 为什么受欢迎 | efto 现状与缺口 | 建议优先级 |
| --- | --- | --- | --- | --- |
| 1 | **Snippets 片段与文本扩展** | 输入关键词自动展开常用文本/模板，是 Alfred Powerpack 的高频卖点 | 已实现：搜索框精确关键词触发 + `{date}`/`{time}`/`{datetime}` 展开 + 粘贴回前台；不做系统级逐字钩子 | P1（已完成） |
| 2 | **Universal Actions 通用动作** | 在任意应用里选中文本/文件，按热键弹出动作菜单（翻译、搜索、打开、复制路径等） | 已实现：第二全局热键捕获选中文本，动作菜单提供复制/粘贴回前台/网页搜索/打开链接或路径 | P1（已完成） |
| 3 | **File Actions + File Buffer** | 对文件做打开方式、移动/复制、删除、批量选择，是文件工作流核心 | 已实现：文件行 `Tab`/`⋯` 动作菜单 + `Ctrl+Enter` 加入缓冲 + 独立缓冲窗口批量复制/移动/删除 | P1（已完成） |
| 4 | **剪贴板增强** | 图片/文件列表、置顶、纯文本粘贴、粘贴到前台窗口，比纯文本历史实用得多 | 已实现：文本/图片/文件列表记录、置顶、`Ctrl+Enter` 粘贴到前台；纯文本粘贴按文本条目原样写出 | P1（已完成） |
| 5 | **自定义 Web Search + Fallback** | `g 关键词`、`wiki 关键词`；本地无结果时自动兜底到搜索引擎 | 已实现：`<关键词> 正文` 前缀短路、可增删搜索引擎、无结果时按开关兜底 | P2（已完成） |
| 6 | **主题与外观自定义** | 启动器每天高频使用，主题/透明度/字体是强个性化需求 | 固定样式；无主题切换与自定义 CSS | P2 |
| 7 | **单项/工作流级热键** | 给常用动作绑定独立热键，减少"唤起→输入→回车"链路 | 已实现：7 个内置工具可各自绑定全局热键；与唤起/通用动作热键冲突时丢弃 | P2（已完成） |
| 8 | **系统命令集** | 睡眠、重启、注销、清空回收站、 eject、屏保，属于"用完即走"的刚需 | 已实现：睡眠/重启/注销/清空回收站/屏保，破坏性动作先弹原生确认 | P2（已完成） |
| 9 | **内建文件索引** | 不依赖第三方软件，开箱即用，索引速度快 | 依赖 Everything + `es.exe`；`roadmap.md` 已规划 USN Journal 直读（P2） | P2 |
| 10 | **文件动作的"打开方式"与路径操作** | "用 VS Code 打开"、"复制完整路径"是开发者高频动作 | 仅系统默认程序打开；无"打开方式"菜单与复制路径动作 | P2 |
| 11 | **Workflow 可视化编辑器** | 不写代码也能搭自动化，降低插件门槛 | JS/TS 插件是代码式，能力足够但门槛高；无可视编排、变量、调试器 | P3 |
| 12 | **设置同步** | 多机同步热键/主题/片段/工作流 | 配置只在本机 `~/.efto`；无云同步/导出导入 | P3 |
| 13 | **Large Type 大字显示** | 把手机号、验证码、地址放大显示，便于抄写 | 无；预览窗口不承担该用途 | P3 |
| 14 | **词典/拼写** | `define`/`spell` 快速查词 | 无；可用插件实现，但无内置入口 | P3 |
| 15 | **通讯录搜索** | 输入姓名直接拨号/发邮件 | 无；依赖系统通讯录，Windows 场景弱 | P3 |
| 16 | **音乐迷你播放器** | 不离开键盘控制播放 | 无；Windows 媒体控制可做，但收益有限 | P3 |
| 17 | **1Password 集成** | 快速填充密码 | 无；依赖闭源集成，且涉及安全边界 | P3 |
| 18 | **Alfred Remote（iOS 遥控）** | 手机远程触发 Mac 上的动作 | 无；跨端成本高，与 Windows 桌面定位不符 | P3 |

### P1 实现结果（2026-11-20）

1. **"片段与动作"包（已完成）**
   - `core/src/snippets.rs` + `~/.efto/snippets.json`；搜索只在关键词整串相等时出结果，`Enter` 复制展开并粘贴回前台。
   - 第二全局热键（默认 `Ctrl+Alt+Space`，可在设置开关/修改）捕获选中文本，`actions` 窗口执行复制/粘贴/网页搜索/打开。
   - 边界：不做系统级逐字键盘钩子；抓不到选中文本时显示空态提示，动作执行明确报错。
2. **"文件工作流"包（已完成）**
   - 文件行 `Tab` / `⋯` 动作菜单：打开、打开方式、定位、复制路径、复制文件、加入缓冲、复制到…、移动到…、删除到回收站。
   - `Ctrl+Enter` 加入文件缓冲（去重、上限 100），`files` 窗口批量处理；删除统一走回收站。
3. **"剪贴板 2.0"（已完成）**
   - 500ms 轮询，优先级 文件列表 > 图片 > 文本；图片落盘 `~/.efto/clipboard-images/`，缩略图按需生成。
   - 置顶条目不参与 200 条上限与 30 天清理；`Ctrl+Enter` 复制并粘贴回前台窗口。
   - 边界：目标窗口以更高权限运行时会明确报错，不假装粘贴成功。

验证：`cargo test --workspace`、`node scripts/check-i18n.js`、`node scripts/check-frontend.js` 全部通过。

### P2 实现结果（2026-11-20）

1. **自定义 Web Search + 无结果兜底（#5，已完成）**
   - `core/src/websearch.rs`：`WebSearch { keyword, name, url }`，URL 必须含 `{q}`；默认内置百度 `b`、Bing `g`、GitHub `gh`、维基百科 `wiki`。
   - 输入形如 `g rust 生命周期`：第一段与关键词大小写不敏感相等且后面有正文才触发，命中后**短路**所有本地搜索源，直接给一条 780 分的结果。
   - 本地完全无结果且 `web_search_fallback` 开启时，追加一条 300 分兜底结果；`web_search_engine` 指定兜底引擎，找不到则退回列表第一项。
2. **系统命令集（#8，已完成）**
   - `core/src/system.rs` 只接受整串精确关键词：`sleep`/`睡眠`、`restart`/`重启`、`logout`/`logoff`/`注销`、`emptytrash`/`清空回收站`、`screensaver`/`屏保`；分数 690，低于应用条目。
   - `app/src/system.rs` 通过 `SetSuspendState`、`ExitWindowsEx`、`SHEmptyRecycleBinW`、`WM_SYSCOMMAND + SC_SCREENSAVE` 执行；重启前启用 `SeShutdownPrivilege`。
   - 重启/注销/清空回收站先弹原生确认框，取消即返回 `已取消`；非 Windows 明确报错。
3. **单项热键（#7，已完成）**
   - 新配置 `tool_hotkeys: { <tool_id>: "<accelerator>" }`，只允许 7 个内置工具；留空表示不绑定。
   - `normalize_tool_hotkeys` 丢弃空键、未知工具，以及与唤起热键/通用动作热键重复的组合；注册失败的组合只写日志，不影响其它热键。
   - 热键回调在独立线程执行 `run_action`，不阻塞主线程；设置页新增“单项热键”页。

验证：`cargo test --workspace`（app 100 / core 213 / jsrt 48）、`scripts/verify.ps1` 全部通过。

### 建议的后续功能包

1. **"片段与动作"包（P1，已完成）**：全局选中内容获取 → Universal Actions 动作菜单 → Snippets 文本扩展。三者共享"全局热键 + 前台应用交互"底座，应一起设计。
2. **"文件工作流"包（P1，已完成）**：File Actions 菜单 + 多选缓冲 + 复制路径/打开方式/移动复制删除。
3. **"剪贴板 2.0"（P1，已完成）**：图片与文件条目、置顶、纯文本粘贴、粘贴到前台窗口。
4. **"搜索兜底与外观"（P2）**：自定义 Web Search 与单项热键已完成；主题系统待排期。
5. **"系统与索引"（P2）**：系统命令集已完成；USN Journal 内建文件索引待排期（与 `roadmap.md` 的 P2 合并推进）。

### 不建议照搬

- **Alfred Remote**：跨端投入大，efto 的定位是 Windows 桌面启动器。
- **1Password 集成**：依赖闭源 SDK 与授权，且会扩大安全攻击面。
- **Powerpack 式功能分层**：efto 目前是 MIT 开源，不引入付费墙；能力默认可用，靠设置开关控制风险。

---

## 4. 多语言支持（i18n）

- **状态**: 已完成（2026-11-19）
- **优先级**: P2
- **目标**: 搜索框、设置页、托盘菜单、内置结果文案与帮助文档支持切换语言；默认跟随系统，首批提供简体中文与英文。插件自身的名称/描述由插件负责，宿主只提供语言环境。

### 现状

- 前端 `frontend/*.html`、`*.js` 与 Rust 侧 `tray.rs`、`windows.rs`、`core` 的内置动作/搜索结果文案均为硬编码中文。
- 帮助文档计划为单一 `docs/help.md`（见第 2 项），目前没有译文机制。
- 插件清单 `efto-plugin.json` 的 `label` / `description` 是插件作者写死的单语言文本。

### 设计（草案）

1. 设置新增 `language`：`"auto" | "zh-CN" | "en-US"`，默认 `auto`；`auto` 读取 Windows 用户界面语言（`GetUserDefaultUILanguage`），非中文回退英文。
2. 文案词典放 `locales/zh-CN.json`、`locales/en-US.json`，key 用 `区域.组件.动作` 命名（如 `settings.general.hotkey`），缺 key 时回退 `zh-CN` 并在日志里报出 key，不在界面上显示 key。
3. 前端用 `data-i18n` 属性 + 轻量 `t(key)` 替换，不引入 npm/i18n 框架；Rust 侧新增小型 `i18n` 模块，词典 `include_str!` 嵌入二进制。
4. 托盘菜单文本无法原地修改，语言变更后重建托盘菜单；搜索框与设置窗口重新加载即可生效。
5. 帮助文档拆为 `docs/help.zh-CN.md` / `docs/help.en-US.md`，按当前语言加载（第 2 项落地时即按此结构创建，不再产生单一的 `docs/help.md`）。
6. 分阶段迁移：先覆盖用户高频可见的前端与托盘/设置文案；`core` 内部错误文案在接口稳定后再统一迁移，避免一次性改动过大。

### 涉及文件

- `crates/core/src/config.rs`（`language` 字段）
- 新增 `crates/core/src/i18n.rs` 或 `crates/app/src/i18n.rs`
- `crates/app/src/tray.rs`、`windows.rs`
- `frontend/*.html`、`frontend/*.js`
- `docs/help.zh-CN.md`、`docs/help.en-US.md`
- `plugin-sdk/efto.d.ts`（新增 `efto.locale`）

### 验收标准

- 切换 `zh-CN` / `en-US` 后，搜索框占位与提示、设置页全部文案、托盘菜单、帮助文档同步切换。
- 缺失 key 回退中文且记录日志；英文界面在 520px 宽搜索框内不溢出、不截断关键操作。
- 插件仍能正常显示自己的单语言名称；宿主暴露 `efto.locale`，插件可据此自行切换文案。
- 静态检查能发现新增的、未走词典的用户可见中文（允许白名单）。

### 默认方案（先按此实现，除非评审推翻）

| # | 待确认问题 | 默认方案 | 理由 |
| --- | --- | --- | --- |
| 1 | 首批语言范围 | 只做 `zh-CN` / `en-US`；词典按 `locales/<tag>.json` 独立存放，后续加语言只加文件；不做第三方语言包安装入口 | 结构上保留扩展性，但不为未验证的需求增加安装/校验机制 |
| 2 | 插件多语言 | 清单**不增加** `label_i18n` / `description_i18n`；宿主暴露 `efto.locale`，插件在 `main.js` 里自行决定显示名与文案 | 清单字段会扩散到索引与搜索；插件运行时本来就能拿到语言，改造成本更低 |
| 3 | `auto` 是否实时跟随系统 | 启动时读取一次；设置页切换语言立即生效（重建托盘、重载窗口）；系统语言变化后需重启 efto | 监听系统语言变更收益低，且需要额外的 Win32 消息处理 |
| 4 | 帮助文档译文来源 | 仓库维护 `docs/help.zh-CN.md` / `docs/help.en-US.md` 并嵌入二进制；**不支持**用户覆盖 | 保持单一事实源，避免译文与版本漂移 |

---

## 5. 快捷键指南（应用感知）

- **状态**: 已完成（2026-11-19）
- **优先级**: P1
- **目标**: 新增一个内置常用工具：唤起搜索框后执行「快捷键指南」，自动识别搜索框下方/唤起前的应用，列出该应用可用的快捷键。定位类似 PowerToys Shortcut Guide，但它是"应用感知版"——PowerToys 只展示 Windows 系统快捷键，本功能要按当前应用切换数据。

### 首批覆盖范围

| 分组 | 应用 | 识别依据（exe） |
| --- | --- | --- |
| 系统 | Windows 桌面/资源管理器/通用 | `explorer.exe`，以及任何应用都适用的系统快捷键 |
| Office | Word / Excel / PowerPoint / Outlook | `WINWORD.EXE` / `EXCEL.EXE` / `POWERPNT.EXE` / `OUTLOOK.EXE` |
| 浏览器 | Chrome / Edge / Firefox | `chrome.exe` / `msedge.exe` / `firefox.exe` |
| 编辑器 | VS Code | `Code.exe`（含 Insiders） |
| 设计 | Photoshop / Figma | `Photoshop.exe` / `Figma.exe`（Figma 网页版回退到浏览器快捷键并提示） |
| IDE | IntelliJ IDEA | `idea64.exe` / `idea.exe` |

未收录的应用不报错：展示 Windows 通用快捷键 + "未收录 <应用名>，可添加自定义数据"的提示。

### 应用识别（关键设计）

1. 热键唤起 efto 前记录系统前台窗口的 `pid` / exe / 标题；搜索框显示后它就成了"被覆盖的应用"。
2. 执行工具时重新校验该窗口仍存在；已关闭则用 `WindowFromPoint` 命中搜索框中心下方窗口，并沿 z-order 跳过 efto 自身进程。
3. 沿 `GetAncestor(GA_ROOT)` 取顶层窗口，再用 `GetWindowThreadProcessId` 得到 exe 名；只读取进程名，**不监听键盘、不注入其他进程**。
4. 识别失败（桌面、权限不足、UWP 宿主）时回退到 Windows 通用快捷键。

### 数据与维护

- 每个应用一份 JSON，放在 `crates/core/src/shortcuts/data/`，构建时 `include_str!` 嵌入二进制，离线可用。
- 数据结构：

```json
{
  "app": { "exe": ["winword.exe"], "name": "Microsoft Word" },
  "groups": [
    { "name": "文件", "items": [{ "keys": "Ctrl+N", "action": "新建文档" }] }
  ]
}
```

- 首版以官方文档为准尽量全量；版本差异（Office 365 / 2021、Photoshop 版本）不拆多份，用条目上的 `note` 标注适用版本。
- 允许用户用 `~/.efto/shortcuts/<exe>.json` 覆盖或补充；同名条目以用户数据优先，加载失败只影响该文件并打日志。
- 单元测试遍历所有内置数据：JSON 可解析、exe 映射不重复、组内 `keys` 不重复、`action` 非空；数据错误在编译期测试暴露，而不是运行时静默。

### 交互（草案）

1. 设置 → 常用工具新增「快捷键指南」，默认加入面板；沿用 `Alt+1..9` 编号。
2. 执行后打开独立窗口（label `shortcuts`，约 760×620），窗口顶部显示识别到的应用名与 exe；未识别时显示"未识别应用"。
3. 内容按分类分组（文件/编辑/视图/导航/调试等），快捷键用键帽样式展示；窗口内提供搜索框过滤，`Esc` 关闭。
4. 数据量大时默认折叠非高频分组；记住上次展开状态（可选）。
5. 识别到的应用与搜索框实际覆盖的应用不一致时，允许手动切换应用数据源（下拉选择）。

### 涉及文件

- 新增 `crates/core/src/shortcuts/mod.rs`（数据模型、exe 查找、内置数据加载）
- 新增 `crates/core/src/shortcuts/data/*.json`
- `crates/core/src/config.rs`（`KNOWN_TOOLS` 增加 `shortcut_guide`）
- `crates/app/src/winproc.rs` / `windows.rs`（前台窗口与 z-order 识别）
- `crates/app/src/commands.rs`、`state.rs`、`lib.rs`
- `crates/app/tauri.conf.json`、新增 `frontend/shortcuts.html` / `shortcuts.js` / `shortcuts.css`
- `docs/help.zh-CN.md`（工具说明）

### 验收标准

- 在记事本、Chrome、VS Code、Word、Excel、PowerPoint、Photoshop、Figma、IDEA 上分别唤起并执行工具，识别到的应用名与快捷键集正确；切换应用后结果跟着变。
- 未收录应用、桌面、权限不足场景回退到 Windows 通用快捷键，不出现空白或报错。
- 数据文件错误（缺字段、重复键）会被测试发现；用户自定义 JSON 损坏时只影响该应用。
- 窗口内搜索、分组折叠、`Esc` 关闭正常；不影响搜索框热键与其他常用工具。
- 不新增搜索源、不改变现有搜索打分与触发判定。

### 默认方案（先按此实现，除非评审推翻）

| # | 待确认问题 | 默认方案 | 理由 |
| --- | --- | --- | --- |
| 1 | 数据来源 | 首版用人工整理的官方快捷键 JSON；不做 UI Automation/菜单资源动态解析 | 动态解析跨应用不稳定、权限与兼容成本高，收益低于静态数据 |
| 2 | 应用识别时机 | 记录唤起前的前台窗口，执行时复核；失效再用 `WindowFromPoint` 兜底 | 直接对应"搜索框覆盖在哪个应用上方"，且实现简单 |
| 3 | 展示形态 | 独立窗口 + 窗口内搜索，不做下拉面板 | 单个应用的快捷键数量远超 520px 下拉框容量 |
| 4 | 首批应用范围 | 严格按上面的表格；Figma 网页版不单独建数据 | 控制数据维护成本，先覆盖高频桌面应用 |
| 5 | 用户扩展 | 支持 `~/.efto/shortcuts/<exe>.json` 覆盖/补充 | 避免每次新增应用都要发版 |
| 6 | 语言 | 首版动作名中文，随 i18n（第 4 项）补英文 | 与多语言条目共用同一套词典机制 |

### 后续可选项（不纳入首版）

- 通过插件 API `efto.shortcuts.register` 让插件贡献快捷键数据。
- 长按 `Win` 显示系统快捷键的 PowerToys 式浮层。
- 从应用菜单/UI Automation 动态提取快捷键并合并进静态数据。

---

## 6. 一键平铺窗口

- **状态**: 已完成（2026-11-19）；2026-11-20 扩展为窗口操作二级菜单（最小化其他/全部、左右半屏、撤销平铺）
- **优先级**: P1
- **目标**: 常用工具新增「平铺窗口」：执行后把当前显示器上所有可见顶层窗口按网格铺满工作区，一次到位，不再手动拖拽。

### 行为约定（草案）

1. 只处理**搜索框所在显示器**，其他显示器上的窗口保持不动；平铺范围是该显示器的工作区（扣除任务栏）。
2. 候选窗口：`EnumWindows` 枚举后依次过滤——可见、有标题、非最小化、非 `WS_EX_TOOLWINDOW`、非子窗口（无 owner）、非 cloaked、非 efto 自身进程、非桌面/任务栏（`Progman` / `WorkerW` / `Shell_TrayWnd`）。
3. 网格：`n` 个窗口取 `cols = ceil(sqrt(n))`、`rows = ceil(n / cols)`；窗口间距 8px，工作区四周留 8px 边距；最后一行不足时均分剩余宽度，避免右侧留空。
4. 最大化窗口先 `ShowWindow(SW_RESTORE)` 再定位；最小化窗口默认跳过（见默认方案）。
5. 用 `BeginDeferWindowPos` / `DeferWindowPos` / `EndDeferWindowPos` 批量移动，`SWP_NOZORDER | SWP_NOACTIVATE`：不改变层级、不抢焦点，减少闪烁。
6. 完成后 toast 报告"已平铺 N 个窗口（跳过 M 个）"；一个候选都没有时明确提示"当前显示器没有可平铺的窗口"。
7. 重复执行结果稳定；系统窗口、UWP 等拒绝调整尺寸的情况计入"失败"而不是静默忽略。

### 设置与入口

- `KNOWN_TOOLS` 增加 `tile_windows`，设置页"常用工具"提供开关，默认加入面板，沿用 `Alt+1..9` 编号。
- 首版只通过常用工具卡片/快捷键触发，不新增独立全局热键（见后续可选项）。

### 实现要点

- 新增 `crates/app/src/tile.rs`：布局计算写成纯函数（输入工作区、窗口数、间距，输出每个矩形），Win32 枚举/移动单独一层，便于单测。
- 复用 `winproc.rs` 的 `EnumWindows`、`GetWindowThreadProcessId`、`IsIconic`；补充 `GetWindowLongPtrW`（`WS_EX_TOOLWINDOW`）、`DwmGetWindowAttribute(DWMWA_CLOAKED)`、`GetWindow(GW_OWNER)`。
- 显示器与 DPI：用 `MonitorFromWindow` + `GetMonitorInfoW` 取工作区，按物理像素计算；efto 进程自身需 Per-Monitor DPI 感知（现有 Tauri 配置已满足则直接复用）。
- 虚拟桌面：首版用 cloaked 过滤掉其他桌面的窗口，不引入 `IVirtualDesktopManager` COM 依赖。
- 失败处理：对每个窗口单独记录 `SetWindowPos` 结果；UWP/受限窗口失败不中断整批。
- CLI：`efto.exe cli tile-windows --dry-run --json` 输出候选窗口与计算出的目标矩形，便于脚本验证与排障。

### 涉及文件

- 新增 `crates/app/src/tile.rs`
- `crates/app/src/winproc.rs`（窗口过滤与 Win32 封装）
- `crates/core/src/config.rs`（`KNOWN_TOOLS` 增加 `tile_windows`）
- `crates/app/src/commands.rs`（内置工具分发）、`crates/app/src/cli.rs`
- `frontend/settings.html` / `settings.js`（常用工具开关）、`frontend/app.js`（工具卡片图标/文案）
- `docs/help.zh-CN.md`（工具说明）

### 验收标准

- 2 / 3 / 5 / 7 个窗口平铺后互不重叠、间距一致、完整覆盖工作区；最后一行宽度均分。
- 最大化窗口能先还原再平铺；最小化窗口不被唤起；efto 自身窗口、桌面、任务栏不参与。
- 多显示器场景只影响搜索框所在屏幕，另一屏窗口位置不变。
- 平铺不改变窗口 z-order、不抢焦点；重复执行结果稳定。
- `cli tile-windows --dry-run --json` 能列出候选与目标矩形；单测覆盖网格计算边界（n=1、最后一行不足、窄工作区）。

### 默认方案（先按此实现，除非评审推翻）

| # | 待确认问题 | 默认方案 | 理由 |
| --- | --- | --- | --- |
| 1 | 平铺哪块屏幕 | 搜索框所在显示器，其他屏不动 | 与"搜索框覆盖在哪"一致，避免误动副屏 |
| 2 | 最小化窗口 | 跳过，不平铺也不还原 | 避免把后台/最小化的应用全部弹出来 |
| 3 | 置顶窗口（`WS_EX_TOPMOST`） | 参与平铺（efto 自身除外） | 用户要求"所有打开的窗口"；若反馈扰民再加排除开关 |
| 4 | 间距与边距 | 固定 8px，首版不提供配置 | 先保证一致观感，配置项等有真实需求再加 |
| 5 | 撤销平铺 | 首版不做；2026-11-20 已补上进程内撤销 | 记录/还原矩形会引入额外状态，先验证主流程 |
| 6 | 触发方式 | 常用工具卡片 + `Alt+1..9`，不新增全局热键 | 与截图/剪贴板/锁屏的入口保持一致 |
| 7 | 虚拟桌面 | 过滤 cloaked 窗口，不做完整 COM 识别 | 覆盖大多数场景，避免引入 COM 依赖 |

### 2026-11-20 扩展：窗口操作菜单

- `tile.rs` 新增 `Op`（grid / left / right / min-others / min-all / undo），`run_op` 统一分发；
  `half_layout` 与 `grid_layout` 一样是纯函数。
- 前端选中「平铺窗口」后不再直接执行，而是在结果列表展开二级菜单；`Esc` 退回默认面板，
  输入任意字符即恢复普通搜索。单项热键仍直接网格平铺。
- 最小化其他窗口保留「搜索框下方的窗口」（前台是 efto 时回落到 `guide::last`），
  最小化只作用于所有显示器的可见窗口，跳过已最小化、工具窗口、桌面/任务栏与 efto 自身。
- 撤销平铺：进程内 `Mutex<Vec<SavedWindow>>` 记录最近一次平铺的 hwnd + 原矩形（含最大化状态），
  还原后清空；重启进程即失效，不落盘。
- CLI：`tile-windows --mode grid|left|right|min-others|min-all|undo --dry-run`。

### 后续可选项（不纳入首版）

- 按应用分组平铺、指定窗口排除列表。
- FancyZones 式预设布局与跨显示器平铺。
- 跨进程持久化的撤销记录（当前只在本次运行内有效）。
- 独立全局热键与"仅平铺当前应用窗口"。

---

## 更新记录

| 日期 | 变更 |
| --- | --- |
| 2026-11-19 | 新建：命令执行、托盘帮助、Alfred 缺口清单 |
| 2026-11-19 | 新增：多语言支持 |
| 2026-11-19 | 多语言：补充 4 个待确认问题的默认方案 |
| 2026-11-19 | 新增：应用感知的快捷键指南 |
| 2026-11-19 | 新增：一键平铺窗口 |
| 2026-11-19 | 完成：命令执行、托盘帮助、多语言（词典放 `frontend/locales/`，宿主 `i18n` 模块共用同一份） |
| 2026-11-19 | 完成：应用感知快捷键指南、一键平铺窗口 |
| 2026-11-20 | 完成：Alfred 对标的 4 个 P1 能力（片段、通用动作、文件工作流、剪贴板 2.0） |
| 2026-11-20 | 完成：Alfred 对标的 3 个 P2 能力（自定义 Web Search + 兜底、系统命令集、单项热键） |
| 2026-11-20 | 扩展：窗口操作二级菜单（最小化其他/全部、左右半屏、撤销上次平铺） |
