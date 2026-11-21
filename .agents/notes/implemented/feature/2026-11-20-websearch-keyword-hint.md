# 只输入网页搜索触发词时给出用法提示

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/core/src/websearch.rs`、`crates/core/src/search/service.rs`、
  `crates/app/src/commands.rs`、`docs/{help.zh-CN,help.en-US,architecture,verification}.md`

## 现象

输入 `g` 时什么提示都没有（关键词+正文才触发搜索），用户不知道这个引擎存在、
也不知道该在后面接什么。

## 做法

- `websearch::hint()`：整串（trim + 忽略大小写）等于某引擎 `keyword` 时返回提示行，
  850 分、`HitPayload::Action("websearch_hint")`，标题「用 Bing 搜索…」，
  副标题给 `g rust` 示例。
- `service::query` 以**附加分组**并入，不短路本地结果；分数低于前缀匹配应用（约 900），
  应用永远排在提示前面。
- 宿主 `run_action` 对 `websearch_hint` 只回一句用法，不发请求。

## 判据

- 不短路：单字母 `g`/`b` 很可能同时是应用搜索输入，短路会直接劫持（违反"搜索源不许互相劫持"）。
- 850 分：高于"包含匹配"（700）、低于前缀匹配（900），提示可见但不抢真实应用。
- 提示行不复用 `HitPayload::WebSearch`：避免"只敲了关键词就按 Enter 打开搜索引擎首页"这种无意义跳转。

## 验证

- `cargo test -p efto-core` 218 passed，新增两个测试（见 verification.md）。
- 未做：真实 GUI 观感、回车提示文案的展示。

## 备注

`websearch::parse` 的行为完全没动；带正文时仍是"短路 + 只返回搜索行"。
