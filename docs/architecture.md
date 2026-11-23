# 架构说明

## 分层与依赖方向

```
        ┌──────────────────────────────────────────┐
        │ frontend/  静态 HTML/CSS/JS（无打包器）    │
        └───────────────▲──────────────────────────┘
                        │ __TAURI_INTERNALS__.invoke / 事件
        ┌───────────────┴──────────────────────────┐
        │ crates/app     Tauri 2 宿主              │
        │  hotkey tray windows protocol pump       │
        │  commands ui  state   host(能力实现)      │
        └───────▲───────────────────────▲──────────┘
                │ Arc<dyn Host>          │ 类型/查询
        ┌───────┴──────────┐    ┌────────┴─────────┐
        │ crates/jsrt      │    │ crates/core      │
        │  engine bridge   │    │  apps search     │
        │  prelude ts pool │    │  plugins store   │
        │  runtime ipc     │    │  config usage    │
        └──────────────────┘    └──────────────────┘
```

依赖是单向的：`app → {jsrt, core}`，`jsrt → 无内部依赖`，`core → 无内部依赖`。
jsrt 不依赖 Tauri，也不需要 core —— 它通过 `ipc::Host` trait 反向索取能力。这样做的收益：

- 插件运行时可以脱离 GUI 单独测试（`cargo test -p efto-jsrt` 真的会起 V8）。
- 换一个 UI（CLI、服务、别的路由器）只需要重写 `Host`。
- core 的索引/搜索逻辑是纯函数式的，`.lnk` 解析、打分、去重都能在单元测试里断言。

### 名字边界

产品身份统一用 `efto`，不要"顺手统一"成别的名字，也不要引入第三方启动器的命名：

- **产品身份 = efto**：crate `efto-core/jsrt/app`、产物 `efto.exe`、`productName`/窗口标题/托盘/日志/UA、
  bundle id `app.efto.launcher`、数据目录 `~/.efto`（用户主目录下的 `.efto`，见 `core/src/fsutil.rs`）、
  环境变量 `EFTO_*`。
- **插件协议只认 efto**：插件用 `efto-plugin.json` 描述，运行时只注入 `efto.*` / `window.efto`，
  UI 走 `efto://`，桥是 `__efto_dispatch`，类型声明是 `plugin-sdk/efto.d.ts`。
- **不提供任何第三方兼容层**：不读别的清单文件名当插件清单、不注入兼容全局对象、不注册兼容协议。
  宿主内部命令名（`plugin_*`、`plugin:message`）是 efto 自己的 IPC 契约，不是兼容面。

### 平台层（Windows / macOS）

macOS 支持的总设计见 `docs/macos-support-plan.md`，这里只记施工约定：

- **公共 API 面不动，平台差异收在模块内部**。三种切法按差异大小选：整块不同 → 模块级
  `#[cfg]`（如 `search/win_settings.rs`、`everything.rs` 只在 Windows 编译）；同职责不同
  实现 → `mod imp`（`exec` / `system` / `icons`）；配置模型 → 平台变体 + `Settings::
  normalize_for_platform()`（`CommandShell` 的 zsh/bash/fish、`KNOWN_TOOLS` 平台子集）。
- **编译全开，接线按平台**：发现源模块（lnk/startmenu/registry/appbundle）不按平台排除
  编译——`IgnoreMatcher` 等公共件住在那里，fixture 测试跨平台跑有价值；取舍发生在
  `indexer` 汇总与搜索接线处。例外是 Windows 专有数据表（win_settings/everything）。
- **宁可不出，不可乱出**：平台不支持的搜索源不接线、平台不支持的内置动作不进搜索结果
  （`BuiltinAction::supported_on_platform`）；被砍能力的执行路径返回"当前平台不支持：…"
  （硬性约定 3）。
- **前端不猜平台**：`platform_info` command 是前端展示的单一事实源（Shell 下拉、工具行、
  功能入口都按它显隐）；热键展示文案的平台装饰（`Alt`→`Option` 等）在 i18n 解析出口
  做（Rust `i18n::decorate_keys` 与前端 `decorateKeys` 同规则，改一侧对齐另一侧），
  热键录入/存储始终是 Tauri 语法。
- **插件目录优先级**：`EFTO_PLUGIN_DIRS` → `settings.plugin_dirs` → `~/.efto/plugins`
  → **安装包内置**（`Resources/plugins`，macOS；Windows 为 exe 同级 `plugins/`）。
  `PluginRegistry::discover` 按插件 id 级去重、保留先发现的那份，所以用户装过/改过的
  插件不会被内置副本顶掉，而新机器上开箱就有全部内置插件。
- **搜索源成对**：文件检索 Windows 走 `everything.rs`（es.exe）、macOS 走 `spotlight.rs`
  （mdfind，exec 白名单的 `max_lines` 行数封顶——全量枚举实测十几秒）；系统设置搜索
  `win_settings.rs` / `sys_settings.rs` 各守各的前缀，都带"不该触发"反例。
- **双端 gate**：平台敏感改动要在两端各跑一次 `scripts/verify.sh` / `scripts/verify.ps1`
  （检查清单的唯一事实源是 `docs/verification.md`，改一侧必对齐另一侧）；无双机时由
  `.github/workflows/ci.yml` 的 windows-latest + macos-latest 矩阵兜底。

## 应用发现（crates/core/src/apps）

来源如下，最后按"真实 exe"归并去重：

| 来源 | 取什么 | 排序先验 | 为什么 |
| --- | --- | --- | --- |
| StartMenu | 用户级 + `ProgramData` + Public 的 `Start Menu\Programs` 递归 | 1.00 | 名称最规范（"Google Chrome"而非 `chrome.exe`），且 `.lnk` 自带参数/工作目录/提权标记 |
| Desktop | 用户桌面（`dirs::desktop_dir()`，跟随重定向）+ `%PUBLIC%\Desktop` 的 `.lnk`/`.url` | 0.98 | 用户放在桌面上的快捷方式不在开始菜单里，不索引就永远搜不到 |
| Registry | HKLM 64/32 视图 + HKCU 的 `...\Uninstall` | 0.92 | 正规产品名、版本号、发布方；只收"能真正启动"的项 |
| DirScan | `Program Files`、`Program Files (x86)`、`%LOCALAPPDATA%\Programs` | 0.78 | 抓绿色软件；深度 ≤ 4、体积 ≥ 64KB、辅助程序名黑名单 |
| Custom | 设置里「自定义索引目录」下的 `.exe` / `.lnk` / `.url` | 0.80 | 用户自己收藏的工具目录；快捷方式名字优先于同目标裸 exe |
| 白名单 | 记事本/画图/计算器/设置/任务管理器等 | 0.90（Manual） | 这些没有卸载项、也不一定在开始菜单里，但用户一定会搜 |
| AppBundle（macOS） | `/Applications`、`/System/Applications`、`~/Applications` 的 `*.app`（Info.plist 取名，bundle id 进别名） | 1.00 | 与开始菜单同级的规范来源；`.app` 是叶子，不递归 `Contents/MacOS` |

