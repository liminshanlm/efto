# 验证记录（2026-11-16，本机 Windows 10.0.19044 / Rust 1.100）

这份文件记录**实际跑过并看到输出**的内容，以及没跑过的部分。目的是让"能编译"和"能工作"不被混为一谈。

## 单元测试：203 通过 / 0 失败（编译告警 0 条）

```
efto-core  123 passed   efto-jsrt  45 passed   efto-app  35 passed
```

覆盖重点：

| 模块 | 关键断言 |
| --- | --- |
| `apps::lnk` | MS-SHLLINK 结构解析 + **真机 204 个快捷方式回归**（见下） |
| `apps::startmenu` | `.lnk`/`.url` 收集、`Startup` 目录跳过、忽略规则 |
| `apps::registry` | 卸载项噪音过滤（VC 运行库/更新包）、`DisplayIcon,索引` 剥离、主 exe 猜测 |
| `apps::scanner` | helper exe 过滤（unins/crashpad）、体积阈值、根目录不被黑名单误杀 |
| `apps::indexer` | 同 exe 去重优先级、快捷方式与其目标合并、缓存版本失效与 roundtrip |
| `search::*` | 拼音全拼/首字母、紧凑匹配、子序列、频次与最近使用加权、空查询默认面板 |
| `plugins::*` | `efto-plugin.json` 解析、`main.js` 缺失回退 `main.ts`、路径越界拒绝、CJK 触发词前缀命中、文件夹/zip 导入（含路径逃逸拒绝、一个包多个插件）、非 zip 包明确拒绝、`efto.output` 行校验、停用插件保留在列表但 `get`/`search` 跳过 |
| `config::*` | 设置 roundtrip、旧配置缺字段回落默认、`disabled_plugins` 缺省为空且可解析 |
| `search::calc` | 优先级与右结合幂、一元负号（`-2^2 = -4`）、函数/常量/单位换算、千分位与全角、除零与未定义放弃、**不劫持应用名**、深度/长度守卫不 panic、千分位分组 1..7 位全枚举 |
| `search::timestamp` | 17 个用例：已知 epoch 常量、四种分隔符与 `T` 连接、中文单位与 12 小时制（下午/上午/中午/凌晨/半）、两段式授权、显式时区优先、小数秒只影响毫秒行、非法历法值（`2026-02-30`/`24:00`/`12:60`）、日关键字偏移、**26 个绝不能出时间行的输入**、歧义 `11/12/2026` 拒绝、两行结构与"所见即所复制"、全角数字、星期名、5000 条随机串不 panic |
| `search::clock` | 6 个用例（含 service 层 1 个）：八种格式逐字断言（与机器时区无关的四条写死，带偏移的两条断言形状 + 本机偏移尾部）、**同一屏内所有行必须指向同一瞬间**、触发词整串相等（`update`/`dated`/`datetime`/`date 2026-11-16`/`时间戳`/`现在几点` 一律不触发）、复制内容=显示内容、格式行分数高于 epoch 行 |
| `search::win_settings` | 6 个用例（含 service 层 1 个）：`ms-settings:` 列出常用页、完整 URI 精确命中、前缀过滤、未知页面仍给"打开"行、**普通输入与无冒号的 `ms-settings` 不触发**、service 层不混入其它源 |
| `store` / `usage` | 原子写、配额、快速点击去重计数、90 天淘汰 |
| `jsrt::ts` | 类型剥离、`import type`/泛型删除、`enum`/`namespace` 拒绝文案、语法错误上报 |
| `jsrt::modules` | require 解析顺序、`../` 越界拒绝、JSON 包装、ESM 拒绝提示 |
| `jsrt::engine`/`runtime` | **真实 V8**：TS 入口端到端、require 嵌套模块、能力调用、定时器钳制、语法错误转事件 |
| `app` | 参数协议、路径沙箱、协议 MIME/越界、label 反查插件 id、状态层索引/使用回写 |

## `.lnk` 解析的真机度量

`parses_real_start_menu_links` 在本机开始菜单上统计（样本 204）：

```
产出 target 142 条，其中结构化 LinkInfo 解析 141 条、启发式 1 条
解析出的 target 实际存在于磁盘：134 条（精确率 0.94）
```

两个曾经的实际缺陷就是被这个测试抓出来的：

1. **IDList 长度字段语义**：文档读起来像"含自身 2 字节"，真机数据（`idlistSize` 与 LinkInfo 实际偏移的差值 141 个样本全部为 `+2`）证明**不含自身**。按文档算会让 LinkInfo 整体偏移错位，结构化解析 100% 失败、只剩启发式兜底（还原率 47%→58%）。
2. **可选块 flags**：曾按 `0x10 = ForceNoLinkInfo` 门控 LinkInfo，真机上该位普遍置位，导致结构化解析被整体跳过（结构化命中 30→141）。现在不依赖任何可选块标志位，改用块头特征 + 内容自证。
3. 附带发现：`StringData` 尽力读会把 RelativePath/DOTS 串当成显示名（`@`、`E`、`..\..\x.exe`），因此加了显示名合理性校验，不合格就退回文件名。

独立交叉验证：用 Node 写了两个探针脚本直接读 `.lnk` 字节（不经过 Rust），得到同样的 141 个结构化命中与同样的 `+2` 偏移规律——排除了"Rust 代码自己验证自己"的可能。

## 前端守卫（静态检查）

GUI 交互没法在本机自动化验证（RayLink 会话抢焦点、截图取不到宿主桌面），所以加了一道静态守卫
`scripts/check-frontend.js`，接进 `verify.ps1`：检查"被 JS 的 `.hidden` 切换、但 CSS 里设过 `display` 的元素"
是否配套了 `[hidden] { display: none }` 规则。

它的由来是一个真实 bug：`.empty { position:absolute; inset:0; display:flex }` 的作者样式**覆盖了 UA 样式表里
`[hidden] { display:none }`**，于是 `empty.hidden = true` 完全无效——空状态层一直铺在结果列表上，
吃掉了每一行的 `click` / `dblclick` / `mousemove`。表现就是用户报的两条现象：
"单击双击没反应"，以及"滚动条在最顶端时 hover 不选中、下滚之后才选中"（下滚后遮罩随内容上移，
下面的行才重新可命中）。一个根因解释两个现象。

修复：`.empty[hidden] { display:none }` 补回优先级 + `.empty { pointer-events:none }` 兜底
（纯信息层不该拦截输入）。守卫的有效性是**反向验证**过的：临时撤掉 `[hidden]` 规则后脚本报错退出码 1，
恢复后通过——避免写一个抓不到目标的自检。

第二版又加了一条跨语言检查：**Rust 侧凡是返回 `HitPayload::CopyText` 的 `ItemKind`，
前端 `isCopy` 必须把它算进"单击即复制"**，否则新加一行可复制结果时会静默退化成"要点两下"。
它同样做了注入验证，而且**第一次验证就暴露出守卫自己太松**：我原本写成
"app.js 里出现过 `hit.kind === 'time'` 就算通过"，把 `isCopy` 那行删掉后守卫仍然通过——
因为 tooltip 的三元表达式里也有同样的字面量。改成只检查 `var isCopy = ...;` 这条赋值之后，
注入即报 `PROBLEM: time 行的 payload 是 CopyText，但 isCopy 没把它算进单击即复制`，恢复即通过。
（当前输出：`3 个 .hidden 控制的元素，2 类单击即复制的行：calc/time`。）

## 内置计算器实测（release 二进制）

```
6*7              -> 42                 sqrt(16)+2^3   -> 12
1,000+1          -> 1,001              -2^2           -> -4
gb(2)            -> 2,147,483,648      123456*789     -> 97,406,784
1+1=             -> 2                  1/0            -> 无计算行（回落应用搜索）
(5)              -> 无计算行            1700000000     -> 无计算行（纯数字）
chrome / notepad / 3.10 64-bit         -> 仍走应用搜索，未被劫持
```

开发过程中被测试抓到并修掉的真实缺陷：

1. `group_thousands` 位数少于 3 时 `i - lead` 发生 **usize 减法下溢 panic**（输入 `12` 就崩）；
2. 一元负号与幂的优先级写错（`-2^2` 得 4 而不是 -4）；
3. 深度守卫的测试本身写错（50 层括号其实没超上限）——改成"上限内必须算得出、超上限必须放弃"两侧都断言；
4. CLI 把以 `-` 开头的参数一律当 flag 丢弃，导致 `cli search -2^2` 报"需要关键词"；改为只识别已知 flag；
5. `impl Trait` 不能用在闭包参数上（`one` 辅助函数），改成泛型函数；
6. 全角/结尾等号：`" 1 + 1 = "` 因尾部空格被拒——归一化阶段先 trim。

## 时间 → 时间戳（release 二进制实测）

```
15:03                 -> 1794812580 / 1794812580000（只有时间 = 今天）
2026-11-16 15:03:22   -> 1794812602 / 1794812602000（秒级 / 毫秒级两行）
昨天 18:00              -> 1794736800（2026-11-15 18:00:00 周日）
2026年11月16日15时03分22秒 -> 同上（中文单位等价）
2026-11-16 下午3点     -> 1794812400 / ...400000（12 小时制）
2026-11-16 上午10点半  -> 1794796200（"半" = 30 分）
现在                  -> 1794816770 / 1794816770006（毫秒行是真子秒）
2026-11-16T15:03:22Z  -> 1794841402（显式 UTC，比本机时区行整整差 8 小时）
明天 15:00 / 今天 / 昨天 -> 按本机日历偏移
1700000000 / chrome / 2026 / 3.10 64-bit / 11/12/2026 / 半天 -> 无时间行
11-16                 -> 仍归计算器，得 -5（两段式没被时间解析器抢走）
```

### 为什么还要一个 Node 探针

时区/历法最阴的地方是**错了也会算出一个看起来很合理的数字**。单元测试里的期望值是照着实现的心理模型写的，
真算错时两边会一起错。所以 `scripts/check-time-probe.js`（已接进 `verify.ps1`）不信任 Rust 的算术：

- 期望 epoch 由 Node 的 `Date(y, m-1, d, h, mi, s, ms)` 现算，走的是**操作系统时区库**，与 chrono 是两条独立实现；
- 反向也要一致：拿 Rust 输出的 epoch 用 `new Date(ms)` 还原墙上时间与星期，必须和下拉框副标题逐字相同；
- 显式时区组用 `Date.parse('...Z'/'...+08:00')` 直接比对；
- "现在"与 `Date.now()` 比绝对误差（<5s）；
- 外加 18 个"绝不能出时间行"的输入，以及计算器边界（`2026-11-16` 不许出 1,999、`11-16` 必须仍出 -5）。

**探针的有效性同样是反向验证的**（两次人为注入缺陷，都要求它必须失败）：

| 注入的缺陷 | 探针反应 |
| --- | --- |
| 注释掉 `parts.meridiem = flags.meridiem`（12 小时制失效） | 3 条 FAIL，全部差 `±43200000` ms = 整 12 小时 |
| 让 `calc` 的让位闸门失效 | 1 条 FAIL：`"2026-11-16" 仍被当减法算出 1,999` |

恢复后 `TIME PROBE: ALL CHECKS PASSED`，退出码 0。

### 开发过程中抓到的真实缺陷（标注每一条是被什么抓到的）

1. **任意中文查询会 panic**（最严重的一个，**单元测试抓到**）：`parse_offset` 为了取符号位直接 `split_at(1)`，
   而 `is_tz_token` 对**每个** token 都会调用它。`微信` 的第一个字符占 3 字节，切片落在字符中间
   → `str::mod.rs` panic。release 用 `panic = "abort"`，表现就是搜索中文时进程直接没了。
   由 `search::service::tests::chinese_and_initials` 抓到（不是我新写的用例抓到，是既有测试被连带打挂）。
   修法是符号判定前置。**这类越界不会由 `unwrap` 引发，编译器也不报，只有真跑中文输入才暴露。**
2. `下午3点` 静默算成凌晨 3 点（**单元测试抓到**）：`strip_meridiem` 把时段词吃掉后忘了把 `flags.meridiem` 传回 `Parts`，
   差 12 小时且没有任何报错——正是"看起来合理但错了"的典型。
3. `明天` 解析成"今年 1 月 1 日"（**提交前的自查/代码走读抓到**，随后补了 `day_words` 用例锁住）：
   `finish` 里 month/day 用 `unwrap_or(1)` 兜底，没有跟着日关键字的 `base` 走。
4. 全角 `２０２６－１１－１６` 不认：`normalize` 只映射了全角数字，漏了全角 `－／＋`。
5. `2026-11-16T15:03:22Z` 的 `Z` 被丢掉（**提交前的自查抓到**）：`T` 连接分支只解析时间不解析时区后缀，
   改成先把 token 流拆开，让时间分支统一处理时区。
6. 我自己写测试时把 `1700000000` 当成 "2023-11-15 06:13:20 UTC"（**写用例时拿 Node 复核常量才发现**），实际它是**北京时间** 06:13:20
   （UTC 是 11-14 22:13:20）。如果只信自己，测试会把时区错误固化成"期望值"。
