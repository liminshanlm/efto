# verify 冒烟插件改用临时夹具

- **Date**: 2026-11-21
- **Status**: done
- **相关**: `scripts/verify.ps1`、`scripts/verify.sh`、`docs/verification.md`

## 问题

两侧 verify 的「plugin-run: plain JS plugin」冒烟都用仓库插件 `hello-js`。
用户在设置页停用某插件后，`~/.efto/settings.json` 的 `disabled_plugins` 会包含它，
而 `PluginRegistry::get` 会过滤停用插件（`registry.rs` 的 `p.enabled` 判断），
于是 `cli plugin-run hello-js` 报「未知插件」并以 exit 3 结束。

结果是一条与构建无关的检查变红：本机把 hello-js 停用后，
`verify.ps1` 稳定报 `1 CHECK(S) FAILED`，容易被误读成代码回归。

## 做法

- 两侧脚本都在 `target/verify-tmp/smoke-plugin/` 生成一次性夹具：
  - `efto-plugin.json`：`{"name":"verify-smoke","version":"0.0.1","main":"main.js"}`
  - `main.js`：`efto.onPluginEnter` 里调用 `efto.output([{ title, subtitle, copy }])`
- 把夹具目录追加到 `EFTO_PLUGIN_DIRS`（Windows `;`、macOS/Linux `:` 分隔），
  冒烟改为 `cli plugin-run verify-smoke greet hello-efto`。
- 夹具 id 不会出现在用户的停用列表里，因此检查结果只取决于构建与运行时。
- 冒烟语义不变：仍覆盖 JS 插件启动 → `onPluginEnter` → payload → `efto.output` 的完整链路。

## 注意

`efto.output` 要求数组，且每行必须有 `copy` 或 `open` 字段；
第一版夹具写成字符串、第二版漏了 `copy`，都被 `plugin-run` 的错误事件拦下
（exit 1），改成 `{ title, subtitle, copy }` 后通过。

## 验证

- `powershell -ExecutionPolicy Bypass -File scripts/verify.ps1` → `ALL CHECKS PASSED`，
  日志里插件数由 11 变为 12（多出 `verify-smoke`），冒烟行 `OK`。
- `verify.sh` 已按同一契约同步（夹具建在 `target/verify-tmp`，冒烟 id 同为 `verify-smoke`）。
