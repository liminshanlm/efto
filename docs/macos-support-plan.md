# macOS 支持分阶段计划

- **Date**: 2026-11-21
- **Status**: Phase 0-4 全部落地（2026-11-21，含截图；见 `docs/verification.md` mac 段与
  `.agents/notes/implemented/feature/` 七条 note）；人手验证清单待走
- **相关**: `docs/architecture.md`、`docs/roadmap.md`、`docs/verification.md`、`docs/plugin-development.md`

本文件是 macOS 移植的唯一计划来源。实现过程中的判据、踩坑与实测记录分别进
`docs/verification.md`（验证过什么）与 `.agents/notes/`（为什么这么做）。

## 0. v1 边界：目标与功能取舍

**v1 = mac 上「唤起 → 搜索 → 启动应用 / 跑插件」完整可用，CLI 与 GUI 双通道。**
凡是需要 Accessibility / Screen Recording 授权、或整块重写 Win32 语义的系统集成，一律推迟。

### 0.1 保留（v1 必交付）

| 能力 | mac 实现要点 |
| --- | --- |
| 全局唤起 | 默认 `Alt+Space`（mac 即 Option+Space）；Cmd+Space 被 Spotlight 占用，不作默认 |
| 应用搜索与启动 | `/Applications`、`~/Applications`、`/System/Applications` 的 `.app` + extra_dirs / custom 目录 |
| 内联搜索源 | 计算器、时间戳/系统时间、输入直连（路径/URL）、命令源（`>` 前缀） |
| 插件运行时 | `jsrt` 全量：V8、TS 剥离、能力调用、插件 UI 窗口、`efto://` 协议 |
| 设置页 / 托盘 / 单实例 / 帮助 | Tauri 原生能力 |
| 剪贴板历史 | arboard 文本 + 图片（已有跨平台实现）；文件列表不支持 |
| 片段 snippets | 纯文本复制，无平台依赖 |
| 锁屏 lock_screen | `pmset displaysleepnow`（或 CGSession -suspend），成本极低 |
| CLI | `index` / `search` / `launch` / `apps` / `plugins` / `plugin-run` / `run-command` / `action`（仅 lock_screen） |

### 0.2 推迟（v1.5 · 系统集成补齐，见 Phase 3）

| 能力 | Windows 现状 | mac 替代 |
| --- | --- | --- |
| 文件检索源 | `es.exe`（Everything） | `mdfind`（Spotlight CLI），分数压应用下 |
| 系统设置搜索 | `ms-settings:` | `x-apple.systempreferences:` |
| 系统动作全家 | Win32 Power/Shutdown | AppleScript / pmset（睡眠/重启/注销/清空回收站/屏保） |
| 文件列表剪贴板 / 粘贴前台 | CF_HDROP / SendInput | NSPasteboard / CGEvent |
| hosts 编辑 + hosts 插件 | UAC 提权 | `osascript … with administrator privileges` |
| `cli ps` / `focus` | Win32 窗口枚举 | CGWindowList + AX API |
| 书签 | Chrome/Edge Windows 路径 | mac 浏览器配置路径 |
| 快捷键指南 | `shortcuts/data/windows.json` | 需要一份 macOS 键位数据集 |

### 0.3 砍掉（v1 编译排除，调用明确报错）

| 能力 | 原因 |
| --- | --- |
| 窗口平铺 / 撤销平铺（`tile.rs`） | Win32 枚举/MoveWindow 整块语义；AX API 成本高且要 Accessibility |
| 通用动作（`universal.rs`） | 模拟 Cmd+C + 抓选区，要 Accessibility |
| 截图 / 长截图 / 标注（`screenshot.rs`） | ScreenCaptureKit + Screen Recording 授权 + 标注层交互 |
| 文件缓冲 file_buffer 的「粘贴到前台」 | 依赖 SendInput 等价物（CGEvent，Accessibility） |

