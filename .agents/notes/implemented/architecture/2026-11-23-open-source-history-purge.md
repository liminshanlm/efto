# 开源前仓库整备：历史清洗 + V8 归档出库

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `docs/verification.md`（「开源前仓库整备」）、`README.md`、`.cargo/config.toml`、`.github/workflows/ci.yml`

## 为什么

准备公开发布到 GitHub。此前 `f903bdd` 已从工作区删除 `tools/es/es.exe`、uTools 兼容文件与
quickweb 第三方 Logo，但它们在历史里仍可取出，公开后等同于继续分发；另外 `vendor/rusty_v8`
两个 `.gz`（约 77MB）入库，且 `.cargo/config.toml` 把 `RUSTY_V8_ARCHIVE` 固定指向 Windows 那份。

## 做了什么

- `git filter-branch --index-filter` 改写 46 个提交，移除：`tools/es/`、`plugin-sdk/utools.d.ts`、
  `scripts/check-compat.js`、`scripts/prove-compat-guard.py`、`plugins/quickweb/res/`、
  `plugins/quickweb/LICENSE.txt`、`plugins/dev-tools/icon/icon_utools.png`、`vendor/rusty_v8/*.gz`，
  以及初版 quickweb logo 的 blob（旧图是上游素材，当前 logo 为自绘）。
- `vendor/rusty_v8/` 进 `.gitignore`；`.cargo/config.toml` 的 `[env]` 保留，仍指向该路径，
  由 `scripts/bootstrap.ps1` 落盘，本地与公开仓库走同一条构建路径。
- 顺手修：CI 注释里的私有 Codeup 说明改成 GitHub Actions 语义；dev-tools 的
  `THIRD_PARTY_NOTICES.md` 删掉已移除的 `icon_utools.png` 条目。

## 判据 / 结果

- `git log --all --name-only` 匹配不到上述路径；`git fsck --full` 无输出；提交数仍 46；
  `size-pack` 5.70 MiB（清洗前 82 MB bundle）。
- 备份 bundle：`.tmp/oss-backup/pre-oss-all.bundle` 与 `pre-icon-chrome.bundle`（`.tmp/` 不入库）。
  **历史已重写，不能再推到旧 Codeup 仓**，需 `git remote set-url` 到新 GitHub 仓后强推。
- `icon_chrome.png` 当时保留，后在用户要求下做了第二次 `filter-branch` 单独剔除，工作区与历史均无残留。
- 未做："空 vendor + 直连下载"未实测（本机 GitHub 直连不通）。
