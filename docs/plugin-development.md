# 插件开发指南（第一阶段）

面向第一次写 efto 插件的人。约定：**本文只写实际实现了的东西**，未实现的会明确标注。

## 1. 目录与清单

插件只有一个清单：`efto-plugin.json`，运行时只暴露 `efto.*`（UI 页面里是 `window.efto`）。

```
my-plugin/
  efto-plugin.json 清单（必需）
  main.js | main.ts  后端逻辑（可选）
  index.html       UI 页面（可选）
  preload.js       UI 前置脚本（可选，第一阶段仅识别、不执行）
  logo.png         图标（可选）
```

清单最小可用形式：

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "main": "main.js",
  "ui": "index.html",
  "platform": [{ "os": "win32" }],
  "cmds": [
    { "type": "input", "label": "我的插件", "cmd": "my" },
    { "type": "over",  "label": "处理文件", "platform": [{ "os": "win32" }] }
  ]
}
```

| 字段 | 说明 |
| --- | --- |
| `name` | 插件 id（必填，全局唯一，重复时后加载者被忽略） |
| `main` | 后端入口。写 `main.js` 但目录里只有 `main.ts` 时，宿主自动回退到 `.ts` 并提示 |
| `ui` / `index` | UI 页面，默认探测 `index.html` / `dist/index.html` / `ui/index.html`（两个字段等价） |
| `platform` | 插件级平台限定 `[{ "os": "win32" }]`（`win32` / `darwin` / `linux`），不写表示全平台。不匹配的插件仍出现在 `cli plugins`（标注"不支持当前平台"），但不进搜索、不能被唤起，运行入口报 `unsupported_platform` |
| `cmds[].type` | `input`（关键字后接输入） / `over`（内容覆盖） / `regex` / `word`；未知值按 `input` 处理，不会报错 |
| `cmds[].cmd` | 触发词，缺省等于 `label`；两者都可作为搜索关键字 |
| `cmds[].platform` | 同插件级 `platform`，只限定这一条 cmd 的可见性 |

### 没有 UI 时怎么反馈

- **有输出**：调用 `efto.output(rows)`，结果内联到搜索框下方列表（与文件检索行一样），**搜索框不收起**。
- **没输出**：只做通知/复制等副作用（例如 `efto.showNotification`），宿主在等待窗口结束后**收起搜索框**。

```js
exports.onPluginEnter = ({ payload }) => {
  efto.output([
    { title: '复制时间戳', subtitle: String(Date.now()), copy: String(Date.now()) },
    { title: '打开项目目录', subtitle: 'D:\\work', open: 'D:\\work' }
  ]);
};
```

行格式：`{ title, subtitle?, copy?, open? }`，`copy` / `open` 至少给一个；单次最多 50 行，任何一行不合法都会整批报错。
`copy` 行单击即复制；`open` 行双击打开。

### 放在哪里才会被找到

按顺序搜索这些目录：

1. `settings.json` 里的 `plugin_dirs`
2. 环境变量 `EFTO_PLUGIN_DIRS`（`;` 分隔，开发时用这个最方便）
3. `~/.efto/plugins`（默认，也是设置页"导入插件"的落点）

目录本身是插件、或它的**一层/两层子目录**是插件，都会被识别（支持 monorepo 的 `packages/xxx` 布局）。改动后按 `Alt+R` 重建索引，或到设置页点"重建索引"。

## 2. 运行模型

- 每个插件一个线程、一个 V8 isolate、一个 context。插件死循环只会拖住它自己。
- `main.js` 里**没有 DOM**，也不是 Node：`require('fs')` / `process` / `Buffer` 都不存在。裸模块名会直接报错并给出解释。
- **模块格式是 CommonJS**：入口与被 `require` 的模块都在函数作用域里执行，因此写
  `require('./lib/x')` / `exports.f = ...` / `module.exports = {...}`，**不要**写 `import`/`export`。
  检测到 ESM 语法会直接报一条带你该怎么做的消息（而不是让 V8 抛 `Illegal...`）：
  ```
  main.ts 用了 ESM 的 import 语法。插件入口与被 require 的模块都在 CommonJS 作用域里执行：
  请把 `import x from './a'` 改成 `const x = require('./a')` …
  ```
- 相对路径 `require('./lib/x')` 可用，扩展名按 `.js .mjs .cjs .json .ts .mts .cts` 与 `index.*` 顺序解析；
  被 require 的 `.ts` 同样由宿主转译，`.json` 自动包成 `module.exports = {...}`。
- 有 `index.html` 时，UI 是**另一个 webview 窗口**，通过 `efto://` 协议从插件目录加载页面，SDK 由宿主注入（页面里不需要引任何 script）。

