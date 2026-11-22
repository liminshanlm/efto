# file-share 从宿主能力改为纯插件 + Node

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `plugins/file-share/main.js`、`plugins/file-share/service.js`、
  `docs/plugin-development.md`、`docs/architecture.md`、`docs/verification.md`、
  `docs/macos-parity-todo.md`、`.agents/notes/implemented/feature/2026-11-22-ipmsg-plugin.md`

## 决策

用户要求：宿主保持小而健壮，网络类能力交给插件用本机 Node 实现。
file-share 原先的只读 HTTP 服务在宿主 `crates/app/src/share.rs`，通过 `efto.share` 暴露。
本次整体搬到插件：

- 删除 `share.rs`、`AppData.share`、`host.rs` 的 `("share", ...)` 分发、
  `prelude.rs` 的 `efto.share`、`efto.d.ts` 的 `Share*` 类型；
- 新增 `plugins/file-share/service.js`：数据口 `0.0.0.0:5421`（0 = 随机）+ 可选 `[::]`，
  控制口 `127.0.0.1:5422`（令牌校验），承载目录浏览、Range、路径校验、RFC 5987 文件名；
- `main.js` 沿用 ipmsg 的 bootstrap 模式：`efto.node.run` 探测旧实例或 detached spawn
  `service.js`，之后每次 `start/stop/status` 都是 `node -e` 调控制口；
- UI 与 kv 结构不变（`state` / `start` / `stop` / `add` / `remove` / `clear`），
  所以 `ui.js`、`index.html` 不需要改。

代价与 ipmsg 一致：插件依赖本机 Node（设置 → 运行时 配置并检测通过）。

## 生命周期（比 ipmsg 多一层）

detached 服务不会随宿主退出。为避免应用被强杀后仍在对外共享文件：

- UI 关闭时 `onUiClosed` 立即 `stop`；
- 数据口在跑时 main 每 45s 发一次 `/status` heartbeat，服务超过 120s 无控制请求就
  `stopServers()`；
- 数据口没在跑时，服务 30 分钟无请求自退。

`onUiClosed` 里先 `applySeq++`，丢掉还在路上的 start 结果，避免窗口关了又恢复分享。

## 踩到的坑

1. **停止不能清空列表**：`/stop` 返回的 `items` 是空数组，直接套用会让 UI 里的条目消失。
   停止路径单独走 `applyStopped()`，只清 running/urls，保留 `state.items`。
2. **`/f/idx` 命中目录要 302**：新端到端测试一开始把目录条目当文件下载，测出 302 是
   Rust 版的既有语义；改成再共享一个文件条目 `/f/1` 验证 Range/HEAD。
3. **没有 Node 时不能阻塞 Stop**：`stopShare()` 先看 `booted`，没起过服务就只清本地状态，
   否则点「取消分享」会反而去拉起一个 Node 进程。

## 验证

- `node --check` 三个 JS；
- `service.js` 单机端到端 22 项全 PASS（令牌 403、随机端口、目录/文件、`%2e%2e` 越界、
  `/f` 目录 302、`filename*`、Range 206/开区间/416、HEAD、405、端口占用报错、
  invalid 路径、stop 后端口关闭、shutdown 退出码 0）；
- 从 `main.js` 里提取 `BOOT_CODE` / `CALL_CODE`，按 `efto.node.run` 的方式用
  `node -e` 实跑 start/status/stop/shutdown 全部 PASS；
- `cargo test -p efto-app --lib` 107 passed；`cargo test -p efto-jsrt` 48 passed；
- **真实宿主实跑**（2026-11-22）：`efto.exe cli plugin-run file-share fs <目录> --json`
  返回 `ok=true running=true errors=0`，两条 `[runtime] node code=0`；对 5421 数据口做 21 项
  HTTP 检查全 PASS（首页/目录/子目录/逐字节下载/Range 206/416/HEAD/空文件/traversal 404/POST 405）。
  运行后按端口与命令行清掉该 `service.js` 进程，测试条目已从插件 kv 移除。

未验证：真实插件窗口观感、两台机器互访、Windows 防火墙首次放行、强杀后的租约行为、大文件/并发。
详见 `docs/verification.md` 的 file-share 小节。