按硬性约定 3：上述能力在 mac 上调用一律返回明确错误（如 `当前平台不支持：窗口平铺`），
设置页不展示入口；历史配置里残留的失效工具 id 由 `normalize_tools` 的既有语义丢弃。

### 0.4 `KNOWN_TOOLS` 平台差异

`config.rs::KNOWN_TOOLS` 拆成按 cfg 返回的 `known_tools()`：

- mac v1 = `clipboard` / `lock_screen` / `snippets`（3 个）
- Windows 维持现有 7 个不变

设置页工具选择、`normalize_tools` 校验、单项热键默认值（`default_tool_hotkeys`）统一从
平台列表出发，不各写一份。

## 1. cfg 怎么切

原则：**公共 API 面不动，平台差异收在模块内部**；沿用仓库已有的 `mod imp` 风格
（`app/winclip.rs`、`app/system.rs` 已如此）。不新建 `crates/platform`，不做 trait 注入——
静态 `cfg` 分发够用，在出现第三个平台之前 trait 层是纯开销（见 AGENTS.md「不引入超出
任务需要的抽象」）。依赖方向不变：`app → {jsrt, core}`。

### 1.1 三种切法（按差异大小选）

**(a) 整块不同 → 模块级 cfg**。发现源这种 Windows 与 mac 没有公共实现的：

```rust
// core/src/apps/mod.rs
#[cfg(windows)]
pub mod lnk;         // MS-SHLLINK 解析，mac 不编译
#[cfg(windows)]
pub mod startmenu;
#[cfg(windows)]
pub mod registry;
#[cfg(target_os = "macos")]
pub mod appbundle;   // .app / Info.plist 扫描
```

`indexer.rs` 汇总处同样 cfg 分支拼装来源；去重、缓存、打分等公共逻辑不动。

**(b) 同职责不同实现 → `mod imp` / 函数内分支**（exec、system、icons、launch、scanner）：

```rust
// app/src/icons.rs
#[cfg(windows)]
mod imp { pub fn file_bytes(path: &Path) -> Result<Vec<u8>, String> { /* SHGetFileInfoW */ } }
#[cfg(target_os = "macos")]
mod imp { pub fn file_bytes(path: &Path) -> Result<Vec<u8>, String> { /* NSWorkspace */ } }
```

公共签名、缓存键、data URL 编码留在外层，平台函数只负责「取到像素/字节」。

**(c) 配置与模型 → 平台变体 + 加载后归一**：

- `CommandShell` 增加 `Zsh` / `Bash` / `Fish`；`impl Default` 按 cfg 分支
  （Windows 仍 `Powershell`，mac 为 `Zsh`）。
- `Settings` 加载后跑 `normalize_for_platform()`：本平台不适用的 shell
  （mac 上的 `powershell` / `pwsh` / `cmd` / `git-bash` / `wsl`）回落平台默认，其余字段原样保留。
  禁止拿着错的 shell 静默执行。
- `AppSource` 加 `AppBundle`（mac 发现渠道）；`TargetKind` 加 `AppBundle`
  （`.app` 交给 `open -a`，区别于裸可执行文件）。两者都是 `Serialize`，新变体在旧缓存里
  不会出现，无需迁移；`ScanReport` 新字段一律 `#[serde(default)]`（`desktop` 字段先例）。
  新变体不改结构，`CACHE_VERSION` 不动。
- `EFTO_PLUGIN_DIRS` 分隔符已按平台分支，不动。

**(d) 编译期不对称的模块**：`tile` / `screenshot` / `universal` / `guide` / `winproc` /
`winclip` / `win_settings` / `everything` 在 mac 上 `#[cfg(windows)]` 排除；它们在
`commands.rs` / `cli.rs` / `service.rs` 的调用点留平台桩——不注册入口或返回明确错误，
不允许静默成功。

### 1.2 模块切法速查表