7. **纯时间落到 1 月 1 日**（**release 二进制人工过样抓到**）：`15:03` / `下午3点` 一度解析成"今年 01-01 15:03"。原因是 `finish` 里
   month/day 的缺省值只看了 `day_shift`（今天/明天那类关键字），没覆盖"完全没给日期"的情况。
   **这条不是单元测试发现的，是文档写完、拿 release 二进制手动过一遍时才看到的**——
   用例表里恰好缺"只有时间"这一类，而 README 已经先把它写成支持了（claim 跑在证据前面）。
   修法是把缺省规则改成"没解析到任何日期分量就用 base（今天或关键字指定的那天）"，
   同时给探针补上 `15:03`/`08:30`/`23:59:59`/`下午3点` 四条。
   补完后先拿**修复前的 release 二进制**跑新用例，确认它报 `差 -7656 小时`（≈317 天，正好是 1 月 1 日与 11 月 16 日的间距），再拿修复后的跑才通过——
   新用例的有效性同样是反向验证过的。

顺带一个流程教训：`cargo test --workspace` **不会**刷新 `target/debug/efto.exe`，
所以改完 Rust 直接跑探针会看到"修了但没修好"的假象（我就吃了一次）。探针之前必须先 `cargo build -p efto-app`，
`verify.ps1` 开头就是这么做的。

## 迁移与验证

| 动作 | 证据 |
| --- | --- |
| 目录改名 | robocopy 排除 `target/`，新旧两棵树 **89 个文件 sha256 全等**（missing/extra/mismatch 均为 0） |
| 数据迁移 | `%APPDATA%` 旧目录 → `%APPDATA%\efto`，3 个文件（app-index.json 403467B / usage.json 345B / last-copy.txt 23B）**字节全等**；实测新目录 mtime 17:12 在写、旧目录 17:00 之后没再动 → 活的确实是新目录 |
| 产物身份 | `MainWindowTitle` 变为 **`app.efto.launcher-siw`**（OS 级可见，证明 identifier 真的换了）；日志 `efto 0.1.0 启动`、`载入索引缓存 979 条`、`发现 2 个插件`、`全局热键已绑定: Alt+Space`；`target/release/efto.exe` 55,606,784 字节 |
| 回归 | `cargo test --workspace` **174 通过 / 0 失败**（core 96 + jsrt 45 + app 33），`verify.ps1` ALL CHECKS PASSED（含时间探针 35 条） |

### 顺手补掉的一个自埋坑

我给 `vsenv.ps1` 加了**中文注释**——正是本项目已经记录过的 PowerShell 5.1 ANSI 解析陷阱
（非 ASCII 字节会吞掉换行、把后面的赋值并进注释，产生极难定位的静默故障）。发现后改成英文，
并把 verify.ps1 的 ASCII 自检从"只查自己"扩成**逐字节审计 scripts/ 下所有 .ps1**
（原来只查自身留了洞：dot-source 的 vsenv.ps1 被写坏时，verify.ps1 自己照样通过）。
审计结果：5 个 .ps1 全部 0 个非 ASCII 字节。

## 忽略规则修复（大小写 + 一条死规则）

起因是问"`**/ assemblies/**` 是干什么的"。答案是：什么都不干（`assemblies` 前多一个空格，
只能匹配名字真以空格开头的目录）。但查它的过程挖出一个更大的：**忽略规则里的 glob 全是
大小写敏感的，而 `is_ignored` 先把路径 `to_lowercase()`** —— 所有含大写字母的规则静默失效。

实测（单条规则 vs 小写化路径，探针跑完即删）：

| 规则 | 样本路径 | 修复前 |
| --- | --- | --- |
| `**/Temp/**` | `C:\Temp\a.exe` | **不命中** |
| `**/temp/**`（对照） | 同上 | 命中 |
| `**/WinSxS/**` | `C:\Windows\WinSxS\...\a.exe` | **不命中** |
| `**/WindowsApps/**` | `C:\Program Files\WindowsApps\a.exe` | **不命中**（只靠 `is_ignored` 末尾硬编码字符串兜底才没漏） |
| `**/$Recycle.Bin/**` | `C:\$Recycle.Bin\S-1-5\a.exe` | **不命中** |
| `**/assembly/**` | GAC 路径 | 命中（全小写，运气好） |
| `**/node_modules/**` | `C:\x\node_modules\y\a.exe` | 命中 |

即：默认 7 条里 3 条全死、1 条靠兜底；固定列表 6 条里同样 3 条死。用户最自然的写法
（`**/Steam/**`、`**/Adobe/**`）过去完全不生效，而且不报错。

**影响面先量过再改**：解析 `%APPDATA%\efto\app-index.json` 的 979 条路径，
含 `temp`/`tmp`/`winsxs`/`recycle`/`windowsapps`/`assembly`/`assemblies`/`~1`/`node_modules`/`.git`
任一整段目录名的 **0 条** —— 因为扫描根只有 Program Files 那几个 + `MAX_DEPTH=4`，
目录扫描另有按名字剪枝的黑名单。修复后用新二进制重建索引：**979 条，
原始 1512 / 合并去重 533 / 开始菜单 196 / 注册表 57 / 目录 1245 / 系统 14，逐项不变** ✓
（先量再改，是为了不把"修好规则"变成"悄悄少了一批应用"。）

改动三处：`compile_glob()` 打开 `case_insensitive`；默认列表里 `"**/ assemblies/**"`
改成 `"**/assemblies/**"`（与 scanner 的目录名黑名单一致）；把 `**/Temp/**` 从
**不可移除的固定列表**移到**可编辑的默认设置**——固定列表只该放"永远不可能是程序位置"
的系统内部目录，Temp 不是。这个决定是被测试逼出来的：改完 `collects_url_shortcut` 和
`helper_exe_filtered` 立刻挂，因为 `tempfile` 的夹具就在 `%TEMP%` 里，规则终于活了。

新增三个测试（`startmenu.rs`）：`ignore_rules_are_case_insensitive`、
`every_default_ignore_rule_matches_something`（逐条断言每条默认规则都能命中真实形状路径，
专治"死规则"这一类）、`defaults_do_not_kill_normal_app_paths`（反向：`Tempero`、
`Assembly Studio`、`SteamWorks` 不许被吃掉，证明规则是整段匹配不是子串）。

**注入验证**（`python scripts/prove-ignore-tests.py`）：退回区分大小写 → 2 个测试失败；
把空格 typo 塞回去 → 1 个测试失败；还原 → exit 0。

### 这个 proof 脚本本身先是假的

第一版三个阶段全部"通过"，其实是**一枪没开**：脚本用 `subprocess.run` 但没设 `cwd`，
cargo 在 `E:\qianwen` 找不到 `Cargo.toml` 也以 **101** 退出，而脚本判定"缺陷被抓到"的条件
正是"退出码非 0"。所以两次注入的 OK 和还原后的 BAD 都跟代码无关。
修法是 `cwd=ROOT`，并且**先识别"cargo 根本没跑起来"这种输出并直接抛错**——
否则任何"用退出码当判据"的脚本都可能这样自我表扬。这是本项目第三次遇到
"守卫/自检太松，注入缺陷照样通过"（前两次：白名单按文件放行、tooltip 字面量冒充交互判定）。

## 系统时间多格式（release 二进制实测）

输入 `date`（本机真有一个 `date.exe`，它作为精确匹配仍排第一，格式行紧随其后）：

```
 1. Date                          [exact] date.exe  (dir_scan)      <- 应用没被挤掉
 2. 2026-11-16 18:59:55           ISO 本地 · UTC+08:00
 3. 2026/11/16 18:59              Windows 中文 · UTC+08:00
 4. 2026-11-16                    仅日期
 5. 18:59:55                      仅时间
 6. 2026-11-16T18:59:55+08:00     RFC3339 带时区
 7. Mon, 16 Nov 2026 18:59:55 +0800  RFC1123 / HTTP
 8. 2026年11月16日 18时59分55秒 周一    中文
 9. 20261116_185955               文件名安全
```

`now` / `现在` / `今天` 在此基础上再多两行 epoch（秒级 + 毫秒级），共 10 行。

交叉校验加在 `scripts/check-time-probe.js` 里，思路是**不拿 Rust 的表去验 Rust 的表**：

- 瞬间从我们自己的 ISO 行反解（`new Date("2026-11-16T18:59:55")` 按本地解释），
  再用 Node 逐条拼出八种期望串比对 —— 这样比较的是"同一屏内部是否自洽"，
  不受两次取时刻的跨秒抖动影响，也不会因为机器时钟走到下一秒而偶发失败；
- 另外单独断言 epoch 行与格式行相差 ≤2 秒；
- **星期名/月份名用 ICU 的 `toLocaleDateString('en-US'|'zh-CN', {weekday|month:'short'})` 复核**，
  而不是照抄代码里那张 `WEEKDAYS_EN` / `MONTHS_EN` 表；
- 再断言 `update`/`dated`/`datetime`/`date 2026-11-16`/`runtime`/`时间戳`/`timezone`/
  `noweb`/`update time`/`chrome`/`今天 15:00` 一律不出格式行。

注入验证：把 RFC1123 的星期索引从 `num_days_from_sunday()` 改成 `num_days_from_monday()`
（差一天的典型写法错误），两层都抓到——探针报
`格式 5 不一致：我们 "Sun, 16 Nov 2026 18:55:50 +0800" vs Node "Mon, ..."` 和
`RFC1123 星期与 ICU 不一致`，单元测试 `every_format_renders_exactly` FAILED；还原后全绿。

### 做这个功能时踩到的三个自己的坑

1. **注入缺陷的脚本自己写坏了**：第一次给 `format!` 的参数列表里插了行 `// MUTATED` 注释，
   注释吃掉了同一行剩下的内容（包括那个逗号）→ 编译失败 exit 101，
   而探针拿的是**磁盘上没重编成功的旧 exe**，于是报"ALL CHECKS PASSED"。
   这是本项目第三次遇到"验证通道其实是空的"。修法是每次注入后**先看 build 的退出码，
   再跑探针**；build 失败时探针结果一律不作数。
2. **前端守卫的启发式太粗**：它按"文件里出现过 `HitPayload::CopyText {`"就把该文件所有
   `ItemKind` 都当成复制行；我在新测试里写了一句 `match ... HitPayload::CopyText {`，
   于是 App/Action/External 被误判，报出三个假问题。改成只认
   `SearchHit::new(ItemKind::X, ... HitPayload::CopyText {` 这种**成对的构造点**后，
   基线恢复通过、注入仍然抓到。局限照实写在注释里：这是文本启发式，不是类型级保证。
3. **探针的行筛选太粗**：格式行与 epoch 行共用 `ItemKind::Time`（都要"单击即复制"），
   原来按 `kind === 'time'` 筛会把两种行混在一起，`今天`/`现在` 的"期望 2 行"断言
   当场变成"实际 10 行"。改成按 `source` 区分（`time` vs `clock`）。

## Everything 文件检索（内联在下拉框，无魔法前缀）

**这个功能返工过一次**：第一版按"真插件"实现（cmd `es` + 插件自己的 UI 窗口），实测能跑
（`es 报告` 30 条 / 201ms），但用户要的是**结果合并进下拉框**，所以撤掉插件、改成内置搜索源。
执行器与解析没有浪费：从 `app/src/exec.rs` 下沉到 `core/src/exec.rs`，内联源与插件能力
`efto.exec` 共用同一套约束；插件的 `index.html` 与那个 Node 逻辑测试一并删掉
（插件 UI 这条路径本来就没在真实 webview 里验证过）。

### 前置事实（都是先查再做的）

| 项 | 实测 |
| --- | --- |
| Everything | 已装 `C:\Program Files (x86)\Everything\Everything.exe`，服务 **Running**，两个进程在跑 |
| es.exe | **原本不存在**（Everything 安装包不含它），`es` 也不在 PATH |
| 获取 | `https://www.voidtools.com/es.zip` 可达（`es.exe` 直链超时不通）；zip sha256 `9412f3b0…beb7b3`，解出的 exe sha256 `224550ed…c0d12`（1.1.0.18），vendor 进 `tools/es/` + `manifest.json` |
| 输出编码 | **GBK 不是 UTF-8**：中文路径原始字节 `0xD7 0xC9 0xD1 0xAF`。经"解码→写 UTF-8 文件→读回"验证解出 `咨询交付\skills\写报告` ✓ |
| 参数坑 | `-count` 不存在（`Error 6: Unknown switch`，正确是 `-n`）；目录的**大小列是空**，不能当 0 B 显示 |
| 噪音 | `ext:exe` 默认第一屏前三条全是 `$RECYCLE.BIN\$I*.exe` |

### 实测（debug / release 二进制）

```
$ efto cli search "报告"
 1. 写报告       [substring] E:\...\咨询交付\skills · 2026-10-23 17:20  (everything)
 2. 客户报告     [substring] E:\...\财富管理\skills · 2026-10-23 17:20  (everything)
 3. 月度运行报告_测试 [substring] E:\dsh\output · 2026-10-08 11:42  (everything)
 ...
```

- **不再需要 `es ` 前缀**：输入 ≥2 个字符就查（单字符会命中海量文件）。
- **延迟**：单次 es 160–300ms。GUI 不再同步等它：`search` 只出应用/插件/内置源，
  文件行由异步命令 `search_files`（`spawn_blocking`）在 180ms 防抖后追加。
  es 侧超时 1s（`-timeout 1000`），进程上限 1.2s。
- **不抢位次**：文件行 160 起递减，低于应用条目最低分（≈234），只填空位；
  `--exact` 不带文件行；`--kind file` 只回文件（应用条目被过滤后文件填满 limit），
  普通查询里文件只填空位，绝不挤掉已有结果。
- **噪音**：`ext:exe` 加过滤后第一行是 `C:\Users\DELL\Downloads\*.exe` ✓
- **0 结果对齐**（防止"结果被吞了"）：`ext:pdf 报告` 返回 0 条，
  单独跑 es 原生**也是 0 条** ✓
