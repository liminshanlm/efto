# AGENTS.md

本文件是 efto 仓库的权威施工约定。改动前先读这里，再读对应的 `docs/` 文档。

## 这是什么

Windows 桌面启动器：**应用快速启动 + JS/TS 插件运行时**。
Tauri 2 + 内嵌 V8（rusty\_v8 152）+ deno\_ast/SWC 类型剥离，不依赖 Node/npm/tsc。

```
Alt+Space 唤起 → 输入 → 回车启动应用 / 唤起插件
```

## 目录与依赖方向

```
crates/core/   应用发现、.lnk 解析、索引、搜索打分、插件清单、存储   无 UI / 无 JS 依赖
crates/jsrt/   rusty_v8 引擎、efto API 垫片、TS 转译、能力线程池   无内部依赖
crates/app/    Tauri 2 宿主：托盘、热键、窗口、efto:// 协议、Host 实现
frontend/      搜索框（纯静态 HTML/CSS/JS，无打包器）
plugins/       示例插件：hello-js、timestamp-ts、file-share（局域网只读共享，Node 服务）、ipmsg（局域网文本消息）、hosts（UAC 提权编辑 hosts）
plugin-sdk/    efto.d.ts
scripts/       构建/验证脚本（.ps1 必须保持纯 ASCII）
docs/          architecture / plugin-development / roadmap / verification
vendor/        rusty_v8 预编译静态库（离线构建用）
```

依赖单向：`app → {jsrt, core}`，`jsrt` 与 `core` 互不依赖。jsrt 通过 `ipc::Host` trait
反向索取能力，因此可以脱离 GUI 单测。

## 构建与验证

必须在 PowerShell 下经脚本跑，不要在 Git Bash 里直接 `cargo`（`/usr/bin/link.exe` 会抢走 MSVC 链接器）。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/bootstrap.ps1            # 首次准备工具链
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 -Cmd "test --workspace"
powershell -ExecutionPolicy Bypass -File scripts/dev-run.ps1              # 起 GUI
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1               # 无 GUI 端到端（cli 通道）
```

- `scripts/verify.ps1` 驱动 `efto.exe cli`（index / search / launch / plugins / plugin-run），
  并跑前端与时间探针检查。
- 单模块可单独测：`scripts/cargo.ps1 -Cmd "test -p efto-jsrt"`（会真的起 V8）。
- 无 UI 的脚本化验证优先走 `cli` 子命令，不新开 GUI 自动化。

## 硬性约定

1. **名字边界不要"顺手统一"**。产品身份统一用 `efto`：crate 名、`efto.exe`、`EFTO_*`、
   `~/.efto`、bundle id。插件统一用 `efto-plugin.json` 描述，运行时只暴露 `efto.*`、
   `window.efto`、`efto://`、`__efto_dispatch`、`plugin-sdk/efto.d.ts`。
   **不提供任何第三方启动器的兼容层**：不读别的清单文件名当插件清单，不注入兼容全局对象，
   不注册兼容协议。宿主内部命令名（`plugin_*`、`plugin:message`）是 efto 自己的 IPC 契约。
2. **搜索源不许互相劫持**。计算器、时间戳、系统时间、Everything 文件检索各有严格入口判定
   （文件源分数必须低于应用条目最低分），"宁可不出，不可乱出"；新增搜索源必须补上"不该触发"的反例测试。
3. **未实现的能力要明确报错**，不能静默成功（见 `docs/roadmap.md`）。
4. **外部程序执行唯一入口是 `core/src/exec.rs`**，GUI 与 CLI 共用，不得各写一份校验。
5. **插件 UI 的三条硬约束**（独立线程建窗口、`ui.ipcMessage` / `ui.postMessage` 方向分离、
   窗口就绪前的消息排队）见 `docs/architecture.md`，改动前必读。
6. **`.ps1` 保持纯 ASCII**（PowerShell 5.1 按 ANSI 解码，中文注释会破坏语句）。
   中文字面量请用码点构造。
7. **TS 只做类型剥离**：不生成运行时代码，遇到 `enum`/带值 `namespace`/装饰器/JSX 明确报错。
8. 线程与锁：锁顺序 `settings → registry → index`；宿主能力不同步等插件线程，插件线程不等主线程。
9. **功能变更时同步更新帮助文档**。用户可感知的行为（命令、参数、插件 API、快捷键、界面提示、
   边界判定）改了，必须在同一次改动里更新对应的 `docs/` 文档（architecture / plugin-development /
   verification / roadmap）以及 `plugin-sdk/efto.d.ts` 里相关的类型与注释；文档没跟上视为未完成。
10. **Windows 侧的功能改动要落一份 macOS 同步清单**。在 Windows 上新增或改变用户可感知功能时，
    同一次改动里往 `docs/macos-parity-todo.md` 追加一行：功能点、Windows 实现位置（文件与入口）、
    macOS 侧待办（缺失 / 需适配 / 已具备）、同步要点与验证方式。macOS 侧补齐后把该行状态改为
    `已同步` 并注明提交。只靠聊天记录或事后回忆不算完成。

## Agent Notes 约定

### 一个事实只有一个家

权威约定在本文件；钉钉源文档是需求事实的来源；`.agents/notes/` 只装 agent 向的施工账本与
踩坑判据（为什么这么改、坑在哪、验到什么程度）。短 note 引用不复制，账本不取代本文件的硬性规则。

### 两轴编码

- `lifecycle` = `proposed` / `implemented` / `rejected` / `archived`
- `class` = `feature` / `bug-fix` / `simplification` / `architecture` / `process` / `testing`

路径：`.agents/notes/<lifecycle>/<class>/yyyy-mm-dd-<slug>.md`
（不含编号，slug 用 kebab-case）。目录随第一条 note 一并创建，不预建空目录——
但"不预建"不等于"可以不写"：第一次需要落 note 时就创建对应目录。

### 何时必须写

以下改动在**汇报完成之前**必须落一条 note，**没有 note 视为未完成**：

- 新增或改变用户可感知的行为（`feature`）
- 修复有根因的 bug（`bug-fix`）
- 重构、简化、架构调整（`simplification` / `architecture`）
- 改变开发/验证流程（`process` / `testing`）

纯文案、纯格式、依赖升级、一次性排障脚本不写 note；拿不准时按"是否改变了事实、判据或
别人后续要遵守的约束"判断。动手前可以先落 `proposed`，完成后按 lifecycle 移到
`implemented/<class>/`，并让 note 的 `Status` 与所在目录一致。

### 短 note 骨架

```markdown
# <标题>

- **Date**: yyyy-mm-dd
- **Status**: proposed | in-progress | done | rejected
- **相关**: 关联的文档 / 原型路径（docs/xxx.md、prototypes/xxx.html…）
```

## 参考文档

- `docs/architecture.md` —— 分层、.lnk 解析、打分优先级、各类判定闸门、线程模型
- `docs/plugin-development.md` —— 插件 API、TS 写法、UI 通信
- `docs/verification.md` —— 已验证内容与"没验证过什么"
- `docs/roadmap.md` —— 未实现项
- `docs/macos-parity-todo.md` —— Windows 已实现功能的 macOS 同步清单
