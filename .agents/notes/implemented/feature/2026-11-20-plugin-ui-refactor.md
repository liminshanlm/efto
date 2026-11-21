# 带 UI 插件统一重写：设计令牌与主应用对齐

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `frontend/settings.css`、`docs/verification.md`、`plugins/ai-chat`、`plugins/file-share`、`plugins/hosts`、`plugins/ocr`、`plugins/quickweb`、`plugins/text-beautify`、`plugins/hello-js`

## 为什么做

七个插件的页面各自长一套：配色、圆角、顶栏高度、空状态和控件密度都不一致，还夹着第三方启动器的痕迹。
目标是在**不改消息契约与功能**的前提下，用同一套设计语言重写结构与交互，让插件页看起来就是主应用的一部分。

## 关键决定

### 设计令牌只有一个来源

- 颜色、圆角、边框、hover、阴影全部照抄 `frontend/settings.css`，不另立一套变量。
- 窗口统一为 52px 顶栏（品牌块 + 插件名 + 右侧操作）、卡片内容区、9px 控件圆角、14px 卡片圆角、
  10px 细滚动条、`focus-visible` 焦点环、`prefers-reduced-motion` 降级。
- 主题写 `document.documentElement.dataset.theme`，用 `efto.isDarkMode()` 判断浅深色。

### 只重写壳，不动契约

- 保留全部 `efto.ipcMessage` / `efto.onMessage` 消息名和宿主调用；`text-beautify` 的 `ui.js`、
  `main.js` 一行未改，保证既有测试不受影响。
- 动态生成的 DOM class 与新 CSS 对齐（如 hosts 删除按钮的 `btn ghost small`）。
- 用户可见的第三方启动器文案全部清掉：quickweb 清单的 `homepage: https://u.tools`、
  README、注释，以及 dev-tools 的两份 README；`plugin-sdk/efto.d.ts` 与
  `docs/plugin-development.md` 里“对应 uTools 的 setFeature”之类的表述改成中性描述。

### 顺带修掉的真 bug

- `efto.isDarkMode` 是函数，`ai-chat` / `ocr` 当布尔用，主题永远深色。
- `file-share` 用 `statusEl.textContent` 更新状态，会把状态圆点 `<i>` 一起清掉。
- `hosts` 动态删除按钮缺 `btn` class，新样式没命中。

## 验证

- 8 个改动的 JS 文件 `node --check` 全过。
- HTML `id` 与 JS 引用比对：7 个插件引用齐全，无悬空 ID。
- `plugins/text-beautify/tests` 7 个文件全过（JS 68 / JSON 31 + 400 轮属性测试 / main 15 /
  SQL 169 / UI 37 / XML 19）。
- Playwright + 本机 Edge 逐页截图（1024×650）；hosts / quickweb 用临时 `window.efto` 桩喂样例数据。
  没有溢出、错位或控件重叠。

## 明确没做 / 没验

- 没在真实 WebView2 的 `efto://` 页面里手测：拖拽、原生文件选择框、流式消息、模型下拉仍只有静态验收。
- 没有真实 DeepSeek Key，ai-chat 的联网/OCR 内容渲染未过样。
- dev-tools 为第三方预编译包，内部仍有不可见的 `utools` 适配分支，不反编译；
  `plugins/quickweb/res/ubrowser.png` 按用户要求保留。
- `scripts/check-compat.js` 是旧兼容守卫，与 AGENTS.md“不提供兼容层”冲突，运行会失败；
  它未接入 `verify.ps1`，本次未动。
