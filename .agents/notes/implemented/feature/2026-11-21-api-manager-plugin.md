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
8. **集合内分组不改存储形状**：`collection.requests` 仍是扁平数组，只加
   `collection.folders[{id,name,parentId,createdAt}]` 与 `request.folderId`（`''` = 集合根）。
   旧数据在 `normCollection` 里补 `folders: []`、`folderId: ''`，不做迁移脚本。
   分组支持多级嵌套；删除分组按 `descendantFolderIds` 级联删子分组与其中的请求。
   导入 Postman 时按嵌套 `item` 建分组路径，导出时用 `buildPostmanItems` 还原嵌套文件夹。
   命令层用「id 优先、否则按完整路径（`A / B`）匹配、再按名称匹配」解析分组，
   多个同名命中直接报错，不做猜测。
9. **Swagger 抓取与摘要全部放 Node**：`lib/swagger.js` 把一段 Node 源码交给
   `efto.node.run`，在 Node 侧完成地址识别（doc.html / api-docs / 规范 JSON）、
   规范发现（`v3/api-docs/swagger-config` → `swagger-resources` → `v3/api-docs` →
   `v2/api-docs`，并同时尝试 doc.html 子路径与站点根）、schema 示例生成与按 tag 归类，
   只回传紧凑 JSON。原因：规范常有数 MB，而插件 stdout 上限 1MB，回传原文必被截断。
   映射规则：每个 Swagger 分组（knife4j 左上角分组）= 一个集合，tag = 集合内分组，
   路径参数 `{id}` → `{{id}}`；分组名/集合名撞车时合并进已有集合。
10. **Swagger URL 统一走 `{{baseUrl}}`**：请求 URL 不再写死 `http://host:port/...`，
   而是 `{{baseUrl}}` + 路径；同时按规范里的服务地址新建（或更新）同名环境
   （名字取去掉协议的地址，如 `10.10.58.153:32300`，变量 `baseUrl`），
   并经 `parsed.activateEnv` 让 `applyImport` 把它设为当前环境——否则 `{{baseUrl}}`
   解析不到、导入的请求直接不可发送。多个分组共用同一地址时只建一个环境；
   springfox 只给相对 `basePath` 时用规范 URL 的 origin 补齐。
11. **导入可指定目标集合**：导入弹窗加「导入到」下拉（自动 / 现有集合 / 新建集合 + 名称），
   CLI 对应 `--collection <名称>`（同名复用、否则新建）与 `--collection-id <id>`，
   JSON 命令用 `collection` / `collectionId`；三种入口（粘贴 JSON、文件、Swagger）都带目标。
   指定目标时所有来源集合都写进同一个集合：多个来源集合时把来源名作为一级分组再拼
   原有 folderPath（Swagger 分组 → 一级、tag → 二级），只有一个来源时保留原分组路径；
   不指定目标时保持原行为（按来源名建/并集合）。

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

- 集合分组 + Swagger URL 导入（2026-11-21 补测）：
  - CLI `folder add Demo 用户` / `folder add Demo 登录 --parent 用户` / `folder list Demo` /
    `folder rename` / `folder delete` 全部返回正确；删除父分组时级联删掉子分组里的请求
    （`removedFolders` / `removedRequests` 计数与预期一致）。
  - `add` / `update` 用 `{"folder":"用户/登录"}` 把请求挂到分组，`show` 输出 `folder: "用户 / 登录"`。
  - `export Demo` 得到 Postman v2.1 嵌套 `item`（分组为文件夹层级）；
    反向导入一个两层 `item` 的 Postman 集合后，请求 `folder` 为 `用户 / 子组`。
  - 真实地址 `http://10.10.58.153:32300/doc.html`：`/v3/api-docs/swagger-config` 返回 3 个分组，
    导入后得到 3 个集合、68 个请求；tag → 分组、`{id}` → `{{id}}`、
    `show sendSmsCode` 的 JSON 体示例 `{"mobile": ""}` 均符合预期。
  - URL 统一为 `{{baseUrl}}` + 路径（如 `{{baseUrl}}/open/personal/login/sms-code`）；
    `env` 显示自动新建并激活的环境 `10.10.58.153:32300`（`baseUrl = http://10.10.58.153:32300`）。
  - 导入目标集合：`--collection 导入目标测试` 只生成 1 个集合（68 请求），`folder list`
    得到 20 个分组，Swagger 分组名为一级、tag 为二级；`--collection-id col-…` 追加进同一
    集合、不新建集合。UI 无头验证「导入到」三种选择分别发出无目标字段 / `collectionId` /
    `collection`，无 `pageerror`。
  - UI 无头（playwright-core + 本机 Edge + `window.efto` 桩）五步验证通过：
    嵌套请求缩进 8px → 22px、`save-request` 携带 `folderId:"fld-1"`、`delete-folder`、
    `import-url` 均正常发出，无 `pageerror`（仅 favicon 404）。
  - 验证产生的临时集合与临时脚本已全部清理，未留在用户工作区。

未验证：窗口尺寸是否严格为 1180x780（真机为独立大窗，未做像素级测量）；
Swagger 导入只测了 JSON 规范，YAML 规范、OAuth2 安全定义与 `servers.variables` 未覆盖。