```
UI 页面 ──efto.ipcMessage(name,data)──▶ 宿主 ──▶ main.isolate(onMainMessage)
UI 页面 ◀──────efto.onMessage(name, cb)─────宿主──────efto.ipcMessage / broadcast
```

## 3. API

插件只有 `efto.*`（没有其他全局对象，写了会直接 ReferenceError）。

```js
exports.onPluginEnter = ({ payload }) => {};
exports.onMainMessage = (msg) => {};
efto.showNotification('hi');
efto.output([{ title: '42', copy: '42' }]);
```

`efto.onXxx(cb)` 与 `exports.onXxx = cb` 两种写法都支持，会被合并登记，同一个事件可以有多个处理函数。

### 已实现的能力

| 分类 | API | 备注 |
| --- | --- | --- |
| 生命周期 | `onPluginEnter` `onPluginOut` `onMainMessage` `onMessage` `onWindowHide/Show` | `Enter` 的 `payload` 是关键字之后的输入 |
| **内联输出** | `efto.output(rows)` | 行格式见第 1 节，调用后搜索框保持可见 |
| 存储(旧) | `kv.get/set/remove/all/keys/size/clear` | KV，值为任意 JSON |
| 存储(新) | `db.put/get/remove/bulkGet/bulkPut/allDocs/info/changes/changeEmitter` | 文档形式（`_id`/`_rev`），底层就是上面那套 KV，`_rev` 自动递增 |
| 外部程序 | `exec({ name, args, timeoutMs })` → Promise | 只认白名单名字（目前 `es`），详见下方“efto 扩展” |
| **脚本运行时** | `efto.node.run({ code, args?, timeoutMs? })` / `efto.python.run(...)` → Promise | 解释器需在 设置 → 运行时 配置并检测通过 |
| 窗口 | `showUI/hideUI` `showMainWindow/hideMainWindow` `setSubTitle` `closeWindow` | UI 类能力全部异步投递，绝不阻塞主线程 |
| **内网共享** | `efto.share.start({ port, items })` / `stop()` / `status()` | 只读 HTTP 文件服务，`items: [{ path, name? }]`，详见下方 |
| **hosts** | `efto.hosts.path()` / `read()` / `save({ text, encoding? })` → Promise | 写 Windows hosts，保存走 UAC 提权并自动备份 |
| **大模型** | `efto.ai.chat({ messages, model?, temperature?, maxTokens?, timeoutSecs?, stream?, onDelta?, webSearch?, readUrl? })` → Promise / `efto.ai.models()` | 调用 设置 → 模型 里的（默认）模型，OpenAI 兼容接口；`onDelta` 接收流式增量；`webSearch`/`readUrl` 开启 DeepSeek 服务端联网 |
| **OCR** | `efto.ai.ocr({ path, purpose?, model? })` → Promise | 调 DeepSeek OCR 识别 PDF / Word / PPT / 图片，返回逐页文字 |
| **搜索项** | `efto.setSearchItems([{ title, cmd }])` | 把插件自己的数据挂到启动器搜索；回车后 `cmd` 作为 `info.code` 回传；宿主落盘，重启后仍可搜；传 `[]` 注销 |
| 跳转 | `redirect(label, opts)` / `redirects` | 带 `url` 交给系统浏览器 |
| 系统 | `copyText` `showNotification` `shellOpenPath` `shellShowItemInFolder` `openExternal` `getPathByName` `quit` | `pasteText` / `pasteImage` / `imagePreview` 可用（Windows 剪贴板与本地图片；`pasteImage` 返回临时 PNG 路径，`imagePreview` 返回 data URL） |
| 环境 | `platform` `version` `locale` `dbPath` `isWindows/isMacOS/isLinux` `isDarkMode()/isLightMode()` `getPathForFile` | |
| 通信 | `ipcMessage(name, data)`：UI→main 用 `efto.ipcMessage`，main→UI 用 `efto.ipcMessage`；`broadcast(channel, data)`（跨插件） `sendToDefaultOutput` | |
| 定时器 | `setTimeout` `setInterval` `clearTimeout/Interval` | 由宿主调度，微任务在每轮事件后推进 |
| 日志 | `console.log/warn/error/...` | 进宿主日志，`RUST_LOG=debug` 可见 |