实施口径（与初稿的偏差）：**编译全开，接线按平台**——发现源模块不做模块级 `cfg`
（`IgnoreMatcher` 在 startmenu.rs 里、`.lnk` 解析的 fixture 测试跨平台跑有价值），
只在 `indexer` 汇总与搜索接线处按平台取舍；`win_settings` / `everything` 是例外
（Windows 专有数据表 + 触发语义，模块级 `#[cfg(windows)]`）。

| 模块 | 切法 | mac v1 |
| --- | --- | --- |
| `core/apps/{lnk,startmenu,registry}.rs` | 编译全开；入口在 mac 自然返回空（无 APPDATA / winreg） | ✅（空转） |
| `core/apps/appbundle.rs`（新） | 编译全开（fixture 测试跨平台），仅 macos 的 indexer 接线 | ✅ |
| `core/apps/{scanner,launch,indexer,model}.rs` | (b) 函数/分支 | ✅ |
| `core/exec.rs` | (b) 扩展现有 cfg 分支（shell 发现、CREATE_NO_WINDOW） | ✅ |
| `core/system.rs` + `app/system.rs` | (b) `mod imp` | 仅 lock_screen |
| `core/search/win_settings.rs`、`everything.rs` | (a) `#[cfg(windows)]` 整模块 + 调用点 cfg | 不编译 |
| `core/search/{calc,clock,timestamp,fuzzy,pinyin,command,service}.rs` | 不动（BuiltinAction 加平台过滤） | ✅ |
| `core/hosts.rs` | (b) 已有 cfg 分支 | 读可用 / save 未收口 |
| `core/config.rs` | (c) 枚举变体 + `normalize_for_platform()` + `KNOWN_TOOLS` 平台子集 | ✅ |
| `app/winclip.rs`、`winproc.rs` | (a) 已有 `mod imp`；mac 新增 `macclip.rs` / `axproc.rs`（v1.5） | ❌ |
| `app/tile.rs`、`screenshot.rs`、`universal.rs`、`guide.rs` | 保持 Windows 实现 + 非 Windows 桩（报错） | 不进搜索 |
| `app/icons.rs` | (b) `mod imp`（NSWorkspace via objc2，已实现） | ✅ |
| `app/{hotkey,tray,windows,protocol,pump,share,state,commands,cli}.rs` | 基本不动（补平台桩与文案） | ✅ |
| `jsrt/*` | 不动（本就无 Windows 依赖） | ✅ |
| `frontend/` | `platform_info` 单一事实源 + i18n 键位装饰（已实现） | ✅ |
| `plugins/hosts` | manifest `platform: [{os:"win32"}]`（插件级平台门，已实现） | 列表标注不支持 |
| `scripts/*.ps1` | 保留（硬性约定 6），另增 `scripts/*.sh` 四件套 | ✅ |

### 1.3 插件清单：`platforms` 字段（v1 一并做）

`efto-plugin.json` 增加可选 `"platforms": ["windows"]`（缺省 = 全平台）。mac 上发现插件时
跳过不匹配项并在 `plugins` 列表标注「不支持当前平台」；被点到时明确报错。
hosts 插件标记 `["windows"]`。
→ 必须同步 `plugin-sdk/efto.d.ts` 注释与 `docs/plugin-development.md`（硬性约定 9）。

### 1.4 脚本层

- `scripts/*.ps1` 保留纯 ASCII（硬性约定 6），Windows 工具链不动。
- 新增 `scripts/bootstrap.sh` / `cargo.sh` / `dev-run.sh` / `verify.sh` 镜像 mac 路径。
- 两份 verify 的检查清单以 `docs/verification.md` 为唯一事实源；改一侧必对齐另一侧，
  两个脚本头注释互相指认（防漂移，见风险 7）。

## 2. 分阶段计划

### Phase 0 · 工程基座（两个平台都编译得动）✅ 2026-11-21 落地

