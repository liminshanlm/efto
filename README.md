# efto

第一阶段：**Windows 应用快速启动** + **JS/TS 插件运行时**。
桌面栈是 Tauri 2，插件引擎是内嵌 V8（rusty_v8 152），TypeScript 由 SWC（经 deno_ast）在宿主内直接剥类型，不依赖 Node、npm 或 tsc。

```
Alt+Space 唤起 → 输入 → 回车启动应用 / 唤起插件
```

> **关于名字**：**efto** 取自 **Efficiency Tools**（效率工具），目标是把"找应用、开文件、跑小工具"这类高频操作压到一个输入框里。
>
> 产品身份统一用 **efto**——cargo 包名 `efto-core` / `efto-jsrt` / `efto-app`、产物 `efto.exe`、
> 窗口标题与托盘、bundle id `app.efto.launcher`、数据目录 `~/.efto`（Windows: `C:\Users\<name>\.efto`）、
> 环境变量 `EFTO_*`。
>
> 插件只认 efto 自己的契约：`efto-plugin.json` 清单、`efto.*` / `window.efto`、`efto://` 协议。
> 不提供任何第三方启动器的兼容层。

## 现在能做什么

| 能力 | 状态 | 说明 |
| --- | --- | --- |
| 应用快速启动 | ✅ 已验证 | 开始菜单 `.lnk`、注册表卸载项、`Program Files` 等目录裸 exe、Windows 自带工具白名单 |
| 中文/拼音搜索 | ✅ 已验证 | `wx` / `weixin` → 微信；`jsb` → 记事本；子序列、词首、紧凑匹配、使用频次加权 |
| 输入直连 | ✅ 已验证 | 输入 URL、`ms-settings:`、已存在的路径 → 直接打开；**输入算式 → 下拉框直接给结果，Enter 复制** |
| 时间 → 时间戳 | ✅ 已验证 | 输入 `2026-11-16 15:03:22` / `下午3点` / `明天 15:00` / `现在` → **秒级与毫秒级两行**，单击即复制。epoch 数值由 Node 的 `Date`（操作系统时区库）独立交叉校验 |
| 系统时间多格式 | ✅ 已验证 | 输入 `date` / `now` / `时间` / `现在` → **八种常用格式**（ISO 本地、Windows 斜杠、仅日期、仅时间、RFC3339、RFC1123/HTTP、中文、文件名安全），单击即复制那一种格式。星期/月份名由 Node 的 ICU 表独立复核 |
| JS 插件 | ✅ 已验证 | `efto-plugin.json` + `main.js`（**CommonJS 作用域**），运行时只注入 `efto.*` |
| TS 插件 | ✅ 已验证 | `main.ts` 宿主内 SWC 转译（类型剥离），支持相对路径 `require` 嵌套 `.ts` 模块 |
| 插件 UI | ⚠️ 代码就绪，未在真实 webview 里跑过 | `efto://` 协议托管插件目录，宿主注入 `window.efto`，UI ↔ main 双向消息 |
| 插件持久化 | ✅ 已验证 | 每插件一个 JSON 文件，10MB 配额；KV 与文档式接口（`efto.db.put/allDocs`）都可写 |
| Everything 文件检索 | ✅ CLI 端到端已验证 / ⚠️ 界面渲染未实测 | 搜索框直接输入（≥2 字符）→ **文件结果补在下拉框应用条目之后**（无需魔法前缀），单击选中、双击或 Enter 打开。查本机 Everything 索引，中文路径按 GBK 正确解码 |
| 发布产物 | ⚠️ 工作流就绪，首次 Actions 运行待验证 | GitHub Release 提供绿色版 zip（`efto.exe` + `plugins/`）、NSIS 安装包与单个插件包；未签名，开机自启与自动更新仍未做 |

**真实数据与验证过程见 `docs/verification.md`**（单元测试、204 个真机快捷方式解析度量、端到端 CLI 验证 + Node 独立时间探针，以及明确列出"没验证过什么"）。

未实现的能力会**明确报错**而不是静默成功：`node_modules` 依赖解析、桌面快捷方式、UWP/MS Store 应用、ESM 语法的入口（见下）。详见 `docs/roadmap.md`。

