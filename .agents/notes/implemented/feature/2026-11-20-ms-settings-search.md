# ms-settings: 系统设置页搜索源

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/core/src/search/win_settings.rs`、`crates/core/src/search/service.rs`、
  `crates/app/src/{commands,cli}.rs`、`docs/{architecture,help.zh-CN,help.en-US,verification}.md`

## 现象

输入 `ms-settings:bluetooth` 没有任何结果：`external_hit` 只把 `scheme://…` 当 URI，
`ms-settings:` 这种无 `//` 的协议进不了直连分支；同时也没有任何源负责列出设置页。

## 做法

- 新增 `search/win_settings.rs`：**整串以 `ms-settings:` 开头**才触发。
  - 冒号后为空：按内置常用顺序列出设置页（`display` / `bluetooth` / `windowsupdate` …）。
  - 有内容：按 URI 精确/前缀/包含、中文标题、英文别名过滤。
  - 内置表未收录的 URI：保留一条「打开 ms-settings:…」行，交给 `shell_open`。
- 结果行是 `ItemKind::External` + `HitPayload::Open`，与输入 URL/路径共用执行路径，
  因此保持"单击选中、双击/Enter 打开"。
- 冒号后已输入内容时短路独占结果：否则收藏夹会对 `ms-settings:bluetooth`
  这类长串做子序列匹配，混进无关书签。只输入 `ms-settings:` 时不短路，
  允许已有的根设置条目排在列表前面。
- GUI 的 `search_files` 与 CLI `search` 在文件源前加 `win_settings::is_query` 闸门，
  避免 `ms-settings:` 后面补一串 Everything 文件。

## 判据

- 严格前缀触发，普通输入（`display`、`设置`）与缺冒号的 `ms-settings` 一律不触发，
  符合"搜索源不许互相劫持"。
- 列表分数 700 左右，低于 `ms-settings:` 根条目的应用精确匹配（≈900），
  所以直接回车仍打开设置首页，列表只做候选。
- 未知 URI 不静默失败：保留一条打开行，由系统决定是否识别。

## 验证

- `cargo test -p efto-core search::` 通过（新增 6 个用例，含 service 层独占/不劫持断言）。
- `cargo check -p efto-app`、`cargo build -p efto-app` 通过。
- 真实 CLI 验证：`cli search "ms-settings:"` 列出显示/蓝牙/网络等；
  `ms-settings:bluetooth` 精确命中且只出设置源；`ms-settings:blue` 过滤出蓝牙；
  普通 `display` 不出设置页。
- 未做：真实 GUI 下逐项点击设置页、旧版 Windows 缺少某些 URI 时的行为。