- **不可用静默**：找不到 es.exe / Everything 没跑时返回空列表，不再给提示行；
  可用性由设置页的 Everything 状态负责。
- **交互**：行是 `ItemKind::File` + `HitPayload::Open` → 走"单击选中、双击/Enter 打开"，
  不进 `isCopy`（那条判定只属于复制类行）。

### 测试：`search::everything` 7 个

短输入不查（空串 / 单字符 / 单汉字）、CSV 引号与空大小列、噪音过滤双向（该滤的滤、`Tempero` 不误伤）、
展示格式化（B/KB/MB、ISO→`YYYY-MM-DD HH:MM`）、行形状（File + Open + 副标题三要素）、
**分数压在应用之下**（<234 且逐行递减）、**真实索引端到端**（Everything 不可用时打印原因跳过，不静默通过）。

`exec` 的 12 个测试随文件搬到 core 后全部保留（白名单拒绝、摘要锁定、真实 es.zip 解包、
GBK 解码、超时 kill、参数含 shell 元字符不被解释、写能力开关被拒、`-n` 上限…）。

### 注入验证（每次都要求"必须失败"）

| 注入的缺陷 | 结果 |
| --- | --- |
| `lookup()` 改成"白名单外也返回条目" | `only_allowlisted_names_run` FAILED：`"cmd" 应被白名单拒绝，实际 执行超时` |
| 参数归一化退回"只去前导符号、不去内部连字符" | `write_capable_switches_are_refused` FAILED：`-set-run-count 应被拒绝` |
| 撤掉内联源的回收站过滤 | `ext:exe` 的输出里出现 `RECYCLE` → `verify.ps1` 的断言会失败 |

### 这一轮我自己写错的地方

1. **形态判断错了**：我给了"内置源 / 真插件"两个选项，却没意识到在这个项目里"插件"必然意味着
   **独立窗口**，而用户要的是"下拉框内联"。教训：这类"结果放哪"的选择，应该先描述两种形态的
   **最终画面**再让人选，而不是只给两个词。
2. **又一次给 `.ps1` 写了中文注释**（`fetch-es.ps1`）—— 被上一轮加的"逐字节审计所有 .ps1"当场拦下。
3. **CSV 引号夹具写错**：样本里给了单个 `"`，而 CSV 规则是 `""`。解析器是对的，改测试。
4. **第一次"撤掉过滤"的注入没生效**：python 转义字符串没匹配上、assert 提前抛出，
   于是 `grep -c RECYCLE` 得到的 0 是**基线**而不是结论。改成按行匹配后才算验证。
5. **`exec.rs` 里留了两段装饰性代码**（恒假的 env 循环、假的"Mark of the Web"），后者尤其糟：
   看着像安全措施，实际什么都没做。删掉。

### 没验证到的

- **界面渲染与鼠标交互**：本机锁屏 + RayLink 会话，截图取不到宿主桌面。CLI 能证明"行生成、
  排序、副标题、来源正确"，但"列表长什么样、双击是否真打开"要你点一下。
- **Everything 没在跑时的表现**：停服务需要管理员权限，没做；代码路径有测试覆盖
  （非零退出码 / 找不到 es.exe 都返回空结果，不报错），真机没演练。
- **运行时自动下载**（`exec::fetch_and_install`）：zip 解析器用真实 es.zip 测过 ✓，
  但整条下载分支在本机走不到（有 vendor 副本）。实际验证的是 `scripts/fetch-es.ps1 -Force`。

## 插件 UI：白屏 + 卡死 + 消息不通（一处报障，查出六处缺陷）

用户报：打开 hello-js 插件页面后**窗口白屏、关不掉**。桌面解锁后用 computer_use 驱动
（点搜索框 → 输入 `hello-js` → Enter）1:1 复现：插件窗口有标题栏但整片白，
主搜索框卡在"执行中…"，连窗口的 X 都点不动。

### 六处根因（都用日志定位，不是猜的）

1. **建窗口的线程错了（死锁）**。
   原代码在 `run_on_main_thread` 的闭包里调 `build()` —— 闭包本身就在主线程，
   而 Tauri 的 `build()` 内部会再把创建投递到主线程并**同步等结果** → 自锁。
   去掉包装后**仍然卡**：加日志打出调用线程名，结果是 `"main"` —— 我们的 `invoke_hit`
   是同步 Tauri 命令，跑在主线程；而 WebView2 初始化是异步的，完成回调要靠主线程消息循环，
   主线程正阻塞在 `build()` 里等它。
   **修复**：`open_plugin_ui` 固定 spawn 一个 `plugin-ui-open` 线程去 build，调用方立即返回。
2. **主进程 → UI 的消息发错了方向**。prelude 里主进程侧的 `efto.ipcMessage` 调的是
   `ui.ipcMessage`，而那个方法实现的是 **UI → main**，等于把消息投回自己。
   新增 `ui.postMessage`（host → UI 页面），CLI 侧加同名回显便于断言。
3. **UI 订阅事件失败且完全静默**。SDK 调 `plugin:event|listen` 时漏了 `target`
   （Tauri 2 的 `EventTarget` 是 `{kind:"Any"}` 这种结构），订阅直接失败，
   而报错只进 webview 控制台 —— 宿主日志里一片安静。
   补上 `target`；同时新增 `plugin_ui_log` 命令，把页面侧 warn/error 汇进宿主日志。
4. **窗口还没建好时消息被丢**。插件常见写法是"先 `ipcMessage` 再 `showUI`"，
   而 `showUI` 是异步的、窗口要 200~300ms 才建好。加了按窗口的待发队列 +
   **页面就绪握手**（SDK 注册好监听后调 `plugin_ui_ready`，宿主才补发）。
   为什么不用"页面加载完就补发"：实测 `on_page_load` 早于监听注册，照样丢。
5. **`deliver()` 不认 `exports.onXxx`**。它只查 `efto.onXxx` 这种直接赋值写法，
   而 hello-js 用的是 `exports.onMainMessage = ...`；且 `ui_message` 被投给了
   `onMessage` 而不是 `onMainMessage`。现在两种写法都归一到同一个 `onMainMessage(msg)` 签名。

### 修复后的实测（debug 与 release 各跑一遍）

- 插件页真的渲染出数据：打招呼内容 `-js`、累计次数 `12`、本次进程 `1`、语言/文档数 `zh-CN / 2`
- `<pre>` 里是真实消息 JSON（`收到 greet`）
- 窗口 X 能关闭；再次唤起会重建窗口并正常渲染
- 日志链路完整：`greet 排队等待窗口` → `greet 已补发` → `握手后补发 1 条`
- 主窗口从"执行中…"变成"已唤起 hello-js"（`invoke_hit` 正常返回）

### 回归守卫

`window_creation_stays_off_the_main_thread`（`windows.rs`，源码级）：
`open_plugin_ui` 的函数体里不得出现 `.run_on_main_thread(`、必须 spawn 独立线程、必须真的调 `build()`。
第一版守卫写得太宽（把 `show_main`/`hide_main` 里合法的 `run_on_main_thread` 也算进去），
测试当场失败 → 缩小到只看该函数体。GUI 线程问题没法单测，这是能自动化的最大范围。

### 第六处：插件的 `console.*` 从来没生效

查上面那条"插件不回消息"时想用 `console.log` 打点，才发现插件日志一直是哑的：
prelude 里 `hostCall('log', lvl, JSON.stringify([...]))` 传的是 **JSON 字符串**，
而桥接早已改成按数组解析；更关键的是**两个宿主都没实现 `log` 能力**（模块不存在），
于是 `console.*` 被 `try/catch` 静默吞掉 —— 白屏、点了没反应这类问题在宿主侧等于全瞎。
修了两处：prelude 改传数组；GUI/CLI 宿主都实现 `("log", level)` 并按级别写进 tracing。

### 现在这条链路有自动化证明了

`runtime::tests::ui_message_reaches_exports_on_main_message_and_reply_goes_to_post_message`
（真实 V8）：起一个临时插件，只注册 `exports.onMainMessage` → 调 `runtime.ui_message(...)`
→ 断言宿主收到了 `ui.postMessage`，且参数是插件回的那条。
这条测试锁住的正是三处曾经断裂的地方：exports 写法、`ui_message` 的投递名、以及回复的方向。

> GUI 里我最后一次点击发生在 `deliver()` 修复**之前**的构建上（时间线：12:53 点按钮 → 13:05 才修完），
> 所以当时看不到回复是预期的。修复后的等价路径已由上面的无头测试覆盖；
> 真机上"点按钮 → `<pre>` 出现 whoami 结果"仍建议你亲手点一下 —— 我这台机器的输入
> 会被 QwenWork/RayLink 反复抢走，实在送不进那个窗口。

## 端到端（无 GUI 通道，`efto cli`）

`scripts/verify.ps1` 全部通过：

```
frontend guard         OK   静态检查 .hidden/display 冲突 + CopyText 行的单击即复制覆盖
time probe             OK   Node 独立算 epoch 对比 release/debug 二进制真实输出（35 条 ok）
index built            OK   979 条应用（原始 1512，去重合并 533；开始菜单 196/注册表 57/目录 1245/系统 14），228ms
plugins discovered     OK   hello-js, timestamp-ts
search jsb             OK   拼音首字母命中记事本
search 记事本(中文)     OK
plugin-run hello-js    OK   onPluginEnter→db.put(rev)/db.set→showNotification→setSubTitle→ipcMessage
plugin-run timestamp-ts OK   双向：带时区的时间 -> 固定秒/毫秒 epoch；epoch -> 本地时间 + ISO 8601
plugin-run 未知插件     OK   退出码 3（查无结果）且给出可读错误
everything inline      OK   ext:exe 的文件行补在应用结果之后，无回收站噪音
capabilities --json    OK   子命令 / 退出码 / dry-run 面全部对得上
files --dry-run        OK   只报告不打开（真实文件路径）
focus 未知 pid         OK   退出码 3（查无结果）
tile-windows --dry-run OK   13 个候选 / 跳过 11 个（最小化、工具窗口、桌面、任务栏），输出工作区与目标矩形
launch 记事本           OK   notepad 真实启动（pid 存活确认后停止，无残留）
```

## 没做/没验证的（不要当成已具备）

- **GUI 交互本身没跑过**：这台机器的远程桌面会话会抢占前台焦点，Tauri 窗口/热键/托盘的人工验证没法自动化。窗口层代码是按 tauri 2.12 源码签名写的并通过了类型检查与单元测试，但"Alt+Space 弹出搜索框"这一步需要你在本机确认一次。
- 插件 **UI 页面**（`efto://` 协议 + 注入 SDK）只做了路径/越界/MIME 的单元测试与协议解析测试，未在真实 webview 里加载过。
- 剪贴板、通知依赖系统组件（arboard/WebView2 通知），未在无人值守环境断言可见效果。
- **DST 分支没有真跑过**：本机时区是 UTC+8（中国不用夏令时），`from_local_datetime` 的
  "空档返回 None / 回拨取 earliest" 两条分支只有代码路径、没有可执行样本。要证它需要把进程时区
  改成 `America/New_York` 之类再跑一遍（`chrono::Local` 跟随系统时区，测试里无法安全切换）。
  显式 `+08:00`/`Z` 那条路径是覆盖了的（探针里 6 组 + 单元测试 5 组）。
- **窗口聚焦的真实切换没验**：`cli ps` 的窗口枚举、`cli focus` 的匹配与"查无结果"（退出码 3）
  都跑过，但 `SetForegroundWindow` 把窗口真正提到前台这一步受 Windows 前台锁影响，
  自动化里只走了 `--dry-run` 与失败分支。
- **`cli action` 只验了报告面**：`reindex` / `open_plugin_dir` / `open_data_dir` / `lock_screen`
  跑了 `--dry-run`，`settings` / `screenshot` / `clipboard` / `quit` 的 `needs_gui` 报错跑过；
  真去锁屏、真开资源管理器没有在无人值守下断言。
- **窗口操作只验了 dry-run**：`cli tile-windows --dry-run --json` 的窗口枚举、过滤与网格/半屏矩形计算
  跑过；`--mode grid|left|right|min-others|min-all|undo` 逐个跑过（min-others 确认保留窗口不在候选里，
  undo 无记录时返回「没有可还原的平铺记录」）；真实移动/最小化/还原窗口没有在无人值守环境断言。
  新增单测覆盖 `half_layout` 与网格边界（n=1、最后一行不足、窄工作区、Op 名称往返）。
- **快捷键指南只验了数据层**：13 份内置 JSON（新增 Trae）的解析、exe 映射、回退与用户覆盖合并有单测；
  "唤起前是哪个应用"依赖真实前台窗口切换，未做 GUI 手测。
  `import_set` / `import_into` 两个新单测覆盖导入合并幂等、脏数据拒绝与 dry-run 不写盘（`tempfile` 隔离）。
  CLI 实测：`import-shortcuts --file … --dry-run --json` 输出 `added:2, updated:0, groups:1` 且退出码 0，
  stdin 模式同样成功；缺文件返回 `io_error`（退出码 1），坏 JSON 返回 2；`capabilities --json`
  含 `import-shortcuts`、`--file`/`--exe` 与 `shortcuts_import.format` 示例。未真实写盘导入 Trae 数据。
  布局修过一次：分组卡片在列向 flex 里被压扁、条目被裁切且不出现滚动条，
  用 760x620 的无头 Edge 注入 12 组数据量到 `scrollH=2116 / clientH=385`、`canScroll=true`、
  滚动条占 10px，确认修复；真实 WebView2 下的观感仍未手测。
- 索引/搜索的性能没有基准测试；979 条 <1ms 是推断（线性扫描 + 该量级），未压到万级条目。

