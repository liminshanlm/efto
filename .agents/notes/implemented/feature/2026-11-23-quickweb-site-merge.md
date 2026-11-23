# 网页快开：站点合并、工具分组与新增站点

- **Date**: 2026-11-23
- **Status**: done
- **相关**: `plugins/quickweb/{lib/sites.js,main.js,efto-plugin.json,index.html,ui/app.js,ui/app.css,README.efto.md}`、`docs/verification.md`

## 背景

旧站点表把「打开主页」和「站内搜索」写成两条记录（`iSlide` / `搜索 PPT 模板`、
`知网` / `搜索论文` 等），管理界面出现同域名重复卡片；52 个站点里工具类与内容类混在
「网页」分组；同组多个站点共用 `code`，删一个会连带隐藏同组其它项。

## 决策

1. **一个站点一条记录**：`{ code, cmd, url, search?, group }`。带关键词且存在 `search`
   时拼站内搜索，否则打开主页；原来成对的「搜索 xx」触发词并入站点名，清单从 74 条降到 57 条。
2. **工具单独分组**：新增 `tool` 分组（12 个在线工具），管理界面顶部加分组 tab
   「全部 / 网页 / 工具 / AI / 聚合」；筛选框有内容时跨分组搜索并忽略当前 tab。
3. **新增站点**：知乎（web，支持站内搜索）、雪球（aggregate，支持站内搜索）；
   顺带修正 `TechWeb` 拼写，并把 A股/港股/美股、资讯、图库拆成唯一 code。
4. **兼容已删除项**：`hidden_sites` 比较前去掉旧版 `@group` 后缀，升级后已删的内置项不复活。

## 验证

- `node --check`：`main.js`、`ui/app.js`；清单 JSON 解析通过；`cli plugins` 显示 `quickweb v1.1.0`、57 条命令。
- 桩宿主脚本 14 项通过：主页跳转、关键词编码、无 `search` 忽略关键词、未知触发词提示、
  旧 `@group` 隐藏项生效、清单与站点表互相覆盖（`sites=52 cmds=57`）。
- Playwright（msedge，1024×720）+ `window.efto` 桩渲染：tab 栏计数正确、同域名只剩一张卡片、
  工具分组独立、关键词标记正常。

## 残留

- 真实 WebView2 / `efto://` 下的 tab 点击、卡片删除与悬浮态未手测。
- 管理界面选中某个 tab 的截图未采集（CLI 无法点击），只核对了默认「全部」视图。
