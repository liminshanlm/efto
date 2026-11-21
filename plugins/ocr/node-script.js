// ocr/node-script.js —— 只负责把识别结果写到本地文件，由 main.js 交给 efto.node.run 执行。
// 识别本身走 efto.ai.chat（宿主读图），不再需要 Node。
//
// 为什么写成函数再 toString：既不丢语法高亮，也不用在字符串里转义引号。
// 这段代码跑在**独立 Node 进程**里（efto 用 node -e 执行），因此可以用 fs。
//
// 入参：process.argv[1] = JSON 字符串 { op: 'save', outPath, text }
// 出参：stdout 一行 JSON

function ocrMain() {
  'use strict';

  var fs = require('fs');
  var path = require('path');

  function done(obj) {
    process.stdout.write(JSON.stringify(obj));
    process.exit(0);
  }

  function fail(msg) {
    done({ ok: false, error: String(msg) });
  }

  var raw;
  try {
    raw = process.argv[1];
  } catch (e) { /* 下面统一报错 */ }
  if (!raw) fail('缺少参数');

  var job;
  try {
    job = JSON.parse(raw);
  } catch (e) {
    fail('参数不是合法 JSON: ' + e.message);
  }

  if (job.op !== 'save') fail('不支持的操作: ' + job.op);

  try {
    var dir = path.dirname(job.outPath);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(job.outPath, String(job.text || ''), 'utf8');
    done({ ok: true, path: job.outPath });
  } catch (e) {
    fail('写文件失败: ' + e.message);
  }
}

exports.SOURCE = '(' + ocrMain.toString() + ')();';
