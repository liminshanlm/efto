# file-share：取消分享后重开又变回分享中

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `plugins/file-share/main.js`、`docs/verification.md`「file-share 取消分享后重开又自动分享」

## 现象

点「取消分享」→ 关闭插件窗口 → 重新打开，顶栏又显示"正在分享"，共享地址也回来了。

## 根因

`onPluginEnter` 的判据是"KV 里有 items 就自动启动"：

```js
if (rawItems.length) { sharing = true; apply(); }
```

「取消分享」只把进程内 `sharing` 置 false，既没落盘，也没影响下次进入的判断。
关闭窗口时 `onUiClosed` 停掉服务，但条目还在 KV 里，于是重开必然重新 `apply()`。

## 修法

- KV 新增 `wantShare` 表示用户意图：`start`/`add` 置 true，`stop`/`clear` 置 false；
  `onPluginEnter` 改为 `rawItems.length && wantShare` 才自动启动。
- 缺键时按 true 处理：升级前只存过 items 的用户保持原有"打开即恢复分享"行为。
- 停止状态也要能看列表：新增 `itemsMeta` 缓存宿主校验后的
  `{name,path,is_dir,size}`，`apply()` 成功后落盘、停止时打开据此渲染；
  停止状态下删条目同步更新，避免下次打开"复活"。
- `onUiClosed` 仍然停服务，但不再改动 `wantShare`：单纯关窗口不等于取消分享。

## 验证

- `node --check plugins/file-share/main.js` 通过。
- 未验证：GUI 里真实走一遍 取消分享 → 关窗口 → 重开；停着时外部删除文件后的列表展示。
