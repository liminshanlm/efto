# 设置页 `.desc` 英文断词换行（break-all → overflow-wrap: anywhere）

- **Date**: 2026-11-25
- **Status**: done
- **相关**: `frontend/settings.css`、`docs/images/settings.en-US.png`、`.agents/notes/implemented/process/2026-11-25-readme-screenshots.md`

## 为什么

`.desc`（设置页每行的说明文案）用的是 `word-break: break-all`，它允许在**任意字符**处换行。
中文没有空格，这条规则本来是给中文长句和长路径用的；但在英文界面下，一句正常的英文会在行尾
被硬生生切开："falls back t" / "o English"（渲染 README 的英文截图时肉眼可见）。

## 做了什么

`frontend/settings.css` 的 `.desc` 改为 `overflow-wrap: anywhere`：单词能放下就按词换行，
只有"一个词本身比整行还宽"（超长 URL、无空格串）时才断在字符处——正是这条规则原本想覆盖的场景。

**没有动**同一文件 `.plugin-meta`（第 449 行附近）的 `break-all`：那是等宽字体的插件元信息，
内容多为路径/标识，按字符断是刻意的。`frontend/preview.css`、`actions.css`、`clipboard.css`
以及各插件样式同理，本次不改。

## 判据 / 结果

- 重新跑 `scripts/gen-readme-shots.mjs` 生成设置页截图，英文版由 "falls back t / o English" 变成
  "falls back / to English"、"on macOS, Alt is the / Option key"；中文版排版与改前一致（无回归）。
- `node scripts/check-frontend.js` 通过（该守卫只查 `.hidden` 与 `display` 冲突，不受影响）。
- 属于两平台共用的前端样式，无需登记 `docs/macos-parity-todo.md`（规则只登记有平台差异的
  用户可感知行为，这一行在 Windows 与 macOS 上表现相同）。
- 未在真实 webview 里手测：深浅两套主题与中英两种语言下 `.desc` 的观感只核对过渲染截图。
