# macOS 文件检索收紧到常用目录并加强噪音过滤

- **Date**: 2026-11-22
- **Status**: done
- **相关**: crates/core/src/search/spotlight.rs、crates/core/src/exec.rs、docs/architecture.md、docs/verification.md、docs/roadmap.md

## 背景

macOS 没有 Everything，替代方案是系统自带的 Spotlight（`mdfind`）。这条通道早就接好了
（`search/spotlight.rs`），但实现是**全盘** `mdfind -name <q>`，在真实用户机器上不可用。

## 实测数据（本机，Apple Silicon / macOS 14.6.1）

| 查询 | 耗时 | 结果数 | `~/Library` 噪音 |
|------|------|--------|------------------|
| `mdfind -name config` 全盘 | 5.0s | 4135 | 3584（87%） |
| 限定 Desktop/Documents/Downloads | 0.3s | 166 | 0（全是用户文件） |
| `mdfind -name 报告` 全盘 | 0.7s | 15 | 部分 |
| 限定后 | 0.075s | 15 | 0 |

配合 1.2s 超时 + `max_lines: 40`，全盘模式等于只拿到被截断的噪音（典型表现：搜 `config`
返回的全是 Quark 的 `Library/Application Support` 配置）。

## 改了什么

1. **查询限定目录**：`mdfind_args` 生成「若干 `-onlyin <绝对目录>` + `-name <查询词>`」。
   目录来自 `search_dirs()` = `~/Desktop`、`~/Documents`、`~/Downloads` 中**存在**的那些。
   一个都不存在 → `hits()` 直接返回空，**不退化成全盘**（否则用户会莫名其妙等 5 秒）。
2. **噪音过滤扩展**：`is_noise()` 新增 `node_modules`、`.git`、`~/Library/{Caches,
   Application Support, Containers, Group Containers}`，以及 `.DS_Store`/`.localized`
   这类系统文件名。`~/.zshrc` 这类用户配置**不**过滤（它们常被主动搜索）。
3. **exec 参数白名单放宽到位**：`rejected_arg_mdfind` 现在接受 `-onlyin`，但要求：
   绝对路径（相对路径会被 mdfind 按当前目录解释，等于绕开范围约束）、最多 8 个、
   必须排在 `-name` 之前、目录长度 ≤1024 且不含 NUL。
4. **设置页平台分流**：macOS 显示「文件检索（Spotlight）」只读状态（新命令
   `file_search_status`：通道/可用性/目录列表），Everything 区块只在其它平台显示 ——
   之前 mac 用户会看到 es.exe 下载按钮和「未检测到 Everything」的误导文案。

## 为什么多个 `-onlyin` 合并成一次调用

实测 mdfind 的多个 `-onlyin` 是 **OR** 语义，且合并成一条命令比逐目录调用快 3 倍
（58ms vs 171ms，见验证记录）。所以拼一条命令而不是循环。

## 验证

- 单元测试：`mdfind_onlyin_requires_absolute_dirs`（相对路径/缺目录/顺序/超量全拒）、
  `noise_filter_blocks_dependency_and_cache_dirs`（正反例都有）、
  `mdfind_args_put_onlyin_before_name`；有效 437 passed。
- `scripts/verify.sh` 新增两条**主动探针**检查（不依赖用户桌面上恰好有什么文件）：
  1. 在 `~/Documents` 建唯一名探针 → 最长等 10s 索引 → `cli search <probe> --kind file`
     必须出 `(spotlight)` 行且无 trash/node_modules/.git 噪音；
  2. **反例**：在 `/tmp` 建探针，同样搜索，不得出现在结果里（证明范围没越界到全盘）。
- 真机对比：`config` 5.0s/4135 条 → 0.93s/2 条（全是 `~/Downloads` 用户文件）。

## 判据（后续别再踩）

- **不要为了"更全"就把 `-onlyin` 去掉**。全盘 mdfind 在这台机器上是 5 秒 + 87% 噪音，
  等于功能不可用。要覆盖范围外文件就上自建索引（见 roadmap）。
- `verify.sh` 里 spotlight 检查必须用**主动创建的探针**，不能用「搜 `json` 看有没有结果」——
  限定目录后这种写法会退化成 SKIP，看起来"通过"其实什么都没验证。
- 判断"文件行是否越界"要用**路径**做判据，不能用探针文件名：CLI 的「无匹配: <查询词>」
  提示里也含探针名，会误报失败。

## 追加（同日）：检索目录改为用户可配置

初版把范围硬编码成"桌面/文档/下载"，但用户文件常在别处（代码仓库、移动硬盘）。
改为：

- `Settings.file_search_dirs: Vec<PathBuf>`（新增字段，默认空）+ 
  `Settings::effective_file_search_dirs()` 作为**唯一事实源**：
  默认存在目录在前、用户追加在后，相对路径 / 不存在 / 重复一律丢弃。
- `search/spotlight.rs::search_dirs()` 改为读这份配置（不再自己拼 home 目录）；
  另导出 `default_search_dirs()` 供设置页区分"默认"与"用户添加"。
- 新增命令 `add_file_search_dir` / `remove_file_search_dir`：只接受绝对路径且存在的
  目录；移除只作用于用户追加列表（默认三个目录不可移除）。
- 设置页目录列表：默认目录只读并标「默认」，追加目录带「移除」；「添加目录」
  复用 `pick_plugin_source { kind: "dir" }` 的系统目录选择框，不新增选择器实现。

**判据**：设置页显示的目录列表必须与 `-onlyin` 实际用的列表同源（都来自
`effective_file_search_dirs`）。前端不要自己拼默认目录，否则"显示的"和"生效的"会漂移。