## Release 构建

`cargo build --release -p efto-app` 通过，产物 `target/release/efto.exe`（55MB，内含 V8）。
release 二进制同样跑通了 `cli search jsb`、`cli plugins` 与 `cli plugin-run timestamp-ts ts 1700000000`（结果与 debug 一致，索引 979 条 / 375ms）。
未做：MSI/NSIS 安装包、代码签名、自动更新。

## GUI 层实测到什么程度

在真实桌面上启动 release 二进制后确认：

| 项 | 证据 | 结论 |
| --- | --- | --- |
| 进程与窗口存在 | `MainWindowTitle` 从"exe 路径黑框"变为 `app.efto.launcher-siw`（WebView2 宿主窗口） | 窗口创建成功 |
| 全局热键 | 按 Alt+Space 后 `foreground_window_title` 变成 `efto`，且 Windows 系统菜单没弹出 | **钩子注册成功且未被系统/其他程序抢占** |
| 失焦隐藏 | 焦点回到别的窗口后，我们的窗口不再在前台 | `hide_on_blur` 生效 |
| 首次唤出不闪退 | 首次唤出后主窗口保持可见（EnumWindows 实测），预热预览窗口补了 `.focused(false)`，不再抢前台触发假失焦（2026-11-20，见 `.agents/notes/implemented/bug-fix/2026-11-20-main-window-first-show-flash.md`） | 首次唤出"一闪而过"已修 |
| 控制台窗口 | 见下方"控制台窗口：一次我自己引入的回归" | 已修，最终方案是 `GetConsoleProcessList` 判定后隐藏 |
| 启动日志 | `载入索引缓存 979 条`、`发现 2 个插件`、`全局热键已绑定: Alt+Space`、`托盘已就绪: main-tray`、`Asset favicon.ico not found; fallback to index.html` | 索引缓存秒开、插件目录生效、前端资源被正常伺服 |

### 控制台窗口：一次我自己引入的回归

现象：桌面左上角盖一个标题为 exe 路径的黑色窗口——那是**控制台**，不是搜索框。

我第一次的修法是 `#![cfg_attr(windows, windows_subsystem = "windows")]`，看似干净，实际引入两个更严重的回归，被 `scripts/verify.ps1` 当场抓到（所有 Check 的输出变成空、`$text` 为 null）：

1. GUI 子系统的 exe 从 cmd/PowerShell 启动时**调用方不会等待**，`exe cli ... > file` 立刻返回、文件为空 → CLI 验证通道整体失效；
2. 失去 stdout，`cli` 子命令没有任何输出。

最终方案：保持控制台子系统，启动 GUI 时用 `GetConsoleProcessList` 判断——**只有该控制台仅由本进程使用时才隐藏**（即双击启动、系统新建的控制台）。从终端里启动时控制台属于用户的 shell，绝不隐藏。修后：`MainWindowTitle` 变为 `app.efto.launcher-siw`（黑控制台消失），`verify.ps1` 全部通过。

**仍未确认的**：搜索框的**视觉渲染**（透明/无边框/居中/字体/配色）与插件 UI 窗口的实际观感。原因是这台机器在 RayLink 独享桌面会话里，computer-use 的截图取到的是会话画面而非宿主桌面，且会话会持续抢回前台焦点。这两点需要人工看一次，检查表见 README"如何在本机验证 GUI"。

顺带发现并修掉的一个真实不一致：`EFTO_PLUGIN_DIRS` 以前只有 CLI 路径读取，GUI 启动日志里是"发现 0 个插件"；现在 env 解析统一到 `Settings::effective_plugin_dirs()`，两条路径同一套规则，并有回归测试。

## 默认面板与内置工具（2026-11-16）

`cargo test --workspace` 232 项通过（core 146 / jsrt 47 / app 39），`scripts/verify.ps1` 全部通过。

已用单元测试钉住的行为：

- 默认面板：空查询返回"最近使用 + 内置工具"两组、`group` 依次是
  `recent,recent,tools,tools`、工具行 `payload.type == action` 且 `name == screenshot`；
  关掉某个工具后只剩被勾选的、全关返回空列表（`service.rs`）。
- 工具关键词：搜"截图"/"clipboard"/"锁屏"分别命中对应内置指令，不会互相串（`service.rs`）。
- `Settings.tools`：缺字段用默认三项；`["lock_screen","bogus","screenshot","screenshot"]`
  规整成 `["screenshot","lock_screen"]`（`config.rs`）。
- `RecentStore`：同 id 前插去重、上限 20、失效项过滤（`recent.rs`）。
- `ClipboardStore`：空白不记、同文本前插去重、上限 200、30 天清理、搜索与删除（`clipboard.rs`）。
- 截图裁剪：正确取区域内像素、越界坐标夹到图内、PNG 经 data URL 往返后尺寸不变（`screenshot.rs`）。
- 前端守卫：`.group-head` 不影响 `.hidden`/`.empty` 的既有校验，`check-frontend.js` 通过。

**没验证的**：

- 截图的**真实像素与多显示器/高 DPI 行为**（`BitBlt` 路径只有裁剪与编码的单测，
  `capture_virtual_screen` 需要真实桌面，未在无人值守环境跑）。
- 剪贴板轮询与系统剪贴板的交互（写入图片后 Word/微信能否粘贴），以及 500ms 轮询的延迟手感。
- 锁屏（`LockWorkStation`）实际锁屏效果 —— 会中断当前会话，只做了"失败必报错"的代码路径。
- 三个窗口（截图覆盖层 / 钉图 / 剪贴板）的真实渲染与拖拽手感，同样受远程桌面抢焦点限制。

## 搜索结果系统图标（2026-11-17）

`cargo test --workspace` 234 项通过（core 146 / jsrt 47 / app 41），`scripts/verify.ps1` 全部通过。

已用单元测试钉住的行为：

- `reads_system_icon_for_explorer`：真的调用 `SHGetFileInfoW` 取 `C:\Windows\explorer.exe` 的图标，
  断言结果是 `data:image/png;base64,` 前缀且长度 > 200（不是空图或占位图）。
- `missing_file_is_an_error`：不存在的路径必须返回 `Err`，由宿主转成 `None`，前端保留首字母头像。
- 前端守卫：`iconCache` / `iconPending` / `paintIcon` 的加入不影响既有 `.hidden` 与"单击即复制"检查。

**没验证的**：

- GUI 里的实际显示效果（远程会话截不到宿主桌面），以及不同缩放比例下 32px 系统图标的清晰度。
- 老式无 alpha 图标走 AND mask 反推的分支——没有专门构造样本，只有代码路径。
- 插件 png logo 直接透传后的显示效果（尺寸/留白由插件图片自身决定）。

## 局域网文件共享插件 file-share（2026-11-17）

参考 `https://gitee.com/yuDeJiJie/file-share`（Apache-2.0），后端从
Node/express 改写为宿主 Rust 能力 `efto.share`，插件只保留 UI 与路径列表。

`cargo test --workspace` 240 项通过（core 146 / jsrt 47 / app 47）；本次 `scripts/verify.ps1`
除本机未装 Everything 的 `es` 用例外全部通过（插件被正常发现，4 个 cmd，含 `fs` 触发词）。

已用单元测试钉住的行为（`crates/app/src/share.rs`）：

- `serves_index_and_file_with_range`：`GET /` 能列出共享项；`GET /f/0` 返回原文；
  `Range: bytes=6-10` 返回 206、`Content-Range: bytes 6-10/11` 和正确片段。
- `rejects_path_escape`：`/s/0/../secret.txt` 与编码形式 `%2e%2e` 都返回 404。
- `reports_invalid_items`：不存在/空路径进入 `invalid`，不阻塞其它条目。
- `range_parser_accepts_open_ended`、`disposition_keeps_utf8_name`。
- `capability_contract_starts_updates_and_stops`：`share.start` 真起监听并能下载；
  同端口再次 start 只更新条目；`stop` 后 `status` 为未运行。

同日 UI 迭代：新增 `fs` 触发词；插件页改为卡片式布局；文件选择走新增的
`plugin.pickFiles()`（宿主命令 `plugin_pick_files` + rfd），拖拽走 Tauri 原生 drag-drop
事件（`plugin.onFileDrop/onFileDrag`），不用 HTML5 `drop`（拿不到绝对路径）；
"在文件夹中显示" 走新增的 `plugin.shellShowItemInFolder`。取消分享后删条目不会隐式重启服务。
共享服务同时尝试监听 `[::]`，仅在监听成功且存在非回环 IPv6 时返回 `urls6`。

**没验证的**：

- GUI 里插件窗口的实际观感，以及从另一台设备真实下载（当前只有本机回环请求）。
- GUI 里的原生文件选择框与从资源管理器拖拽（需要真实桌面交互，未纳入自动化）。
- Windows 防火墙首次放行弹窗；跨网段/多网卡时只展示主 IPv4，IPv6 只取主地址。
- 大文件（>4GB）与多连接并发、断点续传在浏览器里的实际表现。
- 中文文件名在各浏览器落盘时的最终名称（只验证了响应头编码）。

## 2026-11-20：file-share 条目删不掉

现象：file-share 里点条目的「移除」没反应，下次打开条目还在。

根因：宿主 `share.rs` 把 `items[].path` 返回成 `canonicalize()` 之后的路径，Windows 上带
`\\?\` 前缀；插件 KV 里存的是用户原始路径。`remove` 用回传的 path 去匹配 `rawItems`，
永远匹配不上，界面看不出任何变化。

修法：`parse_items` 回传调用方传入的路径文本，仅 `ShareItem.root` 保留规范化结果（路径
越界校验仍用 root）。插件侧删除/去重统一走 `normPath()`（剥掉 `\\?\` 前缀、忽略大小写），
兼容升级前已经存进 `itemsMeta` 的旧路径。新增单测
`echoes_input_path_instead_of_canonical_root` 锁定回传语义。

验证：`cargo test -p efto-app share` 7 passed / 0 failed；
`node --check plugins/file-share/main.js` 通过。没验证：GUI 里真实点击「移除」。

## 2026-11-20：file-share 取消分享后重开又自动分享

现象：点「取消分享」→ 关闭插件窗口 → 重新打开，状态又变成"正在分享"。

根因：`plugins/file-share/main.js` 的 `onPluginEnter` 只要 KV 里有 `items` 就无条件
`apply()` 自动启动；「取消分享」只改了进程内的 `sharing`，没有把"用户不想分享"落盘。

修法：

- KV 新增 `wantShare`：`start` / `add` 置 `true`，`stop` / `clear` 置 `false`；
  `onPluginEnter` 只有 `rawItems` 非空**且** `wantShare` 为真才自动启动。旧数据没有该键时
  按 `true` 处理，升级后行为与改动前一致。
- 停止状态下仍要展示列表：新增 `itemsMeta`（宿主校验后的 `{name,path,is_dir,size}`），
  成功 `apply()` 后落盘，停止状态打开时据此渲染；关闭窗口停服务但不动 `wantShare`。
- 停止状态下删除条目同步更新 `itemsMeta`，不会在下次打开时"复活"。

验证：`node --check plugins/file-share/main.js` 通过。没验证：GUI 里实际走一遍
`取消分享 → 关窗口 → 重开`；也未覆盖"停着的时候文件被外部删除/移动"的展示。

## 2026-11-20：file-share 其他机器能开页面但下载报网络错误

现象：本机下载正常；另一台机器能打开 `http://<局域网IP>:5421/`，点文件下载报"网络错误"。

根因：`share.rs` 的 accept 循环把除 `WouldBlock` 外的**任何** accept 错误都当成终止条件
（`Err(_) => break`）。Windows 上客户端在 accept 之前断开就会让 `accept()` 返回
`WSAECONNRESET`（浏览器预连接被放弃、页面请求被取消都会触发）。监听线程一旦 break，
端口仍在监听、内核继续排队 SYN，但再没人 accept：已经渲染出来的页面看着正常，之后
所有请求都卡到网络错误，而 UI 状态仍是"正在分享"。

修法：accept 出错改为记 warn + 200ms 退避后继续监听，只有 `stop` 能结束线程；
每个连接的处理错误也带对端地址记一条 warn，不再整段静默丢弃。

验证：`cargo test -p efto-app share` 7 passed / 0 failed。没验证：真实"客户端在 accept
前断开"的复现（需要构造 RST 时序）；修后在另一台机器上的实际下载——**已补齐**（见下方
"再补充"末尾：10035 根因修复后，另一台机器（10.10.174.110，跨网段）用新 exe 实测下载成功）。

补充（同日）：远端修复后仍报网络错误，进一步确认是**浏览器把 `http://<局域网IP>:5421/`
升级成了 https**。共享服务是纯 HTTP、没有证书，TLS 握手不可能成功。现在明文端口收到
TLS ClientHello（首字节 `0x16 0x03`）时会立刻断开并记一条 warn
（"共享端口收到 TLS 握手：对方在用 https 访问…"），不再等到读超时/莫名 431。
客户端侧的处理见那台机器：关掉「始终使用安全连接 / HTTPS 优先」，或用
`curl -o t.tgz http://<ip>:5421/f/0` 绕过浏览器验证服务端正常。
没验证：远端的浏览器策略到底是哪一项；也还没有给共享服务加 HTTPS（自签证书）的能力。

再补充（同日，最终根因）：上一段"https 升级"仍是误诊。给 `init_tracing` 加上文件日志
（stderr + `~/.efto/efto.log` 双写，GUI 启动时 stderr 无人接收、等于没有日志）之后，
远端下载瞬间抓到实锤：

