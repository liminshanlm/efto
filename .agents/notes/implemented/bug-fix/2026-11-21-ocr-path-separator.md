# ocr 插件保存路径硬编码 `\`，mac 拼出畸形文件名

- **Date**: 2026-11-21
- **Status**: done
- **相关**: `plugins/ocr/main.js`（save-result 动作）

## 现象

mac 上 OCR 结果保存后，文件名是 `Downloads\OCR_1795…​.txt` 这种**带反斜杠的单个文件名**
（落在 Downloads 里），而不是 `Downloads/OCR_….txt`——"在文件夹中显示"能揭示出来，
但名字是坏的。

## 根因

`outPath = dir + '\\OCR_' + …` 硬编码 Windows 路径分隔符。`\` 在 macOS 不是分隔符，
成了文件名的一部分。顺带把"填写 node.exe 路径"的提示改成 Node.js（平台中立措辞）。

## 做法

`String(dir).replace(/[\\\/]+$/, '') + '/OCR_' + …`：先归一尾部分隔符再用 `/` 拼接。
Node fs 在 Windows 同样接受 `/`，两端行为一致。

## 判据

- `node --check` 过；`cli plugin-run` 各插件冒烟全绿；
- text-beautify 自测 19 项/7 文件在 mac 全过（该套件不依赖 efto 宿主）。

## 教训

插件代码里的路径拼接一律用 `/`（或从 API 拿分隔符），`\\` 字面量只允许出现在
真正处理 Windows 形状的地方（如 file-share 的 `\\?\` 前缀剥离——那处无害且必要）。
