# 第三方许可清单重做与已知缺口

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `LICENSE`、`THIRD_PARTY_LICENSES.md`、`licenses/`（新增 `BSL-1.0.txt`）、
  `crates/app/tauri.conf.json`（`bundle.licenseFile` 未设置）、`.github/workflows/release.yml`（打包步骤）

## 为什么

原清单只是几张表 + 一段"MPL 说明"，没有回答最关键的问题：**哪个产物带哪份文本**。核下来的
实际状态是：绿色版 zip 只拷了 `LICENSE` 与 `THIRD_PARTY_LICENSES.md`，`licenses/*.txt` 一个都
没进去；NSIS 没配 `bundle.licenseFile`；mac `.app` 里两份文件都没有。也就是说清单里那一堆
`licenses/*.txt` 相对下载用户全是死链，而 MIT/BSD 要求二进制分发必须随附 Notice 与文本。

## 核查结论（2026-11-23，`Cargo.lock` 670 个包）

- **无 GPL/AGPL 独占依赖**。唯一沾 GPL 的是 `r-efi` 的 `MIT OR Apache-2.0 OR LGPL-2.1-or-later`
  三选一，走 MIT 分支。
- MPL-2.0 实为 8 个：cssparser 0.37/0.38、cssparser-macros、selectors 0.38/0.41、dtoa-short、
  option-ext、smartstring（`MPL-2.0+`）。原文档只列了 cssparser/selectors。
- 原文档的许可族清单漏了 BSL-1.0（clipboard-win / error-code / ryu / ryu-js）、Unlicense（14 个
  双许可）、CC0-1.0（dunce）、0BSD（adler2）；Unicode-3.0 是 ICU4X 那 27 个包。
- 依赖图里没有任何 crate 带 `NOTICE` 文件（核对解包后的 registry 源码），所以 Apache-2.0 的
  NOTICE 聚合义务未被触发。
- `licenses/` 缺 BSL-1.0 文本，已从上游 `ryu-1.0.23/LICENSE-BOOST` 原样拷入。

## 判据（后续要遵守的）

1. 依赖图是唯一事实源，本文档里的数字和名单随时会过期；新增依赖必须重跑文末的重核命令，
   重点看有无新增 `MPL-*` / `(A|L)GPL-*`，以及有没有 `license` 字段为空、只给 `license-file`
   的包（那种必须把文本 vendored 进 `licenses/`）。
2. 说某个插件"自绘"前先核证据。原文档写"插件 logo.png 全是自绘"，但 `dev-tools` 的
   `icon/` 与 `favicon.ico` 来自上游 Ctool（MIT）——已在文档里改成"除 dev-tools 外自绘"。
3. `file-share` 的 logo/UI 来源在仓库里没有任何记录（清单里 homepage 指向 gitee 上游），
   在确认之前不得按原创描述；这条已写进文档的"需要确认"。
4. 改 `dev-tools/assets/` 里的产物时要保留上游 `@license` / `Copyright` /
   `SPDX-License-Identifier` 注释，重压缩重混淆会丢。

## 打包链路一起修了（本次）

三处缺口不再是缺口：

- 绿色版 zip：`.github/workflows/release.yml` 的 staging 步骤增加 `licenses/` 整个目录的拷贝。
- NSIS：`crates/app/tauri.conf.json` 设 `bundle.licenseFile: "../../LICENSE"`，安装包有许可页。
- mac `.app` / 两个 Windows 产物：`bundle.resources` 追加 `LICENSE`、`THIRD_PARTY_LICENSES.md`、
  `licenses`（mac 落到 `Contents/Resources`，NSIS 落到安装目录、与 `efto.exe` 同级）。

之后要加许可文本，改的是这两处配置，不是只改本文件——已写进文档。

## 还没解决的一条

`licenses/MIT.txt` 是通用 MIT 模板、没有版权行，而 MIT/BSD 要的是 **Notice**。逐 crate 的
版权归属清单只能靠生成，正解是从 `Cargo.lock` 跑 `cargo-bundle-licenses` / `cargo-about`
并接进 release job。本次只把这条如实留在文档里，没有接工具链。

## 验证

- `THIRD_PARTY_LICENSES.md` 的相对链接逐个存在性检查通过（7 个全 OK）；两张表列数一致。
- `LICENSE` 的 MIT 正文与 `licenses/MIT.txt` 做 diff，除本项目版权行外一字不差（即未改动许可
  条款本身，只在末尾加了一段范围说明）。
- `crates/app/tauri.conf.json` 用 `json` 解析通过，`bundle.licenseFile` 与三条 `resources`
  的源路径逐个 `os.path.exists` 存在；`release.yml` 用 `yaml.safe_load` 解析通过，确认改动落在
  `windows-x64` job 的 `Package portable zip` 步骤里。
- 依赖扫描用 `cargo metadata --format-version 1 --locked`（本机 cargo 不在 PATH，经
  `PATH="$HOME/.cargo/bin:$PATH"` 调起），许可表达式按原样统计，未做 SPDX 归一化。

**没验证的**：NSIS 许可页与 `bundle.resources` 的实际落盘位置都没有真跑构建（本机是 macOS，
没跑 `tauri build --bundles nsis`，也没跑 app/dmg 打包）。配置层校验通过不等于产物正确，
`licenseFile` 的相对路径按"相对 tauri.conf.json"理解，第一个 `v*` tag 才知道真章。

