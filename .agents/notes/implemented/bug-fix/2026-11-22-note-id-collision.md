# 便签 id 只用纳秒会撞车

- **Date**: 2026-11-22
- **Status**: done
- **相关**: crates/app/src/notes.rs

## 症状

`cargo test -p efto-app` 偶发失败，落在 `notes.rs` 的便签用例上（连续两轮里挂一次）。
第一次当作"并发跑 GUI 时的偶发干扰"放过了，第二轮又复现 → 不是环境噪声。

## 根因

`new_id()` 只取 `SystemTime::now()` 的纳秒时间戳。macOS 上两次连续调用的时钟读数可能
**完全相同**（缓存时钟 + 精度不足），于是两张便签撞同一个 id。

后果不止"测试偶发失败"：`set_text` / `set_geometry` 都按 id 定位，撞 id 意味着
**改 A 便签会改到 B**。这是真实可触发的数据错乱，只是要靠"极快地连开两张便签"才撞得上。

## 修法

纳秒时间戳 + 进程内单调序号（`AtomicU64`）后缀：

```rust
format!("{nanos}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
```

时间戳保留"可排序"语义，后缀保证同进程内永不重复。跨进程撞车的窗口从"同纳秒"
缩到"同纳秒且同序号"，等于消失。

## 验证

`cargo test -p efto-app` 连跑 3 轮全部通过；`cargo test --workspace` 443 passed。
