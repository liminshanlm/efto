# 快捷键指南：分组卡片被压扁、无滚动条

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `frontend/shortcuts.css`、`docs/verification.md`「快捷键指南只验了数据层」

## 现象

快捷键指南窗口在条目多时，卡片高度被压缩，组内快捷键行被裁掉，右侧也不出现滚动条。
用户看到的是"内容超出范围却被遮挡"。

## 根因

`.groups` 是列向 flex 容器且 `overflow-y: auto`；`.group` 作为 flex item 默认 `flex-shrink: 1`，
又因为自身 `overflow: hidden`，`min-height: auto` 计算为 0。于是分组被压缩到容器高度内，
内部 `.items` 被 `overflow: hidden` 裁掉——**永远不会溢出，自然也不会有滚动条**。

这是 flex 自动最小尺寸规则（overflow 非 visible 时失效）与"滚动容器里的卡片"叠加的典型坑，
与窗口大小、数据量无关：只要组数够多就会复现。

## 修法

- `.group` 加 `flex: none`，卡片按内容高度排布，由 `.groups` 负责滚动。
- 给 `.groups` 补与其他窗口一致的滚动条样式（`scrollbar-width: thin` +
  `::-webkit-scrollbar` 10px、透明 track、`--line-strong` thumb），并补 `--line-strong` 令牌
  （深/浅色各一）。WebView2 默认是随滚动浮现的覆盖式滚动条，不显式定义时用户看不到。

## 验证

- 760x620 无头 Edge 注入 12 组、每组 3 条：修复前每组只剩标题（截图比对），
  修复后 `scrollHeight=2116 / clientHeight=385`、`canScroll=true`、滚动条占 10px
  （`offsetWidth-clientWidth`），卡片完整展开。
- 未验证：真实 WebView2 窗口中的滚动观感与鼠标滚轮；折叠/筛选交互未回归手测。
