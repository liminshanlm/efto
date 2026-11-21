# 二阶段路线图

按"投入产出比 + 风险"排。P0 是让它真正日常可用，P1 是插件生态，P2 是差异化。

## P0 · 可用性地基

| 项 | 现状 | 目标 |
| --- | --- | --- |
| 图标磁盘缓存与协议 | 已按需提取系统图标（`SHGetFileInfoW` → PNG data URL，进程内缓存 512 条） | 落盘缓存并暴露 `efto-icon://`，供插件页面复用同一套图标 |
| 增量索引 | 定时全量扫描（首次约几秒） | 注册表 `NotifyKey` + 开始菜单目录 `ReadDirectoryChangesW`，事件驱动 + 5 分钟兜底全量 |
| 索引启动速度 | 冷启动全扫 | 缓存带 mtime 指纹，命中即秒开；后台补齐差异 |
| 卸载残留清理 | 死链靠"目标是否存在"过滤 | 增加启动时校验 + 定期剔除 |
| UWP / `App Paths` | 未索引 | 补 `shell:AppsFolder` 枚举与 `HKLM\...\App Paths`，让 `code`/`wt` 这类命令名可直接命中 |
| 窗口位置记忆 | 每次居中 | 记录上次显示位置/多显示器选择 |
| 全局快捷键可配 UI | 只能在设置页改（立即生效） | 已提供设置窗口；后续补录制式快捷键输入 |
| 崩溃隔离 | 插件死循环会占住自己线程 | `IsolateHandle::terminate_execution` + 3s 无响应看门狗、能力调用超时、`efto cli plugin-run --timeout` |

## P1 · 插件生态对齐

| 项 | 说明 |
| --- | --- |
| **ESM 入口与模块** | 现在入口与被 require 的模块都是 CommonJS 作用域（ESM 语法会被明确拒绝并给出改法）。要支持 `import/export`，用 rusty_v8 的 `Module` API 走真实模块实例化 + resolve/load 回调；被 require 的 ESM 模块也走同一条路 |
| 更完整的 API | `over`（拖入文件/选中文本注入，`over` 类型 cmd）、`createShortCut`、`setProgressBar`、`db` 的真实 `_changes` 序列与冲突合并、`oneId` |
| 插件导入来源 | 已支持：文件夹 / `.zip` 复制到 plugins 目录并校验清单（含路径逃逸拒绝、一个包多个插件）；仍缺来源与哈希记录 |
| 权限声明 | manifest 增加 `permissions: ["db","network","shell"]`，宿主按声明放行；未声明调用返回明确错误（现在是全放行） |
| 插件市场接口 | 本地目录 + 可配置 registry（默认关闭），支持更新检查 |
| node 兼容层（可选） | 评估 `deno_core` 或 Node sidecar，给需要 npm 生态的重度插件一条出路；决定点是"是否愿意让包体积和启动时间翻倍" |
| TS 完整转译 | 把 `enum`/装饰器/JSX 也支持：接 `swc` 的完整 pass 链（`typescript::strip` + `proposal` + `react`），代价是编译期依赖更重 |
| sourcemap | 目前报错定位在转译后代码行；生成 inline sourcemap 并让宿主在报错时映射回 `.ts` 行号 |

## P2 · 差异化能力

- **本地语义检索**：给应用/插件/文档建向量索引（`candle` 或 ONNX Runtime），支持"上周编辑过的表格"这类自然语言查询。
- **everything 式文件搜索**：NTFS USN Journal 直读，毫秒级全盘文件名搜索（不依赖外部 CLI 的内建能力）。
- **截图增强**：OCR（Windows `OCR` API）。已做选区截图 + Snipaste 风格标注（矩形/椭圆/箭头/画笔/荧光笔/马赛克/文字/橡皮、色板、粗细、撤销重做，已交付 2026-11-20）+ 保存/复制/钉在屏幕 + 滚动长截图（用户手动滚动、相邻帧拼接）。
- **剪贴板增强（已交付 2026-11-20）**：图片与文件列表、条目置顶、粘贴到前台窗口；历史文本记录保留。
- **自定义网页搜索与系统命令（已交付 2026-11-20）**：`<关键词> 正文` 前缀搜索、可配置引擎与无结果兜底；睡眠/重启/注销/清空回收站/屏保，破坏性动作带原生确认。
- **单项工具热键（已交付 2026-11-20）**：7 个内置工具可各自绑定全局热键，冲突组合在保存时丢弃。
- **多用户/便携模式**：`--portable` 把数据目录放 exe 旁边，方便 U 盘。
- **发布工程**：MSI/NSIS 安装包（`tauri-bundler`）、代码签名、自动更新（`tauri-plugin-updater`）、崩溃上报。

## macOS 支持

分阶段计划见 `docs/macos-support-plan.md`：v1 只保「唤起 → 搜索 → 启动应用 / 跑插件」，
窗口平铺 / 通用动作 / 截图等系统集成推迟。**Phase 0/1 已落地（2026-11-21）**：
mac 工具链脚本（`scripts/*.sh`）、`verify.sh` 端到端全绿、CI 矩阵（`.github/workflows/ci.yml`）、
`.app` bundle 发现、zsh 命令源、平台化配置回落；实测见 `docs/verification.md` mac 段。
Phase 2（GUI 宿主）未动工。

## 已知技术债（不排期，顺手清）

1. `SearchService::search_apps` 是 O(n) 全扫，条目上万需要换成 prefix/trigram 索引。
2. `PluginRegistry::search` 每次调用都重建 `SearchText`（含拼音表查找），应在发现阶段缓存。
3. 能力调用 `module.method` 字符串匹配是线性 `match`，插件多了以后需要拆到独立模块。
4. CLI 宿主与 GUI 宿主有重复实现（这是有意的：验证通道），但 db 行为差异要在文档里写清（UI 侧 Promise / main 侧同步）。
5. `ureq` 2.x 与 `windows` 版本耦合，升级时需要重跑 `http.rs` 的真实网络测试。
6. 编译警告（~100 条，主要是未使用导入/字段）需要一次集中清理，避免淹没真实告警。
