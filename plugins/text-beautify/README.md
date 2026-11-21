# text-beautify — 文本美化插件

把一段挤成一团的 JSON / XML / SQL / JS 原文整理成带缩进的多行格式：左侧贴原文，右侧看结果。

## 触发

在 efto 里输入下列任一关键字回车：

| 关键字 | 说明 |
| --- | --- |
| `文本美化` | 主入口 |
| `美化` | 同上 |
| `格式化` | 同上 |
| `beautify` | 同上 |

关键字后面可以直接跟文本，进窗口后自动填入左侧并美化一次，例如：

```
美化 {"a":1,"b":[1,2,3]}
```

## 界面

- 顶部：类型切换（JSON / XML / SQL / JS）、缩进（2 空格 / 4 空格 / 制表符）、美化、复制结果、粘贴原文、清空。
- 类型切换后，选项区会换成该类型的专属开关：JSON 的「标量短数组保持单行」、XML 的「属性超长换行阈值」、SQL 的「关键字大写 / 列清单按逗号换行」、JS 的「简单字符串统一成单引号」。
- 左侧输入后 420ms 自动美化，也可以点「美化」或按 `Ctrl+Enter`。
- 解析失败时红色横幅给出**带行列号**的原因，结果框留空，不输出半成品。
- 结果只读、可选中；「复制结果」走宿主剪贴板，失败会退回浏览器复制并说明原因。

## 设计约束

- **纯本地**：四个格式化器都是零依赖的纯函数（`lib/fmt-*.js`），不联网、不调模型。
- **不改数据**：JSON 不走 `JSON.parse`/`stringify`，`1.0` 不会变 `1`、大整数不丢精度、字符串转义与键顺序原样保留；SQL / XML / JS 的字符串、注释、标识符一字不改。
- **双端可用**：`lib/fmt-*.js` 同时导出给浏览器（`window.TBFormat.<type>`）和 Node（`module.exports`），所以能在命令行里直接跑回归测试。
- **错就明说**：所有格式化器保证返回对象、绝不抛异常；无法解析时 `{ok:false, error}` 带位置。

## 目录

```
efto-plugin.json   清单
main.js            后端：窗口与剪贴板中转（内嵌 V8，无 Node/DOM）
index.html         UI 页面
ui.js              UI 逻辑：类型切换、选项、防抖美化、剪贴板
style.css          样式（窄窗口自动上下分栏）
logo.png           图标
lib/fmt-json.js    JSON 美化器
lib/fmt-xml.js     XML 美化器
lib/fmt-sql.js     SQL 美化器
lib/fmt-js.js      JS/TS 美化器
tests/             各格式化器与 UI 的回归测试（node tests/run-all.js）
```

## 测试

```powershell
node tests/run-all.js
```

单个：

```powershell
node tests/json-test.js
node tests/json-fuzz-test.js   # 随机生成 400 份 JSON，验证"输出可解析 + 值等价 + 幂等"
node tests/xml-test.js
node tests/sql-test.js
node tests/js-test.js
node tests/main-test.js        # 后端：开窗、种子文本推送、剪贴板失败透传
node tests/ui-test.js
```

`ui-test.js` 用一个极小的假 DOM 跑通 UI 的初始化与四条美化路径，能抓住「元素 id 写错 / 初始化抛异常 / 点美化没反应」，但不能替代在 efto 窗口里的复测。

回归覆盖的核心不变量：**美化不改数据**——JSON 用属性测试随机生成 400 份结构，断言输出能被 `JSON.parse`、解析值与原文深度相等、二次美化结果不变；SQL / XML / JS 各自在测试里用独立实现的 tokenizer 逐 token 比对输入与输出，保证字符串、注释、标识符一字不改。

## 安装

插件已安装在 `C:\Users\shanlimin\.efto\plugins\text-beautify`（该目录是指向 `E:\workspace\efto\plugins` 的联接，所以插件源码同时出现在 `E:\workspace\efto\plugins\text-beautify`）。改完代码在 efto 里按 `Alt+R` 重建索引即可生效。

开发期间想只加载这一个插件，可以临时指定目录：

```powershell
$env:EFTO_PLUGIN_DIRS = 'E:\dsh\text-beautify'
```

## 已知限制

- 不做语义校验：JSON 只查结构，XML 不查 DTD，SQL 不查表名，JS 不做类型检查。
- XML 的混合内容（`<b>x</b> y`）只保证每段文本单独成行并 trim 首尾，不还原行内空白邻接关系。
- JS 的「单引号统一」只作用于无转义歧义的普通字符串，模板字符串与含引号的字符串保持原样。
- 每个格式化器都有嵌套/规模上限（如 JSON 200 层），超限明确报错而不是卡死窗口。
