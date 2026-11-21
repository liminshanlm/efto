# macOS release 构建（release.sh / .app / .dmg）

- **Date**: 2026-11-21
- **Status**: done（未签名构建可用；签名/公证待 Apple Developer 证书）
- **相关**: `scripts/release.sh`、`crates/app/tauri.conf.json`、`docs/macos-support-plan.md` 附录

## 现象

之前只能 `cargo build` 出裸二进制：`bundle.active` 是关的、tauri CLI 没接、
`.cargo/config.toml` 的 V8 环境只钉 Windows 产物（裸 `tauri build` 会拿错库）。

## 做法

- `bundle.active: true`（targets: all）；`scripts/release.sh`：复用 cargo.sh 的
  V8 环境（按 arch 选 vendor 产物 + sha256）+ tauri CLI 三级回退
  （`tauri` → `cargo tauri` → `npx @tauri-apps/cli@2`，纯构建期工具不进产品依赖），
  在 crates/app 下执行 `tauri build`。
- 顺手收掉 `CGDisplayBounds` 重复声明告警：显示器枚举/显示 scale 归
  `crate::tile::ax` 一家（`mod ax` 提 pub(crate)，ax 增 `display_scale()`），
  screenshot 的 ffi 只留 CGWindowListCreateImage / 位图上下文符号。

## 判据（本机实测）

- 产物：`target/release/efto`、`bundle/macos/efto.app`（54.31 MiB，含自动
  生成的 efto.icns、`LSUIElement=1`、`CFBundleIdentifier=app.efto.launcher`）、
  `bundle/dmg/efto_0.1.0_aarch64.dmg`（19.98 MiB）。
- release 二进制冒烟：`cli capabilities --json`（platform: macos）与 `cli search '6*7'`
  计算器命中；.app 真机启动（进程可见）→ 退出正常。
- 签名状态：`codesign -dv` = **adhoc / linker-signed**（无 Developer ID 的预期形态）。
- 全量测试 144/241/48 绿（含真机截图捕获）。

## 没验证 / 留给后续

- **Gatekeeper**：未签名/未公证的 dmg 外部分发会被拦（右键打开或
  `xattr -dr com.apple.quarantine`）。签名/公证按 plan 附录走
  （signingIdentity + notarytool），需要 Apple Developer 证书。
- x86_64（Intel）产物未构建（vendor 只有 aarch64；`release.sh` 按本机 arch 取库）。
- dmg 挂载安装路径（拖 Applications）人工过一遍。
