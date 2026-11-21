# api-manager（API 管理）

Postman 风格的 API 管理与测试插件：集合、集合内分组、环境变量、请求历史，图形界面与命令行共用同一份工作区。

- 触发词：`api`
- 打开界面：`api` 回车（或 `api ui`）
- HTTP 请求由本机 Node 运行时执行（设置 → 运行时 → Node.js），插件进程本身不发网络请求。

## 界面

左侧是集合树与历史；集合内可建多级分组（分组行有 `＋` 新建请求、`⊞` 子分组、`✎` 重命名、
`×` 删除）。顶部填 Method / URL，`{{变量}}` 按当前环境替换；中间 Params / Headers /
Body / Auth；底部显示响应状态、耗时、大小与响应头，响应体可复制。

- 新请求、新集合（侧栏按钮，弹窗输入名称）、保存（Ctrl+S）、发送（Ctrl+Enter）
- 环境变量弹窗：多环境、设为当前、键值编辑
- 导入：粘贴 JSON 或选择文件，支持 Postman Collection v2.1（保留文件夹层级）、Postman Environment、本插件导出；
  弹窗里可选「导入到」某集合或新建集合，导入内容全部挂在该集合下
- 导入 Swagger：在导入弹窗填 `http://host:port/doc.html` 之类地址，自动发现 `v3/api-docs` /
  `swagger-resources` 等规范；默认每个 Swagger 分组导入为一个集合，组内按 tag 建分组；
  指定目标集合时，分组成该集合下的一级分组
- 导入目标：不选则按来源自动创建/合并集合；选现有集合按 id 追加；选「新建集合」输入名称
  （同名集合已存在则直接复用）
- 导出：整个工作区（原生 JSON）或单个集合（Postman v2.1，分组导出为嵌套文件夹）

## 命令行 / Agent

在启动器输入框里写 `api <命令>`；在 CLI 里 `api` 是触发词，后面的文本是负载：

```powershell
$env:EFTO_PLUGIN_DIRS = "e:\workspace\efto\plugins"
.\target\debug\efto.exe cli plugin-run api-manager api "list" --wait-ms 2000 --json
.\target\debug\efto.exe cli plugin-run api-manager api "send Get Todo" --wait-ms 20000 --json
.\target\debug\efto.exe cli plugin-run api-manager api "run GET https://example.com" --wait-ms 20000 --json
```

常用文本命令：

| 命令 | 说明 |
| --- | --- |
| `list [关键字]` | 列出请求（名称、Method、URL、集合） |
| `show <id\|名称>` | 查看请求完整 JSON |
| `send <id\|名称>` | 发送已保存的请求 |
| `run GET <url>` | 临时发送，可带 `--header K:V` / `--body <json>` / `--content-type` / `--env <名称>` / `--var k=v` / `--timeout <ms>` / `--no-follow` |
| `add <json>` | 新增请求（JSON 含空格时改用 `--json-file`），可用 `"folder":"分组/子分组"` 指定分组 |
| `update <id> <json>` | 更新请求字段（同上），`"folder"` 可把请求移动到分组 |
| `delete <id>` | 删除请求 |
| `collections` / `collection add <名称>` | 集合列表 / 新建 |
| `folder list <集合>` | 列出分组（含请求数） |
| `folder add <集合> <名称>` | 新建分组（JSON 可用 `parent` 指定父分组） |
| `folder rename <集合> <分组> <新名>` / `folder delete <集合> <分组>` | 重命名 / 删除分组（级联删除其中请求） |
| `env` / `env use <名称>` / `env set <名称> k=v` | 环境列表 / 切换 / 设置变量 |
| `history [n]` / `history clear` | 最近请求 / 清空 |
| `import --file <path>` / `import --url <swagger 地址>` / `export [集合]` | 导入（文件 / Swagger URL）/ 导出 |
| `import ... --collection <集合>` | 导入到指定集合（不存在则新建；也可用 `--collection-id`） |

JSON 命令更适合 agent，结果统一以一行 `EFTO_API {...}` 输出：
`cli ... --json` 时从返回对象的 `logs[].text` 里取（`efto.output` 的行在 `lines` 里）。
不带 `--json` 时这一行直接打到 stdout，可直接 grep `^.*EFTO_API `。

```json
{"cmd":"list"}
{"cmd":"send","id":"req-xxxx"}
{"cmd":"run","method":"POST","url":"https://httpbin.org/post","bodyMode":"json","body":"{\"a\":1}","headers":{"X-Token":"t"}}
{"cmd":"add","collection":"Demo","folder":"用户/登录","request":{"name":"List","method":"GET","url":"{{baseUrl}}/users"}}
{"cmd":"folder","action":"add","collection":"Demo","name":"登录","parent":"用户"}
{"cmd":"folder","action":"list","collection":"Demo"}
{"cmd":"import","url":"http://10.10.58.153:32300/doc.html","collection":"我的服务"}
{"cmd":"env","action":"set","name":"Local","vars":{"baseUrl":"http://127.0.0.1:8080"}}
{"cmd":"export","collection":"Demo"}
```

### Swagger 导入

- 地址可以是 doc.html 页面、`/v3/api-docs`、`/v2/api-docs` 或直接指向规范 JSON。
- 自动探测顺序：`/v3/api-docs/swagger-config`（SpringDoc/knife4j 分组）→ `/swagger-resources`
  （springfox）→ `/v3/api-docs` → `/v2/api-docs`；会同时尝试 doc.html 所在子路径与站点根。
- 每个 Swagger 分组（knife4j 左上角分组）导入为一个集合，操作按第一个 tag 建分组；
  分组名与集合名重名时合并进已有集合。
- 指定了目标集合时，所有分组都进入该集合：Swagger 分组成一级分组、tag 成二级分组。
- 请求 URL 写成 `{{baseUrl}}` + 路径，同时按规范里的服务地址新建（或更新）同名环境
  （如 `10.10.58.153:32300`，变量 `baseUrl`），并把该环境设为当前环境；
  之后要换测试/生产地址只改环境变量即可。
- 路径参数 `{id}` 转成 `{{id}}`，使用前在环境里补同名变量；query/header 参数带入请求，
  JSON 请求体按 schema 生成示例（`example` / `default` / `enum` 优先，其余给空值）。
- 规范超过 8MB、或接口返回非 JSON（YAML）会明确报错。

### Windows 引号坑

PowerShell 5.1 会把参数里的 `"` 当成参数引号吃掉，含空格的 JSON 还会被拆成多个参数。
把 JSON 命令写进文件，用 `--json-file` 读取最稳：

```powershell
.\target\debug\efto.exe cli plugin-run api-manager api "--json-file D:\cmd.json" --wait-ms 2000 --json
```

不经过 PowerShell（例如 `cmd /c`）时才可以直接内联 JSON。

### 输出约定

- 机器可读：`console.log('EFTO_API ' + JSON.stringify({ok, cmd, data|error}))`
- 人类可读：`efto.output` 行（状态码/耗时/大小、响应体、完整 JSON 复制行），最多 50 行
- CLI 下插件写 KV 会在进程退出前落盘，跨命令/跨进程可见

## 边界

- 没有 Node 运行时时发送请求和 Swagger 导入都会明确报错，并提示去设置里配置。
- 请求 JSON 经命令行传递时限制 24000 字符；响应体在 Node 侧截断到 131072 字节并标记 `truncated`。
- Swagger 抓取与摘要都在 Node 里完成（规范最大 8MB），只把按 tag 归类后的操作列表回传主进程。
- 只做 HTTP(S) 请求与集合管理，不含 Mock Server / 自动化测试脚本 / 证书管理。
