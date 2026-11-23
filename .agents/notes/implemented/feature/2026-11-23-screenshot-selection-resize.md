# 截图选区支持拖右/下边框与右下角改大小

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `frontend/screenshot.js`、`frontend/screenshot.html`、`docs/architecture.md`、
  `docs/help.{zh-CN,en-US}.md`、`docs/verification.md`

## 需求

框选之后只能整体移动或重拖空白处重选，尺寸定死了。要能像截图工具的抓手那样，
把鼠标移到右/下边框或右下角上拖动改大小。

## 改动

- `frontend/screenshot.js`：
  - 新增 `resizeEdgeAt(p)`：以框线为中心、两侧各 6px 的命中带，返回 `'e' | 's' | 'se'`；
    `edgeCursor()` 给对应的 `ew-resize` / `ns-resize` / `nwse-resize`。
  - `pointerdown` 里**先判边框再判「选区内移动」**——命中带与选区重叠，顺序反了右/下边框
    永远会被当成移动。判定与移动一致地限定在 `!tool && !longMode`（有标注工具时仍然优先画标注）。
  - `pointermove` 的 `resizing` 分支只改 `w`/`h`、原点不动：`e` 改宽、`s` 改高、`se` 两个都改；
    夹紧到窗口内、最小 3px（与 `insideSel` 的下限一致，再小就没法移动/双击了）。
    真的改了尺寸才收浮层，单纯点一下边框不闪。
  - `pointerup` 收尾并 `placeBar()` 重新贴住新选区。
- 标注不随改大小平移（`drawAnnotations` 本来就按选区裁剪，缩小就裁掉），也不进撤销栈。
- 提示文案 + 中英帮助文档 + `docs/architecture.md` 的覆盖层交互段落同步。

## 判据

- 为什么不给左/上边框也做：用户明确要的是右、下、右下角；加上另外三条边会让
  "选区内拖动 = 移动" 的可用区域被命中带吃掉太多，收益不对称。
- 为什么命中带 6px：框线只有 1px，不给容差根本点不中；6px 以内、且不占满整个角
  （角落 6×6 单独判为 `se`）足够可点又不至于误触。
- 为什么不缩放标注：改大小只是改取景范围，标注形状保持原样更符合"先截图再调框"的直觉；
  需要整体缩放时移动/重画更直接。

## 验证

`node --check frontend/screenshot.js`、`node scripts/check-frontend.js` 通过；
`scripts/cargo.sh "build -p efto-app"` 通过。GUI 手测见下。

## 没验证

- 真机 GUI：右/下边框与右下角的命中手感、Retina 与外接屏下的坐标跟随、改大小后工具栏
  重新贴位、与双击复制/长图模式的互不干扰都还没点过。
