# 网页搜索设置页控件挤在一起

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `frontend/settings.css`（`.snippet-list` / `.snippet-head` / `.engine-url`）、`docs/verification.md`

## 现象

设置 → 网页搜索：每个搜索引擎的「关键字 / 名称 / 移除」紧贴卡片左右边缘，URL 输入又比上面一排矮，
两行引擎之间没有呼吸感，整体像糊在一起。

## 根因

`.snippet-list` 放在 `padding: 0` 的 `.card` 里，自己既没有内边距也没有外边距，输入框只剩卡片
`6px` 的默认缝；同一行里 `.text-input`（13px 字号）、`.snippet-remove`（固定 30px 高）、
`.engine-url`（12px 等宽）三种高度，视觉上必然参差。

## 修复

- `.snippet-list`：`padding: 14px 17px`、`gap: 12px`（与 `.card > .row` 的 `13px 17px` 对齐）。
- `.snippet-row`：gap `10px`、`padding-bottom: 14px`。
- `.snippet-head` 内 `.text-input`、`.snippet-remove` 与 `.engine-url` 统一 `height: 36px`。
- 片段页复用同一套 class，一并受益；未新增任何页面专属规则。

## 验证

无头 Edge 760x620 注入两行引擎卡片：`listPad=14px/17px`、`gap=12px`，
关键字/名称/移除/URL 四控件均 `36px` 高，行间 `12px`，520px 内容宽下未溢出。
未在真实 WebView2 设置窗口截图核对。