### 未实现（会返回明确错误，不会静默失败）

桌面快捷方式创建、`node_modules`、`createWorkflow`、`over`（拖拽文件注入）、`db` 的复制/冲突合并语义。

### efto 扩展：`efto.exec`

让插件执行外部程序。白名单按平台生效（`core/src/exec.rs`，平台门在参数拦截之后，
保证边界语义跨平台可测）：`es`（Everything CLI，仅 Windows）与 `mdfind`（Spotlight，
仅 macOS）。系统工具类条目走固定绝对路径、免钉 sha256（系统升级会换二进制），
且按条目限制参数形状（mdfind 只允许 `[-name, 查询词]`）；其余条目必须钉摘要。

> 现状说明：**目前没有一方插件在用它**。首个使用者（Everything 文件检索）已改成
> 内置搜索源（结果内联在下拉框），执行器下沉到 `core/src/exec.rs`，内联源与这个能力共用同一套
> 白名单/摘要/参数约束。保留该能力是因为它已被 12 个测试覆盖，也是后续"需要跑外部工具"的
> 插件的基础；如果确定不需要，删掉 `host.rs` 里的 `("efto","exec")` 分支即可。

```js
const r = await efto.exec({
  name: 'es',                       // 只认白名单名字，目前只有 'es'
  args: ['-n', '30', '-csv', '-no-header', '-full-path-and-name', query],
  timeoutMs: 5000                   // 默认 3000，上限 15000，超时 kill
});
// r = { ok:true, code, stdout, stderr, path, truncated }
```

宿主侧的四层约束（`crates/app/src/exec.rs`，JSON 边界在 `host.rs::exec_capability`）：

1. **白名单**：`name` 只认表里的键。`path` / `command` / `shell` / `cwd` / `env` / `file`
   这些字段**一律拒绝** —— 能指定路径就等于绕过白名单。
2. **sha256 锁定**：按"仓库 `tools/es` → 数据目录 → PATH → Everything 安装目录"顺序找，
   **逐个校验摘要**，用第一个匹配的。都不匹配就报错并列出期望/实际摘要，绝不"降级执行"。
   为什么这么严：按名字去 PATH 里找 `es.exe`，任何能往 PATH 目录写文件的人就等于拿到任意代码执行。
3. **参数不经 shell**：数组直接进 CreateProcess，`;` `&` `|` `>` 拼不出额外语义。
4. **参数级拦截**：白名单只回答"能跑哪个程序"，没回答"能让它干什么"。es 自己就能
   `-export-csv <路径>` 写任意文件、`-set-run-count` 改 Everything 的库、`-save-settings` 落配置，
   这些对只读检索没用，所以直接拒；并且**归一化后再比**（es 允许省略选项内部连字符，
   `-exportcsv`、`/export-csv` 都得挡住）。`-n` 上限 500，输出上限 1MB，超时 kill。

