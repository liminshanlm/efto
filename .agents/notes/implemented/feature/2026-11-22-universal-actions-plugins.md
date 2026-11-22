# 通用动作菜单调用翻译 / AI 插件

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `docs/architecture.md`（通用动作）、`docs/help.zh-CN.md` / `docs/help.en-US.md`、
  `docs/verification.md`（2026-11-22 通用动作菜单调用翻译 / AI 插件）、`docs/macos-parity-todo.md`

## 需求

配置了模型、且存在翻译 / AI 插件时，划词热键菜单要能直接调用这两个插件。

## 做法

- `universal::action_ids(text, translate, ai)` 增加两个可选动作 id `translate` / `ai`：
  - 只在调用方传入可用标记时出现；
  - 只在 `text.trim()` 非空时出现（空选区交给模型没有意义）。
- `commands::selection_plugins` 判定可用性：`Settings::models` 非空（`normalize_models` 保证
  有模型就有默认模型）且注册表 `get("translate")` / `get("ai-chat")` 命中（已启用、平台支持）。
- `commands::actions_run` 新增两个分支，复用 `open_plugin`：
  - `translate`：无 UI 后端插件，`efto.output` 经宿主 `output.set` 内联到主窗口；
  - `ai`：插件自己 `showUI`，payload 由 `onPluginEnter` 读取并 `ipcMessage('prefill')`。
- 执行前复核模型与选中文本；菜单展示后用户改设置的情况明确报错，不静默成功。
- 前端 `actions.js` 补两项标题/说明，两份 locale 各加两条文案。

## 取舍

- 插件用内置 id 直连（`translate` / `ai-chat`），不做"任意插件都能挂进划词菜单"的通用机制：
  需求只要这两个，泛化会带来菜单污染与未定义 payload 约定。
- 不居中排序到首位：`Enter` 仍执行首项"复制"，保持既有快捷键心智。

## 验证

`cargo test -p efto-app universal`（2 项）、`node --check frontend/actions.js`、
`node scripts/check-frontend.js`、`node scripts/check-i18n.js` 全绿。
GUI 端到端（配模型后点两项）未手测，已记入 `docs/verification.md`。
