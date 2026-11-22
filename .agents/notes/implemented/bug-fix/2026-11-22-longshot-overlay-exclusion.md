# 长截图抓帧把覆盖层拍进结果：sharingType 对显示器捕获无效

- **Date**: 2026-11-22
- **Status**: done
- **相关**: docs/verification.md「长截图抓帧把覆盖层拍进结果」、crates/app/src/screenshot.rs `capture_region_below`

## 问题

长截图连续抓帧时覆盖层要留在屏幕上（用户要看边框、提示、已拼行数），所以靠
`NSWindow` 的 `sharingType = NSWindowSharingNone` 把它从捕获里排除。

实测这个属性**对显示器捕获无效**：`CGDisplayCreateImage` 生成的帧里仍有压暗层
和顶部提示条。后果不止是"图里多一条提示"——`scroll_delta` 是拿相邻帧做重叠
比对来估计滚动行数的，固定不变的覆盖层元素会污染匹配，滚动估计可能失准。

## 修复

mac 的抓帧改用 `capture_region_below(x, y, w, h, overlay_window_id)`：
`CGWindowListCreateImage` 带 `.optionOnScreenBelowWindow`，以覆盖层窗口为参照
逐屏合成。覆盖层在 `NSScreenSaverWindowLevel`（1000），比菜单栏（24）高，
所以"在它之下"只排掉覆盖层自己，菜单栏仍会出现在帧里。

## 判据

同一时刻同一区域对比两种抓法：含覆盖层的帧整屏压暗、提示条区域 p99=241；
排除后的帧无压暗、无提示条、菜单栏完整（同区域 mean=223）。
Windows 侧不变（`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` 是真生效的）。
