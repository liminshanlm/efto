# Markdown 插件提示外部修改并支持重新加载

- **Date**: 2026-11-26
- **Status**: done
- **相关**: `docs/verification.md`（Markdown 插件外部修改提示）、
  `docs/help.zh-CN.md` / `docs/help.en-US.md`、`docs/macos-parity-todo.md`

## 需求

打开的文件被其他地方改过以后，要提示刷新内容，避免拿着过期内容继续编辑/保存。

## 实现

纯插件前端，宿主零改动：

- `plugins/markdown/ui.js`：
  - `state` 增加 `lastDisk`（最后一次从磁盘读到或写回的内容）、`saving`、`polling`。
  - `applyDoc` 把新内容同时记成 `lastDisk` 并隐藏横幅。
  - `pollDisk` 由递归 `setTimeout`（`schedulePoll`，默认 2s，文件 >2 MiB 时 10s）驱动，读
    `efto.file.read(state.path)` 与 `lastDisk` 比对内容；不同则抬 `lastDisk` 到新值并显示横幅
    （有未保存修改时文案额外提示会丢弃）。`state.bytes` 取打开/读盘返回的 `bytes`。
  - `reloadFromDisk` 复用 `confirmDiscard()`，重新读盘走 `applyDoc`。
  - `save` / `saveAs` 在发送时快照 `state.text`，成功后把 `lastDisk` 设成该快照。
- `plugins/markdown/index.html`：`#workspace` 前插入 `#extBanner`（文案 + 「重新加载」+「知道了」）。
- `plugins/markdown/style.css`：`.ext-banner` / `.ext-text` / `.ext-btn`，用现有 accent 令牌。

## 关键判据

- 宿主没有 stat/mtime/watch API，只能用轮询读内容比对；按内容比对不受 mtime 粒度影响，
  写回后立刻拿旧 mtime 也不会误判。代价是每 2s 读一次文件（上限 8 MiB）。
- **三条防误报**：`saving` 期间跳过轮询；写回成功时 `lastDisk` 用发送时的快照而不是
  写完再读（避免这期间的编辑被当成内容差异）；发现变化后立刻把 `lastDisk` 抬到新值，
  同一处改动只提示一次，点「知道了」之后也不会再弹。
- 轮询读取失败（文件被删、权限变化）静默跳过，不打断编辑。
- 窗口隐藏（最小化/被完全遮挡，`visibilityState === 'hidden'`）时不读盘，隐藏期间计时器本身
  也被 WebView2 节流；`visibilitychange` 回到前台立刻补一次，避免外部改动等到下一个 2s。
- 文件超过 2 MiB 时轮询从 2s 降到 10s：大文件每次都要整份 `fs::read` + UTF-8 校验 + 跨 IPC
  拷贝，2s 一次会有可见的 CPU/内存抖动。间隔由递归 `setTimeout` 按 `state.bytes` 决定，
  保存后不立刻改 `bytes`（下一次轮询自愈），最多多付一个周期的 2s 读。

## 验证

- `node --check plugins/markdown/ui.js` 通过；`node plugins/markdown/tests/run-all.js` 24 项通过。
- **没验证**：真实窗口里外部改动后横幅是否出现、「重新加载」在有未保存修改时是否先
  `window.confirm`、「知道了」是否只关提示、自己保存是否不误报；接近 8 MiB 文件每 2s
  读取的实际开销；文件被删除后的表现（需 GUI 手测）。