1. mac 工具链脚本（`.sh` 四件套）；`cargo test --workspace` 在 mac 真跑（jsrt 会起 V8）。
2. rusty_v8 darwin 产物进 `vendor/` 或落「联网取用」判据；`bootstrap.sh` 是唯一入口。
3. `aarch64-apple-darwin` 全 workspace 编译绿：Win32 模块 cfg 排除，调用点补平台桩。
4. `tauri.conf.json` 补 `bundle.macOS`（LSUIElement 无 Dock 图标、usage description 文案）；
   定夺主窗口 `transparent: true` 的取舍（需 `macOSPrivateApi`，见风险 3）。
5. `docs/verification.md` 起 mac 段落，写明「没验证过什么」。

**退出判据**：mac 上 `cargo test --workspace` 绿；`verify.sh` 跑完并与 `verify.ps1`
公共检查结果一致。

### Phase 1 · core 平台层 + CLI 主链路 ✅ 2026-11-21 落地

1. `apps/appbundle.rs`：扫 `/Applications`、`~/Applications`、`/System/Applications`、
   extra_dirs / custom 里的 `*.app`（Info.plist 取 CFBundleName / CFBundleIdentifier /
   CFBundleExecutable）；`ignore_globs` 换 mac 默认集（`**/node_modules/**`、`**/.git/**`
   沿用，`WindowsApps` / `WinSxS` 等删除）。
2. `apps/scanner.rs`：`default_roots()` 平台分支；helper 过滤规则换 mac 语义
   （跳过 `.app` 内嵌 helper，按路径深度 / CFBundlePackageType 判定）。
3. `apps/launch.rs`：`.app` → `open -a <path> [--args …]`；裸可执行 → 直接 spawn；
   URI / 文件夹 → `open`。失败回退一次 `open`（对齐现有「点了没反应就回退 Shell」策略）。
4. `exec.rs`：shell 自发现换 zsh/bash（`/etc/shells` + PATH）；`CommandShell` 平台变体 +
   `normalize_for_platform()`；**不做命令语法翻译**——命令源在 mac 是 zsh 语义、
   Windows 是 PowerShell/cmd，`split_args` 的 Windows 引号规则只服务 Windows 启动路径，
   差异写进帮助文档。
5. `system.rs`：imp 平台分支；v1 只做 lock_screen，其余返回「当前平台不支持」。
6. `win_settings` / `everything` 排除出 mac 编译；`service.rs` 接线按 cfg。
7. **反例测试（硬性约定 2）**：mac 低优先发现渠道（appbundle 的 DirScan / Custom）
   必须断言「结果分数低于应用条目最低分（300×0.78≈234）」；新搜索源的整串触发边界
   照 `win_settings` 先例写「不该触发」反例。
8. `cli` 主链路 mac 端到端。

**退出判据**：`efto cli` 主链路 mac 可用，`verify.sh` 全绿；Windows 侧 `verify.ps1` 无回归。
**文档（硬性约定 9）**：`architecture.md` 补平台层小节；`plugin-development.md` 写
exec/shell 与插件清单 `platforms` 差异；`roadmap.md` 挂本计划；`verification.md` 记实测。

### Phase 2 · GUI 宿主（v1 交付）✅ 2026-11-21 代码落地（GUI 冒烟通过；人手验证清单见 verification.md）

1. 热键：默认 `Alt+Space`（Option+Space）；与输入法切换冲突时文档指引改绑。
   实测 tauri-plugin-global-shortcut 在 mac 是否免权限（风险 4）。
2. 托盘、单实例、设置/帮助/剪贴板窗口；前端与帮助文档的键位文案平台敏感
   （Alt → Option），i18n 检查脚本对齐。
3. `icons.rs` mac 实现：优先 NSWorkspace `iconForFile` → PNG data URL；
   备选 `sips` / `qlmanage` 外部命令兜底（风险 2）。
