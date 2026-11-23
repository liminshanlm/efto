# 合规清理：quickweb 自绘图标、es.exe 去仓库化、许可与 uTools 遗留

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `plugins/quickweb/*`、`crates/core/src/exec.rs`、`crates/app/src/commands.rs`、`frontend/settings.*`、`LICENSE`、`THIRD_PARTY_LICENSES.md`、`licenses/*`、`plugin-sdk/`、`docs/verification.md`、`docs/macos-parity-todo.md`

## 背景

静态合规审计发现：quickweb 再分发大量第三方 Logo 且来源不可考；`tools/es/` 跟踪了
voidtools 的 `es.exe`/`es.zip`；仓库声明 MIT 但没有 LICENSE，也没有第三方许可清单；
`plugin-sdk/utools.d.ts` 与两个旧兼容脚本仍与 AGENTS「不提供兼容层」冲突。

## 决策与改动

1. **quickweb 按数据优先重写**
   - `lib/sites.js` 只保留 `code/cmd/url/group/needsQuery`，删除全部 `icon` 字段与 `res/` 图标。
   - UI 图标改为自绘：触发词首字 + 名称散列色相（`--avatar-hue`），不再加载任何图片。
   - 插件图标用 System.Drawing 重新绘制为透明圆角 PNG；版本从上游 4.1.2 改为 1.0.0。
   - 删除只覆盖 React 的 `LICENSE.txt`（当前 UI 是原生 JS，不再随带 React）。
   - `res/` 下清空后剩余的空目录一并移除，插件目录只留 `logo.png`。
2. **es.exe 不再随仓库分发**
   - 删除 `tools/es/{es.exe,es.zip,manifest.json}`，`.gitignore` 忽略 `/tools/es/`。
   - `exec.rs` 去掉「仓库 tools/es」候选；新增 `install_from_local`，zip 与裸 exe 都强制 sha256。
   - 设置页新增「手动下载」（打开官方 `es.zip`）与「从本地安装」（文件选择 + sha256），
     自动下载失败时提示走这两个入口。
   - zip 解析测试改为合成 stored/deflate 夹具，不再依赖真实 es.zip。
3. **许可材料**
   - 新增根 `LICENSE`（MIT，efto contributors）。
   - 新增 `THIRD_PARTY_LICENSES.md` 与 `licenses/`：V8 BSD-3、MIT、Apache-2.0、MPL-2.0、Unicode-3.0。
   - `efto-app`、`efto-jsrt` 补 `license.workspace = true`。
4. **dev-tools**
   - 新增 `THIRD_PARTY_NOTICES.md`，记录 Ctool MIT 来源与 assets 内嵌库的许可标记。
   - `icon_utools.png` / `icon_chrome.png` 当次按用户要求保留（未被 efto UI 引用）；2026-11-23
     开源整备时两者均已从工作区与 Git 历史中移除，见 `.agents/notes/implemented/architecture/2026-11-23-open-source-history-purge.md`。
5. **uTools 遗留清理**
   - 删除 `plugin-sdk/utools.d.ts`、`scripts/check-compat.js`、`scripts/prove-compat-guard.py`。
   - `crates/core/src/plugins/registry.rs` 的 `DynamicItem` 注释改为指向 `efto.setSearchItems`。
   - `docs/verification.md` 同步说明仓库只保留 `efto.*` 契约。

## 验证

- `cargo check -p efto-core -p efto-app` 通过（仅既有 dead_code 警告）。
- `cargo test -p efto-core --lib exec`：25 passed；`cargo test -p efto-app --lib host`：13 passed。
- `node scripts/check-i18n.js`：463 keys、2 语言一致。
- `node scripts/check-frontend.js`、`node --check frontend/settings.js`、quickweb 两个 JS 语法检查通过。
- 未验证：设置页新按钮的真实 WebView2 点击、quickweb 管理界面的真实宿主渲染、真实手动下载 zip 的安装链路。

## 残留

- dev-tools 的两个第三方图标文件按用户要求保留。
- `tools/es` 删除后，本机若仅在仓库缓存中存在 es.exe，需要重新下载或从本地安装。
