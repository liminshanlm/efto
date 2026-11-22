# 常用工具支持拖动调整顺序

- **Date**: 2026-11-22
- **Status**: done
- **相关**: frontend/settings.{html,js,css}、docs/architecture.md、docs/help.{zh-CN,en-US}.md

## 需求

设置页只能勾选常用工具，顺序由"勾选历史 + BUILTIN_ORDER 追加"决定，用户无法调整。
而顺序直接决定默认面板的展示顺序与 `Alt+1..9` 的编号，必须可拖动重排。

## 实现

- 新增「面板顺序」卡片（`#toolOrderCard`）：逐行显示已加入的工具，带拖动手柄、序号、
  名称与类型标签（内置/应用/插件）；`app:`/`plugin:` 条目额外给「移除」按钮
  （内置工具的增删仍走上面已有的勾选框，避免两套入口）。
- **拖动用 pointer 事件**，不用 HTML5 dnd：WKWebView 里 `dragover` 不稳。
  拖动中只在"越过相邻项中线"时交换 DOM 位置，`pointerup` 后按 DOM 顺序回写
  `toolOrder`，随后 `renderToolOrder()` 重编号。
- 保存以 `toolOrder` 为准（不再是"从旧 tools 过滤 + 按 BUILTIN_ORDER 补"），
  但 `reconcileOrderAgainstFresh` 会把"设置页打开期间主面板新加的 app:/plugin: 条目"
  追加到末尾而不是丢弃。

## 踩到的坑（重要）

`reconcileToolOrder()` 依赖**勾选框的 checked 状态**取舍内置工具。首版把它放在
`renderSettings` 里"给 `$('toolScreenshot').checked = ...` 赋值之前"，导致顺序列表
初始化时所有内置工具都被判为"未勾选"而删除，列表只剩 `app:` 条目
（实测 `before=app:/applications/deepseek harness.app`，且提示 "too few items"）。

**判据**：任何依赖 DOM 控件状态的派生渲染，必须排在控件赋值之后。

## 验证

用 pointer 事件在真实 webview 里模拟"把第 1 项拖到最后"，宿主日志记录前后顺序：

```
PROBE before=clipboard,shortcut_guide,screenshot,app:/applications/deepseek harness.app
PROBE after=shortcut_guide,screenshot,app:/applications/deepseek harness.app,clipboard
PROBE saved=shortcut_guide,screenshot,app:/applications/deepseek harness.app,clipboard
```

`saved` 与磁盘 `~/.efto/settings.json` 的 `tools` 一致 → 拖动结果真的落库。
截图确认 UI：4 行带手柄/序号/类型标签，`clipboard` 已到第 4 位，
「顺序已调整，点"保存设置"生效」提示正常。

- `cargo test --workspace`：438 passed。
- `check-i18n`（449 keys）/ `check-frontend` / `scripts/verify.sh` 全通过。

## 追加（同日）：支持在搜索框面板里直接拖动

用户提出"直接在搜索框里面可以调整顺序"，于是同一套排序又实现了一份面板内拖动。

- 工具卡片（`group === 'tools'` 且带 `hit.pin`）在 `makeTile` 里记录 `dataset.pin`
  并调 `attachToolDrag()`。
- 拖动结束后按 DOM 顺序读回 `data-pin`，直接 `set_settings { tools }` 保存并
  `search()` 刷新（面板里的序号 `Alt+1..9` 由 `renderPanel` 重算）。
- 与设置页那份实现共享两个约定：pointer 事件（不用 HTML5 dnd）、
  越过邻居中线才换位。
- 新增 `suppressTileClick`：松手后浏览器会补一个 click，不挡掉就会把工具执行掉
  （`makeTile` 的 click 里判这个标志）。
- 拖动阈值 6px，避免轻微抖动被当成拖动；点右上角 `×`（`.tile-remove`）不进入拖动。

**判据**：面板里 `document.querySelector('.tiles')` 的第一个是"最近使用"组
（卡片没有 `data-pin`），找工具组必须用 `.tile[data-pin]` 的 `parentNode`，
否则拖动会挂到错误的容器上（诊断时就踩了这个，`panel-before` 为空）。

### 验证

用 pointer 事件在真实面板里模拟"把第 1 张卡拖到最后"：

```
PROBE panel-before=clipboard,shortcut_guide,screenshot,app:/applications/deepseek harness.app
PROBE panel-after=shortcut_guide,screenshot,app:/applications/deepseek harness.app,clipboard
PROBE panel-saved=shortcut_guide,screenshot,app:/applications/deepseek harness.app,clipboard
```

截图确认：面板里 4 张卡片重排后序号跟着变（剪贴板从 1 → 4），
搜索框为空时即默认面板态。
