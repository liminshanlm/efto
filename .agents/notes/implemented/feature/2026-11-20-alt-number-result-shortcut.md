# 搜索结果 Alt+数字 直接执行

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `docs/help.zh-CN.md` / `help.en-US.md` 快捷键表、`frontend/app.js` 键盘处理与行 tooltip

## 为什么

启动器的核心指标是"意图到动作"的击键数。`Alt+Space → 输入 → ↓↓ → Enter` 是 4 步；
结果列表支持 `Alt+N` 后变成 `Alt+Space → 输入 → Alt+N`。默认面板此前已有 `Alt+1..9`，
但搜索结果没有，属于同一交互模型里的缺口。

## 做了什么

- `frontend/app.js` 的 keydown：`Alt+1..9` 在**默认面板**仍映射第 N 张常用工具卡片
  （`toolHitIndex`，保持既有语义）；在**非面板列表**（搜索结果、窗口操作菜单、插件输出）
  映射 `hits[N-1]`，越界时不做任何事、不 `preventDefault`。
- 行 tooltip 前 9 行追加 `Alt+N` 提示（可复制行用 `main.row.copyWithKey`，
  普通行用 `main.row.runWithKey`，工具行复用已有的 `main.tile.runWithKey`）。
- i18n 新增两个 key（中英各一），`check-i18n` 396 keys 对齐。

## 判据

- 列表顺序即 `hits` 顺序：分组标题不是 hit，不参与编号，因此编号与可见行严格一致。
- 默认面板不改成"第 N 行"：面板前几行是最近使用，若按行编号会与帮助文档里
  "Alt+N = 第 N 个常用工具"冲突，也会让老用户按错。
- 只做快捷键，不新增可视数字徽标：行宽与对齐不变，提示放在已有的 hover tooltip 里。

## 验证

- `node --check frontend/app.js`、`node scripts/check-i18n.js`（396 keys × 2）、
  `node scripts/check-frontend.js` 通过。
- 未验证：真实 GUI 中按 `Alt+1` 执行第一行、越界时无副作用、窗口操作菜单内的 `Alt+N`。