4. `clipwatch`：arboard 文本/图片不动；去掉 CF_HDROP 分支，文件列表明确不支持。
5. 插件 UI 窗口：`windows.rs` 的 `run_on_main_thread` 投递模式在 mac 保留
   （NSWindow 同样主线程约束）；`protocol.rs` 核实 WKWebView 下 `efto://` 的实际
   scheme 归一行为（现注释只写了 Windows 差异，补 mac 结论）。
6. hosts 能力与 hosts 插件：manifest 标 `platforms: ["windows"]`，API 调用明确报错。
7. 设置页工具列表走 `known_tools()`；被砍入口不展示。
8. GUI 手工验证清单（热键、唤起、启动 `.app`、插件 UI、托盘、失焦隐藏）。mac GUI
   自动化不申请 System Events 权限，沿用「无 UI 验证优先走 cli」的约定。

**退出判据**：GUI 日常可用 = v1 发布候选。
**文档**：`help.*.md`、`plugin-development.md`、`verification.md` 同步。

### Phase 3 · v1.5 系统集成补齐（按性价比排序）✅ 2026-11-21 落地（1-7 全做；入口开关与
快捷键指南数据除外，见 verification.md 已知取舍）

1. `mdfind` 文件检索源：≥2 字符才查、基础分 160 压应用下、超时短、失败静默；
   反例测试照 `everything.rs` 先例。
2. 系统设置搜索（`x-apple.systempreferences:` 数据表 + 整串前缀触发）。
3. 系统动作全家（AppleScript / pmset）。
4. 文件列表剪贴板 + 粘贴前台（NSPasteboard / CGEvent），配 Accessibility 引导。
5. hosts 编辑（osascript 管理员提示）+ hosts 插件 mac 化。
6. `cli ps` / `focus`（CGWindowList + AX）。
7. 书签（mac 浏览器路径）、快捷键指南 macOS 数据集（`shortcuts/data/` 新增）。

### Phase 4 · 重量级 ✅ 2026-11-21 部分落地：窗口平铺/撤销、通用动作放出、权限引导、
发布工程脚手架已做；**截图捕获层已补**（2026-11-21 真机实测）；
快捷键指南 macOS 数据集已补（2026-11-21）

- 截图 / 长截图 / 标注（ScreenCaptureKit + 权限引导 UI）
- 窗口平铺 / 撤销平铺（CGWindowList + AX API + 私有 CGS）
- 通用动作（CGEvent 模拟 Cmd+C）
- 签名 / 公证 / 自动更新 / arm64+x86_64 双架构发布

## 3. macOS 平台约束（用户可感知，文档必写）

| 约束 | 影响 |
| --- | --- |
| Cmd+Space = Spotlight | 默认唤起用 Option+Space；个别输入法占用 Option+Space，允许改绑 |
| Accessibility 授权 | 平铺 / 通用动作 / `ps` / `focus` / 粘贴前台需要；v1 不碰，v1.5 起配引导页 |
| Screen Recording 授权 | 截图需要（Phase 4） |
| `transparent: true` | 需 Tauri `macOSPrivateApi`（私有 API，App Store 分发受限；dmg 直发无碍） |
| 签名与公证 | 无签名时 Gatekeeper 拦截；发布工程在 Phase 4 |
| 双架构 | arm64 / x86_64；rusty_v8 预编译产物分架构 |
| 数据目录 | `~/.efto` 不变；`EFTO_PLUGIN_DIRS` 分隔符已按平台分支 |

## 4. 风险清单

1. **rusty_v8 darwin 产物**：`vendor/` 现在只有 Windows 静态库；离线构建要补 darwin
   目录，联网构建走官方取用。`bootstrap.sh` 是唯一入口，别让构建路径分叉。
2. **图标获取**（已定夺 2026-11-21）：NSWorkspace via objc2。tauri 依赖树本就有
   objc2 0.6/objc2-app-kit 0.3，版本对齐后零新增 C 工具链；`qlmanage`/`sips` 兜底未采用。
