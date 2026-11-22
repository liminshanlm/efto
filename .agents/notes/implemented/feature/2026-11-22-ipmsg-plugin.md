# ipmsg 插件：局域网文本消息（精简版飞鸽传书，纯插件 + 本机 Node）

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `plugins/ipmsg/`、`docs/plugin-development.md`（`efto.node` 长驻服务模式）、
  `docs/verification.md`、`docs/macos-parity-todo.md`

## 目标与边界

两台机器都打开插件即可互相发现，只发文本、不传文件；主动发消息前必须对方同意。
没有就"是否兼容标准 IPMSG 协议"展开实现：按 efto 自有精简协议落地，原因是标准 IPMSG
的命令码/编码/文件传输语义会显著扩大范围，而本次用户要的是"精简版、只要发消息"。

## 为什么不在宿主实现

第一版把 UDP 发现 + TCP 收发做成了宿主能力（`efto.ipmsg`，`crates/app/src/ipmsg.rs`），
已通过 3 个 Rust 单测。用户随后明确要求：**不要改宿主，让插件用 Node 的能力，
本地有 Node 环境才能启用，尽量保持宿主小而健壮**。据此回滚了全部宿主改动
（`host.rs` / `lib.rs` / `state.rs` / `prelude.rs` / `efto.d.ts`），删除 `ipmsg.rs`。

`efto.node.run` 本身是一次性脚本（带超时、会被 kill），不能常驻监听。解法：

1. main.js 用 `efto.node.run` 执行 bootstrap；
2. bootstrap 探测 `127.0.0.1:24251` 上的旧实例，没有就
   `spawn(process.execPath, ['service.js', token], { detached: true, stdio: ['ignore', fd, fd] })`
   + `unref()`。孙进程不在宿主 `child.kill()` 的射程内，服务得以常驻；
3. UI（webview）直接 `fetch` 控制口，main 只下发端口和令牌。

代价是插件**依赖本机 Node**：没装 Node 或 设置 → 运行时 未配置/检测失败时，插件明确
显示"需要 Node"并提供"重试启动"，不静默失败。

## 端口与协议

- 发现：UDP 24250，每 2s 向 `255.255.255.255` 发
  `{"efto_ipmsg":1,"id","name","port"}`；9s 无信标剔除。
- 消息：TCP 24250，连上发一行 JSON、读一行回复后关闭。
  `type` = `hello` / `request` / `msg` / `approve` / `reject`。
- 控制口：HTTP `127.0.0.1:24251`，`/status` `/init` `/send` `/respond` `/stop`。
  令牌走查询串 `?t=`，POST body 用 `text/plain`，避免 CORS 预检；服务端回
  `Access-Control-Allow-Origin: *` 与 `Allow-Private-Network: true`。
- 授权按方向独立：A 首次 `send` → `request`；B 的 UI 出现 `requests` → `respond(accept)`；
  B 记 `trusted`，A 收 `approve` 记 `approved` 并重投 outbox（上限 20 条）。
- 全明文，只面向可信局域网；版本号或 id 校验失败明确拒绝，不静默成功。

## 生命周期

- UI `pagehide` 时 `navigator.sendBeacon('/stop')`；main 不再重复停服
  （会和"刚关就重开"抢跑）。
- 兜底：服务 30 分钟没收到 `/status` 轮询就自退。用 `/status` 而不是 webview 定时器
  作为活性信号，是因为后台 webview 的定时器会被节流，容易误判空闲。
- 令牌持久化在插件 kv，bootstrap 用它探测旧实例，避免重复起服务。

## 踩到的坑

1. **广播早于 setBroadcast**：最初信标 `setInterval` 写在模块顶层，早于 `udp.bind`
   回调里的 `setBroadcast(true)`。首包可能以 EACCES 触发 socket `error`，被
   `shutdown(1)` 直接带走。改为在 bind 回调里启动信标。
2. **`/respond` 不等于清空自己的 outbox**：`respond(accept)` 只是本机同意收对端的消息
   （记 `trusted` + 回 `approve`）。自己的 outbox 要等**收到对方 `approve`** 才重投。
   端到端脚本第一版把两者混为一谈，测出"接受后 outbox 未清空"，是测试写错而非实现错。
3. **UI 侧 `efto.kv.get` 是异步的**：如果 svc 消息先到就 `/init`，会把服务端信任列表
   覆盖成空。加 `localReady` 门，kv 载入完成后才允许连接。
4. **空令牌**：控制口令牌是唯一本地鉴权，空串会让任何本机进程都能调。service.js
   启动时无令牌直接退出码 2。

## 验证

- `node --check` main.js / ui.js / service.js。
- 单机端到端脚本（真起 `service.js` + 模拟对端 UDP 注册与 TCP 请求）：23 项全 PASS——
  无令牌拒启、错误令牌 403、UDP 发现、未授权 `pending` + outbox、收到 `approve` 后
  重投 `msg`、`/respond` 回 `approve`、入站消息取走即清空、未信任 `not_allowed`、
  `/stop` 退出码 0。
- 未验证：两台真实机器同网段收发、Windows 防火墙放行、WebView2 里
  `http://efto.localhost` → `127.0.0.1` 的 fetch（CORS/PNA）、真实 UI 交互；
  详见 `docs/verification.md` 同名小节。
