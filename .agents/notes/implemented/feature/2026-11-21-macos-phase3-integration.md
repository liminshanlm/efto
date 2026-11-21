# macOS 支持 Phase 3：系统集成补齐（mdfind/系统设置/系统动作/文件剪贴板/hosts/ps-focus/书签）

- **Date**: 2026-11-21
- **Status**: done（人手验证清单见 `docs/verification.md` mac Phase 3 段）
- **相关**: `docs/macos-support-plan.md`、`crates/core/src/{exec,search/spotlight,search/sys_settings,bookmarks}.rs`、
  `crates/app/src/{system,hosts,winclip,winproc}.rs`、`scripts/verify.sh`

## 现象

Phase 0/1/2 后 mac 的"搜索/启动/插件"可用，但文件检索、系统设置、系统动作、文件剪贴板、
hosts 写入、窗口列表/聚焦全是"明确报不支持"的桩。

## 做法

- **mdfind 文件源**（`search/spotlight.rs`，镜像 everything.rs 的边界纪律：≥2 字符、
  基础分 160 压应用下、失败静默）。exec 白名单重构为**按条目**判定：`Allowed` 增
  `absolute_path`（系统工具固定路径，免钉摘要——系统升级会换二进制内容，PATH 劫持面
  由绝对路径消掉）+ `os` 平台门 + `max_lines`。
- **系统设置源**（`search/sys_settings.rs`，整串 `x-apple.systempreferences:` 前缀触发，
  28 页数据表，结构对齐 win_settings.rs；自带平台门，其它平台恒空）。
- **系统动作**（sleep=pmset / restart/logout=System Events / empty_trash=Finder /
  screensaver=open；confirm 真弹框）与 **hosts 提权**（osascript `do shell script …
  with administrator privileges`，复用既有 uac 请求文件安全设计）由并行子代理实现。
- **winclip/winproc**（NSPasteboard 文件剪贴板、trashItemAtURL 回收站、CGEvent 粘贴前台、
  CGWindowList 窗口列表、NSRunningApplication 聚焦）由并行子代理实现。
- **书签**：Chrome/Edge 的 User Data 根目录补 mac 路径（`~/Library/Application Support`）。

## 判据（含踩坑）

- **mdfind 全量枚举不可等**：常见查询（"json" 8439 行）实测 **12.95s**，1.2s 超时会把
  结果整个吞掉。解法是 exec 的 `Allowed::max_lines`：收满 N 行就杀进程、带部分结果返回
  （`read_capped_lines` + 主循环提前收）。超时保平安、行数保延迟，两个都要。
- **提权 HOME 注入**：`do shell script … with administrator privileges` 以 root 跑，
  HOME 解析成 root 家目录，`apply_main` 的 `~/.efto/uac/` 边界校验会拒掉合法请求——
  拉起命令前显式 `HOME='<发起方 home>'`，校验零改动（硬性约定 4）。
- **osascript 失败分类**：自身退出码恒 1，真实错误在 stderr 尾 `… (N)`：
  -128=用户取消、-1743=Automation 被拒、正数=内层命令退出码。
- **权限矩阵**：NSPasteboard/trash/activate/CGWindowList 的 pid+进程名免权限；
  CGEvent 键盘模拟要 **Accessibility**（无权限 tracing::error! 明确指引，不空跑）；
  窗口**标题**要 Screen Recording（无权限 title 空为允许降级）。
- 单测全绿：app 128 / core 238 / jsrt 48；verify.sh ALL CHECKS PASSED（spotlight 真机出
  文件行、系统设置源 OK + 不劫持反例、hosts 插件真读 /etc/hosts）。

## 没验证 / 留给后续

- 五系统动作实跑、hosts 真实提权写、CGEvent 实键发送——破坏性/交互/有副作用，
  留人手清单（verification.md）。
- **功能开关未翻**：file_buffer / 通用动作的底层（winclip）已就绪，但 `KNOWN_TOOLS` /
  `BuiltinAction::supported_on_platform` 的 mac 收口**保持不动**——CGEvent 成功路径未实测，
  翻开关等于放未验证能力上线；人手验过 Accessibility 路径后再翻（Phase 3.5）。
- 快捷键指南 macOS 数据集（shortcuts/data）未做；Windows 侧回归待 CI。
