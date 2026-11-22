# 截图调试模块未按平台门控，Windows 测试编译失败

- **Date**: 2026-11-22
- **Status**: done
- **相关**: `crates/app/src/screenshot.rs`、`crates/app/src/tile.rs`

## 现象

上游 macOS 提交（`40d55e6..2aad278`，截图/i18n/窗口层）合入后：

- `cargo build --workspace` 通过；
- `cargo test --workspace` 在 Windows 上**编译失败**：

```
error[E0433]: cannot find `ax` in `tile`
  --> crates\app\src\screenshot.rs:822:33
   let disp = crate::tile::ax::displays();
note: found an item that was configured out
  --> crates\app\src\tile.rs:13-15
   #[cfg(target_os = "macos")]
   #[path = "ax.rs"]
   pub(crate) mod ax;
```

## 根因

`screenshot.rs` 底部的 `debug_dump` 是 macOS 专用的手工调试模块
（`dump_virtual_screen`，`#[ignore]`），依赖 `crate::tile::ax`。
但模块只标了 `#[cfg(test)]`，没有限定 `target_os = "macos"`，
于是 Windows 的 lib test 也会编译它，而 `tile::ax` 在 Windows 不存在。

`cargo build` 不编译 `cfg(test)` 代码，所以二进制构建看不出问题——
这是"build 绿、test 红"的典型平台门控漏洞。

## 修法

```rust
#[cfg(all(test, target_os = "macos"))]
mod debug_dump {
```

最小改动，不触碰运行时代码与 Windows 路径。

## 验证

- `scripts/cargo.ps1 -Cmd "test --workspace -j 1"` → 416 passed / 0 failed
  （app 115、core 253、jsrt 48）。
- `scripts/verify.ps1` → ALL CHECKS PASSED（前端守卫、i18n 412 keys、
  时间探针、索引 935 条、12 插件、plugin-run 夹具/TS/hosts/translate、
  Everything 内联、run-command、files、focus、launch+精确 PID 清理）。

## 待留意（未改）

Windows 侧还有 5 条新警告，均不阻塞：

- `windows.rs:546` `display` 参数未使用——该参数只在同函数内的
  `#[cfg(target_os = "macos")]` 块里传给 `tile::ax`，Windows 只有一个覆盖层窗口，
  不需要按屏定位，属预期，不是功能缺口；
- `permissions.rs:50/61`、`windows.rs:762/789` 在 Windows 为 dead code（mac 专用）。
