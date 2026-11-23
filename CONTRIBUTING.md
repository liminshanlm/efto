# 贡献指南

efto 是 Windows 桌面启动器 + JS/TS 插件运行时。改动前先读 [AGENTS.md](AGENTS.md)（施工约定）
和 [docs/architecture.md](docs/architecture.md)（分层与判定闸门），再看对应模块。

## 环境

- Windows 10/11 x64，或 macOS（arm64；部分 Windows 专属功能不可用，见
  [docs/macos-parity-todo.md](docs/macos-parity-todo.md)）。
- Rust（见 `rust-toolchain.toml`）、MSVC 工具链、WebView2 Runtime。
- 首次准备：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/bootstrap.ps1
```

`bootstrap` 会把 rusty_v8 预编译库下载到 `vendor/rusty_v8/`（不入库）。直连 GitHub 超时的网络，
先按 `README.zh-CN.md`「构建」第 2 条改成镜像。

## 提交前必须跑

```powershell
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 -Cmd "test --workspace"
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 -Cmd "check --workspace"
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1      # 无 GUI 端到端
```

不要在 Git Bash 里直接跑 `cargo`（`/usr/bin/link.exe` 会抢走 MSVC 链接器）。

## 改动约定（摘要，完整见 AGENTS.md）

- 名字边界不要"顺手统一"：产品身份统一 `efto`，清单统一 `efto-plugin.json`，
  不提供任何第三方启动器的兼容层。
- 搜索源不许互相劫持；新增搜索源必须补"不该触发"的反例测试。
- 未实现的能力要明确报错，不静默成功。
- 外部程序执行只走 `crates/core/src/exec.rs` 一个入口。
- 功能变更同一次改动里更新 `docs/`（architecture / plugin-development / verification / roadmap）、
  `plugin-sdk/efto.d.ts` 与 `docs/macos-parity-todo.md`。
- `.ps1` 必须保持纯 ASCII（PowerShell 5.1 按 ANSI 解码）。
- 新建插件必须有 `logo.png` 并在清单写 `logo`；提交第三方素材前确认许可并登记
  `THIRD_PARTY_LICENSES.md`。
- 主应用保持短小精悍：能用插件实现的复杂功能不进宿主。

## 插件

插件目录在 `plugins/`，每个插件是自包含目录。开发与本地试跑见
[docs/plugin-development.md](docs/plugin-development.md)；打包成可分发 zip：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-plugins.ps1
powershell -ExecutionPolicy Bypass -File scripts/package-plugins.ps1 -Only markdown
```

产物在 `dist/plugins/<name>-<version>.zip`（zip 根就是插件目录），用户可在
设置 → 插件 里导入，或解压到 `~/.efto/plugins/`。

## Agent Notes

影响行为、判据或流程的改动要落一条 `.agents/notes/<lifecycle>/<class>/yyyy-mm-dd-<slug>.md`。
纯文案、依赖升级、一次性排障脚本不用写。格式见 AGENTS.md。