去重键是 `resolve_key()`：`.lnk` 会解析出真实目标 exe，所以"开始菜单的微信快捷方式"和"`Tencent\WeChat\WeChat.exe`"合成一条，保留开始菜单的名字，把另一条的文件名并入别名。

### 文件关联（`apps/assoc.rs`）

插件清单的 `fileTypes` 决定它能打开哪些扩展名。设置页点「注册到「打开方式」」后，
`assoc::register` 只写 `HKCU\Software\Classes`：`efto.<插件id>` 的默认名 / `DefaultIcon` /
`shell\open\command`（`"<efto.exe>" --open-file "%1" --plugin "<id>"`），以及每个
`.ext\OpenWithProgids` 下的登记项；免管理员、不改默认程序，取消注册只删这些项。
系统随后以 `efto.exe --open-file <路径>` 启动本程序：`main.rs` 交给 `lib.rs::parse_open_file`，
单实例机制把 argv 转给已运行实例；`commands::open_file_with_plugin` 校验扩展名后，
以 `cmd = "open-file"`、`payload = 路径` 唤起对应插件。非 Windows 平台明确报错。

### `.lnk` 解析（`apps/lnk.rs`）

自己实现 MS-SHLLINK 二进制解析，不走 COM：不需要 STA/COM 初始化，索引可以并行、也不需要消息泵。