3. **透明窗口**：`macOSPrivateApi: true` 才能透明；若不能接受私有 API，备选是不透明
   背景色 + 圆角素材（视觉降级）。Phase 0 定夺并记入 verification.md。
4. **热键免权限假设**：tauri-plugin-global-shortcut 在 mac 走 Carbon
   `RegisterEventHotKey`（免授权）还是 CGEventTap（要 Input Monitoring）尚未实测，
   Phase 2 验证，结论进 `verification.md`。
5. **缓存兼容**：`AppSource` / `TargetKind` 加变体不动 `CACHE_VERSION`；`ScanReport`
   加字段必须 `#[serde(default)]`。两平台缓存不互通（同一台机器只会单平台），不写迁移。
6. **命令源语义**：`>` 前缀命令在 mac 是 zsh 语义、Windows 是 PowerShell/cmd；
   不搞语法翻译，设置页与帮助文档写明差异（硬性约定 9）。
7. **verify 双份漂移**：`verify.sh` 与 `verify.ps1` 以 `docs/verification.md` 为唯一
   事实源；改一侧必对齐另一侧，否则视为未完成。

## 5. 施工纪律

- **硬性约定 2**：mac 每个新搜索源补「不该触发」反例测试（appbundle 低优先渠道、
  mdfind、系统设置）。
- **硬性约定 3**：未实现/不支持的能力显式报错，不静默成功；设置页不展示被砍入口。
- **硬性约定 9**：每阶段结束同步 `architecture` / `plugin-development` / `verification` /
  `roadmap` / `help.*` 与 `plugin-sdk/efto.d.ts`；文档没跟上视为未完成。
- **Agent Notes**：每阶段「新增/改变用户可感知行为」在汇报完成前落
  `.agents/notes/<lifecycle>/<class>/` note（mac 支持主体是 `feature`，verify 脚本双份
  对齐是 `process`）；动手前可先落 `proposed`，完成后移 `implemented` 并让 Status 一致。
- **名字边界（硬性约定 1）**：mac 上仍是 `efto`——`~/.efto`、`efto-plugin.json`、
  `efto.*` API、`efto://` 协议；**不做任何第三方启动器（Alfred / Raycast）的兼容导入**。


## 附：macOS 发布工程（签名 / 公证 / 双架构）

构建入口：`scripts/release.sh`（设置 V8 环境后跑 `tauri build`，产出
`target/release/bundle/` 下的 .app 与 .dmg；未签名产物本机可用）。

前置（签名/公证）：Apple Developer ID（付费）与 Developer ID Application 证书。没有证书时 dmg 可以直发，
但 Gatekeeper 会拦（用户右键打开或 `xattr -dr com.apple.quarantine`）。

1. **签名**：`tauri.conf.json` 的 `bundle > macOS > signingIdentity` 填
   `Developer ID Application: …`（或环境变量 `APPLE_SIGNING_IDENTITY`）后
   `tauri build`——tauri-bundler 会做 `codesign --deep --options runtime`。
2. **公证**：`xcrun notarytool submit efto.dmg --keychain-profile efto-notary --wait`
   → `xcrun stapler staple efto.dmg`。凭证先用 `xcrun notarytool store-credentials` 录入。
3. **双架构**：`tauri build --target universal-apple-darwin`；rustup 需装
   aarch64/x86_64 两个 target，且 `vendor/rusty_v8/` 要备两个架构的静态库
   （当前只有 aarch64，x86_64 产物由 `scripts/bootstrap.sh` 按需下载）。
4. **自动更新**：`tauri-plugin-updater`（Windows 侧同样未接，见 roadmap「发布工程」）。

人工验证：另一台 Mac 双击 dmg → 拖进 Applications → 首启无 Gatekeeper 拦截；
`spctl -a -vv /Applications/efto.app` 显示 accepted。
