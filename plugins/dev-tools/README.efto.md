# dev-tools —— efto 插件（Ctool 移植）

本插件把 [Ctool 程序开发常用工具](https://github.com/baiy/Ctool)（Chrome / Edge 上的开发者工具箱，MIT，作者 baiy）移植到 efto 启动器。

## 来源

- 上游：Ctool 2.3.0（2023-09）
- 许可：MIT（见 `LICENSE`，版权归 baiy）。本插件的适配层（`main.js`、`ui/bridge.js`、`lib/routes.js`）为新增代码。
- 第三方资产与内嵌库的许可说明见 `THIRD_PARTY_NOTICES.md`。efto 不提供第三方启动器兼容层，本插件也不宣称兼容。

## 目录结构

```
dev-tools/
├── efto-plugin.json      # 清单：109 条触发命令
├── main.js               # 适配层：触发词 → Ctool 路由
├── lib/routes.js         # 生成的路由表（工具/功能 → Ctool 路由 + 触发别名）
├── index.html            # 插件 UI 入口（先加载适配层，再加载 Ctool 主包）
├── ui/bridge.js          # UI 侧适配：剪贴板、路由注入、窗口复用处理
├── assets/               # Ctool 2.3.0 前端产物（235 个模块）
├── icon/                 # 上游图标
├── logo.png              # 插件图标
├── LICENSE / README.md   # 上游许可与说明
```

## 相对上游的改动

1. **入口重写**：上游 `tool.html` 直接用固定尺寸的 `#ctool` 容器；这里改成 `index.html` + 自适应高度，让页面跟着宿主窗口大小走。
2. **容器 API 改走 efto**：
   - 设置与历史（原 `dbStorage` / `db`）→ 浏览器 localStorage；
   - 插件进入回调（原 `onPluginEnter`）→ main 通过 `efto.ipcMessage('route', …)` 推给页面；
   - `navigator.clipboard` / `permissions` → 换成宿主 `efto.copyText` / `efto.pasteText`。
3. **触发词 → 工具**：上游靠动态注册 feature 实现，efto 没有这个机制，改成清单里预置 109 条 `cmds`，由 `main.js` 查表翻译成路由。
4. **窗口复用**：efto 的插件 UI 窗口按插件复用，第二次触发不会重建页面。适配层检测到路由变化后整页 `reload`，让 Vue 组件重新 setup 以吃到新的路由和预填输入。

## 触发方式

输入触发词回车即可，例如：

| 触发词 | 打开 |
| --- | --- |
| `Ctool` | 首页（自动跳到上次使用的工具） |
| `md5` / `hash` | 哈希 |
| `base64` / `base64-encoder` / `base64-decoder` | Base64 编解码 |
| `aes` / `aes-decrypt` | AES 加解密 |
| `时间戳` / `time-timestamp` | 时间戳转换 |
| `unit` / `单位换算` | 单位换算 |
| `qrcode` / `二维码` | 二维码生成 |
| `crontab` | Crontab 校验 |

触发词后面可以跟内容，会被预填到页面的输入框：

```
base64-encoder hello world
md5 abc
qrcode https://github.com/baiy/Ctool
crontab */5 * * * *
```

完整 109 条触发词见 `efto-plugin.json` 的 `cmds` 字段。

## 已知边界

- **图片复制不支持**：宿主只提供文本剪贴板，Ctool 里「复制二维码图片」这类操作会明确报错，只能改用下载。
- **下载走浏览器默认行为**：二维码/条形码的下载用 `<a download>` 触发，WebView2 是否落盘取决于宿主对下载的处理，未做专门适配。
- **联网工具照旧不可用**：`IP地址查询`、`Websocket 调试` 依赖外部服务/长连接，与原版一样需要网络。
- **剪贴板「自动读取」**：需要宿主剪贴板可读；`efto.pasteText()` 在 GUI 下可用，CLI 下不可用。
- **首屏依赖 assets 完整**：`assets/tool-4bde39df.js` 是 Ctool 主包（2.2 MB），缺失会显示明确的错误提示而不是白屏。

## 验证记录

- `efto.exe cli plugins`：清单解析通过，109 条命令全部识别。
- `efto.exe cli plugin-run dev-tools <触发词> <内容>`：路由解析正确（含多功能工具的 `-feature` 后缀触发词）。
- 69 条工具/功能路由在无头浏览器里逐条挂载验证通过。