另外：**子进程输出按控制台代码页解码**。实测 es.exe 在 zh-CN 机器输出 GBK 而不是 UTF-8，
中文路径直接 `from_utf8_lossy` 会变成替换字符，所以走 `GetConsoleOutputCP()`/`GetACP()`
+ `MultiByteToWideChar`。

换 es 版本时：跑 `powershell -File scripts/fetch-es.ps1 -Force`，把打印出来的 exe 摘要
填回 `exec.rs` 的 `ALLOWLIST`，再 `cargo test -p efto-app --lib exec`。

### efto 扩展：`efto.node` / `efto.python`

让插件用本机的 Node.js / Python 执行一段脚本。解释器**只能**在 设置 → 运行时 里配置，
检测通过后所有插件都能调用；插件无法指定或替换解释器路径。

```js
const r = await efto.node.run({
  code: 'console.log(process.argv.slice(1).join(","))',
  args: ['a', 'b'],     // 追加在脚本之后，作为 argv
  timeoutMs: 10000      // 默认 10s，上限 60s
});
// r = { runtime: 'node', ok: true, code: 0, stdout, stderr, path, truncated }

await efto.python.run({ code: 'print(1 + 1)' });
```

行为约定：

- 以 `node -e <code>` / `python -c <code>` 执行，不落临时文件；参数数组不经过 shell。
- 工作目录是插件根目录；stdin 关闭，stdout/stderr 各截断到 1MB，超时会 kill 并报错。
- 参数里出现 `path` / `interpreter` / `program` / `command` / `shell` / `cwd` / `env` / `file`
  一律拒绝：能自己指路径就等于绕过设置。
- 未配置或检测失败时**明确报错**，不会降级去找 PATH 上的同名程序。
- 只写名字（如 `node`）时由 CreateProcess 从 PATH 解析；建议在设置里填完整路径。

### efto 扩展：`efto.share`

把文件/文件夹以**只读 HTTP** 方式暴露给同网段设备。服务实现是 Rust 标准库
（`crates/app/src/share.rs`），插件进程里没有 Node、没有监听套接字。

```js
const st = efto.share.start({
  port: 5421,                          // 0 = 随机端口；默认 5421
  items: [{ path: 'D:\\资料\\报告.pdf' }] // 目录会自动提供浏览与下载
});
// st = { running, port, urls: ['http://192.168.1.5:5421/'],
//        urls6: ['http://[2408:...:1]:5421/'],
//        items: [{name,path,is_dir,size}], invalid: [{path,reason}] }

efto.share.status();                   // 同上；未启动时 running=false
efto.share.stop();
```

行为约定：

- 只暴露 `items` 里的路径；请求路径会做规范化 + 前缀校验，`..` 和编码绕过都返回 404。
- 支持 `GET` / `HEAD`、`Range`（206 断点续传）、目录列表、RFC 5987 中文文件名。
- `urls6` 只有在 IPv6 监听真的建起来、且本机存在非回环 IPv6 地址时才有值；否则是空数组，
  UI 应据此禁用 IPv6 入口，不要伪造一个连不上的链接。
- 不含上传、账号与权限；`invalid` 里的路径**不会**被静默忽略，会在返回值里列出来。
- `items[].path` 回传的是**调用方传入的路径文本**（不是 canonicalize 后的 `\\?\` 形式），
  插件可直接拿它做去重、删除匹配；共享根目录的规范化只在宿主内部使用。
- 同端口重复 `start` 只更新条目，不会重启监听；换端口才重建。

示例插件：`plugins/file-share`。

### efto 扩展：`efto.hosts`

读写 Windows hosts 文件（`%SystemRoot%\System32\drivers\etc\hosts`）。
插件进程里没有 `fs`，读写、校验、备份和提权全部在宿主完成。

```js
const doc = efto.hosts.read();
// doc = { path, backup, text, encoding: 'utf8' | 'ansi', bytes }

