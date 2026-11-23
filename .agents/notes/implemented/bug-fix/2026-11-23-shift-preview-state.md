# Shift 预览偶发无反应

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `frontend/app.js`（`syncPreview` / Shift keydown）、`frontend/locales/*`、`docs/help.{zh-CN,en-US}.md`

## 现象

搜索结果里按住 Shift 预览，有时完全没有反应；同一类结果（如应用条目）也会间歇性失灵。

## 根因

1. **`previewOpen` 与实际窗口脱节**：宿主在结果不支持预览时会主动收起预览窗，但前端
   `syncPreview` 的 `!ok` 分支只清 `previewKey`、没清 `previewOpen`。下一次按 Shift 被当成
   "再按一次关闭"，调用的 `preview_hide` 本来就是空的 → 看起来按键失灵，要按第二次才有用。
2. **关闭判定只看 `previewOpen`**：预览开着、但选中项已经换成别的条目（松手后按方向键换行、
   或重新输入换了结果）时，Shift 被当作关闭旧预览，而不是刷成当前条目 → "按了没反应 / 没弹出我要的"。
3. **不支持预览的结果没有任何反馈**：插件动作、命令、网页搜索建议、复制项等走
   `preview_show` 的 `None` 分支，静默返回 `false`，用户无法区分"不支持"和"按键坏了"。

## 修复

- Shift 只有满足"预览开着 且 `previewKey` 就是当前选中项"才关闭；否则一律走刷新分支。
- `!ok`、无选中项、`preview_show` 抛错时都把 `previewOpen` 置回 `false`，状态与宿主一致。
- `!ok` 时用 toast 明确提示"该结果不支持预览"（新增 `main.status.previewUnsupported` 中英文案）。
- 保留 180ms 按住判定（避免 Shift 切中英文 / Shift+A 误弹）；快速点按仍不会弹预览。

### 追加：第 3 次起彻底没反应

用户实测"第一次开、第二次关、之后按 Shift 全无反应"，说明开关判定不能放在前端缓存上：

- `previewOpen` 是宿主窗口的副本，任何一处漏同步都会让后续 Shift 落进错误分支。
- `preview:closed` 监听里把 `shiftHeld` 置 `false`：用户仍按着 Shift 时事件先到，随后 keyup
  在 `stopPreview` 的 `if (!shiftHeld) return` 处早退，`preview_release` 不再上报。

改为宿主权威：

- 新增 `preview_toggle` 命令，按 `PREVIEW_ACTIVE` + `preview::current_id()` 判定这次按下是
  关掉（`hidden`）还是展示当前条目（`shown` / `unsupported`）；`preview_show` 抽出 `show_hit` 复用。
- `preview.rs` 记录 `hit.id`；`windows::show_preview` 多传 `hit_id`。
- 前端：`previewOpen` 只作缓存；本地认为已开时立即 `preview_hide`，否则带 180ms 判定调用
  `preview_toggle`；`preview:closed` 只清 `previewKey`/`previewOpen`，不碰 `shiftHeld` 与计时器；
  `syncPreview` 只在预览已开时跟随选中项。

## 验证

- `node --check frontend/app.js`、`node scripts/check-i18n.js`（464 keys）、`node scripts/check-frontend.js` 通过。
- `scripts/cargo.ps1 -Cmd "check -p efto-app"` 通过（仅既有的 dead_code 警告）。
- 逻辑核对四条路径：同条目再按关闭、换条目再按刷新、取消项再按提示、宿主收起后状态复位。

### 追加：轻点 Shift 永远不弹

新构建后用户反馈"第一次就没有打开预览"。用 CDP `Input.dispatchKeyEvent`（走真实输入管线）
对运行中的实例复测，宿主与状态机都是对的：

- `preview_toggle` 连调三次 → `shown` / `hidden` / `shown`，预览页 DOM 有内容。
- 每次按住 320ms 四连按 → 渲染计数 0→1→1→2→2，符合开/关/开/关。
- **轻点 60ms / 120ms → 渲染计数不变，无任何提示。**

根因：180ms 按住判定只实现了"按住到延迟后打开"，轻点在 keyup 处清掉计时器就结束，
而用户平时就是轻点 Shift。修复：`stopPreview` 里计时器还在、且没变成组合键时，
按轻点处理，直接 `togglePreview(true)`；`togglePreview` 增加 `releaseAfter`，
等宿主侧开关落地后再发 `preview_release`，避免 release/show 乱序把 `PREVIEW_HELD` 卡住。
Shift+A / Shift+方向键在 keydown 已清计时器并置 `previewSuppressed`，不受影响。

## 残留

- 真实 WebView2 下的 Shift 连续操作（开→关→开→关）未手测；180ms 内松手现在按轻点处理。
