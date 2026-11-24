# hosts 保存"成功但重新加载仍是旧内容"：改为原地写入

- **Date**: 2026-11-24
- **Status**: done
- **相关**: `docs/architecture.md`（插件能力：Windows hosts 写入）、
  `docs/verification.md`（hosts 保存"回滚"排查与原地写入）、
  `docs/macos-parity-todo.md`、`crates/app/src/hosts.rs`

## 现象

hosts 插件里新增条目 → 保存弹出 UAC → 授权后提示"已保存" → 重新加载仍是没有新条目的旧内容。
用户补充：以管理员身份直接编辑 hosts 可以改成功。

## 取证

结论全部来自磁盘与日志，没有靠猜插件状态机：

- `hosts.bak_2026_11_24_16_27_26` 保留了 16:27:25 那次写入的完整内容，是插件
  `serialize()` 的归一化格式且含用户新增条目 → efto 的提权写入本身成功。
- 下一次 reload 时磁盘上已是被回滚的旧内容：`hosts.efto.bak`（写前备份）与当时 hosts
  一致，均无新条目。
- hosts 目录里的 `hosts.bak_<时间>` 快照最早可追到 2025-08，早于 efto 提供该能力；
  仓库内无任何代码写这种文件名；机器上运行 aTrust/Sangfor 套件 → 端点安全软件的
  hosts 防篡改（快照 + 回滚）。
- efto 日志中没有 16:38 那次改动的记录，当前 hosts 的未归一化内容来自用户手工编辑。

## 修复

`crates/app/src/hosts.rs::write_bytes` 由「临时文件 `hosts.efto.tmp` +
`MoveFileExW(MOVEFILE_REPLACE_EXISTING | WRITE_THROUGH)` 原子替换」改为
「先 `fs::copy` 备份到 `hosts.efto.bak` → `OpenOptions(truncate)` 原地写入 → `sync_all`」，
并删除 Windows/macOS 两个 `replace_file` 分支。

写入后等待 1s 再读回比对（`VERIFY_SETTLE`）：不一致返回「写入 … 后被回滚：写入 N 字节，
读回 M 字节，请检查 aTrust … 白名单」，提权助手的 `resp.ok=false` 让插件显示错误，
不再把被回滚的写入显示成"已保存"。

判据：替换式写入（换 inode / 换文件）会触发端点安全软件的回滚，原地写入与记事本等编辑器
保存一致，不被拦。代价是写入中断会留下半截文件，因此备份必须先于写入完成。

## 验证

`cargo test -p efto-app hosts`：5 项通过，含改名的
`write_bytes_backs_up_then_writes_in_place`（第二次写入更短内容必须截断、不产生临时文件）。

**待复测**：GUI 里保存 → 重新加载是否立即可见，hosts 目录是否还新出现 `hosts.bak_<时间>`。
若仍被回滚，则需走 aTrust 白名单/策略，代码侧没有更保守的写入方式。