// 编辑 doc.text 后保存；会弹一次 UAC（用户取消会 reject）
const r = await efto.hosts.save({ text: doc.text, encoding: doc.encoding });
// r = { ok, path, backup, bytes }
```

行为约定：

- **编码跟随读取结果**：读取时先试 UTF-8，失败按系统 ANSI 代码页（zh-CN 上是 GBK）解码；
  保存时沿用同一编码。ANSI 无法表示的新字符会明确报错，不会写成 `?`。
- **保存前校验**：每行必须是 `IP 主机名...`，IP 支持 IPv4 / IPv6（含 `%` 作用域后缀）；
  `#` 开头且紧跟合法 IP 的行按"已禁用条目"校验。任何一行不合法都整份拒绝，错误带行号。
- **备份**：写入前把当前文件复制到同目录 `hosts.efto.bak`（覆盖上一次），再写临时文件并原子替换。
- **UAC**：保存时用 `ShellExecuteExW(verb="runas")` 拉起 `efto.exe hosts-apply <request>`，
  不需要 efto 本身以管理员身份运行。用户点"否"会立刻返回"已取消"，不是超时。
- **安全边界**：提权子进程只接受 `~/.efto/uac/` 下的请求文件，目标固定为系统 hosts，
  请求里没有"写哪个文件"的字段 —— 避免变成"点一下 UAC 就能以管理员写任意文件"。
- Windows 与 macOS 可用：macOS 读写 `/etc/hosts`，保存走 osascript 管理员授权拉起同一个
  `hosts-apply` 提权子进程（请求文件/校验/原子写的安全边界两边共用）；其它平台调用
  path/read/save 明确报"当前平台不支持：hosts"。

示例插件：`plugins/hosts`（条目编辑 + 原文编辑，保存时自动备份）。

### efto 扩展：`efto.setSearchItems`

把插件自己的数据挂进启动器搜索（`setSearchItems` / `removeSearchItem`）。
清单里的 `cmds` 是静态的；运行时新增的数据（例如用户自己加的网站）用这个接口。

```js
// 每次变化后整量同步；title 是搜索结果标题，cmd 回传给 onPluginEnter
efto.setSearchItems([
  { title: '我的博客', cmd: '我的博客' },
  { title: '团队文档', cmd: '团队文档' }
]);

efto.onPluginEnter(({ code }) => {
  // code 就是上面登记的 cmd
});

// 全部注销
efto.setSearchItems([]);
```

行为约定：

- **匹配**：标题与输入完全相等 > 标题前缀 > 标题包含（双向），分高者优先；
  同一条数据与插件清单里的 cmd 同时命中时取分高的那条。
- **回车**：结果是普通的插件结果，回车按 `cmd` 唤起插件，`info.code` 即 `cmd`。
- **持久化**：宿主把条目写到 `~/.efto/plugin-search/<插件id>.json`，重启后无需插件启动也能搜到；
  插件下次启动/改动后再次同步即可覆盖。
- **边界**：空标题/空 cmd 丢弃；同一 `cmd` 只留第一条；最多 200 条；插件未装载时调用会明确报错。

### efto 扩展：`efto.ai`

调用 设置 → 模型 里配置的大模型。只支持 OpenAI 兼容的
`POST {base_url}/chat/completions`，OpenAI / DeepSeek / Kimi / 通义 /
Ollama / LM Studio 都适用。未配置模型时**明确 reject**，不会静默返回空串。

