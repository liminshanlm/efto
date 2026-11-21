# 截图标注：Snipaste 风格工具栏与合成提交

- **Date**: 2026-11-20
- **Status**: done
- **相关**: docs/architecture.md（截图标注小节）、docs/verification.md（2026-11-20 截图标注）、docs/help.zh-CN.md / help.en-US.md、frontend/screenshot.{html,js}、crates/app/src/{screenshot.rs,commands.rs}

## 需求

截图工具原本只有复制/保存/长图/钉在屏幕。用户要求实现 Snipaste 截图工具栏的标注能力
（参考图：矩形/椭圆、画笔、荧光笔、马赛克、文字、橡皮 | 撤销重做 | 取消/钉住/保存/复制）。

## 方案与关键决策

1. **标注画在前端，宿主只收成品**。所有形状画在覆盖层 canvas（裁剪到选区）；提交时前端把
   「选区原始画面 + 标注」合成物理像素 PNG data URL，经 `screenshot_finish` 的 `overlay`
   参数回传，宿主 `screenshot.rs::overlay_png` 解码后直接替代 `Shot::crop`。
   - 为什么不在宿主重绘：宿主拿到的只有像素级形状描述才行，等于做第二套绘制引擎；
     前端已有 canvas 上下文与 scale 换算，合成一次就够了。复制/保存/钉图三路共用该入口。
   - `overlay` 可缺省：没画标注时走原 `crop` 路径，旧行为零变化。
2. **工具语义**：矩形/椭圆/箭头/马赛克一次成型后自动退出工具；画笔/荧光笔/橡皮保持连续。
   数字键 1-8 切换。橡皮按形状包围盒（文字用 measureText 测宽）+8px 命中删除。
3. **撤销/重做用快照栈**（上限 60）：形状间存在删除（橡皮）操作，命令式 diff 不值当。
4. **长图与标注互斥**：拼接管线按"原始画面帧"工作，标注层无法参与；有标注时「长图」置灰。
5. **顺带修的既有 bug**：纯点击会清空选区，"双击复制"实际失效——用 `selBeforeDrag`
   在纯点击（无拖拽位移）时恢复上一个选区。

## 踩坑判据

- **strict mode 下读未声明变量直接 ReferenceError**：重写 screenshot.js 时丢了
  `longMode`/`longRows`/`longTimer` 的 `var` 声明，首次拖选区就崩；`node --check`
  只查语法不查引用，查不出来。判据：IIFE 重写后必须 grep 一遍所有被引用的状态名。
- **headless 自测装置**：桩 `__TAURI_INTERNALS__` + 合成 PointerEvent
  （需 no-op 掉 `setPointerCapture`，合成事件没有真实 pointer 会抛 NotFoundError）
  + headless Edge `--dump-dom --virtual-time-budget`，结果写进 DOM 再从 dump 里读。
  装置必须用显式 UTF-8 读写构建测试页：PS 5.1 `Get-Content` 默认 ANSI 读 UTF-8 会
  双重编码，吃掉中文 title 属性里的引号、把后续按钮变成文本节点——症状是
  `getElementById` 拿到 null、脚本初始化半途而废，极具迷惑性（window.onerror +
  `--allow-file-access-from-files` 才定位到）。

## 验证程度

- `cargo test -p efto-app` 107 项通过（含 `overlay_png` 往返与坏输入两个新测试）。
- 前端 headless 自测 40/40：8 工具、撤销重做、色板、粗细、快捷键、文字提交、橡皮选择性
  （像素级断言）、overlay 合成（尺寸/背景/描边颜色/文字渲染）。
- 未验证：真实 GUI 的多显示器/DPI 坐标换算、马赛克颗粒度、输入法组合输入、钉图尺寸适配
  （headless 里 scale=1）。见 docs/verification.md。