```
WARN 共享连接处理失败（10.10.174.110:65431）：无法立即完成一个非阻止性套接字操作。 (os error 10035)
```

`TcpListener` 为 accept 轮询设了 `set_nonblocking(true)`，而 **Windows 上 `accept()`
返回的流会继承监听 socket 的非阻塞模式**。`handle()` 里的 `read()` 在对方数据尚未到达时
立即返回 `WSAEWOULDBLOCK` 而不是等待，连接被当作错误掐断——这才是"页面能开、下载必败"
的真正根因（能打开页面也是竞态运气：索引请求的数据偶尔赶在 read 之前到达）。
**为什么本机测试永远发现不了**：loopback 上数据在 accept 返回前进内核缓冲区，read 总有
数据；跨网段客户端有 RTT，accept→read 的间隙数据必然未到。之前专门写的 socktest 探针
也因此得出"接受的套接字是阻塞模式"的错误结论。

修法：accept 后 `stream.set_nonblocking(false)`（恢复阻塞后 `set_read_timeout(5s)` 才
真正生效）。回归测试 `serves_client_that_sends_request_after_a_delay`：客户端 connect
后故意 sleep 200ms 再发请求，模拟 RTT。**反向验证过**：注释掉那一行恢复阻塞的代码，
该测试必失败（客户端收到 10053 连接被中止，与真实症状一致），恢复后 8/8 通过。

**真机闭环**：用带此修复的 exe 重启后，另一台机器（10.10.174.110，跨网段）从
`http://10.30.71.88:5421/` 实际下载成功。

## 插件运行时配置 efto.node / efto.python（2026-11-17）

设置页新增「运行时」，配置 node.exe / python.exe 路径；检测通过后插件可调用
`efto.node.run({ code, args?, timeoutMs? })` / `efto.python.run(...)`。

`cargo test --workspace` 245 项通过（core 150 / jsrt 47 / app 48），`scripts/verify.ps1` 通过。

已用单元测试钉住的行为：

- `script_runtime_names_are_strict`：只认 node / nodejs / node.js / python / python3，
  其它（含空串）返回 None，不会猜。
- `run_script_refuses_unconfigured_and_empty_code`：未配置解释器时是
  `NotConfigured("Node.js")` 且文案指向设置页；空白脚本直接拒绝，不白起解释器等 stdin。
- `run_script_reports_missing_interpreter_path` / `probe_interpreter_rejects_empty_and_missing`：
  带路径的解释器不存在时报 `BadInterpreter`，不会去 PATH 找同名程序。
- `runtime_boundary_requires_settings_and_refuses_paths`：插件参数里出现
  `path`/`interpreter`/`program`/`command`/`shell`/`cwd`/`env`/`file` 一律拒绝；
  `args` 必须是字符串数组；未知 runtime 只回 "只支持 node / python"。
- `missing_fields_use_defaults`：旧 settings.json 没有运行时字段时默认未配置（`None`）。

**没验证的**：

- 设置页运行时页面的真实交互（检测按钮、保存后状态、检测失败样式）——需 GUI 手测。
- 真机 Node.js / Python 的实际执行（`-e`/`-c` 参数长度、非 ASCII 输出、Python 在
  Windows 上的 stdout 编码）；当前只验证了参数校验与探测失败路径。
- 插件端拿到 `Promise` 后的错误呈现（未配置时的 toast/控制台输出）。

## hosts 编辑插件与 `efto.hosts`（2026-11-17）

新增原生插件 `plugins/hosts`（条目编辑 / 原文编辑 / 启用禁用 / 自动备份）与宿主能力
`efto.hosts.path/read/save`；保存时用 `ShellExecuteExW(verb="runas")` 拉起提权助手写入。

已用单元测试钉住的行为：

- `accepts_normal_entries_and_comments`：IPv4/IPv6、`::1`、行内注释、被注释条目都能通过。
- `rejects_bad_ip_and_missing_host` / `rejects_bad_hostname_characters`：非法 IP、
  缺少主机名、主机名里的 `/`、`^` 都会报错并带行号。
- `disabled_entry_is_validated_but_plain_comment_is_not`：`#10.0.0.1 bad/name` 按条目
  报错；`#10.0.0.256 dev.local` 与 `# 说明：10.0.0.256 ...` 按普通注释放行。
- `reports_line_numbers_and_stops_at_20`：错误数量封顶，第一条一定是第 1 行。
- `utf8_roundtrip` / `ansi_roundtrip_uses_the_system_code_page`：UTF-8 与系统 ANSI
  代码页都能往返；BOM 去掉；ANSI 不可表示字符走报错分支。
- `nul_and_oversize_are_refused`：NUL 与超过 1MB 的内容拒绝。
- `write_bytes_backs_up_then_replaces`：两次写入后备份是上一版、目标是最新版、
  临时文件已消失。
- `apply_rejects_requests_outside_the_uac_directory`：提权侧拒绝数据目录之外的请求文件。
- `save_capability_refuses_bad_text_and_encoding`：坏文本、未知编码、缺 `text` 都在
  JSON 边界被拒，不会触达文件系统。
- `scripts/verify.ps1` 增加 `plugin-run: hosts plugin reads native capability`：
  在 CLI 宿主下确认 `plugins/hosts` 能加载并读到真实 hosts（回传了 `path` / `bytes` /
  `encoding`，界面消息被"无窗口"分支忽略），说明宿主能力与插件接线是通的。

**没验证的**：

- 当前环境没有以管理员身份实测真实写入系统 hosts（会修改用户机器），UAC 弹窗、
  `hosts-apply` 子进程实际落盘、`MoveFileExW` 替换的真实结果都需要 GUI 手测。
- GBK 注释在真实 zh-CN hosts 文件里的显示效果（单测用的是同一 ACP 往返，不是真实文件）。
- UAC 提权到另一个管理员账户时的表现（代码会拒绝，见 `docs/architecture.md`）。

## 长截图（2026-11-17）

`cargo test -p efto-app` 65 项通过。新增单测钉住拼接核心：

- `scroll_delta_finds_shift`：构造整体上移 37 行的两帧，估计值正好是 37。
- `scroll_delta_reports_zero_when_static`：同一帧返回 0（表示不追加）。
- `scroll_delta_rejects_unrelated_frames`：内容无关的两帧返回 `None`，不会乱拼。

**没验证的**：真实滚动拼接效果需要 GUI 手测（截图 → 选区 → 「长图」→ 覆盖层保留、鼠标穿透，
在底下窗口拖动滚动条），以及不同 DPI、吸顶栏、平滑滚动动画下的位移估计精度；
`WDA_EXCLUDEFROMCAPTURE` 在 Windows 10 2004 以下不可用，此时边框可能进结果图。

## AI 对话插件与 DeepSeek 联网 / OCR（2026-11-18）

`plugins/ai-chat` 接 `efto.ai.chat`（流式）与新增的 `efto.ai.ocr`；联网搜索与网页阅读
按 DeepSeek 的服务端工具实现（`tools: [{type:'deepseek_web_search'}, {type:'deepseek_read_url'}]`），
不引入第三方搜索服务。

`cargo test --workspace` 289 项通过（core 168 / jsrt 48 / app 73）；`node --check` 对
`main.js` / `ui.js` 通过；`cli plugins` 能发现 `ai-chat`；`cli plugin-run ai-chat ai 你好`
加载后端并投递 enter 成功（`[ui→page:prefill]`，线程正常退出）。

已用单元测试钉住的行为：

- `build_request_declares_deepseek_web_tools`：`webSearch` / `readUrl` 为 true 时才写入
  `tools`，都没开时请求体里没有 `tools` 键。
- `stream_state_collects_tool_events_and_citations`：`server_tool_event` 产出 `toolEvent`，
  最后一个 chunk 的 `delta.server_tool_calls` / `delta.annotations` 被累积到最终结果。
- `ocr_endpoint_targets_v1_tools`：`https://api.deepseek.com`、`.../v1`、
  `.../v1/chat/completions` 三种写法都归一到 `.../v1/tools/ocr`。
- `async_stream_deltas_reach_the_plugin_callback`（真实 V8）：两条 `Stream` 事件按序到达
  `onDelta`，最后 `Reply` 解开 Promise，请求体含 `stream: true`。

**没验证的**：

- 真实 DeepSeek 账号下的联网搜索、网页抓取与 OCR 结果（本机未配置 API Key）；
  `deepseek_web_search` 要求 `deepseek-flash`，换模型时的报错文案未实测。
- `multipart` 上传在真实网关上的兼容性：只在本机按字节构造、未对大文件（接近 100MiB）
  与 180s 超时边界做过压测。
- 插件 UI 里的联网开关、文档 chip、工具状态与"参考来源"列表的实际观感（需 GUI 手测）。

## 当前编译告警

0 条。`cargo test --workspace` 与 `cargo build --release` 均无告警输出
（历史上约 79 条 `missing_docs` 等技术债已逐条补齐 API 说明，而不是关掉 lint）。

## 2026-11-19：命令执行 / 托盘帮助 / 多语言

### 单元测试

```
cargo test -p efto-app --lib   91 passed
```

- 新增 `crates/app/src/i18n.rs` 的 3 个测试：两份词典 key 完全一致、托盘/窗口标题
  用到的 key 在中英文里都存在、缺失 key 原样返回（不 panic）。
- 命令执行相关测试在 `crates/core`（前缀解析、各 Shell 参数拼装、超时/取消、输出截断，
  见 `search::command` 与 `exec` 模块）。
- **一次网络抖动**：`ai::chat_stream_reads_sse_from_http` 在整仓跑时曾以
  `os error 10053`（本机回环连接被中止）失败，单独重跑 `-p efto-app --lib` 91 个全过；
  这与本次改动无关，但说明该用例对本机回环时序敏感。

### 静态守卫

- `scripts/check-i18n.js` 接进 `scripts/verify.ps1`：词典 key / 占位符一致，
  HTML 的 `data-i18n*` 与 JS 的 `T()/TF()` 引用的 key 必须存在，
  HTML 里未走词典的中文会报行号（语言名白名单）。
- `scripts/check-frontend.js` 仍然通过。

### 实际验证过的

| 项 | 结果 |
| --- | --- |
| `cargo check --workspace` | 通过，无告警 |
| `cargo test -p efto-app --lib` | 91 passed（含 i18n 词典一致性） |
| `node scripts/check-i18n.js` | OK（281 keys × 2 语言，HTML/JS 引用全部命中） |
| `node scripts/check-frontend.js` | OK |
| `scripts/verify.ps1` | ALL CHECKS PASSED（含 `run-command --dry-run` 与真实 `echo`） |

### 没验证过的

- **帮助窗口的视觉效果**：窗口能创建、Markdown 渲染逻辑有前端守卫，但没有在真实
  webview 里逐项核对表格/代码块/目录高亮。重复点击不新开窗口依赖 `get_webview_window`
  分支，未做 GUI 自动化。
- **英文界面在 520px 搜索框内的排版**：词典已提供英文，未逐条截图检查截断/溢出。
- **系统语言实时变化**：`auto` 只在启动/切换设置时解析一次（`GetUserDefaultUILanguage`），
  运行中改 Windows 显示语言需要重启 efto，未做监听。
- **命令执行 GUI 手测**：`>ipconfig`、Git Bash、WSL、`Esc` 取消等交互路径未在真实
  桌面会话里跑；CLI 通道（`run-command`、超时、`--dry-run`）已由 `verify.ps1` 覆盖。
- **插件 `efto.locale`**：现在跟随 `settings.language` 解析结果，但没有写一个插件实测切换。

## 2026-11-20：P1 片段 / 通用动作 / 文件动作与缓冲 / 剪贴板 2.0

### 单元测试

```
cargo test --workspace
efto-core: 203 passed
efto-app:  100 passed
efto-jsrt:  48 passed
```

- `snippets`：关键词必须整串相等才命中（`si` / `signature` / `sig extra` 均不出片段），
  `{date}`/`{time}`/`{datetime}` 展开，关键词含空格/重复/正文为空时保存报错。
- `clipboard`：文件列表去重按 Windows 路径大小写不敏感、置顶条目不参与 200 条上限与过期清理、
  v1 纯文本 JSON 仍可反序列化。
- `universal::action_ids`：固定动作始终存在，链接/已存在路径才追加“打开”。
- `winclip::dropfiles_bytes`：CF_HDROP 的 DROPFILES 头与宽字符双 NUL 布局。

### 静态守卫

- `node scripts/check-i18n.js`：OK（两份词典 key / 占位符一致，HTML/JS 引用全部命中）。
- `node scripts/check-frontend.js`：OK。

### 实际验证过的

- `cargo check --workspace` 与 `cargo test --workspace` 全绿；`efto-core` 单独构建也通过
  （`ShellExecuteW` 的 `Win32_UI_WindowsAndMessaging` feature 已显式声明，不依赖 app crate 的 feature 合并）。
- 搜索侧片段来源接进 `query_with_sources` 后，指引与既有搜索测试未回归。

### 没验证过的

- **真实桌面输入注入**：`SendInput` 抓取选中文本、`Ctrl+Enter` 粘贴回前台、片段粘贴，
  未在前台为 UWP/管理员窗口时手测；UIPI 拦下 `SetForegroundWindow` 时只确认会报错。