```js
// 不传 model 用默认模型；也可以传配置里的 id、显示名或底层模型名
const r = await efto.ai.chat({
  messages: [
    { role: 'system', content: '只输出译文' },
    { role: 'user', content: 'hello' }
  ],
  temperature: 0,
  maxTokens: 512,      // 可选
  timeoutSecs: 60      // 可选，5 ~ 300，默认 60
});
// r = { content, reasoning, model, modelId, modelName, finishReason, usage }

// 流式：传 onDelta 即自动开启（也可以显式 stream: true）。
// delta = { text?, reasoning? }，最终仍 resolve 完整结果。
const r2 = await efto.ai.chat({
  messages: [{ role: 'user', content: '写一段自我介绍' }],
  onDelta: (delta) => {
    if (delta.text) efto.ipcMessage('delta', delta); // 转发给插件 UI，边生成边显示
    if (delta.reasoning) efto.ipcMessage('reasoning', delta);
  }
});

// 图片输入（模型需支持读图，例如 gpt-4o、qwen-vl）：
// image.path 是插件进程拿到的本机绝对路径，由宿主读盘并转成 data URL。
await efto.ai.chat({
  messages: [{
    role: 'user',
    content: [
      { type: 'text', text: '识别图片里的文字' },
      { type: 'image', path: 'D:\\shots\\a.png' }
    ]
  }]
});

efto.ai.models();
// { default, models: [{ id, name, model, base_url, has_api_key }] }

// DeepSeek 联网：搜索与网页抓取都在服务端完成，客户端不执行工具。
// 需要模型名是 deepseek-flash，且 base_url 指向 DeepSeek。
const r3 = await efto.ai.chat({
  messages: [{ role: 'user', content: 'DeepSeek 最近发布了什么？' }],
  webSearch: true,   // deepseek_web_search
  readUrl: true,     // deepseek_read_url（自动读取消息里的链接）
  onDelta: (delta) => {
    if (delta.toolEvent) efto.ipcMessage('tool', delta.toolEvent); // 正在搜索/阅读
    if (delta.text) efto.ipcMessage('delta', delta);
  }
});
// r3.annotations = [{ type: 'url_citation', url_citation: { url, title, start_index, end_index } }]
// r3.serverToolCalls = [{ index, calls: [{ type, kind, action, status, sources, page }] }]

// DeepSeek OCR：文件只在宿主侧读盘并上传，插件拿不到字节，也不会进入对话上下文。
const ocr = await efto.ai.ocr({ path: 'D:\\reports\\q3.pdf' });
// ocr = { id, purpose, pageCount, fileName, pages: [{ index, text, finishReason }] }
```

行为约定：

- **地址与密钥只能来自设置**：插件不能传 `base_url` / `api_key`，宿主也不读这些字段 ——
  否则任何插件都能拿到用户的密钥。
- `messages[].role` 只接受 `system` / `user` / `assistant`。
- `messages[].content` 可以是字符串，也可以是内容数组：`{ type: 'text', text }` /
  `{ type: 'image', path }`，其它 `type` 一律明确报错。图片只支持
  png / jpg / jpeg / webp / gif / bmp，单张上限 10MB；宿主读盘失败、格式不支持、文件为空
  都会带下标报错。
- HTTP 非 2xx、响应缺少 `choices[0].message.content`、内容为空都会 reject，
  错误里带状态码与服务端 `error.message`。
- 默认模型由 `settings.json` 的 `default_model` 决定；列表非空时后端保证它指向现存模型。
- `webSearch` / `readUrl` 是 DeepSeek 的服务端工具，只有 DeepSeek 端点支持，且推荐
  `deepseek-flash`；请求会显式带上 `tools: [{ type: 'deepseek_web_search' }]` /
  `[{ type: 'deepseek_read_url' }]`。其它网关会由服务端返回错误，宿主原样带出。
  流式时 `onDelta` 还会收到 `{ toolEvent }`（`call_started` / `call_completed`），
  最终结果额外带 `serverToolCalls` 与 `annotations`（引用链接）。
