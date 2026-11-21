# P2：自定义网页搜索、系统命令集、单项热键

- **Date**: 2026-11-20
- **Status**: done
- **相关**: `docs/todo.md` 第 3 项 #5/#7/#8、`docs/architecture.md`、`docs/help.zh-CN.md`、`docs/help.en-US.md`、`docs/verification.md`、`docs/roadmap.md`

## 为什么做

Alfred 对标清单里还有 3 个能独立闭环的 P2 能力：`g 关键词` 式网页搜索与无结果兜底、系统命令集、给内置工具绑单项热键。它们都不依赖新的外部服务，做完就能用，因此先落地，主题系统与内建文件索引继续留在 P2。

## 关键决定

### 网页搜索：前缀必须短路，兜底必须垫底

- 模型是 `WebSearch { keyword, name, url }`，`url` 必须含 `{q}`，默认 `b`/`g`/`gh`/`wiki`。
- 触发条件：第一个空白分段与某个关键词大小写不敏感相等，**且后面有正文**。单独输入 `g` / `g ` 不触发，避免把正常搜索吞掉。
- 命中后 `service::query` 直接返回一条 780 分结果，不再跑应用/计算器/时间戳/URL/路径/Everything；`search_files` 同样早退。
- 本地完全无结果且 `web_search_fallback` 开启时，追加一条 300 分兜底结果。兜底永远排在真实结果之后；引擎找不到时回落到列表第一项。
- 不新增独立命中类型之外的搜索源排序逻辑：`HitPayload::WebSearch { url }` 只带最终 URL，由宿主 `shell_open`。

### 系统命令：整串精确匹配 + 原生确认

- 关键词：`sleep`/`睡眠`、`restart`/`重启`、`logout`/`logoff`/`注销`、`emptytrash`/`清空回收站`、`screensaver`/`屏保`。
- 分数 690，低于应用精确匹配，是"填空位"来源；`sleep now`、`重启电脑` 等带附加文字的输入不触发。
- 破坏性动作（重启/注销/清空回收站）先 `MessageBoxW` 确认；重启前启用 `SeShutdownPrivilege`。
- 执行集中在 `app/src/system.rs`；非 Windows 明确报错，不静默成功。

### 单项热键：配置层收敛，注册失败只记日志

- 配置为 `tool_hotkeys: BTreeMap<tool_id, accelerator>`，只允许 `KNOWN_TOOLS` 里的 7 个内置工具。
- `normalize_tool_hotkeys` 丢弃空键、未知工具、以及与唤起热键/通用动作热键重复的组合；注册失败（解析失败或被系统占用）只写日志，不影响其它热键。
- `hotkey::rebind` 用 `CURRENT_ITEMS` 做幂等判断；成功后把工具热键登记进去，解绑时一并清空。
- 回调在独立线程跑 `run_action`，避免阻塞主线程。

## 验证

- `cargo test --workspace`：core 213 / app 100 / jsrt 48 全过。
- `scripts/verify.ps1`：ALL CHECKS PASSED。
- `node scripts/check-i18n.js`（380 keys × 2 语言）与 `node scripts/check-frontend.js` 通过。
- 静态守卫抓到设置页 3 个不存在的 `settings.tool.*` key（`lockScreen`/`tileWindows`/`shortcutGuide`），改为复用既有 key 后通过。

## 明确没做 / 没验

- 没在真机执行重启、注销、清空回收站、睡眠、屏保（避免中断会话）；只验证了确认框与错误路径。
- 没逐个组合在真实系统上按单项热键；被其他软件占用时只记日志。
- 自定义搜索引擎实际跳转页面未逐条核对；英文两页未做 520px 截图检查。
- 不给任意搜索结果绑热键（仅内置工具），也不做工作流级热键。