## 下载

发布产物挂在 [Releases](https://github.com/liminshanlm/efto/releases)：

| 产物 | 用途 |
| --- | --- |
| `efto-<ver>-windows-x64-portable.zip` | 绿色版：解压后 `efto.exe` 与 `plugins/` 同级，双击即用，不写注册表 |
| `efto_<ver>_x64-setup.exe` | NSIS 安装包：装到本机，插件随包内置 |
| `<plugin>-<version>.zip` | 单个插件：设置页「导入插件」，或解压到 `~/.efto/plugins/<name>/` |

要求 Windows 10/11 x64，且系统已装 WebView2 Runtime（Win11 自带，Win10 多已随 Edge 更新装好）。
产物没有代码签名，SmartScreen 会提示「未知发布者」，点「更多信息 → 仍要运行」。

发布由 `.github/workflows/release.yml` 在推 `v*` tag 时自动构建，维护者操作：

```powershell
# 先改 crates/app/tauri.conf.json 的 version，再打同名 tag
git tag v0.1.0
git push origin v0.1.0
```

## 目录结构

```
crates/
  core/     应用发现、.lnk 解析、索引、搜索打分、插件清单、存储  ← 无 UI / 无 JS 依赖，可单独测试
  jsrt/     rusty_v8 引擎、efto API 垫片、TS 转译、能力线程池
  app/      Tauri 2 主程序：托盘、热键、搜索窗口、efto:// 协议、Host 能力实现
frontend/   搜索框（纯静态 HTML/CSS/JS，无打包器）
plugins/    示例插件：hello-js（JS）、timestamp-ts（TS + require）
plugin-sdk/ efto.d.ts —— 给 TS 插件作者做类型提示
scripts/    vsenv / cargo / gen-icons / dev-run / bootstrap / package-plugins
vendor/     V8 预编译静态库落地点（bootstrap 下载，不入库，见下文）
```

## 构建

前置：Windows 10/11、WebView2 运行时（Win10 一般自带）、Visual Studio 的 **"使用 C++ 的桌面开发"** 工作负载、Rust stable。

```powershell
# 1) 首次准备工具链（rustup 走 rsproxy 镜像，VS 组件需 UAC）
powershell -ExecutionPolicy Bypass -File scripts/bootstrap.ps1

# 2) 编译 + 单元测试
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 -Cmd "test --workspace"

# 3) 跑起来（自动进入 MSVC 开发环境，并把仓库 plugins/ 注册为插件目录）
powershell -ExecutionPolicy Bypass -File scripts/dev-run.ps1
```

### 两个环境坑（已经踩过并写进脚本）

1. **Git Bash 的 `link.exe` 会抢走 MSVC 链接器**。`/usr/bin/link.exe` 是 coreutils 的硬链接工具，rustc 只按 PATH 找 `link.exe`，于是所有 build script 链接失败并报 `link: extra operand`。`scripts/vsenv.ps1` 会先导入 `VsDevCmd.bat` 环境，再把 `\usr\bin`、`\mingw64\bin` 从 PATH 里剔掉。**不要**在 Git Bash 里直接跑 cargo。
2. **rusty_v8 的预编译库默认从 GitHub Releases 下载**，部分网络环境直连会超时。`scripts/bootstrap.ps1` 会把它下到 `vendor/rusty_v8/`（38MB，已 gitignore，不入库），`.cargo/config.toml` 用 `RUSTY_V8_ARCHIVE` + `RUSTY_V8_ARCHIVE_SHA256` 指向该路径，之后构建可离线。直连不通时先改镜像再跑 bootstrap：
   ```powershell
   $env:RUSTY_V8_MIRROR = "https://gh-proxy.com/https://github.com/denoland/rusty_v8/releases/download/{tag}/{file}"
   ```

## 使用

- **Alt+Space** 呼出/隐藏搜索框（可在 `settings.json` 改 `hotkey`）
- 输入应用名、拼音或首字母 → `↑`/`↓` 选择、`Enter` 执行、`Esc` 逐级退出
- **鼠标**：普通应用**单击只选中、双击才执行**（防手滑开程序）；计算结果与时间戳**单击即选中并复制**
- `Alt+R` 重建索引；`Alt+P` 查看插件列表；托盘菜单同样的入口
- 输入 `https://…`、`ms-settings:`、或一个存在的文件夹路径 → 直接打开；
  `ms-settings:` 会列出常用设置页（如 `ms-settings:display`），继续输入可过滤
- 输入算式 → 第一行就是计算结果，`Enter` 复制到剪贴板。支持 `+ - * / % ^`、括号、
  一元负号、`pi/e/tau`、`sqrt log ln round floor ceil abs sin cos tan min max pow` 等函数、
  `kb/mb/gb/tb` 单位换算、千分位 `1,000+1`、全角 `１＋１`、结尾 `=`。
  判定很严：**不会劫持应用搜索**——`chrome`、`3.10 64-bit`、`1700000000`（纯数字）、
  `(5)`（纯括号包裹）、`1/0`（除零）都不出计算行。
- 输入插件关键字（如 `时间戳 1700000000`）→ 启动插件后端并唤起，剩余输入作为 payload 传给插件
- 搜索框直接输入（≥2 字符）→ **Everything 文件检索，结果补在下拉框应用条目之后**
  （没有魔法前缀；es/Everything 不可用时静默不出，可用性见设置页）。
  查询串原样透传 Everything 语法：`报告 ext:pdf`、`size>10mb`。每行显示
  文件名 + 所在目录 + 大小 + 修改时间，**单击只选中、双击（或 Enter）打开**，和其它可执行行一致。
  最多 5 行，分数压在应用条目之下，不会把应用挤走；默认滤掉 `$RECYCLE.BIN`、`node_modules`、
  `Temp`、`WinSxS`、`.git`（实测 `ext:exe` 原本第一屏全是回收站条目）。
- 输入时间 → **秒级、毫秒级两行时间戳**，单击或 `Enter` 复制纯数字（所见即所复制）。
  支持 `2026-11-16`、`2026/11/16 15:03:22`、`2026.11.16`、`20261116`、`2026-11-16T15:03:22Z`、
  `2026年11月16日15时03分22秒`、`下午3点`、`上午10点半`、`11月16日`（本年）、`15:03`（今天）、
  `今天/昨天/明天/后天`、`现在`/`now`（毫秒行是真子秒）、显式时区 `+08:00`/`UTC`、全角数字。
  判定同样严格：**不劫持应用搜索，也不与计算器抢食**——
  `chrome`、`2026`（光年）、`1700000000`（它本身就是时间戳）、`3.10 64-bit`、
  `11/12/2026`（分不清 11 月 12 还是 12 月 11，直接放弃）、`11-16`（留给减法）都不出时间行；
  反过来 `2026-11-16` 会让计算器闭嘴（它以前会顺手算出 1,999）。
- 输入 `date` / `now` / `time` / `today` / `时间` / `日期` / `现在` / `当前` / `今天` / `系统时间`
  → **八种常用格式的系统时间**，单击复制那一种格式：

  ```
  2026-11-16 18:59:55              ISO 本地
  2026/11/16 18:59                 Windows 斜杠
  2026-11-16                       仅日期
  18:59:55                         仅时间
  2026-11-16T18:59:55+08:00        RFC3339（带时区偏移）
  Mon, 16 Nov 2026 18:59:55 +0800  RFC1123 / HTTP Date 头
  2026年11月16日 18时59分55秒 周一    中文
  20261116_185955                  文件名安全（无分隔符）
  ```

  三条要点：**触发必须整串相等**（`update`、`dated`、`datetime`、`date 2026-11-16` 都不触发，
  它们是搜应用或算时间戳）；**一屏一个时刻**（八行与时间戳两行来自同一次取时，跨秒也不会
  出现两种时刻）；**不压掉应用**（本机真有个 `date.exe`，精确匹配 ≈1000 仍排在格式行 ≈700 之前）。

> 窗口/热键/托盘这一层需要在有前台焦点的本机人工确认一次：本项目的自动化验证走的是下面的 CLI 通道，
> 因为远程桌面会话会抢占焦点，无法可靠断言"窗口真的弹出来了"。

## 如何在本机验证 GUI

自动化能覆盖的都覆盖了（见 `docs/verification.md`），剩下这几步只能靠眼睛，2 分钟：

```powershell
# 1) 直接跑 release 产物（不需要开发环境）
$env:EFTO_PLUGIN_DIRS = "E:\workspace\efto\plugins"   # 想用仓库里的示例插件
& E:\workspace\efto\target\release\efto.exe

# 2) 按 Alt+Space —— 应该弹出居中搜索框
# 3) 输入 jsb 或 记事本 → Enter 启动；Esc 关闭；点别处窗口自动隐藏
# 4) 鼠标：单击应用行只高亮不启动，双击才启动；输入 6*7 后单击计算行 → 立即复制并收起
#    输入 2026-11-16 15:03:22 → 应出现秒级/毫秒级两行，单击任一行复制那串数字
#    输入 date 或 now → 应出现八种格式的系统时间，单击复制对应那一种格式
# 5) 输入 打招呼 你好 → 应弹系统通知并打开插件 UI 窗口
# 6) 托盘图标右键：显示搜索框 / 重建索引 / 打开插件目录 / 退出
```

逐项对照（哪一项不对，就是哪一个模块的问题）：

| 现象 | 说明 | 若不符合 |
| --- | --- | --- |
| 弹出无边框、居中、半透明的输入条 | `tauri.conf.json` 的 `app.windows` | 检查 `decorations/transparent/center`；改配置后要重新 `cargo build` |
| 输入有结果、`↑↓` 高亮跟随 | `search` 命令 + 前端渲染 | 开 `$env:RUST_LOG="debug"` 看 `search` 是否报错 |
| `Enter` 真的启动程序 | `invoke_hit` → `ShellExecute` | 看日志里有没有 `已启动 pid=` |
| 点窗口外自动隐藏 | `hide_on_blur` | 设置里关掉可复现对比 |
| 托盘菜单出现 | `tray.rs` | 图标在通知区域折叠里，先展开 |
| 插件 UI 页面显示内容 | `efto://` 协议 + 注入 SDK | 日志里找 `Asset ... not found` 或协议 403/404 |
| **没有黑色控制台窗口盖在桌面上** | `windows_subsystem` + `AttachConsole` | 这是本轮刚修的，若复现说明构建没生效 |

热键被占用时（微信/QQ/输入法也可能抢 Alt+Space）：改 `~/.efto/settings.json` 里的 `hotkey`，重启程序即可（运行期改需要重启，热键重绑只在启动与 `set_settings` 时执行）。

## 不启动 GUI 也能用：cli 通道

同一个二进制带一个 `cli` 子命令，用于脚本化验证、CI，以及插件作者测自己的 main 逻辑：

```powershell
$env:EFTO_PLUGIN_DIRS = "E:\workspace\efto\plugins"
.\target\debug\efto.exe cli index                      # 重建索引 + 统计（--json 出机器可读结果）
.\target\debug\efto.exe cli search chrome              # 加 --json 出结构化结果
.\target\debug\efto.exe cli search chrome --limit 5 --kind app --exact   # 限定条数/类型/完全匹配
.\target\debug\efto.exe cli launch 记事本 --dry-run     # 搜索命中，只报告不启动
.\target\debug\efto.exe cli launch --id "<search 结果里的 id>"           # 按 id 启动，适合 agent
.\target\debug\efto.exe cli apps --limit 20 --json     # 枚举索引内的应用
.\target\debug\efto.exe cli plugins                    # 已发现插件、入口、问题
.\target\debug\efto.exe cli plugin-run timestamp-ts ts 1700000000   # 真跑一次插件
.\target\debug\efto.exe cli files "D:\报告.docx" --reveal            # 在资源管理器里定位
.\target\debug\efto.exe cli action reindex             # 宿主动作（reindex/open_plugin_dir/open_data_dir/lock_screen）
.\target\debug\efto.exe cli ps --json                  # 可见顶层窗口（pid/hwnd/进程/标题）
.\target\debug\efto.exe cli focus --pid 1234           # 把窗口切到前台（也可给标题关键词）
.\target\debug\efto.exe cli capabilities --json        # 能力清单：子命令、flag、退出码、文件检索状态
pwsh scripts/verify.ps1                                # 上面的端到端组合
```

日志固定写 stderr，`--json` 的 stdout 只有 JSON；失败时输出
`{"ok":false,"code":"...","message":"..."}`。

退出码：`0` 成功、`1` 执行失败（如启动失败、GUI 专属动作）、`2` 用法错误、
`3` 查无结果——脚本据此区分"没搜到"和"出错了"。

`--dry-run`（`launch` / `files` / `action` / `focus`）只报告将要做什么，不产生副作用；
`--no-track` 让 `launch` 不写使用记录，适合自动化反复调用而不污染推荐排序。

`plugin-run` 会启动插件后端、投递一次 `onPluginEnter`、收集事件并打印能力调用回显
（`[notify]` / `[copy]` / `[ui-msg:*]`），有 `Error` 事件时非零退出——适合直接进 CI。

配置文件：`~/.efto/settings.json`（Windows: `C:\Users\<name>\.efto\settings.json`，首次启动自动生成）。索引缓存 `app-index.json`，使用记录 `usage.json`，插件数据 `plugin-data/<插件名>.json`。
日志走 `tracing`，`$env:RUST_LOG = "debug"` 后重跑可以看到插件线程、能力调用与协议请求的细节。

## 写一个插件

最小插件只要两个文件（`efto-plugin.json` + `main.js`），放进 `~/.efto/plugins/my-plugin/`
（或开发时用 `EFTO_PLUGIN_DIRS` 指向你的工作目录）：

```json
{ "name": "my-plugin", "version": "1.0.0", "main": "main.js",
  "cmds": [{ "type": "input", "label": "我的插件", "cmd": "my" }] }
```

```js
// main.js
efto.onPluginEnter(({ code, payload }) => {
  efto.showNotification(`收到 ${payload}`);
  efto.kv.set('last', payload);
});
```

完整 API 表、TS 写法、UI 页面通信方式与调试技巧见 **`docs/plugin-development.md`**；两个可运行示例在 `plugins/hello-js` 与 `plugins/timestamp-ts`。

插件可加 `"engines": { "efto": ">=0.1.0" }` 声明宿主版本要求；不满足时插件仍会列出（附原因），但不会进搜索、也不能被唤起。把插件目录打包成可分发的 zip：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-plugins.ps1            # 全部插件
powershell -ExecutionPolicy Bypass -File scripts/package-plugins.ps1 -Only markdown
```

产物在 `dist/plugins/<name>-<version>.zip`，zip 根即插件目录，可直接在 设置 → 插件 导入，或解压到 `~/.efto/plugins/`。

## 明确的边界

- 没有 Node.js 运行时：不能 `require('fs')`、不能装 npm 包。需要文件/系统的场景走 `efto.*` 能力（`shell*`、`kv` / `db`、`node` / `python`），需要大模型走设置里配置的 `efto.ai`。
- **模块格式是 CommonJS**：入口与被 `require` 的模块都在函数作用域里执行，
  所以写 `require('./x')` / `exports.f = …`，不写 `import`/`export`。检测到 ESM 会给出改成 CJS 的具体写法，
  而不是让 V8 抛一句看不懂的语法错误。真正的 ESM 模块支持在二阶段。
- TS 只做**类型剥离**：`enum`、带值的 `namespace`、装饰器、JSX 会被拒绝并提示"请预编译"，而不是产出一个语义不同的 JS。
- `efto.db` 是本地 JSON（KV + 文档接口），没有 CouchDB 风格的复制/冲突合并与 `_changes` 序列号语义。
- 图标目前是文字首字母头像，真正的 `.lnk`/exe 图标提取放在二阶段（需要 shell API）。
- 不索引 UWP/MS Store 应用与 `App Paths`；第一阶段只看开始菜单、注册表与安装目录。

## 许可

MIT，全文见 [LICENSE](LICENSE)。示例插件可自由参考与修改。

随二进制分发的第三方组件（V8、Tauri、deno_ast/SWC、MPL-2.0 与 Unicode 数据等）及其许可文本见
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)。

想参与开发见 [CONTRIBUTING.md](CONTRIBUTING.md)；安全问题与插件信任模型见 [SECURITY.md](SECURITY.md)。
