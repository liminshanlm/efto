# api-manager 插件（Postman 风格 API 管理）

- **Date**: 2026-11-21
- **Status**: done
- **相关**: plugins/api-manager/（README、main.js、lib/、ui.js、index.html）、crates/app/src/cli.rs（plugin-run 落盘）、docs/plugin-development.md、docs/verification.md

## 需求

在 efto 里新建一个 API 管理插件：复刻 Postman 的基础能力（请求构造、集合、环境变量、
历史、导入导出），既可开 UI 使用，也可用命令管理/查询，结果便于 agent 消费。

## 方案

1. **一份工作区，两个入口**。工作区（collections / environments / activeEnvId / history）
   整体存在 `efto.kv` 的单 key `workspace` 里；UI 只负责编辑与展示，所有持久化与请求执行
   都在 main。UI 侧没有 `efto.db`，状态通过 `ipcMessage('state', {workspace})` 下发。
2. **HTTP 只走 `efto.node.run`**。main 没有 fetch/fs，`lib/http.js` 把 Node 源码
   （`nodeScript.toString()`）交给用户在设置里配好的 Node；UI 与命令共用同一条链路。
   请求 JSON 从 `argv[1]` 传入（>24000 字符直接拒绝），响应在 Node 侧截断到
   131072 字节并标记 `truncated`，防止撑爆宿主 1MB stdout。
3. **中文输出只走转义**。Node stdout 一律 `\uXXXX` 转义，避免 GUI 无控制台时按 GBK
   解码出现乱码。
4. **命令协议双通道**：人读文本命令（`api list` / `api send Get Todo`），
   agent 读 JSON 命令（`{"cmd":"send","id":"..."}`）。机器可读结果固定前缀
   `EFTO_API ` + 单行 JSON；`plugin-run --json` 从 `logs[].text` 取，非 `--json`
   直接打 stdout。人类可读行仍用 `efto.output`（每行都带 `copy`）。
5. **新增 `--json-file <path>`**：Windows PowerShell 5.1 会把参数里的 `"` 当引号吃掉，
   含空格的 JSON 还会被拆成多个 argv。agent 想把 JSON 命令传进来，最稳的是写文件后
   `api --json-file D:\cmd.json`.
6. **修宿主 CLI 落盘**：`cli plugin-run` 之前只把插件 KV 留在内存，进程退出即丢。
   `CliHost` 增加 `flush()`，`cmd_plugin_run` 在 `runtime.stop` 后调用，并把
   `flush_error` 计入 `--json` 输出与退出码。
7. **修插件日志可达性**：`console.*` 之前只写 tracing，`--json` 模式下被完全丢弃，
   `EFTO_API` 结果拿不到。`bridge` 的 `log` 改走 `PluginEvent::Log` 事件通道
   （GUI 由 pump 落 tracing，行为不变）；`cmd_plugin_run` 的 `--json` 新增 `logs` 数组。

## 判据与取舍

- 请求 JSON 过大时不截断请求体，直接明确报错（截断等于发错请求）。
- 集合/环境同名导入时合并而不是新建重复项；请求名冲突时保留 id、更新内容。
- `findRequest` 支持 id 精确、名称精确、id 前缀、名称包含；多个命中返回错误，
  不做"猜一个"。
- 未配置 Node 运行时：报错并提示「设置 → 运行时 → Node.js」，不静默成功。
- 每次发送/执行后写历史（上限 100 条）并 `persist()`，随后同步启动器搜索项
  （`api:open:<id>`，最多 200 条）。

## 验证

- `node --check` 通过：main.js、lib/*.js、ui.js；清单 JSON 可解析。
- CLI 端到端（`EFTO_PLUGIN_DIRS=plugins`）：
  - `cli plugins` 列出 api-manager；
  - `help` / `list` 输出正常；
  - `send Get Todo` 真实 HTTP（Node code=0，返回 jsonplaceholder 响应）；
  - `add` 新增 `SmokeGet`，另起进程 `list` 仍可见（证明 flush 修复生效）；
  - `--json-file` 读取文件新增 `From File` 成功。
- `cargo test -p efto-app cli_host`：3 个测试通过，含新增
  `cli_host_flush_persists_plugin_data`。
- `cargo test -p efto-jsrt route_`：2 个通过（`log` 仍不打到 Host）。
- `plugin-run --json` 的 `logs` 里能取到 `EFTO_API {...}`（bridge 改事件通道后验证）。
- UI 无头验证（Playwright + 注入的 `window.efto` 桩，直接加载 `index.html`）：
  - 集合树渲染、切换请求、URL/Method/Auth 回填、Ctrl+S 产生 `save-request`；
  - 发送后响应区显示 `200 OK` / `12 ms · 26 B` / 格式化 JSON；
  - Params 行数、Auth bearer 输入、导入/环境/导出弹窗、导出结果 textarea、历史面板均正常；
  - 全程无 `pageerror` / console error。
  - 该轮抓到一个真 bug：`.modal-root` 用 `display:grid`，`hidden` 属性被覆盖，
    全屏遮罩一直拦截点击（界面等于点不动）。已加 `.modal-root[hidden]{display:none}`。
  - 补丁轮（用户反馈「没看到新增集合的按钮」）：侧栏 `side-actions` 增加
    `#btn-new-collection`（「＋ 新集合」），点击走原生 `prompt`，非空才发
    `save-collection`；取消/空名不发消息。main 的 `save-collection` 已有实现，无需改宿主。
    Node + playwright-core + 本机 Edge 无头复测：按钮存在、发出正确 payload、侧栏出现
    `UI 测试集合 · 0`、取消与空名零消息、无 `pageerror`。

- 真机 GUI（2026-11-21，debug 构建 + `EFTO_PLUGIN_DIRS=plugins`）：
  `Alt+Space` → `api` 回车打开独立插件窗口（`http://efto.localhost/api-manager/index.html`），
  集合树 / Method / URL / 发送按钮正常；集合重命名弹出原生 `prompt`、删除请求弹出原生
  `confirm`，取消后数据未变；发送 `Get Todo` 返回 `200 OK`（4566 ms · 70 B），响应体正常渲染。

未验证：窗口尺寸是否严格为 1180x780（真机为独立大窗，未做像素级测量）。
