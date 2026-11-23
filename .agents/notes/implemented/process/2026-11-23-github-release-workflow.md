# GitHub Release 发布工作流

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `.github/workflows/release.yml`、`README.md`（下载一节）、`docs/verification.md`

## 背景

仓库迁到 GitHub 后，产物仍是本地 `target/release/efto.exe`，外部用户没有可下载的入口。
GitHub 提供两条通路：Actions artifacts（需登录、临时）与 Release assets（公开、长期）。
本项选择 Release：打 `v*` tag 时构建并上传。

## 产物设计

| 资产 | 内容 | 对应用法 |
| --- | --- | --- |
| `efto-<ver>-windows-x64-portable.zip` | `efto.exe` + `plugins/` + `LICENSE` | 解压即用（`bundled_plugin_dir` 读 exe 同级 `plugins/`） |
| `efto_<ver>_x64-setup.exe` | NSIS 安装包 | `tauri build --bundles nsis`，`bundle.resources` 把插件打进安装目录 |
| `<plugin>-<version>.zip` | 单插件 | 设置页导入或解压到 `~/.efto/plugins/<name>/` |

## 关键判据

- **不复用 `scripts/bootstrap.ps1` / `cargo.ps1`**：两者硬编码 `C:\BuildTools-MSVC` 并依赖 UAC
  安装 VS，runner 上必然失败；runner 自带 rust 与 MSVC，直接调 cargo / tauri CLI 即可。
- **V8 不入库但必须校验**：下载后与 `.cargo/config.toml` 里的 sha256 比对，避免升级 tag 时
  文件与哈希漂移；哈希只有一处事实源。
- **只构建一次**：绿色版复用 `tauri build` 产出的 exe，避免 cargo build 与 tauri build 各编一遍。

## 结果（2026-11-23）

- 首次实跑 run `45129852465` 全部步骤 success，Release `v0.1.0` 共 15 个资产：绿色版
  26.22 MB、NSIS 18.05 MB、13 个插件 zip。
- 绿色版下载解压后 `efto.exe cli plugins` 输出「发现 13 个插件」（exit 0）；插件 zip 根为
  `efto-plugin.json` 等文件本身。
- `ci.yml` 两个 gate（windows / macos）在 runner 上均通过，发布无需为 runner 另开构建路径。

## macOS（2026-11-23 补充）

- 一开始只出 Windows 是有意为之：macOS 未签名会被 Gatekeeper 拦、且 parity 清单仍有待同步项。
  本次决定先发未签名的 Apple Silicon 包（Intel 暂不出）。
- **坑**：`runs-on: macos-14` 在 2026 年已下线，job 启动即 cancelled，annotation 为
  `configuration 'macos-14-arm64-us-default' is not supported`；换 `macos-15`（arm64）即恢复。
- `tauri build --bundles app,dmg` + `APPLE_SIGNING_IDENTITY=-`（ad-hoc），产物
  `efto-0.1.0-macos-arm64.dmg` 24.78 MB；未公证，用户需右键打开或去隔离属性。
- 两个构建 job 只上传 artifact，新增 `publish` job 统一下载后创建 Release，避免并发建
  release 撞 422。
- 验证方式：下载 dmg 检查尾部 UDIF `koly` trailer；未在真实 macOS 上挂载安装。

## 未验证 / 后续

- NSIS 安装包的实际安装与 SmartScreen 提示未实测；dmg 的 Gatekeeper 解除路径未实测。
- 未签名（macOS 未公证）：代码签名、公证与自动更新仍留在 roadmap。
- Intel x64 包未提供（可后续用 `macos-15-intel` + 交叉编译 aarch64）。
