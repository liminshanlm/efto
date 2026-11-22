# 签名后的 .app 一唤起插件就闪退（V8 hardened runtime 缺 JIT entitlement）

- **Date**: 2026-11-22
- **Status**: done
- **相关**: docs/macos-support-plan.md（发布工程）、crates/app/entitlements.plist、crates/app/tauri.conf.json

## 现象

装到 `/Applications/efto.app` 的签名产物，搜索框输入 `fy` 回车后进程直接消失。
崩溃报告 `~/Library/Logs/DiagnosticReports/efto-*.ips`：

- `EXC_BREAKPOINT / SIGTRAP`，faulting thread 名 `efto-plugin-translate`
- PC 处字节是 `brk #0`，栈回落到 `v8::base::OS::Abort`
- 两次崩溃 offset 完全一致，说明是确定性路径而非竞态

## 排查要点（弯路）

1. 先怀疑 `panic = "abort"`：release 下 `catch_unwind` 失效，V8 回调里任何 panic
   都会杀进程。方向对，但栈不是 Rust panic。
2. CLI 路径（`efto cli plugin-run translate`）怎么都对，GUI 才对不上 —— 说明差异在
   **运行环境**而不是插件逻辑。
3. 关键一步：用同样的 offset 在**自己带符号的构建**里定位 PC。做法是把崩溃报告里的
   `instructionByteStream`（`beforePC`+`atPC`）当字节模式在本地二进制里搜，找到匹配
   位置再映射回 VM 地址，得到 `v8::base::OS::Abort()+60`；用该点算出的偏移差
   (`0x218b80`) 平移其余帧，整条栈就落到
   `V8::FatalProcessOutOfMemory` → `Isolate::Init` → `Isolate::New`。
4. 直接跑 `/Applications/efto.app/Contents/MacOS/efto cli plugin-run ...` 复现，
   V8 自己打印出真凶：
   `Fatal process out of memory: Failed to reserve virtual memory for CodeRange`

## 根因

签名时启用了 hardened runtime（`codesign flags=0x10000(runtime)`），但没有给
`com.apple.security.cs.allow-jit`。hardened runtime 下 V8 无法保留可执行内存给
JIT CodeRange，isolate 一建就 fatal。`target/release/efto` 未签名、没开 hardened
runtime，所以 CLI 一直是好的 —— 这也解释了为什么 verify.sh 全绿却挡不住这个问题。

## 修复

- 新增 `crates/app/entitlements.plist`：`allow-jit` +
  `allow-unsigned-executable-memory`。
- `tauri.conf.json` 增加 `bundle.macOS.entitlements = "entitlements.plist"`。
- `docs/macos-support-plan.md` 发布工程一节写明这条约束与故障表现。

adhoc（`signingIdentity: "-"`）也会带 `runtime` flag，所以两条签名分支都需要这份
entitlements；tauri-bundler 在两种身份下都会应用它（已分别构建验证）。

## 验证

```bash
scripts/release.sh
codesign -d --entitlements - target/release/bundle/macos/efto.app   # 应含 allow-jit
target/release/bundle/macos/efto.app/Contents/MacOS/efto cli plugin-run translate '' 
```

- 修复前：同一条命令输出 `Failed to reserve virtual memory for CodeRange` 并 abort。
- 修复后：`V8 已初始化` → 正常输出「请输入要翻译的文本」，退出码 0。
- adhoc 身份构建同样验证通过。

## 判据（后续别再踩）

**任何启用 hardened runtime 的 macOS 产物，只要内嵌 V8，就必须带 allow-jit
entitlement。** 验证脚本跑的是未签名二进制，测不出这个问题；发布前必须对
`bundle/macos/efto.app` 本体做一次插件唤起冒烟。
