# 截图：框选后可拖动移动选区

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `frontend/screenshot.js`、`frontend/screenshot.html`、`docs/architecture.md`、
  `docs/help.zh-CN.md` / `help.en-US.md`、`docs/verification.md`、`docs/macos-parity-todo.md`

## 行为

没有激活标注工具时，pointerdown 落在已有选区内 → 整体移动选区（尺寸不变）；落在选区外
仍然重新框选。拖动时选区被夹紧在窗口内（`0..innerWidth-w` / `0..innerHeight-h`）。

## 取舍

- **标注随选区一起平移**：标注坐标是屏幕绝对坐标，不移的话拖动后标注会盖到别的内容上。
  `shiftShape` 按类型平移：画笔/荧光笔的点集、箭头两端、其余（矩形/椭圆/文字/马赛克）
  只动 `x/y`。马赛克会按新位置重新采样底层画面，语义上是"这块马赛克跟着框走"。
- **移动不进标注撤销栈**：撤销/重做只回退 `shapes`，若把"选区位置"也塞进历史，要么改
  整套快照结构，要么 Ctrl+Z 后标注回到原位而选区留在新位置（错位）。选择让移动本身不可撤销
  ——它只是一个摆放动作，拖回去即可。选区内单击/双击仍可正常复制。
- **第一次真正位移才收起浮层**：单击/双击也会先进入 moving 态，如果按下就隐藏工具栏，
  双击复制时界面会闪一下。
- **长图模式不参与移动**：长图按固定 x/y 区间滚动拼接，中途改选区会让已抓的帧对不上。
- 文本编辑态仍是 pointerdown 先 commit，和移动不冲突。

## 验证

- `node --check frontend/screenshot.js`；`node scripts/check-frontend.js` 通过。
- 未验证：真实 GUI 的拖动观感、多屏/DPI 下的边界夹紧、标注随动后的相对位置、拖动后
  双击复制/保存结果。详见 `docs/verification.md`「截图选区内拖动移动」。