- **剪贴板图片/文件落地效果**：写出的 PNG 与 CF_HDROP 能否被 Word、微信、资源管理器正常粘贴，未手测。
- **第二全局热键冲突**：被其他软件占用时只记日志、不影响唤起热键，未逐个冲突场景验证。
- **两个新窗口的渲染**：`actions` / `files` 的视觉效果、多显示器位置、英文排版未截图核对。
- **批量文件操作边界**：网络路径、同名目标改名、大目录复制/移动的耗时与取消未实测。
- **入口限制**：文件缓冲目前只能从搜索结果加入（`Ctrl+Enter` 或动作菜单），不支持从资源管理器拖入。
- **通用动作只处理文本**：资源管理器里选中的文件不会进入动作菜单（`SendInput Ctrl+C` 后只读文本剪贴板）。

## 2026-11-20：P2 网页搜索 / 系统命令 / 单项热键

### 单元测试

```
cargo test --workspace
efto-core: 213 passed
efto-app:  100 passed
efto-jsrt:  48 passed
```

- `websearch`：关键词必须是第一个空白分段且后面有正文（单独 `g`/`g ` 不触发），
  大小写不敏感，`b`/`g`/`gh`/`wiki` 默认引擎与自定义引擎 roundtrip，`normalize` 丢弃非法项并去重，
  `{q}` 编码覆盖非 ASCII，关键词命中 780 分高于兜底 300 分。
- `service`：网页搜索前缀短路其它源；只有本地完全无结果且 `web_search_fallback` 开启时才追加兜底；
  系统命令只在整串精确匹配时出现（`sleep now` / `restar` / `重启电脑` 均不触发）。
- `system`：动作 id/标题/确认标记一一对应，`from_id` 拒绝未知 id。
- `config`：`tool_hotkeys` 只保留已知工具、丢弃空键、与唤起/通用动作热键重复的组合；
  网页搜索列表收敛后兜底引擎自动回落到第一项。

### 静态守卫

- `node scripts/check-i18n.js`：OK（380 keys × 2 语言，HTML/JS 引用全部命中）。
- `node scripts/check-frontend.js`：OK。

### 实际验证过的

- `cargo test --workspace` 全绿；`scripts/verify.ps1` ALL CHECKS PASSED
  （含索引、搜索、插件、命令、Everything、启动清理）。
- 设置页新增两个页面后，`check-i18n.js` 抓出了 3 个不存在的 `settings.tool.*` key，
  改成复用既有 key 后通过——说明静态守卫确实覆盖了新页面。

### 没验证过的

- **破坏性系统命令**：重启/注销/清空回收站只确认会弹原生确认框与错误路径；
  没有在真机上实际执行重启、注销或清空回收站（避免中断会话）。
- **睡眠/屏保**：`SetSuspendState` / `SC_SCREENSAVE` 广播未在真机执行，只在代码层检查返回码。
- **单项热键注册**：只验证配置收敛与注册失败只记日志；没有逐个组合在真实系统上按下去验证，
  也没有覆盖被其他软件占用时的冲突提示。
- **网页搜索实际打开**：`shell_open` 打开浏览器已验证会调用系统默认浏览器，但未逐条核对
  自定义 URL 里 `{q}` 的最终跳转页面。
- **英文界面排版**：新增的“网页搜索”“单项热键”两页未在 520px / 设置窗口宽度下截图核对英文截断。

## 2026-11-20：带 UI 插件的统一重写

范围：`ai-chat`、`file-share`、`hosts`、`ocr`、`quickweb`、`text-beautify`、`hello-js`。
`dev-tools` 是第三方预编译 Vue 产物，不改源码，只清理 README / 清单里用户可见的第三方启动器文案。
所有插件的消息名、`efto.*` 调用与功能语义保持不变，只重写 HTML 结构、CSS 与交互细节。

### 统一契约

- 设计令牌直接取自 `frontend/settings.css`（深色 `--bg:#121419` / `--card:#1b1e26` /
  `--accent:#78a2ff`，浅色 `--bg:#f4f6fa` / `--accent:#3b6fe0`），不另造一套配色。
- 窗口框架统一为 52px 顶栏（品牌块 + 插件名 + 右侧操作）、卡片内容区、9px 控件圆角、
  14px 卡片圆角、10px 细滚动条、`focus-visible` 焦点环、`prefers-reduced-motion` 降级。
- 主题统一走 `document.documentElement.dataset.theme = efto.isDarkMode() ? ...`。
  `efto.isDarkMode` 是**函数**，不是布尔属性；`ai-chat` 与 `ocr` 原先把它当属性用，
  结果永远判真、永远深色，这次一并修掉。

### 顺手修掉的真实缺陷

| 插件 | 缺陷 | 修法 |
| --- | --- | --- |
| ai-chat / ocr | `efto.isDarkMode` 当布尔用，主题恒为深色 | 改为函数调用 + `data-theme`，异常回退深色 |
| file-share | `statusEl.textContent = ...` 会把状态圆点 `<i>` 一并清掉 | 圆点与文本分开，文本放进 `#statusText`，状态写 `data-state` |
| hosts | 动态删除按钮 class 是 `ghost small`，新样式选择器是 `.btn.ghost.small` | 动态 class 补 `btn` |
| text-beautify | 测试用 fake DOM 没有 `document.documentElement` | 主题只在 `index.html` 头部内联脚本里设置，`ui.js` 不动 |

### 静态验证

- `node --check` 逐个通过：`ai-chat/ui.js`、`file-share/ui.js`、`hosts/ui.js`、
  `ocr/node-script.js`、`quickweb/ui/app.js`、`text-beautify/ui.js`、`hello-js/main.js`、`quickweb/main.js`。
- 一次性脚本比对「HTML 的 `id`」与「JS 里 `getElementById` / `querySelector('#…')` 的引用」：
  7 个插件全部引用齐全（ai-chat 18 / file-share 19 / hosts 18 / ocr 12 / quickweb 4 /
  text-beautify 15 / hello-js 8 个 id）。
- `plugins/text-beautify/tests` 7 个文件全过：JS 68 项、JSON 31 项 + 400 轮属性测试、
  main 15 项、SQL 169 项、UI 37 项、XML 19 项。

### 视觉验收（Playwright + 本机 Edge）

用 `npx playwright screenshot --channel msedge --viewport-size 1024,650` 打开各插件的
`index.html`；`hosts` 与 `quickweb` 的内容由 IPC 数据渲染，另建了带 `window.efto` 桩的临时页
（只放在 `%TEMP%`，不进仓库）喂样例数据后截图。逐张核对了：

- ai-chat：侧栏会话列表、消息空状态、附件/文档 chip、输入区与工具栏布局正常。
- file-share：顶栏状态、分享地址卡片、拖拽区、空列表提示、按钮层级正常。
- hosts：路径栏、添加映射行、表头与数据行的五列网格对齐，禁用行删除线与降透明度生效。
- ocr：左右分栏、拖拽区、底部粘贴/清空、右栏重新识别/复制/保存位置正常。
- quickweb：分组标题带计数、卡片网格自适应、自定义列表与新增行对齐、关键词标签正常。
- text-beautify：类型分段控件、双栏编辑器、选项与底栏状态正常。
- hello-js：问候卡片、三项统计、按钮组、消息日志区正常。

没有发现溢出、错位或控件重叠。

### 没验证过的

- **真实 WebView2 里的渲染与 IPC**：截图用的是浏览器内核 + 桩，不是宿主注入 `window.efto`
  的 `efto://` 页面；真实宿主下的拖拽、原生文件选择框、流式消息仍未在 GUI 里手测。
- ai-chat 的模型下拉、流式打字、来源列表，以及 ocr 的识别结果区域，只验证了静态布局，
  没有真实 API Key 下的内容渲染。
- `dev-tools` 的预编译包内部仍有 `utools` 适配分支，但设置页的「uTools」标签只在
  `kn.isUtools()` 为真时出现，efto 下不可见；本次不反编译第三方产物。
- 旧兼容守卫 `scripts/check-compat.js` 与当前仓库（AGENTS.md 已明确不提供兼容层）不一致，
  运行会报 11 处失败，属于历史遗留、未接入 `verify.ps1`，本次未改动。

## 2026-11-20：插件窗口主题跟随应用设置

插件 UI 页面此前恒为深色，与主窗口的浅/深设置不一致。两个独立原因：

1. `assets/plugin-sdk.js` 把 `isDarkMode` 写成布尔属性，而插件页面按 SDK 契约写
   `efto.isDarkMode()` → 调用抛 `TypeError` → 页面 `catch` 回退深色。现已改为函数，
   并补上 `isLightMode()`。
2. 宿主只给常驻窗口套了 `set_theme`，插件窗口没套，`prefers-color-scheme` 只跟系统走。
   现在 `open_plugin_ui` 按设置锁定窗口主题，`apply_theme` 也会同步已打开的
   `plugin-ui-*` 窗口；7 个一方插件的页面都监听 `matchMedia` 的 change 事件即时重画。

验证：

```
cargo check -p efto-app                                       通过
cargo test -p efto-app windows                                5 passed / 0 failed
node --check crates/app/assets/plugin-sdk.js                  OK
node --check plugins/file-share/ui.js                         OK
7 个插件 index.html 的内联脚本用 Node 解析检查                 OK
```

没验证过：GUI 里实际打开各插件并切换 跟随系统/浅色/深色 目视核对三态。

## 2026-11-20：窗口操作二级菜单

「平铺窗口」从单一动作扩展为二级菜单：网格平铺、最小化其他窗口（保留搜索框下方的那个）、
最小化全部窗口、左/右半屏、还原上次平铺。单项热键仍然直接网格平铺，不经过菜单。

### 单元测试与静态检查

```
cargo check -p efto-app                       通过，0 告警
cargo test -p efto-app tile -- --test-threads=1  9 passed / 0 failed（94 filtered out）
node --check frontend/app.js                  OK
node scripts/check-i18n.js                    OK（394 keys × 2 语言）
node scripts/check-frontend.js                OK
```

9 个 tile 单测覆盖：`n=0` 空布局、`n=1` 占满工作区减边距、`n=2` 水平对分、
`n=3` 最后一行不足时占满整行、`n=1..=9` 矩形互不重叠且都在工作区内、窄工作区尺寸仍为正、
`half_layout` 左右对分并保留间距、极小工作区不出现 0 或负尺寸、`Op` 名称往返与未知值返回 `None`。

### CLI dry-run 实测（`efto.exe cli tile-windows`）

| `--mode` | 结果 |
| --- | --- |
| 默认（`grid`） | 工作区 1920x1040 @ (0,0)；10 个候选，逐个给出矩形；跳过 11 个 |
| `left` | 目标窗口为搜索框下方的「help.zh-CN.md - efto - TRAE CN」，矩形 948x1024 @ (8,8) |
| `min-others` | 列出 11 个待最小化窗口，消息明确「保留「help.zh-CN.md - efto - TRAE CN」」，保留窗口不在候选里 |
| `min-all` | 列出 12 个窗口（含其他显示器上的窗口），不受当前显示器限制 |
| `undo` | `没有可还原的平铺记录`，无候选时按既有约定返回退出码 3 |
| 未知模式 | 退出码 2，提示可用取值 |

`--json` 输出包含 `work` / `candidates` / `layout` / `tiled` / `skipped` / `failed` / `message`。

### 没验证过的

- **真实 GUI 交互**：点击「平铺窗口」展开菜单、`Esc` 返回搜索结果、输入即退出菜单，
  只做了 `node --check` 与代码走读，未在真实 WebView2 窗口里点过。
- **实际移动/最小化/还原窗口**：所有 `--mode` 都只跑了 `--dry-run`；`SetWindowPos` /
  `ShowWindow` / `BeginDeferWindowPos` 的真实效果、UWP 等受限窗口被计为失败的复核分支，
  没有在无人值守环境断言（会改动当前桌面）。
- **撤销状态的边界**：`LAST_TILE` 只在进程内保存最近一次平铺，重启 efto 后为空；
  最小化操作不写入撤销记录，因此「还原上次平铺」不会恢复被最小化的窗口。
- **多显示器**：网格/半屏按搜索框所在显示器计算，最小化作用于所有显示器；
  真实多屏 + 不同 DPI 下的坐标换算未实测。

## 2026-11-20：单项热键默认值与按键录入 / 操作窗口改名 / 网页搜索排版

- 单项热键默认 `Ctrl+Alt+1..7`（顺序同 `KNOWN_TOOLS`）。老配置文件在**整份表为空**时由
  `seed_tool_hotkeys` 补齐；`tool_hotkeys_seeded` 标记区分"从未配置"与"主动全部解绑"，
  设置页保存空表后不再回填。
- 设置页 7 个单项热键输入框改为按键捕获：`keydown` + `e.code` 归一成 Tauri 写法，
  Backspace/Delete/Esc 清空，Tab/Shift+Tab 仍切换焦点，裸字母/数字必须带修饰键。
- 「平铺窗口」工具项改名为「操作窗口」；关键词保留 `平铺窗口` / `平铺` / `排列窗口` 等旧入口，
  二级菜单里的「网格平铺」行仍叫「平铺窗口」。
- 网页搜索设置页：引擎行与卡片边缘贴死、关键字/名称/移除按钮/URL 四种控件高度不一致。
  `.snippet-list` 补 `14px 17px` 内边距、行间距 `12px`，`.snippet-head` 内控件与 `.engine-url`
  统一 `36px` 高（片段页共用同一套样式）。

### 验证

```
cargo test -p efto-core                          216 passed / 0 failed（新增 tool_hotkeys_seed_defaults_only_once）
cargo check -p efto-app                          通过
node --check frontend/settings.js                OK
node scripts/check-i18n.js                       OK（396 keys × 2 语言）
```

无头 Edge（760x620）注入两行引擎卡片量取：`listPad=14px/17px`、`gap=12px`、
关键字/名称/移除/URL 四个控件均为 `36px` 高，行间 `12px`；520px 宽页面内关键字 140、
名称 276、移除 52，未溢出。