- 头部 76 字节按文档校验（magic 0x4C + CLSID + flags），剩余字段是**固定 52 字节**——这里最初算成 48，导致后面所有偏移错位，测试里用真实快捷方式抓到了。
- `LinkInfo` 块靠"块头特征"识别（BlockSize ∈ 28..4096 且 HeaderSize ∈ {0x1C,0x20,0x24,0x28,0x2C}），而不是靠 flags：真实文件里 flags 的可选块位在 MS-SHLLINK 与 IShellLink 之间编号不一致，靠它猜会错。
- `StringData`（Name/RelativePath/WorkingDir/Arguments/IconLocation）按固定顺序**尽力读**，任一块校验失败就停；参数和工作目录读不到也不影响启动，因为我们把 `.lnk` 本身交给 ShellExecute，Windows 会解析全部字段。
- 结构化解析失败时退回启发式：在原始字节里找 UTF-16LE 的 `X:\` 或 ANSI 绝对路径。UTF-16 的字节序是 `C 0 : 0 \ 0`，最初按 `X : \` 连续匹配所以永远命中不了。

`tests::parses_real_start_menu_links` 会拿本机真实快捷方式断言"≥60% 能还原出存在的路径"，这是防回归的关键测试。

### 忽略规则（噪音目录过滤）

两层，语义不同，别混：

- `scanner.rs` 的**目录名黑名单**（小写整段匹配）：`node_modules`/`temp`/`tmp`/`locales`/
  `resources`/`plugins`/`extensions`/`packages`/`assemblies`/`winsxs`/`serviceprofiles`，
  外加任何以 `$` 或 `.` 开头的名字。只在深度 >0 生效——调用方显式指定的根（depth 0）不被剪枝。
- `IgnoreMatcher` 的 **glob 规则**：`settings.ignore_globs`（用户可编辑）+ 一份固定噪音列表。
  它同时作用于开始菜单遍历和目录扫描，匹配的是**完整路径**。

三个曾经的坑，都写在代码注释里：

1. **glob 必须 `case_insensitive(true)`**。`is_ignored` 先把路径 `to_lowercase()` 再匹配，
   而 globset 默认区分大小写 —— 于是所有含大写字母的规则（`Temp`/`WinSxS`/`WindowsApps`/
   `$Recycle.Bin`，以及用户自然写出的 `**/Steam/**`）过去**全部静默失效**。
2. **固定列表里不该放 `Temp`**。"永远不可能是程序位置"的系统内部目录才配当不可移除的硬规则
   （WinSxS / WindowsApps / GAC `assembly` / `$Recycle.Bin` / 8.3 短名 `~1`）；
   Temp 只是常见噪音，做成硬规则会剥夺用户索引临时目录里绿色软件的能力。
   它现在只出现在可编辑的默认设置里。
3. **死规则要靠测试暴露**。默认列表里曾有一条 `"**/ assemblies/**"`（多一个前导空格），
   只能匹配名字真以空格开头的目录，等于没写；因为当时忽略规则一条测试都没有，它躺了很久。
   现在有 `every_default_ignore_rule_matches_something` 逐条断言每条规则都能命中真实形状的路径。

## 搜索打分（`search/`）

一行线性扫描（千级条目 <1ms，够用且零维护）。派生形式：`lower` / `compact`（去空格标点）/ `pinyin`（全拼）/ `initials`（汉字拼音首字母 + 英文词首字母）。

优先级：完全匹配 1000 > 前缀 900 > 紧凑前缀 860 > 词首 800 > 拼音前缀 780 > 包含 700 > 首字母前缀 700 > 子序列 ≤600。再乘来源先验，加使用频次/最近使用（上限 150）。插件 `cmd` 特殊处理：**输入以触发词开头即命中**（`翻译 hello` 里 "翻译" 是关键字、"hello" 作为 feature），纯关键字 950、带负载 900。

停用插件保留在注册表里（`PluginDescriptor.enabled = false`），但 `PluginRegistry::get` 与 `search` 都过滤 `enabled`，因此"停用"等于从搜索与唤起入口消失，而设置页仍能列出并重新启用。停用名单是 `settings.disabled_plugins`（插件 id，大小写不敏感）；启停后宿主只重建插件注册表（`AppData::reload_plugins`），不动应用索引，并停掉不再启用的 isolate。

内置计算器（`search/calc.rs`）走另一条判定路径：先归一化（全角转半角、千分位、trim），再要求"出现运算符/函数调用"，然后用递归下降**完整消费**整串——任何残留字符都判为"不是算式"。这套严格条件的唯一目的就是**不劫持应用搜索**：`chrome`、`3.10 64-bit`、`1700000000`、`(5)`、`1/0` 都不该冒出计算行。求值器带输入长度上限与递归深度上限，除零/溢出/未定义函数一律返回 None（不显示）而不是报错，因为"计算器没结果"应该是静默回落，而不是打扰用户。结果作为 `HitPayload::CopyText` 返回，`Enter` 复制由 app 层做（core 不依赖剪贴板库）。

时间 → 时间戳（`search/timestamp.rs`）与计算器共用同一套"宁可不出，不可乱出"的纪律，
外加三条只属于它的规则。① **两段式日期要让路**：`11-16`、`2-3` 既是日期写法也是算式，
判给计算器；只有出现过 `月/日/号` 或后面跟了时间（`11-16 15:00`）才当日期。
反过来 `2026-11-16` 判给日期，`calc::evaluate` 入口处有 `looks_like_time_input` 闸门主动让位，
否则输入一个日期会顺手算出 1,999。② **歧义即放弃**：`11/12/2026` 分不清月/日顺序就不出结果，
只有能靠"谁大于 12"消歧的 `16/11/2026`、`11/16/2026` 才接受；`looks_like_date` 明确拒绝
10/13 位纯数字，因为那本身就是时间戳，属于反向换算。③ **时区三层优先级**：
输入自带 `Z`/`+08:00` > 测试注入的兜底时区 > `chrono::Local`。走 `Local` 时
DST 空档（`from_local_datetime` 无解）返回 None，回拨重复取 `earliest()`。
中文写法不走字符串替换的二级解析，而是 `normalize`（全角/标点/小写，**保留**中文单位）
→ `Flags::scan`（记下 月日号=授权、时分秒=授权裸小时、上午下午=12 小时制）
→ `translate_units`（`2026年11月16日15时03分` → `2026-11-16 15:03`）——`日` 必须翻译成
空格而不是删除，否则日期和时间会黏成 `2026-11-1615:03`。
两条时间戳都是 `HitPayload::CopyText`（core 不依赖剪贴板库），`echo` 里带
`components`/`iso`/`zone`，前端 `kind === 'time'` 与 `'calc'` 一样单击即复制。

系统时间多格式（`search/clock.rs`）是它的姊妹路径：输入整串等于 `date`/`now`/`time`/
`today`/`时间`/`日期`/`现在`/`当前`/`今天`/`系统时间` 时，给八种常用格式
（ISO 本地、Windows 斜杠、仅日期、仅时间、RFC3339、RFC1123/HTTP、中文、文件名安全）。
三条约束：**整串相等才触发**（`update`/`dated`/`datetime`/`date 2026-11-16` 一律不碰，
所以归一化只做全角→半角 + 小写 + trim，绝不做包含匹配）；**时刻由调用方传入**
（`service::query` 取一次 `Local::now()`，同时喂给 clock 与 timestamp，否则跨秒会出现
同一屏里 `17:48:59` 和下一秒的 epoch）；**分数 700→693**，低于应用精确匹配（≈1000）、
高于时间戳两行（690/686），所以格式行在最上面而不会把同名应用挤掉。
RFC1123 与中文行的星期/月份名不用 chrono 的 `%a`/`%b`（依赖 locale），
而是自己按固定英文表拼 —— 代价是要靠 Node 的 ICU 表独立复核（探针里就是这么做的）。

系统设置页（`search/win_settings.rs`）同样只认严格入口：**整串以 `ms-settings:` 开头**
才触发。冒号后为空时列出内置的常用设置页（`display`/`bluetooth`/`windowsupdate` 等）；
有内容时按 URI 前缀/包含、中文标题与英文别名过滤；内置表里没有的 URI 仍保留一条
「打开 ms-settings:…」行，交给系统外壳判断。命中行是 `ItemKind::External` +
`HitPayload::Open`，与直接输入 URL 同一条执行路径。冒号后有内容时该源独占结果
（避免收藏夹/应用的子序列混入）；普通输入、以及没有冒号的 `ms-settings`
一律不触发；该前缀同时让 Everything 文件源让路。

### 默认面板（空查询）

搜索框刚打开（空查询）时后端不再返回空列表，而是 `SearchService::query_with_panel` 拼出的两段：

1. `group = "recent"`：`core/src/recent.rs` 的 `RecentStore` 快照（`~/.efto/recent.json`，上限 20）。
   存整条 `SearchHit` 而不是只存 id —— 插件与内置指令无法凭 id 重建，应用命中也需要快照里的
   `LaunchTarget` 才能直接执行。宿主在 `invoke_hit` 的 `Launch`/`Plugin`/`Action` 分支各记一次；
   `CopyText`/`Open` 不记（复制结果与打开文件不算"最近使用"）。
2. `group = "tools"`：`settings.tools` 里的内置工具 id（默认 `screenshot`/`clipboard`/`lock_screen`/
   `tile_windows`/`shortcut_guide`/`snippets`/`file_buffer`，即 `config::DEFAULT_TOOLS`），
   按 `KNOWN_TOOLS` 规整、未知 id 丢弃。`color_picker`（颜色提取器）与 `sticky_note`（便签纸）
   在 `KNOWN_TOOLS` 内、可在设置页勾选加入，但**默认不进**常用工具。
   设置页的工具开关写的也是同一字段，**数组顺序就是面板顺序与 Alt+1..9 编号**。
   设置页的「面板顺序」列表支持鼠标拖动排序（pointer 事件，非 HTML5 dnd ——
   WKWebView 里 dragover 不稳）；拖动结果直接改写该数组，保存时以它为准，
   但会把"设置页打开期间主面板新加的 app:/plugin: 条目"追加到末尾而不是丢弃。

**面板内可直接拖动排序**：工具卡片带 `hit.pin`（即 `settings.tools` 里的值），
拖动结束后按 DOM 顺序读回 `data-pin` 写回 `settings.tools`。用 pointer 事件而非
HTML5 dnd（WKWebView 里 `dragover` 不稳），6px 阈值区分点击与拖动；拖动结束会
置 `suppressTileClick` 挡掉浏览器补发的 click —— 否则松手就把工具执行了。

分数只用来保证"最近使用"永远排在工具之前（600 递减 vs 500 递减）。`group` 是
`Option<String>` 且 `skip_serializing_if` 省略，普通搜索结果完全不带该字段；
前端把它渲染成两条"展示用"标题行（`.group-head`，`pointer-events:none`、不进键盘序列），
工具行单击即执行（用户明确要求），其他行保持"单击选中、双击执行"。

### 内置工具（`crates/app/src/{screenshot,clipwatch,tile,guide}.rs`）

这些工具都走 `run_action`，命中 id 与 `BuiltinAction::name()` 一致，所以既能从默认面板进、
也能直接搜"截图"进。

- **截图**：`hide_main` → 等 140ms（`hide_main` 是投递到主线程的，不等就会把搜索框截进去）
  → `screenshot.rs::capture_virtual_screen()`（`EnumDisplayMonitors` 求所有显示器的物理外接矩形 +
  `GetDC`/`BitBlt`，`SRCCOPY|CAPTUREBLT` 连分层窗口一起抓，`GetDIBits` 拿 BGRA 后换 RGBA）
  → 存入 `AppData.shot`（**先存再开窗**，覆盖层页面一加载就会来取图）
  → 独立线程 `build()` 一个无边框置顶窗口，**铺满整个虚拟桌面**，`screenshot.html` 用 CSS 像素画遮罩与选区，
  提交时按 `shot.width / innerWidth` 换算回物理像素交给 `screenshot_finish`。
  裁剪、PNG 编码、保存/复制/钉在屏幕都在宿主侧；钉图落盘 `~/.efto/pins/<id>.png`，
  窗口关闭时（`WindowEvent::Destroyed`）删掉临时文件。
- **截图标注**：选区出现后工具栏提供 Snipaste 风格标注——矩形/椭圆/箭头/画笔/荧光笔/马赛克/文字/橡皮
  （数字键 1-8 切换）、8 色色板、三档粗细、快照式撤销/重做（上限 60 步）。标注全部画在覆盖层 canvas
  上并裁剪到选区内；提交时前端把「选区原始画面 + 标注」合成为物理像素 PNG data URL，经
  `screenshot_finish` 的 `overlay` 参数交回宿主，`screenshot.rs::overlay_png` 解码后直接替代对
  整屏图的裁剪——复制/保存/钉图共用这一个入口，宿主不做第二套绘制。没有标注时 `overlay` 缺省，
  走原来的 `Shot::crop` 路径。长图模式不支持叠加标注（有标注时按钮置灰）。
- **选区移动**：在没有激活标注工具时，指针落在选区内按下并拖动会整体平移选区（尺寸不变，
  边缘夹紧在窗口内），已提交的标注按同一位移量平移，保持相对选区的排版；因此马赛克会重新
  采样移动后的底层画面。移动位置本身不进标注撤销栈（Ctrl+Z 只回退标注），选区内单击/双击
  不会产生多余历史；长图模式选区固定，不参与移动。选区外按下仍然是重新框选。
- **长截图**：覆盖层工具栏的「长图」按钮把选区（物理像素）交给 `screenshot_long_start`。
  覆盖层**留在屏幕上**（`transparent(true)` 窗口，藏掉整屏图后只画遮罩、边框和滚动提示），
  同时 `set_ignore_cursor_events(true)` 让鼠标穿透，并用 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`
  把自己从屏幕捕获里排除；用户直接在底下的窗口里拖动滚动条。后台线程 `efto-longshot` 每 180ms
  用 `capture_region` 抓同一块区域，`scroll_delta` 拿上一条 48 行横带（跳过顶部 1/5 的吸顶栏）
  在两帧之间按 SAD 找最佳位移 `d`，把新帧底部 `d` 行追加到结果；覆盖层轮询
  `screenshot_long_status` 把边框下边缘按已拼接行数往下画。
  有过追加后 2.5s 无新增、或 90s 硬上限、或按 Esc（临时全局热键）时收尾：
  PNG 存到 `Pictures/efto/Screenshots/LongShot_*.png`（被拒时退到 `~/.efto/Screenshots`）
  并复制到剪贴板，用系统通知反馈。
  限制：吸顶/吸底内容会重复或缺失；滚动快于约 0.75 屏/180ms 时可能漏行；结果上限 3000 万像素。
- **颜色提取器**：`run_action("color_picker")` 复用截图同一套「收起搜索框 → 抓整屏」路径
  （`capture_after_hiding_main`），把冻结画面存进 `AppData.shot`，再开与截图同规格的全屏透明覆盖层
  （`color-picker.html`，Windows 一个虚拟桌面窗口、macOS 每屏一个）。页面按
  `view_w / innerWidth` 把光标位置换回图内物理像素，用离屏 canvas 取该点 RGB，显示 15×15 放大镜、
  十字线与 HEX/RGB；单击或 `Enter` 调 `color_picker_finish` 把 HEX 写进剪贴板并关窗，`Esc` 调
  `color_picker_cancel`。画面在唤起瞬间冻结，动画/视频里的颜色以冻结帧为准。
- **便签纸**：`run_action("sticky_note")` 每次新建一条便签（多条独立窗口共存），数据在
  `crates/app/src/notes.rs`（`~/.efto/notes.json`，字段 `id/text/x/y/width/height`）。
  窗口 `note.html`：无边框、`always_on_top`、可缩放、不占任务栏，标题栏拖动走 `note_drag`，
  文本改动去抖 300ms 调 `note_save`（同时回写窗口几何），关闭窗口即删除该条。
  下次启动在 setup 里按存储顺序恢复全部便签（不抢焦点）；宿主退出时 `RunEvent::ExitRequested`
  置 `QUITTING`，`WindowEvent::Destroyed` 见到该标记就不再删除，避免退出时把便签清空。
- **剪贴板**：`clipwatch.rs` 用 500ms 轮询（不用 `WM_CLIPBOARDUPDATE`，省一个隐藏消息窗口），
  优先级 **文件列表（CF_HDROP）> 图片 > 文本**，同一份内容只记一次。启动时先读一次当前剪贴板当基线，
  所以不会把启动前的内容当成新记录。
  - 文本直接写 `ClipboardStore`；文件列表按 `CF_HDROP` 读路径（`winclip::read_files`）；
    图片先写成 `~/.efto/clipboard-images/<签名>.png`，条目只存路径与尺寸标签，缩略图由
    `clipboard_thumb` 按需生成（96px，进程内缓存 128 条）。
  - 置顶条目不参与 200 条上限与 30 天清理；其余按最近使用倒序。删除/清空历史时同步删掉图片文件。
  - 复制一条历史（文本/图片/文件列表）走 `clipboard_copy`；`Ctrl+Enter` 走 `clipboard_paste`：
    复制后隐藏主窗口，再用 `SendInput` 向唤起前的前台窗口发 `Ctrl+V`。目标窗口以更高权限运行时
    `SetForegroundWindow` 会失败，此时明确报错并保留剪贴板内容，不假装粘贴成功。
  - 窗口 `clipboard` 是 `tauri.conf.json` 里的常驻隐藏窗口。
- **锁屏**：`hide_main` 后直接 `LockWorkStation()`；返回 0 视为失败并报错（不能静默成功）。
- **窗口操作**：`tile.rs` 的 `grid_layout` / `half_layout` 是纯函数（工作区、窗口数/左右、8px 间距/边距 → 矩形），
  Win32 层用 `EnumWindows` 过滤可见、有标题、非最小化、非 `WS_EX_TOOLWINDOW`、无 owner、
  非 cloaked、非 efto 自身进程、非桌面/任务栏；网格只取搜索框所在显示器，最小化作用于所有显示器；
  最大化窗口先 `SW_RESTORE`，再用 `BeginDeferWindowPos` 批量移动（`SWP_NOZORDER|SWP_NOACTIVATE`），
  批量失败时逐个 `SetWindowPos` 并统计失败数。`Op` 枚举统一承接 grid / left / right /
  min-others / min-all / undo：前两者取「搜索框下方的窗口」（前台是 efto 时回落到 `guide::last`），
  最小化用 `SW_MINIMIZE` + `IsIconic` 复核，undo 把上一次平铺的 hwnd 与原矩形（含最大化状态）
  存在进程内 `Mutex`，只保留最近一次。CLI 的 `tile-windows --mode … --dry-run --json` 复用同一层。
- **快捷键指南**：`core/src/shortcuts/data/*.json` 用 `include_str!` 嵌入每个应用一份 JSON
  （Windows、Office、浏览器、VS Code、Photoshop、Figma、IDEA、Trae），
  `resolve(exe)` 未命中时回退 Windows 通用数据；用户覆盖在 `~/.efto/shortcuts/<exe>.json`，
  同名分组按 `keys` 合并、用户优先。`winproc::foreground` 在热键按下、搜索框显示前记录前台窗口，
  `guide` 模块保存它（跳过 efto 自身），执行工具时复核窗口仍存在，再打开常驻隐藏的 `shortcuts` 窗口。
  `import_set` 供 CLI `import-shortcuts` 使用：先校验（app.name/exe、分组非空、keys/action 非空、
  组内 keys 去重、exe 不能含 `/\:*?"<>|`），再与目标文件合并（同名分组 + keys 忽略大小写，
  导入数据优先），统计 added/updated 后用 `fsutil::write_atomic` 落盘；`--dry-run` 只算不写，
  目标文件解析失败则拒绝覆盖。
- **片段**：数据在 `core/src/snippets.rs`（`~/.efto/snippets.json`），搜索侧只在**关键词与输入整串相等**
  （trim、大小写不敏感）时产出一条 `ItemKind::Snippet` 行（分数 720），不做前缀/子序列匹配，
  以免日常输入被片段劫持。正文支持 `{date}`/`{time}`/`{datetime}`；`Enter` 复制展开结果并在有前台窗口时
  延迟粘贴回去。不做系统级逐字键盘钩子。设置页新增"片段"页负责增删改与保存。
- **文件动作 / 文件缓冲**：搜索结果里的文件行带 `⋯`，`Tab` 打开主窗口内的动作菜单，
  支持打开、打开方式、在资源管理器中定位、复制路径、复制文件、加入缓冲、删除到回收站；
  `Ctrl+Enter` 直接加入缓冲。缓冲由 `Shared.file_buffer` 持有（去重、上限 100），
  `files` 窗口批量复制/移动/删除/清空。删除统一走 `SHFileOperationW` + `FOF_ALLOWUNDO`。
- **通用动作**：第二个全局热键（默认 `Ctrl+Alt+Space`，设置页可关/可改，与唤起热键冲突时自动换一个）
  触发 `universal::capture`：记住前台窗口 → `SendInput` 发 `Ctrl+C` → 读剪贴板文本 →
  打开常驻 `actions` 窗口。动作固定为复制、粘贴回前台、网页搜索；内容像链接或已存在路径时追加"打开"。
  抓不到选中文本时窗口显示空态提示，动作执行返回明确错误。
  当 `Settings::models` 非空且注册表里有 `translate` / `ai-chat` 插件时，另有选中文本时追加
  "翻译"与"AI 追问"：翻译按内置动作调用无 UI 后端插件（`efto.output` 结果内联到主窗口），
  AI 插件自行打开 UI 并通过 `onPluginEnter` 的 payload 预填选中文本；执行前复核模型与插件，
  菜单过期时返回明确错误而不是静默失败。
- **单项热键**：`settings.tool_hotkeys` 把工具 id 映射到 Tauri 加速键，默认 `Ctrl+Alt+1..7`
  依次对应 `config::DEFAULT_TOOLS`（颜色提取器/便签纸不默认占用热键）；默认值挂在 `Settings::default()` 上（`#[serde(default)]` 只补缺失字段），
  设置页清空后保存的是显式空表，不会被重新补回，否则"解绑"永远无效。
  `hotkey::rebind` 在注册唤起热键与
  通用动作热键之后逐个注册，回调里 `ShortcutState::Pressed` 时把 `run_action` 投到独立线程
  （锁顺序与线程约束不变，主线程只做窗口消息）。空键、未知工具、与另外两个热键重复的组合在
  `normalize_tool_hotkeys` 阶段丢弃；`Shortcut::from_str` 或系统注册失败的组合只写日志，不影响其它热键。
  设置页的输入框是"按键捕获"：`keydown` 拦截默认行为，把 `e.code` 归一成 Tauri 写法
  （`KeyA`→`A`、`Digit1`→`1`，其余直接用 Code 名），裸字母/数字必须带修饰键，Backspace/Delete/Esc 清空。

### 网页搜索（`core/src/websearch.rs`）

自定义搜索引擎是 `WebSearch { keyword, name, url }`，`url` 必须含 `{q}`（`http`/`https`）。
默认内置 `b`（百度）、`g`（Bing）、`gh`（GitHub）、`wiki`（维基百科），设置页可增删改。

输入 `g rust 生命周期`：用第一段空白切出关键词与正文，关键词大小写不敏感等于某个引擎的
`keyword` 且正文非空才触发。单独输入 `g` 或 `g ` 不执行搜索，只补一条 850 分的
`websearch_hint` 提示行（`HitPayload::Action`，`Enter` 由宿主回一句用法）：它不短路本地结果，
分数也压在前缀匹配的应用（约 900）之下，所以只做提示、不抢位。
命中后在 `service::query` 里**短路**所有本地源，只返回一条 780 分的 `HitPayload::WebSearch`，
`Enter` 由宿主 `shell_open` 打开。本地完全无结果且 `web_search_fallback` 开启时，
追加一条 300 分的兜底结果（`web_search_engine` 指定引擎，找不到就退回列表第一项），
所以兜底永远排在真实结果之后，不会抢位。`search_files` 对网页搜索前缀同样早退，不触发 Everything。

### 系统命令（`core/src/system.rs` + `crates/app/src/system.rs`）

只在整串精确等于关键词时出结果（高分模式会误伤 `sleep timer`、`重启电脑` 这类输入）：
`sleep`/`睡眠`、`restart`/`重启`、`logout`/`logoff`/`注销`、`emptytrash`/`清空回收站`、
`screensaver`/`屏保`。分数 690，低于应用精确匹配，属于"填空位"的来源。

`HitPayload::SystemAction` 只带 action id；宿主 `system::run` 先隐藏主窗口，
破坏性动作（重启/注销/清空回收站）弹原生 `MessageBoxW` 确认，取消返回 `已取消`。
执行分别是 `SetSuspendState`、`ExitWindowsEx(EWX_REBOOT/EWX_LOGOFF | EWX_FORCEIFHUNG)`、
`SHEmptyRecycleBinW`、`WM_SYSCOMMAND + SC_SCREENSAVE` 广播；重启前先启用
`SeShutdownPrivilege`，失败与非 Windows 场景都明确报错，不静默成功。

### 搜索结果图标（`crates/app/src/icons.rs`）

`SearchHit.icon` 一律是文件系统路径（开始菜单存目标 exe、扫描器存 exe、注册表存 `DisplayIcon`、
插件存 `plugin_root + manifest.logo`），前端渲染时按需向宿主要图标：

- 前端只对**当前可见结果**调 `icon_for`，并各自缓存 `path -> data URL`（`null` 表示取不到、不再重试）；
  同一路径的重复行由 `paintIcon` 一次性替换。
- 宿主 `AppData::icon_for` 再包一层进程内缓存（上限 512，超限清空）；取不到返回 `None`，
  前端保留原有的首字母头像，不会出现破图。
- 取图两条路：插件 logo 本身是 `.png` 就直接 base64 透传（否则系统只会给一个通用图片图标）；
  其余走 `SHGetFileInfoW(SHGFI_ICON | SHGFI_LARGEICON)` → `GetIconInfo` → `GetDIBits`(32bpp，自顶向下)
  → BGRA 换 RGBA。老式图标 alpha 全 0 时用 AND mask（1bpp，`bit=1` 为透明）反推 alpha。
- 每次都 `DestroyIcon` / `DeleteObject` / `DeleteDC`，并把像素编码成 PNG data URL —— 前端拿到的
  是自包含字符串，不依赖协议与文件生命周期。

### 内联文件检索（`search/everything.rs` + `core/src/exec.rs`）
Everything 是**一等搜索源**，和 `calc`/`clock` 同一条路：结果直接进下拉框，不弹插件窗口。
**没有魔法前缀**：输入 ≥2 个字符就查（单字符会命中海量文件）。
行是 `ItemKind::File` + `HitPayload::Open`，所以走"单击选中、双击/Enter 打开"，不参与"单击复制"。
分数 160 起按相关度递减，必须低于应用条目的最低分（≈234）：文件只填空位，不抢位次。
GUI 侧走独立异步命令 `search_files`（`spawn_blocking`），晚到的文件行追加在应用结果之后；
es 侧超时 1s（`-timeout 1000`）、进程上限 1.2s，不可用时静默返回空（可用性看设置页）。

### macOS 的文件检索范围（`search/spotlight.rs`）

与 Windows 的 es.exe 通道不同，macOS 用系统自带的 `mdfind`，因此**没有"安装/校验"环节**：
`exec` 白名单里它是绝对路径条目（`/usr/bin/mdfind`，免摘要），安全性靠参数白名单。

查询由「若干 `-onlyin <绝对目录>` + `-name <查询词>`」组成（`-onlyin` 必须是绝对路径，
否则会被 mdfind 按当前目录解释，等于让调用方间接扩大范围）。目录来自
`Settings::effective_file_search_dirs()`（唯一事实源）：

1. 默认 `~/Desktop`、`~/Documents`、`~/Downloads` 中**存在**的那些；
2. 用户在设置页追加的 `settings.file_search_dirs`（必须绝对路径且存在，否则丢弃）；
3. 去重后默认目录在前、追加目录在后。

一个目录都没有时 `hits()` 直接返回空，**不退化成全盘检索**。
设置页通过 `file_search_status` / `add_file_search_dir` / `remove_file_search_dir`
三个命令读改这份配置，显示的目录与实际检索的目录必须一致。

多个 `-onlyin` 是 OR 语义且可合并成一条命令（实测比逐目录调用快 3 倍）。
结果仍要过 `is_noise()`：垃圾桶、`node_modules`、`.git`、`~/Library` 下的
Caches / Application Support / Containers 一律丢弃（这些不是"用户文件"）。

设置页在 macOS 上展示「文件检索（Spotlight）」只读状态（`file_search_status`
命令给出通道、可用性与目录列表），Everything 区块只在其它平台显示。

### 浏览器收藏夹（`core/src/bookmarks.rs`）
**默认关闭**，只有设置里打开 `enable_bookmarks` 才会被传入搜索管线。打开后读取
`%LOCALAPPDATA%` 下 Chrome / Edge 每个 Profile 的 `Bookmarks` JSON，按文件 mtime 缓存，
收藏夹变动后下一次搜索自动重建。只收录 `http`/`https` 书签（`javascript:` 等直接跳过），
行是 `ItemKind::Bookmark` + `HitPayload::Open`，回车用默认浏览器打开；同一 URL 只保留一条，
分数乘 0.8，保证同等匹配下应用 / 插件 / 内置动作排在收藏夹之前。

### 受限外部程序执行（`core/src/exec.rs`）

同一个执行器被两处使用：上面的内联源，以及插件能力 `efto.exec`（目前没有一方插件用它，
保留是因为它已被完整测试、也是后续插件的基础能力）。
它是本项目唯一能拉起外部程序的地方。它的安全边界是分层的，缺任何一层都还能被绕过，所以四层都在：
白名单名字（拒绝 `path`/`command`/`shell`/`cwd`/`env` 字段）→ 候选文件逐个 sha256 校验
（不匹配就报错，绝不降级执行）→ 参数数组直接进 CreateProcess（不经 shell）→
参数内容拦截（es 的 `-export-*`/`-set-run-count`/`-save-settings`/`-instance` 全拒，
且先归一化再比，因为 es 允许省略选项内部连字符）。
GUI 与 CLI **共用同一个 `exec_capability` 入口**：两处各写一份校验迟早漂移，
这个项目的 `EFTO_PLUGIN_DIRS` 就踩过一次"CLI 认、GUI 不认"。

### 插件脚本运行时（`core/src/exec.rs` 的 `run_script` / `probe_interpreter`）

`efto.node.run` / `efto.python.run` 与 es 共用进程收尾逻辑 `spawn_and_collect`
（stdin 关闭、stdout/stderr 各截 1MB、超时 kill、按控制台代码页解码），但安全模型不同：
解释器是用户在 **设置 → 运行时** 里配置的本机 Node/Python，不可能有固定 sha256，因此换成三条约束：

1. **路径只来自设置**：`runtime_capability` 从 `Settings::node_path` / `python_path` 取解释器，
   插件参数里出现 `path`/`interpreter`/`program`/`command`/`shell`/`cwd`/`env`/`file`
   直接拒绝 —— 否则这个能力就成了任意程序执行。路径为空时，宿主在启动（以及打开运行时设置页）
   会用 `exec::detect_on_path` 按 PATH 顺序探测 `node.exe` / `python.exe` 等固定文件名，
   首个 `--version` 通过者写回 settings.json；已配置的路径永不覆盖，用户仍可改成别的解释器。
2. **同一判据**：设置页"检测"、保存后的状态展示、调用时的失败都走 `probe_interpreter`
   （`--version`）与同一条 `resolve_interpreter`，不存在"显示可用但跑不起来"。
3. **可控成本**：默认 10s、上限 60s，`-e`/`-c` 单参数传源码不落盘，cwd 固定为插件根目录。

GUI 与 CLI 共用 `runtime_capability`，CLI 的解释器同样从 settings.json 读。

## 插件运行时（crates/jsrt）

插件进入运行时之前有两道闸门，都只影响"能不能被搜到 / 被唤起"，不影响是否列出：

| 闸门 | 依据 | 不满足时 |
| --- | --- | --- |
| 平台 | 清单 `platform`（插件级与 `cmds[].platform`） | `cli plugins` 标注"不支持当前平台"；`plugin-run` 报 `unsupported_platform` |
| 宿主版本 | 清单 `engines.efto`，只支持单条比较（`>=` `>` `<=` `<` `=`） | `cli plugins` 标注"宿主版本不匹配"并给出原因；`plugin-run` 报 `host_version_mismatch` |

两者都走 `PluginDescriptor::usable()`（启用 + 平台 + 宿主版本），`registry.get()` 与 `search()`
共用这一个判据，因此不存在"搜索不到但能唤起"或反过来的分叉；`registry.plugins()` 仍返回全量，
设置页据此展示停用原因。`engines.efto` 语法不认识时按不兼容处理（明确报错，不静默放行）。

```
spawn(plugin)
  └─ 线程 efto-plugin-<id>
       ├─ v8::Isolate::new()            1 插件 = 1 isolate
       ├─ Context + install_globals     注入 3 个原生函数
       ├─ eval(PRELUDE)                 efto API 垫片（纯 JS）
       ├─ eval(env) + eval(入口 CJS 包装)
       └─ loop { recv_timeout(最近定时器) → dispatch → perform_microtask_checkpoint }
```

三个原生函数就是全部边界：

| 函数 | 语义 |
| --- | --- |
| `__host_call(module, method, argsJson) -> resultJson` | 同步。JSON 文本进出，一次覆盖对象/数组/标量，不为每种类型写 Handle 转换 |
| `__host_call_async(id, module, method, argsJson)` | 立刻返回；流式能力可多次回投 `HostEvent::Stream{id,delta}`，最终以 `HostEvent::Reply{id,ok,result}` 收尾 |
| `__run_module_source(module, path, req) -> bool` | require：宿主读文件+转译，这里在**同一个 isolate** 里编译成 factory 并调用（因此不需要 eval/new Function） |

为什么每插件一个线程：V8 的 isolate 是单线程所有权模型，插件死循环时需要能单独 dispose/terminate；一线程一 isolate 也让"当前插件上下文"可以放 thread_local，原生回调就不需要从 isolate 里捞状态（rusty_v8 152 的 `GetIsolate` 是 `pub(crate)`，外部拿不到）。

回调是普通 `fn`（rusty_v8 用 `MapFnTo` 接受），没有捕获；当前插件从 `thread_local` 取。定时器表放在 `PluginShared`，事件循环用"最近到期时间"当 `recv_timeout`，因此不需要额外调度线程。

### 插件 UI 窗口的三条硬约束

1. **窗口创建必须在独立线程上**，不能在 `run_on_main_thread` 闭包里、也不能在同步 Tauri 命令
   的主线程上直接调 `build()`。Tauri 的 `build()` 会把创建动作投递到主线程并同步等结果；
   而 WebView2 初始化的完成回调又需要主线程跑消息循环 —— 两边互相等，就是白屏 + 整个应用卡死
   （`open_plugin_ui` 固定 spawn `plugin-ui-open` 线程）。
2. **消息有两个方向，别合成一个方法**：`ui.ipcMessage` 是 UI → main（投递给插件的
   `onMainMessage`），`ui.postMessage` 是 main → UI 页面（`plugin:message` 事件）。
   曾经主进程侧误用了前者，消息被投回自己。
3. **窗口没建好时的消息要排队**，并由页面握手触发补发：SDK 注册好 `plugin:event|listen`
   之后调 `plugin_ui_ready`，宿主才把队列发出去。`on_page_load` 太早，实测会丢。

### 插件文本文件读写（`app/src/fileapi.rs`）

插件的 main 没有 `fs`，UI 也拿不到文件字节；需要读写的插件走宿主受限 API：

- 授权集合 `FileGrants` 按插件 id 保存 canonical 路径。只有两个来源：`plugin_pick_files`
  返回的选中项，以及系统拖拽落在 `plugin-ui-*` 窗口上的路径（`lib.rs` 的
  `WindowEvent::DragDrop`）。未授权路径一律拒绝，插件不能自己拼路径读任意文件。
- `plugin_file_read` / `plugin_file_write` 只接受普通文件、UTF-8 文本，上限 8 MiB；
  读取时去 BOM，写入复用 `efto_core::fsutil::write_atomic`（临时文件 + 替换），失败明确报错。
- `plugin_file_save_as` 弹原生保存框，取消返回 `null`；保存成功后把用户选中的路径加入授权，
  因此“另存为”之后可以继续直接保存。
- UI 侧对应 `efto.file.read/write/saveAs`，接口说明见 `docs/plugin-development.md`。

### 插件能力：局域网文件共享（`plugins/file-share`，纯插件 + Node）

宿主**不再**实现文件共享：旧的 `crates/app/src/share.rs` 与 `efto.share` 能力已删除。
共享服务由插件自带的 Node 脚本 `plugins/file-share/service.js` 提供：

- 数据口默认 `0.0.0.0:5421`（0 = 随机端口），另尝试 `[::]`（`ipv6Only`），成功且存在非回环
  IPv6 地址时才返回 `urls6`；控制口固定 `127.0.0.1:5422`，令牌走查询串。
- 只暴露调用方显式给出的路径：相对片段逐段拒绝 `..` / 分隔符 / 盘符，`realpath` 后再校验前缀，
  越界一律 404；支持 `GET`/`HEAD`、单区间 `Range`、目录浏览、RFC 5987 中文文件名。
- main.js（内嵌 V8）只做编排：用 `efto.node.run` bootstrap 拉起 detached 的 `service.js`，
  再通过控制口执行 `start/stop/status`；UI 协议与 kv 结构不变。
- 生命周期：UI 关闭即 `stop`；数据口在跑时 main 每 45s heartbeat，服务超过 120s
  没有控制请求就自动停止共享（应用被强杀时不会留下对外服务）；无数据口 30 分钟后进程自退。
- 因此该插件**依赖本机 Node**：设置 → 运行时 未配置/检测失败时不可用。

### 插件能力：Windows hosts 写入（`core/src/hosts.rs` + `app/src/hosts.rs`）

`efto.hosts.read/save` 分成两层：`core` 只做纯逻辑（解码、逐行校验、按原编码写回），
`app` 负责提权与落盘。这样校验规则可以脱离 Windows/UAC 单测，GUI 与 CLI 共用同一条路径。

- **编码**：hosts 不是 UTF-8 文件。先按 UTF-8 解码，失败回退 `GetACP()`（zh-CN 为 GBK）；
  保存时沿用读取时的编码，`WideCharToMultiByte` 检测到不可表示字符就报错，不写 `?`。
- **校验**：启用行必须是 `IP 主机名...`；`#` 后紧跟合法 IP 的行按禁用条目校验；
  错误带行号、一次最多列 20 条。
- **提权**：非提权侧把内容写进 `~/.efto/uac/request-*.json`，用
  `ShellExecuteExW(verb="runas")` 拉起 `efto.exe hosts-apply <request>` 并
  `WaitForSingleObject`（120s）。提权侧重新校验 → 备份 `hosts.efto.bak` →
  临时文件 + `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` 原子替换 → 写 `<request>.resp.json`。
- **边界**：提权侧只接受数据目录 `uac/` 下的请求，响应路径由请求路径推导，目标固定为系统
  hosts —— 请求 JSON 里没有"写哪里"的字段。`save` 在 `is_async_module` 里，走能力线程池，
  不会卡住插件 isolate；UAC 取消（1223）立即返回明确错误，不靠超时判断。
- 兼容性说明：如果 UAC 提权到的是**另一个管理员账户**，`%APPDATA%` 会不同，提权侧会拒绝请求。
  普通个人机器（当前用户即管理员）不受影响。

### 线程与锁约束

- 主线程（WebView2 消息循环）：所有窗口创建/显示/隐藏，非主线程一律 `run_on_main_thread` 投递
- `efto-plugin-<id>`：插件 isolate
- `efto-pump`（60ms）：消费插件事件；每 32 轮落盘一次脏 KV
- `efto-refresher`：按 `refresh_interval_secs` 重建索引，连续 3 次结果为空就自动停手
- `efto-share`：文件共享的 accept 循环（80ms 轮询）；每个下载连接一个短线程
- 能力线程池（2..8）：IO 能力，跑完通过插件自己的 inbound 通道回投 Reply；AI 流式调用先回投若干 Stream

**死锁纪律**：宿主能力绝不同步等待插件线程；插件线程绝不等主线程（UI 类能力在 JS 侧就是 Promise）。锁顺序约定 `settings → registry → index`。

## TS 支持为什么"只做类型剥离"

`deno_ast`（内部是 SWC）的 `typescript::strip_type()` 是纯 AST 访问器：删掉注解、`interface`、`type`、类型导入、泛型实参。它**不生成运行时代码**，所以 `enum`、带值的 `namespace`、装饰器、参数属性无法正确处理。

选择：与其产出一个语义错误的 JS（`enum` 原样输出 → V8 语法错误，或更糟地静默行为不一致），不如在转译阶段扫出这些结构并明确报错，提示改用 `const` 对象或预编译产物。这个拒绝逻辑有单测覆盖（`rejects_enum_with_actionable_message`）。

另一个坑：`deno_ast` 的 `transpiling` feature 与 `swc_common` 对 `swc_visit` 的精确版本要求会冲突，需要删掉 `Cargo.lock` 重新生成一次解析。

## 前端为什么不用 npm

这台机器 `registry.npmjs.org` 不可达。Tauri 2 会**无条件**向每个 webview 注入 `window.__TAURI_INTERNALS__`（`src/manager/webview.rs`），里面就有 `invoke` / `transformCallback`，所以手写 30 行 JS 就能替代 `@tauri-apps/api`。

自己按线协议拼 HTTP 请求是走不通的：`Tauri-Invoke-Key` 是运行时随机值且 JS 侧拿不到，请求会被静默丢弃。数据面（插件静态资源）因此走自定义协议 `efto://`；控制面走注入的 `invoke`。

注意 `windows` 上 scheme 的实际形态：页面必须请求 `http://efto.localhost/<plugin>/index.html`，WebView2 不支持任意 scheme，wry 做映射；Rust handler 收到的是被还原后的 `efto://localhost/...`，`host()==localhost`、`path()=="/<plugin>/index.html"`。

## 多语言（i18n）

- 词典只有一份：`frontend/locales/zh-CN.json` / `en-US.json`，扁平 key（`区域.组件.动作`）。
  前端 `i18n.js` 用 `fetch` 加载；`crates/app/src/i18n.rs` 用 `include_str!` 嵌入同一份文件，
  托盘、窗口标题和页面共用，避免两套翻译漂移。
- 语言解析：`settings.language` 为 `auto` 时先看 `LC_ALL`/`LC_MESSAGES`/`LANG`，
  Windows 上再问 `GetUserDefaultUILanguage`，非中文回退 `en-US`；只在启动与切换设置时解析。
- 页面用 `data-i18n` / `data-i18n-html` / `data-i18n-placeholder|title|aria-label`，
  脚本用 `window.eftoI18n.t/tf`；缺失 key 回退中文并记日志，不回显 key。
- 菜单文本无法原地修改，语言变更后重建托盘菜单；设置/剪贴板/帮助窗口的原生标题同步改，
  主窗口收到 `settings:language` 后整页重载。
- `scripts/check-i18n.js` 接入 `verify.ps1`：key/占位符一致、HTML/JS 引用存在、
  HTML 未走词典的中文报行号。

## 第一阶段没做的（有意为之）

安装器/自启动/自动更新、UWP 与 `App Paths` 索引、目录变更监听（现在是定时重建）、`over` 拖拽注入、请求取消、V8 `terminate_execution` 超时看门狗、权限模型（任何插件都能读写自己的 db 并启动进程）。详见 `docs/roadmap.md`。