- `ai.ocr` 只支持 DeepSeek 的 `POST /v1/tools/ocr`：单文件上限 100MiB、总时长 180 秒，
  支持 PDF / Word / PowerPoint / PNG / JPEG / WebP / GIF / BMP / TIFF。文件不会存到
  Files API，也不会自动进入对话；需要模型基于它回答时，由插件自己把返回文字拼进 messages。

示例插件：`plugins/translate`（`翻译 <文本>` / `fy <文本>`，自动中英互译，结果内联展示）、
`plugins/ocr`（选图/拖图后调用默认模型识别文字、表格、公式）、
`plugins/api-manager`（Postman 风格 API 管理：集合 / 环境变量 / 历史；UI 与命令共用同一工作区，
命令结果带 `EFTO_API` 前缀，见插件目录的 README）。

## 4. UI 页面写法

页面里直接用注入好的 SDK：

```html
<script>
  efto.onMessage('greet', (data) => { render(data); });
  document.querySelector('#copy').onclick =
    () => efto.ipcMessage('copy-greet', { text: current });
  efto.db.get('k').then(v => console.log(v));   // UI 侧走 IPC，返回 Promise
</script>
```

UI 侧另有三个文件相关能力：

- `efto.pickFiles()` → `Promise<string[]>`：原生多选文件框，取消返回空数组。
- `efto.onFileDrop(cb)` / `efto.onFileDrag(cb)`：系统文件拖拽。前者在松手时给绝对路径
  `cb(paths)`；后者给 `cb(true/false)` 用于高亮拖拽区（走 Tauri 的拖拽事件，HTML5 `drop` 拿不到真实路径）。
- `efto.shellShowItemInFolder(path)`：在资源管理器中定位文件。

主题：`efto.isDarkMode()` / `efto.isLightMode()` 是**函数**，不是布尔属性。插件窗口跟主窗口
用同一套主题设置（跟随系统 / 浅色 / 深色）：宿主建窗口时按设置锁定窗口主题，设置变更时也会
同步已打开的插件窗口。页面在 `<head>` 里据此写 `document.documentElement.dataset.theme`；
要在窗口打开期间跟随切换，监听 `matchMedia('(prefers-color-scheme: dark)')` 的 `change`
事件重画（一方插件 `file-share` / `ai-chat` / `hosts` / `ocr` / `quickweb` / `text-beautify` /
`hello-js` 都已这样处理）。

注意 main 侧与 UI 侧的 `efto.db` **形态不同**：main 里是同步返回值，UI 页面里是 Promise（跨进程必须异步）。这是本项目刻意的设计，别在 UI 里写 `var v = efto.db.get(k)` 然后直接用。

## 5. TypeScript

`main.ts` 由宿主用 SWC **原地剥类型**，不需要 tsc/构建步骤。支持：类型注解、`interface`、`type`、泛型、`as`、非空断言、`import type`（会被删除）、可选参数与默认值。

**不支持**（会报错并提示"请预编译"）：`enum`、带运行时值的 `namespace`/`module`、装饰器、`export =`/`import =`、JSX/TSX。
另外注意：`import type` 可以（纯类型，被删掉），`import { x } from './a'` 不行（这是运行时的 ESM 加载，见上一节）。

```ts
// 用 const 对象代替 enum（推荐写法）
const Mode = { Auto: 'auto', Sec: 's' } as const;
type Unit = typeof Mode[keyof typeof Mode];

// lib/format.ts —— 被 require 的模块也写 CJS
function mul(n: number): number { return n * 2 }
exports.mul = mul;
```
```ts
// main.ts
var f = require('./lib/format');
exports.onPluginEnter = (info: { payload?: unknown }) => {
  efto.showNotification(String(f.mul(Number(info.payload ?? 0))));
};
```

把 `plugin-sdk/efto.d.ts` 拷进插件目录（或 `tsconfig.json` 的 `include` 里指向它），编辑器就有完整提示。

## 6. 调试

**不启动 GUI 就能跑插件**——这是本项目内置的验证通道，插件作者可以用它在 CI 里测 main 逻辑：

