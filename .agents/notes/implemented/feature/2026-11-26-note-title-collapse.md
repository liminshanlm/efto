# 便签支持改标题与折叠正文

- **Date**: 2026-11-26
- **Status**: done
- **相关**: `docs/architecture.md`（便签纸 / 便签折叠）、`docs/verification.md`
  （便签标题与折叠）、`docs/help.zh-CN.md` / `docs/help.en-US.md`、
  `docs/macos-parity-todo.md`

## 需求

便签要能改标题；正文要能收起和展开。

## 实现

- `Note` 增加 `title` 与 `collapsed`（都 `#[serde(default)]`，老 `notes.json` 直接兼容），
  补 `set_title` / `set_collapsed`。
- `note_load` 从返回字符串改成 `{title, text, collapsed}`；`note_save` 多收一个 `title`。
- 新增 `note_toggle(id, collapsed)`：窗口在 `NOTE_COLLAPSED_HEIGHT`（34px，只留标题栏）与
  存储的展开高度之间 `set_size`；`open_note` 按 `collapsed` 决定初始高度，重启沿用收起状态。
- `note.html` 的标题从静态文字换成 `input`（留空显示占位「便签」），新增 `▾/▸` 折叠按钮；
  折叠态用 `body.collapsed` 隐藏正文与页脚。标题栏拖动同时排除 `button, input`，
  否则点标题会变成拖窗。

## 关键判据

- **`height` 只表示展开高度**。收起前先把当前几何（含刚拖过、还没保存的）记下来；折叠期间
  `note_save` 只看位置/宽度，读存储里的高度回填，不能让 34px 覆盖掉展开高度——否则展开
  后窗口回不到原大小。
- 折叠按钮点击时先 `save()` 再 `note_toggle`：两条 IPC 无论谁先到都安全（`note_save` 在
  折叠态会保留存储高度；`note_toggle` 收起时会重新记录几何）。

## 验证

- `cargo test -p efto-app notes` 4 项通过，新增 `title_and_collapsed_roundtrip`
  （JSON 往返 + 老文件缺字段默认空标题/未折叠）。
- `node --check frontend/note.js`、`cargo build -p efto-app` 通过。
- **没验证**：真实窗口折叠后布局是否立即回流、折叠态拖动后重启位置是否保留、
  高 DPI 下 34px 标题栏是否够用（需 GUI 手测）。
