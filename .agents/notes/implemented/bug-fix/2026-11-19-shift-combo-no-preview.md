# Shift 与其它键组合输入时不再弹预览

- **Date**: 2026-11-19
- **Status**: done
- **相关**: `frontend/app.js`（Shift 预览的按键处理）、`docs/help.zh-CN.md`（Shift 预览说明）

## 现象

Shift 被当作"按住预览"的开关，keydown 即刻打开预览窗口。但 Shift 同时是输入大写字母、
Shift+方向键选词等组合的修饰键——正常打字（Shift+A）会顺带弹出一个预览窗口。

## 根因

判断"用户要预览"的唯一依据是 Shift 的 keydown，而按下的瞬间无法预知紧接着会不会有
另一个键加入组合，于是单按与组合键两种意图被混在同一个事件里。

## 改动

- Shift 按下后延迟 180ms（`PREVIEW_HOLD_DELAY_MS`）再调 `syncPreview`；等待期内任何
  其它键的 keydown 都取消这次开启，并置 `previewSuppressed`，本次按住不再预览。
- 预览已经打开（Shift 按得久）时按其它键，立即 `preview_hide` 收起；若 `preview_show`
  还在途中，其回调看到 `previewSuppressed` 会补一次 `preview_hide`，避免窗口事后才冒出。
- 松手（`stopPreview`）、`main:show`、`preview:closed` 都清掉这个定时器，防止状态残留。

## 判据

- Shift+A 输入大写 / Shift+方向键选词：不弹预览，也不影响输入本身（不 preventDefault）。
- 单独按住 Shift：行为不变（约 180ms 后打开），松手不关窗、再按 Shift 关闭的开关语义不变。
- 已验证：`node --check frontend/app.js`、`node scripts/check-frontend.js` 通过；
  未做 GUI 手测，Shift 组合与长按预览需在 `scripts/dev-run.ps1` 里人工确认。