```powershell
$env:EFTO_PLUGIN_DIRS = "D:\my-plugins"
.\target\debug\efto.exe cli plugins                       # 看发现了什么、清单有什么问题
.\target\debug\efto.exe cli plugin-run my-plugin my-keyword "负载内容"
.\target\debug\efto.exe cli search chrome                 # 顺带验证索引
```
`plugin-run` 会启动插件后端、投递一次 `onPluginEnter`、等待 400ms 收集事件，然后打印能力调用回显
（`[notify]` / `[copy]` / `[ui-msg:xxx]`）与错误，非零退出码表示有 `Error` 事件——可直接接进 CI。

- 异步链路（`efto.node.run` / `efto.ai.chat`）默认等不到结果，用 `--wait-ms N` 放大预算；
  `--json` 输出 `{ok, running, events, logs, lines, errors, stopped, enter_error, flush_error}`，
  适合脚本断言。插件 `console.*` 走事件通道，`--json` 里落在 `logs`（顺带在 `events` 里）；
  `efto.output` 与宿主能力回显落在 `lines`。面向 agent 的机器可读结果用
  `console.log('MAGIC ' + JSON.stringify(...))`，从 `logs[].text` 里取。
- **插件写入的 KV 会在 `plugin-run` 退出前落盘**：宿主先 `runtime.stop` 再
  `StoreManager::flush_all`，所以「一条命令写入、下一条命令读取」跨进程可见；
  `flush_error` 非空时 `ok=false`、退出码非零。
- 负载是独立参数，JSON 负载在 PowerShell 5.1 里会被引号规则吃掉（含空格还会被拆开）。
  需要传 JSON 时优先写文件，再由插件自己提供 `--json-file` / `--file` 之类的入口。

```powershell
# 详细日志（含能力调用、协议请求、JS 异常栈）
$env:RUST_LOG = "debug"; ..\..\scripts\dev-run.ps1
```

- JS 抛出的异常会以 `PluginEvent::Error` 上报，搜索框右下角 toast 显示，同时进宿主日志。
- 插件加载失败（清单不合法、TS 语法被拒）在设置页（搜索"设置" / 托盘 / `Alt+P`）的插件列表里会显示问题描述，不会让应用崩溃。
- 宿主回调内部若有 Rust panic，会被 panic 屏障转成 JS 异常（不会把启动器整个打死），日志里能看到 `内部 panic` 字样。

## 7. 打包与分发

- 目录拷贝，或把目录压成 `.zip` 后在设置页导入。
- 设置 → 插件 → 导入，支持 `.zip` 包或已解压目录；导入时把每个插件复制到
  `~/.efto/plugins/<name>`。同名插件不会被覆盖，需先删除旧目录。
- **一个包可以装多个插件**，两种布局都认：
  - 插件目录直接在包根：`ai-chat/efto-plugin.json`、`beta/efto-plugin.json`
  - 外面多套一层容器目录：`plugins/ai-chat/efto-plugin.json`（扫描最多三层）
  包内某个插件失败（重名、清单坏了）不影响其它插件，设置页会提示未导入的项；
  全部失败才整体报错。
- 导入后自动重建索引，不需要重启。

### 停用 / 启用

- 设置 → 插件 每个插件右侧有「停用 / 启用」按钮，状态持久化在
  `~/.efto/settings.json` 的 `disabled_plugins` 里（存插件 id）。
- **停用 = 不加载**：立即释放该插件已启动的 isolate，从搜索、触发词、`efto://` 唤起入口
  全部消失；目录和清单原样保留，不会删除文件，随时可以再启用。
- **启用 = 重新加载**：重新发现该插件并恢复搜索；运行时按需启动（下次唤起时才建 isolate）。
- CLI 同源：`efto cli search` / `cli plugin-run` 也会跳过停用的插件，
  `cli plugins --json` 里带 `enabled` 字段。
