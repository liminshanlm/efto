'use strict';
// 输出约定：
// - 人类：efto.output 行（GUI 搜索框 / CLI 都会显示）
// - 机器：一行 `EFTO_API {json}` 走 console.log，agents 从 CLI 的 lines 里取

/** 取 result 的机器可读标记行。 */
function marker(result) {
  return 'EFTO_API ' + JSON.stringify(result);
}

function emit(res) {
  var result = (res && res.result) || { ok: false, error: '没有结果' };
  var rows = (res && res.rows) || [];
  try {
    console.log(marker(result));
  } catch (e) {
    // 序列化失败也要有输出
    console.log('EFTO_API ' + JSON.stringify({ ok: false, error: '结果序列化失败: ' + String(e) }));
  }
  if (rows.length) {
    try {
      efto.output(rows.slice(0, 50));
    } catch (e) {
      // 行不合法时退化成一行，避免整个命令没有任何反馈
      efto.output([{ title: '操作完成，但结果行渲染失败', subtitle: String(e), copy: marker(result) }]);
    }
  }
}

function ok(cmd, data, rows) {
  return {
    result: { ok: true, cmd: cmd, data: data === undefined ? null : data },
    rows: rows || []
  };
}

function fail(cmd, error, rows) {
  var msg = String(error == null ? '未知错误' : error);
  return {
    result: { ok: false, cmd: cmd, error: msg },
    rows: rows || [{ title: '操作失败', subtitle: msg, copy: msg }]
  };
}

exports.marker = marker;
exports.emit = emit;
exports.ok = ok;
exports.fail = fail;

