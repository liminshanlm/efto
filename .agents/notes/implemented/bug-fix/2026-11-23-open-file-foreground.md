# 「打开方式」唤起的插件窗口不激活

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `docs/verification.md`、`docs/plugin-development.md`、
  `crates/app/src/main.rs`、`crates/app/src/windows.rs`

## 现象

`.md` 右键「打开方式」后功能正常，文件也载入成功，但插件窗口没有跑到最前面，
用户看不到反馈，以为没打开。

## 根因

`--open-file` 是资源管理器拉起的新进程；它随即被单实例机制结束，argv 转发给**已在运行的
efto 实例**。窗口由后者创建，但后者不是前台进程，Windows 的前台锁会把
`show()/set_focus()` 静默降级为"只显示、不激活"。

## 修复

1. `main.rs`：`--open-file` 分支在进入 GUI 前调用
   `AllowSetForegroundWindow(ASFW_ANY)`，把本次前台切换权限转交给现有实例
   （权限来自用户启动的那个新进程）。
2. `windows.rs`：新增 `force_foreground`（`AttachThreadInput` 挂到前台线程的输入队列后
   `SetForegroundWindow`，必要时先 `SW_RESTORE`），`open_plugin_ui` 的"复用已有窗口"
   与"新建窗口"两条路径都在显示后调用；顺带修掉了插件窗口被最小化时复用不还原的问题。

## 验证

`cargo build -p efto-app` 通过；实跑需重启 efto 后从资源管理器再点一次「打开方式」。
