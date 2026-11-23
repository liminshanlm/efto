# 插件声明 fileTypes 并注册系统「打开方式」

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `docs/plugin-development.md`（fileTypes / open-file 协议）、`docs/architecture.md`（文件关联）、
  `docs/macos-parity-todo.md`、`crates/core/src/apps/assoc.rs`

## 做了什么

插件清单新增 `fileTypes`（如 `["md", "markdown"]`）。设置 → 插件 里，声明了扩展名的插件会出现
「注册到「打开方式」」按钮，点一次写入：

- `HKCU\Software\Classes\efto.<plugin_id>`：默认值 = `"<插件名> (efto)"`、`DefaultIcon`、
  `shell\open\command = "<efto.exe>" --open-file "%1" --plugin "<plugin_id>"`；
- 每个 `HKCU\Software\Classes\.<ext>\OpenWithProgids` 下登记 `efto.<plugin_id>`（REG_NONE 空值）。

免管理员、不改默认程序；取消注册只删 efto 自己写的 ProgID 与 OpenWithProgids 值。
`efto.exe --open-file <路径> [--plugin <id>]` 由 `lib.rs::parse_open_file` 解析，单实例机制把
argv 转给已运行实例；`commands::open_file_with_plugin` 按 `--plugin`（缺省按扩展名）找到插件，
以 `cmd = "open-file"`、`payload = 绝对路径` 调用 `onPluginEnter`。

markdown 插件：`main.js` 识别 `info.code === 'open-file'` 后推 `open-file` 给页面（`init` 握手时补发），
`ui.js` 收到后走 `confirmDiscard()` + `loadPath()`。宿主在路由时把该路径登记进 `FileGrants`
（与拖入同一条授权），否则插件的 `efto.file.read` 会被"只能读取你选择或拖入的文件"拦下。

## 为什么这么定

- **一处声明，两处使用**：`fileTypes` 同时驱动路由与设置页按钮，避免"注册了但打不开"。
- **`--plugin` 不能省**：同一扩展名可能被多个插件声明（如 markdown 与纯文本编辑器），
  用户在「打开方式」里选的是哪一个，必须原样告诉宿主，否则只能按扩展名猜。
- **只写 HKCU**：不弹 UAC、不影响默认程序；`OpenWithProgids` 只增删自己那一项，
  不碰 `.md` 键上的其他关联。

## 验证到哪一步

见 `docs/verification.md`「插件「打开方式」注册（2026-11-23）」：单测 + 编译 + 前端/i18n 检查通过；
资源管理器右键实跑需重启 efto 后由人手确认（测试机已有旧实例，单实例会把 argv 转给旧进程）。