### 没验证过的

- **真实注册**：`Ctrl+Alt+1..7` 是否被其它程序抢占、注册失败只记日志的路径未在真机逐个按过。
- **按键捕获交互**：只在代码层与 `node --check` 上确认，未在真实 WebView2 里按组合键试录。
- **设置页观感**：新内边距/高度是按无头渲染量取的，未在真实设置窗口截图核对英文长度。

## 2026-11-20：只输入网页搜索触发词时的用法提示

- 新增 `websearch::hint(query, engines)`：整串（忽略大小写与首尾空白）等于某个引擎关键词时，
  返回一条 850 分的 `HitPayload::Action("websearch_hint")` 提示行，标题「用 Bing 搜索…」，
  副标题给出 `g rust` 这类示例。
- `service::query` 把它作为**附加分组**并入，不短路本地结果；分数低于前缀匹配的应用（约 900），
  所以应用还是排在前面。带正文时仍走原有逻辑：短路本地源，只留一条 780 分的 `WebSearch`。
- 宿主 `run_action` 新增 `websearch_hint`，回车只回一句「在触发词后加空格和关键词，例如 g rust」。

### 验证

```
cargo test -p efto-core    218 passed / 0 failed
```

新增 `bare_keyword_gets_usage_hint`（大小写/空白、`gx` 与 `g rust` 不提示）与
`web_search_keyword_alone_hints_without_hiding_apps`（应用仍在且分数更高、带正文时仍短路）。
原 `web_search_prefix_short_circuits` 的断言同步改为"没有 WebSearch 载荷、但有提示行"。

### 没验证过的

- 真实 GUI 里提示行的观感与回车后的提示文案展示未手测；`g` 同时命中大量应用的场景只靠分数推断。

## 2026-11-20：截图标注（Snipaste 风格）

### 后端

`cargo test -p efto-app` 107 项通过，含两个新增单测：

- `overlay_png_decodes_data_url_back_to_image`：`data_url()` 的逆操作，PNG 经 data URL 往返后像素一致。
- `overlay_png_rejects_wrong_prefix_and_garbage`：非 PNG data URL 前缀与坏 base64 明确报错，不静默成功。

`screenshot_finish` 新增 `overlay`（可缺省）参数：有标注时前端回传合成好的物理像素 PNG data URL，
宿主 `overlay_png` 解码后直接用作复制/保存/钉图的图（钉图尺寸随之适配）；没有标注时走原来的
`Shot::crop` 路径，行为不变。

### 前端（无 GUI 自测）

测试装置（不进仓库，放在 `%TEMP%`）：桩替换 `__TAURI_INTERNALS__`（`screenshot_begin` 回一张
生成的纯色 PNG data URL，其余命令记录调用）+ 合成 PointerEvent/KeyboardEvent 驱动
`screenshot.html/js`，headless Edge（`--dump-dom` + `--virtual-time-budget`）跑完读结果。40 项断言全部通过：

- 选区与工具栏：拖出选区出工具栏；纯点击不清空选区（双击复制的前提）；未选区按 Enter 出错误提示、
  再次拖选区错误消失。
- 8 个工具：矩形/椭圆/箭头/马赛克一次成型后自动退出，画笔/荧光笔/橡皮保持连续；数字键 1-8 切换、
  再按一次取消；Ctrl+Z / Ctrl+Y 快照式撤销重做，按钮 disabled 状态与内容联动。
- 文字：点击选区开编辑器，Enter 提交、Esc 取消。
  （注意：这里的合成事件没有默认动作，"点击后焦点被抢走"这类问题它覆盖不到，见下文 2026-11-20 焦点修复一节。）
- 橡皮选择性：对提交结果的像素断言证实"只擦掉画笔、保留矩形与文字"。
- 合成提交：双击复制回传 `overlay`；解码后尺寸与选区一致；背景像素保留、矩形描边为所选颜色、
  文字渲染在对应区域。
- 色板 8 色、色点跟随所选颜色；粗细三档循环。

自测顺带抓到并修掉两个 bug：`longMode`/`longRows`/`longTimer` 声明在重写时丢失
（strict mode 下首次拖选区即 ReferenceError，`node --check` 查不出来）；错误提示一经显示永不清除。

### 没验证过的

- 真实 GUI 手感：多显示器与 DPI 缩放下的坐标换算、马赛克颗粒度、中文输入法在文字工具里的组合输入、
  钉图窗口对 overlay 尺寸的适配，均未在真实宿主里手测（headless 自测里 scale=1）。
- 装置本身的坑（已排除，给后来人）：PowerShell 5.1 `Get-Content` 默认按 ANSI 读 UTF-8 文件，
  双重编码会吃掉中文 title 属性里的引号、破坏 HTML 结构（表现为某个按钮变成文本节点）；
  构建测试页必须显式 UTF-8 读写（`[IO.File]::ReadAllText/WriteAllText`）。

## 2026-11-20：插件包多插件导入

`import_path` 由"一个包只导一个插件"改为"包内所有插件都导"：

- 单测（`test -p efto-core plugins::import`，8/8 通过）：新增 `imports_multiple_plugins_at_package_root`
  （插件目录在包根）、`imports_multiple_plugins_inside_wrapper_dir`（多套一层 `plugins/` 容器）、
  `partial_failure_keeps_other_plugins`（一个重名失败、另一个照常导入）。
- 真实包验证（临时集成测试，验证后删除）：`plugins.zip`（10 个插件，带 `plugins/` 容器层）与
  `plugins1.zip`（10 个插件，目录在包根）都完整导入 10/10，`failed` 为空；
  两包解压后总大小 13.48 MB，低于 64 MB 上限。

未验证：设置页导入提示的实际观感（多插件/部分失败两种文案）未在 GUI 里手测。

## 2026-11-20：截图文字工具焦点（修复后）

现象：文字工具在选区内点击，编辑器一闪即关、无法输入。根因是 pointerdown 的默认动作
（焦点落给 body）把刚 `focus()` 的 textarea blur 掉，`blur` 里 `commitText()` 关掉了空编辑器。
修复：文字工具分支先 `preventDefault()`（frontend/screenshot.js）。

验证方式改用 **CDP 真实输入事件**（headless Edge + `--remote-debugging-port`，
`Input.dispatchMouseEvent` / `Input.insertText` / `Input.dispatchKeyEvent`），
因为上一轮的合成 `PointerEvent` 没有默认动作，抓不到这条路径：

- 旧版（删掉 preventDefault）：编辑器 `display=none`、`activeElement` 为空、键入丢失 —— 复现现象。
- 修复版：`display=block`、`activeElement=text-input`、`efto 中文` 落入 textarea；
  Enter 提交后撤销可用；矩形真实拖拽、双击复制提交 `screenshot_finish` 均通过（11/11）。

未验证：真实 GUI 中中文输入法的组合输入（组字中途的 Esc/方向键）未手测。

## 2026-11-21：插件停用 / 启用

设置页每个插件加「停用 / 启用」开关：停用后插件仍留在列表（可随时重新启用），但
`PluginDescriptor.enabled=false`，从 `get` / `search` / 触发词 / `efto://` 消失，已跑的
isolate 被释放；启用后重新发现、按需启动。状态持久化在 `settings.disabled_plugins`。

- 单测（`test -p efto-core`，全绿）：
  - `plugins::registry::disabled_plugins_are_listed_but_not_searchable`：两个插件 discover 后停用
    其一 —— 列表长度不变、`enabled=false`、`search` 为空、`get` 返回 `None`，另一个不受影响；
    `set_disabled(&[])` 后恢复可搜。
  - `config::disabled_plugins_default_empty_and_roundtrip`：旧 `settings.json` 缺字段回落空表；
    `{"disabled_plugins":["timer"]}` 能解析。
- `cargo build -p efto-app` 通过；`node --check frontend/settings.js`、`check-i18n`（404 键 × 2）通过。

未验证：GUI 里真实点开关的观感（压暗样式、toast 文案）、停用带 UI 的插件时窗口关闭时序
（`close_plugin_ui` 是否先于 isolate 释放完成）。

## 2026-11-21：api-manager 插件 + CLI 插件数据落盘

新增 `plugins/api-manager`：Postman 风格 API 管理（集合 / 环境变量 / 历史 / 导入导出，
UI + 命令双通道）。HTTP 走 `efto.node.run`，命令机器可读结果以 `EFTO_API ` 前缀输出。

同时修宿主：`cli plugin-run` 之前不 flush 插件 KV，CLI 进程退出即丢失写入。
`CliHost` 增加 `flush()`，`cmd_plugin_run` 在 `runtime.stop` 后调用；
`--json` 增加 `flush_error`，非空时 `ok=false`、退出码 1。

另修 `--json` 丢插件日志：`bridge` 的 `log` 之前只写 tracing，`plugin-run --json`
拿不到 `console.*`，`EFTO_API` 结果直接消失。现在改走 `PluginEvent::Log`
（GUI 仍由 pump 落到 tracing，观感不变），`--json` 新增 `logs: [{level,text}]`。

- `cargo test -p efto-app cli_host`：3 passed（新增
  `cli_host_flush_persists_plugin_data`）。
- `cargo test -p efto-jsrt route_`：2 passed（`log` 仍不打到 Host）。
- `cargo build -p efto-app --bin efto` 通过。
- CLI 端到端（`EFTO_PLUGIN_DIRS=plugins`，真机有 Node 运行时）：
  - `cli plugins` 列出 api-manager；
  - `plugin-run api-manager api "help"` / `"list"` 正常输出；
  - `"send Get Todo"` 真实请求 jsonplaceholder，`[runtime] node code=0`，返回 `userId/id/title`；
  - `{"cmd":"add",...}` 新增 `SmokeGet`，另起进程 `"list"` 仍能看到（证明落盘修复生效）；
  - `"--json-file <path>"` 读取文件新增 `From File` 成功；
  - 内联 JSON 在 PowerShell 5.1 里需写成 `'{\"cmd\":\"list\"}'`，含空格会被拆分，
    因此新增了 `--json-file` 入口并在 README 里写明。
