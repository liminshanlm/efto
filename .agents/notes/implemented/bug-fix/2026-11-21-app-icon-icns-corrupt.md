# 应用图标损坏：tauri ico→icns 转换把小尺寸图块打成彩条

- **Date**: 2026-11-21
- **Status**: done
- **相关**: `scripts/gen-icons.js`、`crates/app/tauri.conf.json`、`crates/app/icons/efto.icns`

## 现象

系统"屏幕录制"权限列表里 efto 的图标是一团彩色横条纹（用户截图实证）。

## 根因

tauri-bundler 从 `icon.ico`（Windows 图标，16/32/48 三档）生成 mac icns 时，
小尺寸图块解码/编码错误（字节序或行距问题），48×48 图块变彩条；128 档
（来自 128x128.png）是好的。系统列表显示小图 → 看到花图。`iconutil` 解剖
icns 逐档目视实证：48px 坏、128px 好。

## 做法

绕开转换：`gen-icons.js` 用同一套程序化绘制（`png(size)`）直接产出 iconset
全部 10 档（16…1024，无放大损失），`iconutil -c icns` 打成 `icons/efto.icns`，
`bundle.icon` 把 icns 放首位——bundler 拿现成 icns，不给它转换的机会。

## 判据

- 重新打包后 `iconutil -c iconset` 解出的 32@2x（64px，原花图档位）目视干净；
- 全量测试 145/241/48 绿。

## 教训

icns/图标这类二进制资产，**逐档解出来看一眼**才算验过；bundler 的格式转换
不可信时，自己生成目标格式直接喂给它。
