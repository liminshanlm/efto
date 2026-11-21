# 截图文字工具打不了字：pointerdown 默认动作抢走焦点

- **Date**: 2026-11-20
- **Status**: done
- **相关**: docs/verification.md（2026-11-20 截图文字焦点）、frontend/screenshot.js

## 现象

截图选区出现后选「文字」工具、在选区内点击，虚线文本框一闪即消失，无法输入任何字符。

## 根因

`#text-input` 的打开顺序是「pointerdown 处理器里 `focus()`」，但 **pointerdown 的默认动作在处理器之后
执行**：canvas 不可聚焦，浏览器默认把焦点移给 body → textarea 立即 `blur` → `blur` 监听里调用
`commitText()` → 内容为空 → 编辑器被关掉。用户看到的就是"点了没反应/一闪而过"。

## 修复

`pointerdown` 里 `tool === 'text'` 分支先 `e.preventDefault()`，取消这次点击的默认聚焦行为，
焦点留在刚 `focus()` 的 textarea 上。只对文字工具取消，其它工具/双击复制不受影响。

## 判据（为什么之前自测没抓到）

上一轮的无 GUI 自测用 `dispatchEvent(new PointerEvent(...))` 派发合成事件——**合成事件没有默认动作**，
所以"默认聚焦把 textarea 顶掉"这条路径根本不会触发，测试全绿而真实环境必现。
判据：凡是涉及焦点/默认动作/输入的交互，不能用合成事件断言，必须走 CDP
`Input.dispatchMouseEvent` / `Input.insertText` 这类**真实输入**。

## 验证

CDP（headless Edge + `--remote-debugging-port`，真实鼠标/输入事件）对比两份代码：

- 去掉 preventDefault 的旧版：点击后 `display=none`、`activeElement` 为空，键入内容丢失 —— 复现用户现象。
- 修复版：编辑器 `display=block`、`activeElement=text-input`、`Input.insertText('efto 中文')` 落入
  textarea，Enter 提交后撤销可用；矩形真实拖拽、双击复制提交 `screenshot_finish` 均正常。

未验证：真实 GUI 里中文输入法的组合输入（组字过程中的 Esc 行为）未手测。