- `node --check` 通过 main.js / lib/*.js / ui.js。
- UI 无头验证（Python Playwright 直接加载 `plugins/api-manager/index.html`，注入
  `window.efto` 桩模拟 `state` / `send-result` / `saved` / `export-result`）：
  - 集合树与请求项渲染、切换请求、URL / Method / Auth 回填、Ctrl+S 发出 `save-request`；
  - 发送后响应区显示 `200 OK`、`12 ms · 26 B`、格式化 JSON；
  - Params 行数、Auth bearer、导入 / 环境 / 导出弹窗、导出结果 textarea、历史面板正常；
  - 无 `pageerror`、无 console error。
  - 该轮发现并修复：`.modal-root` 的 `display:grid` 覆盖了 `hidden`，全屏遮罩一直
    拦截点击，界面实际不可操作；补 `.modal-root[hidden]{display:none}` 后复测通过。
  - 第二轮（补侧栏「＋ 新集合」按钮，Node + playwright-core + 本机 Edge 无头，注入
    `window.efto` 桩并按 `save-collection` 回推 `state` / `collection-saved`）：
    按钮文案为「＋ 新集合」；点击并输入名称后发出
    `{name:'save-collection', data:{collection:{name:'UI 测试集合'}}}`，侧栏出现
    `UI 测试集合 · 0`；取消或空名不发送任何消息；无 `pageerror`。

- 真机 GUI 验证（2026-11-21，debug 构建 + `EFTO_PLUGIN_DIRS=plugins`）：
  - `Alt+Space` → 输入 `api` 回车，打开独立插件窗口，页面 URL 为
    `http://efto.localhost/api-manager/index.html`；
  - 集合树（`示例 · 1` / `Get Todo · GET`）、Method、URL、发送按钮、环境显示正常；
  - `prompt`（集合重命名：「集合名称 / 示例」）与 `confirm`（「删除请求「Get Todo」？」）
    均弹出 WebView2 原生对话框，点取消后数据未变；
  - 点「发送」返回 `200 OK`（4566 ms · 70 B，jsonplaceholder 响应体正常渲染）。

未验证：窗口尺寸是否严格为 1180x780（真机为独立大窗，未做像素级测量）；
`efto://` 在 WebView2 里实际落到 `http://efto.localhost/`，与文档协议名不同但行为一致。

### 2026-11-21：集合分组 + Swagger URL 导入

- CLI（真实工作区，验证后已清理测试集合）：
  - `collection add` → `folder add / folder list`：`folder list` 返回
    `{total:1, items:[{name:"用户接口", path:"用户接口", requestCount:0}]}`；
  - `--json-file` 执行 `{cmd:"add", collection:"导入验证", folder:"用户接口", ...}`，
    返回 `folder:"用户接口"`、`folderId:"fld-…"`，请求确实落在分组内；
  - `folder delete` 级联：`removedFolders:1, removedRequests:1`；
  - `export 导入验证`：Postman v2.1 的 `item` 为嵌套结构
    （`{name:"组A", item:[{name:"导出检查", request:{...}}]}`）；
  - Postman 导入：两层 `item` 导入后 `list` 显示 `folder:"用户 / 子组"`。
- Swagger（真实地址 `http://10.10.58.153:32300/doc.html`，SpringDoc/knife4j）：
  - `import --url .../doc.html` 自动发现 `/v3/api-docs/swagger-config`，导入 3 个分组集合
    （开放接口 / 服务间接口 / 运营平台管理接口），共 68 个请求；
  - `folder list 开放接口` 显示按 tag 建的 5 个分组；
  - `show sendSmsCode` 的 body 为按 schema 生成的 `{"mobile": ""}`，URL 为
    `{{baseUrl}}/open/personal/sms-code`；
  - `env` 显示自动新建并激活的环境 `10.10.58.153:32300`
    （`baseUrl = http://10.10.58.153:32300`），三个分组共用同一个 `baseUrl`；
  - `list` 中路径参数已转成 `{{id}}`（如 `/inner/personal/users/{{id}}`）。
- 导入到指定集合（同上真实地址）：
  - `import --url ... --collection 导入目标测试`：只生成 1 个集合，`addedRequests:68`；
    `folder list` 显示 20 个分组，Swagger 分组名（开放接口 / 服务间接口 / 运营平台管理接口）
    为一级分组、tag 为二级分组（如 `服务间接口 / 内网认证`）。
  - `import --url ... --collection-id col-…`：追加进同一集合，`addedRequests:68`，
    未新建集合。
  - 验证后已把插件数据恢复为测试前状态（`市场` + `示例环境`）。
- UI（Node + playwright-core + 本机 Edge 无头，注入 `window.efto` 桩）：
  - 集合头 `⊞` 建「分组A」，分组内 `⊞` 建「分组B」，缩进 `8px → 22px`；
  - 分组内 `＋` 新建并保存，`save-request` 携带 `folderId:"fld-1"`，保存后请求行缩进 24px；
  - 删除分组发出 `delete-folder`；导入弹窗填写 doc.html 后发出 `import-url`；无 `pageerror`。
  - 导入弹窗「导入到」下拉：默认「自动」时 payload 无目标字段；选现有集合发
    `collectionId:"col-1"`；选「＋ 新建集合…」出现名称输入框，发出
    `collection:"导入集合"`；三组均无 `pageerror`。

未验证：YAML 规范、OAuth2 等需要在请求前换取 token 的鉴权流程；Swagger 的
`servers[].variables` 未做替换（按字面生成 URL）。

## macOS 验证记录（2026-11-21，Apple Silicon / macOS 14.6.1 / Rust 1.100）

Phase 0/1（见 `docs/macos-support-plan.md`）的实测记录。

### 单元测试：374 通过 / 0 失败

```
efto-app  104 passed   efto-core  222 passed   efto-jsrt  48 passed（真起 V8）
```

顺带修掉 `ai::tests::chat_stream_reads_sse_from_http` 的 fake-server RST 竞态
（server 没读完请求 body 就关连接 → 客户端读响应体偶发 EINVAL），修后单测连跑 8 次全过，
见 `.agents/notes/implemented/bug-fix/2026-11-21-ai-stream-test-rst-race.md`。

mac 新增/改造的测试重点：

| 模块 | 关键断言 |
| --- | --- |
| `apps::appbundle` | Info.plist 名（DisplayName > Name > 目录名）、bundle id 别名、**bundle 内 helper 反例**（`.app` 是叶子）、忽略规则整包吃掉、Utilities 子目录深度内能扫到 |
| `apps::scanner` | mac 走可执行位判定、`.app` 子树不递归、**系统工具白名单必须为空**（ms-settings: 不得漏进 mac 索引） |
| `config::*` | `normalize_for_platform` 跨平台回落（shell + 显式路径清理 + 工具 id 丢弃）、默认工具列表取平台子集、`CommandShell::default` 平台化 |
| `exec::*` | zsh `-lc` 参数拼装、Cmd/PowerShell/WSL 断言仅 Windows、es.exe 端到端仅 Windows、timeout 测试在 es 起不来时 skip 不假绿 |
| `search::service` | **mac 上 `ms-settings:` 不出设置页**（反例）、工具关键词双向断言（支持的必命中 / 平台不支持的必不出现） |
| `apps::startmenu` | 忽略规则 fixture 按平台选路径形状（globset 在 Unix 上 `\` 不是分隔符），反向断言在两种形状下都真的过 matcher |

### `scripts/verify.sh` 端到端：ALL CHECKS PASSED

与 `verify.ps1` 同一检查清单（唯一事实源是本文件）；差异点显式 SKIP 不计通过：

- index / plugins（translate 发现）、拼音 e2e（`fy` → 翻译，mac 无 CJK 系统应用，
  用插件触发词做靶子，与 ps1 的 jsb→记事本 同一目的）、CJK 触发词 `翻译`；
- 计算器 6 组（含 `chrome` / `1700000000` 不劫持反例）；
- plugin-run：hello-js / timestamp-ts 双向（epoch 对与机器时区无关）/ translate 空载荷 /
  未知插件 exit 3；**hosts SKIP（Windows-only）**；
- everything 内联源 **SKIP（Windows-only，mac 的 mdfind 是 v1.5）**；
- run-command dry-run（解析出 zsh）+ echo 回环、capabilities --json、files --dry-run、
  focus 未知 pid exit 3；
- launch：dry-run 解析出 TextEdit → 真启动 → osascript quit（mac 上 `.app` 走 `open`，
  pid 日志是 `open` 的 pid，清理只能靠 quit，与 ps1"杀掉那一个 pid"同义）。
- 时间探针（check-time-probe.js）全量对齐 Node/ICU（含"不劫持"与"计算器让位"边界）。

### 没验证 / 没做

- **Windows 侧回归未跑**：本批改动理论上不碰 Windows 行为，但 `verify.ps1` +
  `test --workspace` 需要 CI 矩阵或 Windows 机器跑一次才算闭环（`.github/workflows/ci.yml`）。
- GUI 全部（热键、托盘、图标、插件窗口、设置页）——Phase 2。
- hosts 保存（无提权路径）、系统动作（睡眠/重启/注销/清空回收站/屏保，明确报不支持）、
  终端模式执行（明确报不支持）——v1.5。
- tile / 截图 / 通用动作 / 快捷键指南 / 文件缓冲（mac 不进搜索结果，执行层报错兜底）——Phase 4。
- `x86_64-apple-darwin`（Intel mac）未构建；vendor 只备了 arm64 产物。

## macOS Phase 2 GUI 验证记录（2026-11-21，Apple Silicon / macOS 14.6.1）

### 已跑过并看到输出

- 单元测试全绿：efto-app 110 / efto-core 222+2 / efto-jsrt 48（含新增：NSWorkspace 图标
  两条、hosts 能力平台报错、清单 platform 门、registry 平台过滤、decorate_keys、
  platform_info 形状）。
- **GUI 冒烟（dev-run.sh 真起）**：启动日志显示索引 111 条 + 插件 11 个 +
  `全局热键已绑定: Alt+Space` + 单项热键自动收敛为 mac 3 工具
  （Ctrl+Alt+1..3 → clipboard/lock_screen/snippets）+ `托盘已就绪` + `就绪`；
  通用动作热键**不再注册**（平台门生效，日志 `通用动作 未启用`）。两次冒烟后进程正常退出。
- 图标链路真机测试：`Finder.app` 能取到 PNG data URL，不存在的路径报"文件不存在"
  （对齐 Windows 语义，不让 NSWorkspace 的通用图标充数）。
- check-i18n / check-frontend / check-time-probe 全过（词典 404 key 双语对齐）。

### 没验证（需要人手在 GUI 里点，按顺序）

1. **按 Option+Space 唤起搜索框**（热键注册成功已证实；实际触发依赖系统权限——
   若没反应，看 系统设置 → 隐私与安全性 → 辅助功能/输入监控）。风险 4 结论待补。
2. 输入应用名回车启动 `.app`；输入 `6*7` 看计算器行；输入 `翻译 hello` 唤起插件 UI
   （验证 WKWebView 下 `efto://` 协议加载插件页面）。
3. 托盘菜单（打开设置/帮助/退出）；设置页：Shell 下拉应只有 zsh/bash/fish/自定义、
   工具行只显示 剪贴板/锁屏/片段、无通用动作行、无终端模式选项、热键提示显示
   Option/Ctrl→Control 装饰后的文案。
4. 剪贴板窗口历史浏览（文本/图片）；`>echo hi` 命令源内联执行。
5. 明确报错路径：搜"截图/平铺"应无结果；`cli plugin-run hosts hosts hosts` 应报
   unsupported_platform；hosts 插件在 `cli plugins` 标注"不支持当前平台"。

### 已知取舍

- 透明窗口走 `macOSPrivateApi`（用户定夺）：dmg 直发无碍，App Store 分发会被拒，
  Phase 4 再评估。
- 图标走 NSWorkspace via objc2（用户定夺）；objc2 栈复用 tauri 的 0.6/0.3 版本，
  无新增 C 工具链。
- clipboard.html 的 `Ctrl+Enter` 保持 Control 语义（mac 键盘可按，Cmd 适配留待后续）。






## macOS Phase 3 验证记录（2026-11-21，Apple Silicon / macOS 14.6.1）

### 已跑过并看到输出

- 单元测试全绿：efto-app 128 / efto-core 238 / efto-jsrt 48（新增：mdfind 参数白名单/
  绝对路径免摘要/行数封顶、sys_settings 前缀与不劫持反例、hosts 提权转义与 HOME 注入、
  系统动作 AppleScript 转义、winclip 粘贴板回环与回收站、winproc 窗口枚举）。
- `scripts/verify.sh` ALL CHECKS PASSED，其中新检查：
  - **spotlight 文件源**：真机 `cli search json --kind file` 出 `(spotlight)` 行、无垃圾桶噪音；
  - **系统设置源**：`x-apple.systempreferences:` 出 settings 行，普通输入"显示"不被劫持；
  - **hosts 插件**：`cli plugin-run hosts hosts hosts` 真读到 `/etc/hosts`（utf8）。
- 子代理真机 e2e：`cli ps` 列出 15 个窗口（pid/CGWindowID/进程名/标题）；
  `cli focus --pid <finder>` 聚焦成功、未知 pid 报"没有匹配的窗口" exit 3。
- **性能发现**：mdfind 全量枚举常见查询 8439 行 / 12.95s——exec `max_lines` 行数封顶后
  提前杀进程返回（测试断言 <3s、行数远小于全量）。

### 没验证（需要人手）

1. 五系统动作实跑（睡眠会真的睡眠！建议先试"屏保"）；确认框与 Automation 授权弹窗。
2. hosts 真实提权写（管理员密码框 → 备份 `/etc/hosts.efto.bak` → 写入生效）。
3. CGEvent 实键链路：通用动作抓取/片段粘贴（**先在 系统设置 → 隐私与安全性 → 辅助功能
   给 efto 授权**，否则只会看到"缺少辅助功能权限"的报错——报错本身也算验证了失败路径）。
4. 文件剪贴板真机回环：复制文件 → 资源管理器/访达里粘贴。
5. 无 Screen Recording 权限时 `cli ps` 的标题降级为本机已有权限，未测降级形态。

### 已知取舍

- **file_buffer / 通用动作的入口**（Phase 4 已放出）：底层（winclip/CGEvent）就绪后随
  Phase 4 翻开关；CGEvent 成功路径仍未实测，缺权限时明确报错并指向设置页"权限"区块。
- 快捷键指南 macOS 数据集未做；`cli ps` 窗口标题依赖 Screen Recording 权限（降级为空）。
- 系统设置页 id 面向 macOS 13+，个别 id 在旧系统可能落到设置首页（打开语义不受影响）。


## macOS Phase 4 验证记录（2026-11-21，Apple Silicon / macOS 14.6.1）

### 已跑过并看到输出

- 单元测试全绿：efto-app 142 / efto-core 240 / efto-jsrt 48（新增 tile+ax 22 个：
  布局、Cocoa→Quartz 坐标换算、显示器归属、AX 错误文案分类、Report 序列化；permissions 形状）。
- **窗口平铺实机**：`cli tile-windows --dry-run` 产出 16 窗 4×4 计划（表格+JSON 形状正确）；
  `--mode left`（真实）精确报 `缺少辅助功能（Accessibility）权限：…在 系统设置 → 隐私与安全
  性 → 辅助功能 中勾选 efto`；`--mode undo` 空记录报"没有可还原的平铺记录"（不误要权限）。
- **权限引导**：设置页"权限"区块仅 mac 展示（辅助功能/屏幕录制状态 + 去授权按钮，
  直开对应系统设置页）；`permission_status` 形状稳定。
- **开关放出**：TileWindows / UniversalActions / FileBuffer 进入 mac 搜索与工具面板
  （`KNOWN_TOOLS` 5 项）；Screenshot / ShortcutGuide 保持收口。

### 没验证（需要人手，接 Phase 3 清单继续）

1. **给 efto 辅助功能授权后**：`cli tile-windows --mode left` 真的平铺 → `undo` 恢复；
   tile 的 `tile_then_undo` 单测会自动跑绿。设置页"权限"区块状态变"已授权"。
2. **给 efto 屏幕录制授权后**：截图捕获层未实现（见下），暂无实抓路径。
3. CGEvent 实键（通用动作抓取/片段粘贴）同 Phase 3 清单第 3 条。
4. hosts 提权写、五系统动作实跑同 Phase 3 清单。

### 已知取舍

- **截图跳过**（用户决定 2026-11-21）：两个实现子代理均被取消且未落盘，screenshot.rs
  保持 Windows-only，mac 入口继续收口（搜"截图"无结果、CLI 报不支持）。独立立项再做。
- 快捷键指南 macOS 数据集未做（ShortcutGuide 保持收口）。
- 真实窗口移动用到 `_AXUIElementGetWindow`（HIServices 长期稳定的私有导出，
  yabai/Hammerspoon 同用）；若未来系统移除该符号是链接期失败，不会静默错乱。
