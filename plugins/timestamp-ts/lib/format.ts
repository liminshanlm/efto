// timestamp-ts/lib/format.ts
// 被 main.ts 用相对路径 require —— 同时验证宿主的 CommonJS + TS 转译链路。
// 入口与被 require 的模块都是 CJS 作用域，不写 import/export。

const WEEKDAYS = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

function pad(n: number, width: number): string {
  const s = String(n);
  return s.length >= width ? s : '0'.repeat(width - s.length) + s;
}

/// 毫秒时间戳 -> `YYYY-MM-DD HH:mm:ss[.SSS] 周X`（本机时区）
function local(ms: number, withMs: boolean): string {
  const d = new Date(ms);
  const date = `${d.getFullYear()}-${pad(d.getMonth() + 1, 2)}-${pad(d.getDate(), 2)}`;
  const time = `${pad(d.getHours(), 2)}:${pad(d.getMinutes(), 2)}:${pad(d.getSeconds(), 2)}`;
  const frac = withMs ? `.${pad(d.getMilliseconds(), 3)}` : '';
  return `${date} ${time}${frac} ${WEEKDAYS[d.getDay()]}`;
}

/// 该时刻本机时区相对 UTC 的偏移（分钟，东八区是 +480）
function zoneOffset(ms: number): number {
  return -new Date(ms).getTimezoneOffset();
}

function zoneLabel(offsetMin: number): string {
  const sign = offsetMin < 0 ? '-' : '+';
  const abs = Math.abs(offsetMin);
  return `UTC${sign}${pad(Math.floor(abs / 60), 2)}:${pad(abs % 60, 2)}`;
}

/// 固定偏移下的 ISO 8601，如 `2026-11-17T14:30:00.000+08:00`
function iso(ms: number, offsetMin: number): string {
  const d = new Date(ms + offsetMin * 60000);
  const body =
    `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1, 2)}-${pad(d.getUTCDate(), 2)}` +
    `T${pad(d.getUTCHours(), 2)}:${pad(d.getUTCMinutes(), 2)}:${pad(d.getUTCSeconds(), 2)}` +
    `.${pad(d.getUTCMilliseconds(), 3)}`;
  if (offsetMin === 0) {
    return `${body}Z`;
  }
  return body + zoneLabel(offsetMin).replace('UTC', '');
}

exports.local = local;
exports.zoneOffset = zoneOffset;
exports.zoneLabel = zoneLabel;
exports.iso = iso;
