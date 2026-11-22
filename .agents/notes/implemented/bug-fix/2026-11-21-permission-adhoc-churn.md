# 权限开了仍反复提示"需要权限"：adhoc 签名每次重编译都换应用身份

- **Date**: 2026-11-21
- **Status**: done（根治：本地稳定签名身份 efto-dev；正式分发仍需 Developer ID）
- **相关**: `crates/app/src/{permissions,screenshot,tile,winclip}.rs`

## 现象

屏幕录制已勾选（设置截图实证开关为开），下次启动用截图仍报"缺少屏幕录制权限"
并弹系统授权框。

## 根因

TCC 授权按**代码签名**认应用。我们是 adhoc 签名：**每次重编译产物的 CDHash 都不同**，
对系统来说就是"另一个应用"——设置里那条"已开启"属于旧产物，新产物并没有授权，
于是 preflight=false → 报错 + `CGRequestScreenCaptureAccess` 又弹系统框。今天反复
rebuild/re-test 正好踩满。裸二进制从终端跑还会把授权记到终端（responsible process），
进一步加重"勾了却不生效"的迷惑性。

## 做法

- **根治**：`scripts/sign-dev.sh` 在 login 钥匙串创建自签代码签名证书 `efto-dev`
  （OpenSSL 3 的 p12 要 PBE-SHA1-3DES 兼容参数才能被 security 工具导入），
  `release.sh` 检测到该身份就让 tauri 以 `efto-dev` 签名——签名身份稳定后
  TCC 授权对后续重编译持续有效（Authority=efto-dev 实证）。
- 缓解：捕获失败路径的系统弹窗每进程最多一次（`request_screen_recording_once`）；
  三处权限文案共用 `REGRANT_HINT`（一个事实一个家）。
- 正式外部分发仍需 Developer ID 签名 + 公证（见 plan 发布工程附录）。

## 判据

- 全量测试绿；`codesign -dv` = Authority=efto-dev；授权一次后跨重编译保持，待用户复验。

## 教训

macOS 权限问题先问"系统认为你是哪个应用"：adhoc 开发产物的身份随编译漂移，
"授权已开但不生效"九成是签名身份对不上，不是 API 用错。
