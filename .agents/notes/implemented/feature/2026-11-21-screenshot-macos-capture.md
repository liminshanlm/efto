# 截图工具 macOS 捕获层（CGWindowListCreateImage）

- **Date**: 2026-11-21
- **Status**: done（真机捕获已实测；长截图滚动链路留人工）
- **相关**: `crates/app/src/{screenshot,windows,commands}.rs`、`crates/app/src/permissions.rs`

## 现象

截图是 mac 移植最后一个平台限定动作（此前两次子代理实现均被取消且未落盘）。
Windows 侧 BitBlt 全链路（整屏/区域/长截图滚动拼接/标注）完整，mac 只有报错桩。

## 做法

- **捕获**（`screenshot.rs`）：`CGWindowListCreateImage`（C 接口）+ 位图上下文归一：
  不管源图什么像素格式/缩放，统一画进 `w×h` 的 RGBA 缓冲（premultipliedLast +
  32Big = 内存序 R,G,B,A），TranslateCTM+ScaleCTM 翻成自顶向下，alpha 恒 255
  ——与 Windows 版口径逐字段一致（Shot/crop/scroll_delta 零改动）。
- **坐标口径**：全部沿用"虚拟桌面物理像素"。CG 全局显示坐标是"点"，按主屏
  backing scale（CGDisplayMode 像素宽/点宽）换算——单屏/同缩放多屏精确，
  混合缩放多屏按主屏缩放统一采样（v1 取舍，已注释）。
- **权限**：捕获前查 `permissions::screen_recording_allowed()`，缺则先
  `request_screen_recording()` 触发系统引导，再报"缺少屏幕录制（Screen Recording）
  权限：…"，绝不返回空白图（硬性约定 3）。
- **长截图**：`capture_region` 同一实现覆盖滚动抓帧；`exclude_from_capture` 补
  mac 版（NSWindowSharingNone via objc msg_send），覆盖层边框不进长图。
- **开关**：`BuiltinAction::supported_on_platform` 不再有平台限定（保留钩子）；
  `KNOWN_TOOLS` 两平台已完全一致，合并为单个常量。

## 判据

- **真机实测**（本机有屏幕录制权限）：`mac_capture_real_screen_or_skip` 真抓整屏
  （非全黑、alpha 全 255）+ 区域捕获尺寸精确 + crop 正确；缺权限报错路径反向 skip。
- 单测 app 144 / core 241 / jsrt 48 全绿；verify.sh ALL CHECKS PASSED；
  GUI 冒烟 `Ctrl+Alt+1 -> screenshot`（mac 工具 7 项与 Windows 对齐）。

## 没验证 / 留给后续

- 截图交互链路的人工点验：唤起 → 截图 → 框选 → 标注 → 复制/保存/钉图；
  长截图的滚动拼接实测（滚动内容、排除覆盖层、Esc 停止）。
- 混合缩放多屏（Retina + 1080p）的取景精度；CGWindowListCreateImage 已 deprecated
  （14 仍工作），未来系统移除时换 ScreenCaptureKit。
