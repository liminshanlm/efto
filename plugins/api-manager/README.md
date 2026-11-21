# api-manager（API 管理）

Postman 风格的 API 管理与测试插件：集合、环境变量、请求历史，图形界面与命令行共用同一份工作区。

- 触发词：`api`
- 打开界面：`api` 回车（或 `api ui`）
- HTTP 请求由本机 Node 运行时执行（设置 → 运行时 → Node.js），插件进程本身不发网络请求。

## 界面

左侧是集合树与历史；顶部填 Method / URL，`{{变量}}` 按当前环境替换；中间 Params / Headers /
Body / Auth；底部显示响应状态、耗时、大小与响应头，响应体可复制。

- 新请求、新集合（侧栏按钮，弹窗输入名称）、保存（Ctrl+S）、发送（Ctrl+Enter）
- 环境变量弹窗：多环境、设为当前、键值编辑
- 导入：粘贴 JSON 或选择文件，支持 Postman Collection v2.1、Postman Environment、本插件导出
- 导出：整个工作区（原生 JSON）或单个集合（Postman v2.1）

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
| `add <json>` | 新增请求（JSON 含空格时改用 `--json-file`） |
| `update <id> <json>` | 更新请求字段（同上） |
| `delete <id>` | 删除请求 |
| `collections` / `collection add <名称>` | 集合列表 / 新建 |
| `env` / `env use <名称>` / `env set <名称> k=v` | 环境列表 / 切换 / 设置变量 |
| `history [n]` / `history clear` | 最近请求 / 清空 |
| `import --file <path>` / `export [集合]` | 导入 / 导出 |

JSON 命令更适合 agent，结果统一以一行 `EFTO_API {...}` 输出：
`cli ... --json` 时从返回对象的 `logs[].text` 里取（`efto.output` 的行在 `lines` 里）。
不带 `--json` 时这一行直接打到 stdout，可直接 grep `^.*EFTO_API `。

```json
{"cmd":"list"}
{"cmd":"send","id":"req-xxxx"}
{"cmd":"run","method":"POST","url":"https://httpbin.org/post","bodyMode":"json","body":"{\"a\":1}","headers":{"X-Token":"t"}}
{"cmd":"add","collection":"Demo","request":{"name":"List","method":"GET","url":"{{baseUrl}}/users"}}
{"cmd":"env","action":"set","name":"Local","vars":{"baseUrl":"http://127.0.0.1:8080"}}
{"cmd":"export","collection":"Demo"}
```

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

- 没有 Node 运行时时发送请求会明确报错，并提示去设置里配置。
- 请求 JSON 经命令行传递时限制 24000 字符；响应体在 Node 侧截断到 131072 字节并标记 `truncated`。
- 只做 HTTP(S) 请求与集合管理，不含 Mock Server / 自动化测试脚本 / 证书管理。
