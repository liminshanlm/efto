# ai 流式测试偶发 EINVAL：fake server 没读完请求就关连接

- **Date**: 2026-11-21
- **Status**: done
- **相关**: `crates/app/src/ai.rs`（`chat_stream_reads_sse_from_http`）

## 现象

`ai::tests::chat_stream_reads_sse_from_http` 在 macOS 上约一半概率失败：
`模型响应读取失败: Invalid argument (os error 22)`，单测单独跑（无并发）也复现，
通过/失败交替出现。

## 根因

测试里的 fake TCP server 只 `read` 一次请求就写响应、随即关闭 socket，**没有消费请求 body**。
服务端 socket 上还有未读数据时 close 会发 RST 而不是 FIN，客户端 `read_line` 读响应体
拿到 EINVAL。请求恰好一次读完时没事，半包时就翻车——所以是 50% 级别的抖动，
不是时序慢的问题（加重试/延时都救不了）。

## 做法

server 按 `Content-Length` 把请求（头 + body）读干净再回包。修后连跑 8 次全过。

## 判据

- `for i in 1..8; do cargo test -p efto-app ai::tests::chat_stream; done` 8/8 通过。
- 这也解释了为什么之前"看起来偶尔抽风"的网络测试多半是同类竞态：**测试里的
  fake server 必须把请求收干净**，这是断言之外的隐性契约。
