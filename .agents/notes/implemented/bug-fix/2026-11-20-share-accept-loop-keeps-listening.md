# share 监听线程被一次 accept 错误带走

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `crates/app/src/share.rs`、`docs/verification.md`（2026-11-20 同题条目）

## 事实

`spawn_accept` 原先的写法是 `Err(_) => break`。Windows 上 `accept()` 会返回
`WSAECONNRESET`（客户端在 accept 完成前断开：浏览器预连接被放弃、页面请求被取消）。
一次这样的瞬时错误就让整个监听线程退出。

## 为什么表现成"只有别的机器下载失败"

- 端口仍处于监听态，SYN 仍由内核排队，已经打开的页面不会自己消失；
- 之后所有请求没人 accept，客户端等到超时，Chrome 下载项显示"失败 - 网络错误"；
- `share_status` 是快照，UI 不会知道线程已经死了，仍显示"正在分享"；
- 本机自测通常在页面刚打开时进行，那时线程还活着。

## 判据

- accept 错误（除 `WouldBlock`）一律 warn + 200ms 退避后继续，只有 `stop` 结束线程；
- 连接处理失败要带对端地址记日志，不能 `let _ = handle(...)` 整段吞掉。

## 没验证

- 真实构造"accept 前 RST"的复现；
- 修后在另一台机器上的实际下载——**已补齐**：10035 根因修复后，另一台机器（跨网段）
  用新 exe 实测下载成功（见下方"追加：真正的根因"）。

## 追加：https 升级

修完 accept 循环后远端仍报网络错误，定位到客户端把 `http://<局域网IP>:5421/` 升级成了
https。共享服务是纯 HTTP、无证书，TLS 握手永远不可能成功。现在明文端口收到 TLS
ClientHello（`0x16 0x03` 开头）会立刻断开并 warn 一条，避免被误判成传输故障；真正的
解法在客户端（关闭 HTTPS 优先 / 用 curl 验证），或在服务端另起一个自签证书的 TLS 监听
（代价是每台设备都要过证书警告，暂未实现）。

## 追加：真正的根因是 accept 出的流继承了非阻塞模式（10035）

accept 循环和 https 都是次要问题。给 `init_tracing` 加文件日志（`~/.efto/efto.log`）
后，远端下载瞬间抓到实锤：

```
WARN 共享连接处理失败（10.10.174.110:65431）：无法立即完成一个非阻止性套接字操作。 (os error 10035)
```

`TcpListener` 设了 `set_nonblocking(true)`（accept 轮询用），而 **Windows 上 `accept()`
返回的流会继承监听 socket 的非阻塞模式**。`handle()` 里的 `stream.read()` 在对方数据
尚未到达时立即返回 `WSAEWOULDBLOCK` 而不是等待——连接被当作错误掐断，客户端看到
"网络错误"。

为什么本机测试永远发现不了：loopback 上 connect→write 的数据在 accept 返回前就进了
内核缓冲区，read 总是立刻有数据。跨网段客户端有 RTT，accept→read 的间隙数据必然未到，
必失败。**之前专门写的 socktest 探针（测"接受的套接字是否阻塞"）也因同样的竞态得出了
错误结论——探针客户端的数据到得太快。**

修复：accept 后 `stream.set_nonblocking(false)`，恢复阻塞后 `set_read_timeout(5s)` 才
真正生效。回归测试 `serves_client_that_sends_request_after_a_delay`（客户端 connect 后
故意 sleep 200ms 再发请求），反向验证过：撤掉修复它必失败（10053 连接被中止，与真实
症状一致），恢复后通过。

排障链的教训：**stderr 日志对 GUI 程序等于没有**，`~/.efto/efto.log` 文件日志上线前
两轮修复（accept 循环、TLS 检测）都是在没有服务端证据的情况下推断的——方向有价值但
都不是根因。
