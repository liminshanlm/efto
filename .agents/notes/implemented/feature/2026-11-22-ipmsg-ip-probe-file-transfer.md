# ipmsg：按 IP 查询对方状态 + 发送文件

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `plugins/ipmsg/`、`docs/verification.md`、`docs/plugin-development.md`、
  `docs/macos-parity-todo.md`、`2026-11-22-ipmsg-plugin.md`

## 需求与边界

在既有「UDP 广播发现 + TCP 文本 + 首次通信同意」上补两件事：输入 IP 查询对方在线状态；
发送文件。仍然不改宿主：局域网能力全部留在 `service.js`，UI 只碰本机控制口。

## 按 IP 查询

- 广播在跨网段/防火墙只放行单播时收不到，因此探测走 **TCP `hello`**（默认 24250），
  拿到对方 `id/name` 后用给定 IP + 端口写进 `peers`，标记 `manual: true`。
- 手动 peer 的 `lastSeen` 不会被广播刷新，9s TTL 会让它自动消失——看起来像"查询成功
  9 秒后掉线"。所以信标定时器里顺带 `refreshManual()`：对超过 4s 没刷新的手动 peer 重发
  `hello`，失败就不刷新，交给 TTL 自然过期。
- 只接受 IPv4 字面量（UI 也要求输入 IP）；`hello` 回来是自己 hostname 时明确报
  "这是本机地址"，不静默成功。
- `/probe {ip, port}` 保留 `port` 参数给测试/自定义端口用，UI 默认只传 IP。

## 文件传输

协议仍是 efto 自定义、单连接、明文：

1. 发送端新开 TCP：`file-offer {name,size}`；
2. 接收端仅接受 `trusted` 里的对方，回 `accept`，之后同一连接的字节全部按 `size`
   流式写入 `~/Downloads/efto-ipmsg/<name>.part`，写完 `rename` 并回 `saved`；
3. 字节超过声明大小立即中断（协议错误），删半成品；连接提前关闭也删 `.part`。

关键决定：

- **接收端不能 `setEncoding('utf8')`**：原文本路径给 socket 设了 utf8，二进制会被替换
  字符破坏。改成 Buffer 累积、按字节找 `0x0A` 解析首行；`file-offer` 通过后剩余字节直接
  交给写流。
- **发送端不把文件字节搬进 webview**：UI 用 `efto.pickFiles()` 拿路径，`POST /sendfile
  {peerId, path}` 让本机 Node 自己读盘、自己连对端。避免几百 MB 走 HTTP→JS→TCP。
- **文件名是边界输入**：`path.basename` + 替换 Windows 非法字符 + 去前导点，
  `..\..\pwn.txt` 只会落成 `pwn.txt`；同名加 ` (n)`，且 `.part` 也参与查重，避免覆盖
  上次半成品。
- 授权复用文本那套：未 `approved` 先 `request`，被拒时把文件路径压进 outbox（与文本
  共用上限 20），收到 `approve` 后 `flushOutbox` 自动补发；文件在补发时被移动/删除则跳过。
- 收文件回执写进 `inbox` 文本消息（含落盘路径），UI 不用新增消息类型。

## 验证

- `node --check` service.js / ui.js；`node scripts/check-frontend.js`。
- 单机端到端 19 项全 PASS（真起 service.js，同机模拟对端 24299）：
  `/probe` 成功与非法 IP、手动 peer 过 9s 仍在线、未授权 `file-offer` 被拒、
  未同意 `/sendfile` 进 outbox 并在 `approve` 后自动补发、300KB 收文件字节一致 + 回执含
  路径、`..\..\pwn.txt` 清理、200KB `/sendfile` 对端字节一致、文件不存在明确报错。
- 未验证：两台真机、GB 级文件/中途取消、下载目录被 OneDrive 重定向、真实 UI 点击。
  详见 `docs/verification.md`「局域网消息插件 ipmsg」。
