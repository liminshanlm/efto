// timestamp-ts/main.ts
// 无 UI 的双向转换（原生插件：宿主用 SWC 原地剥类型，不需要 tsc / npm / 打包）：
//   ts 2026-11-17 14:30      -> 秒级 / 毫秒级 Unix 时间戳
//   ts 1700000000            -> 本地时间 / ISO 8601（按秒解释）
//   ts 1700000000000         -> 本地时间 / ISO 8601（按毫秒解释）
//
// 判定顺序（宁可不出，也不乱猜）：
//   1. 纯数字能构成紧凑日期 YYYYMMDD / YYYYMMDDHHmm / YYYYMMDDHHmmss -> 当时间
//   2. 其余纯数字 -> 当时间戳：小数点前 <= 11 位按秒，否则按毫秒
//   3. 其他文本 -> 当时间解析（相对词、分隔符日期、中文单位、时段词、时区后缀）
//
// 模块格式：入口与被 require 的模块都在 CommonJS 作用域里执行，所以用 require/exports。
var format = require('./lib/format');

const MIN_YEAR = 1000;
const MAX_YEAR = 9999;
const MAX_STAMP_DIGITS = 16;
const MAX_EPOCH_MS = 8640000000000000;

const PM_WORDS = ['下午', '晚上', '夜里', '深夜', '傍晚', '中午', '正午', 'pm'];
const AM_WORDS = ['上午', '早上', '凌晨', 'am'];
// 顺序有意义：长词在前（大后天 先于 后天）
const DAY_WORDS: Array<[string, number]> = [
  ['大后天', 3],
  ['后天', 2],
  ['明天', 1],
  ['明日', 1],
  ['今天', 0],
  ['今日', 0],
  ['昨天', -1],
  ['昨日', -1],
  ['前天', -2]
];

interface Row {
  title: string;
  subtitle: string;
  copy: string;
}

interface Stamp {
  ms: number;
  /// 'time' 表示输入其实是紧凑日期，按时间处理
  unit: string;
}

// ---------------------------------------------------------------------------
// 文本处理
// ---------------------------------------------------------------------------

function normalize(input: string): string {
  let out = '';
  for (const ch of input) {
    const code = ch.charCodeAt(0);
    if (code >= 0xff01 && code <= 0xff5e) {
      out += String.fromCharCode(code - 0xfee0);
    } else if (ch === '\u3000' || ch === '\u00a0') {
      out += ' ';
    } else {
      out += ch;
    }
  }
  return out.replace(/\s+/g, ' ').trim().toLowerCase();
}

function stripWords(text: string, words: string[]): { text: string; hit: boolean } {
  let out = text;
  let hit = false;
  for (const w of words) {
    if (out.indexOf(w) >= 0) {
      hit = true;
      out = out.split(w).join(' ');
    }
  }
  return { text: out, hit: hit };
}

/// 中文日期时间单位 -> 通用 ASCII 形状（与内置时间解析同一套映射）
/// `2026年11月17日15时03分22秒` -> `2026-11-17 15:03:22`
/// `10点半` -> `10:30`
function translateUnits(s: string): string {
  let out = '';
  for (const ch of s) {
    if (ch === '年' || ch === '月') {
      out += '-';
    } else if (ch === '日' || ch === '号' || ch === '秒') {
      out += ' ';
    } else if (ch === '时' || ch === '点' || ch === '分') {
      out += ':';
    } else if (ch === '半') {
      out += out.endsWith(':') ? '30' : '\u00d7';
    } else {
      out += ch;
    }
  }
  out = out.replace(/\s+/g, ' ').trim();
  // `14时` 翻成 `14:`，补成 `14:00`；否则会被当成一个光秃秃的数字
  if (out.endsWith(':')) {
    out += '00';
  }
  return out;
}

/// 拆掉末尾时区。只有前面确实有时间分量时才认 `+08:00`/`Z`，
/// 否则 `2026-11-17` 末尾的 `-17` 会被误当成偏移。
function splitZone(s: string): { text: string; offsetMin: number | undefined } {
  let m = /^(.*\d)\s*(?:z|utc|gmt)$/.exec(s);
  if (m) {
    return { text: m[1], offsetMin: 0 };
  }
  m = /^(.*\d{1,2}:\d{1,2}(?::\d{1,2}(?:\.\d+)?)?)\s*([+-])(\d{2}):?(\d{2})?$/.exec(s);
  if (m) {
    const hh = Number(m[3]);
    const mm = m[4] ? Number(m[4]) : 0;
    if (hh > 23 || mm > 59) {
      return { text: s, offsetMin: undefined };
    }
    const sign = m[2] === '-' ? -1 : 1;
    return { text: m[1], offsetMin: sign * (hh * 60 + mm) };
  }
  return { text: s, offsetMin: undefined };
}

// ---------------------------------------------------------------------------
// 时间 -> 毫秒
// ---------------------------------------------------------------------------

/// 校验并算出 epoch；非法日期（含 DST 空档）统一返回 null。
function buildMoment(
  y: number,
  mo: number,
  d: number,
  h: number,
  mi: number,
  s: number,
  ms: number,
  offsetMin: number | undefined
): number | null {
  if (y < MIN_YEAR || y > MAX_YEAR || mo < 1 || mo > 12 || d < 1 || d > 31) {
    return null;
  }
  if (h > 23 || mi > 59 || s > 59 || ms > 999) {
    return null;
  }
  if (offsetMin !== undefined) {
    const epoch = Date.UTC(y, mo - 1, d, h, mi, s, ms) - offsetMin * 60000;
    const back = new Date(epoch + offsetMin * 60000);
    const ok =
      back.getUTCFullYear() === y &&
      back.getUTCMonth() === mo - 1 &&
      back.getUTCDate() === d &&
      back.getUTCHours() === h &&
      back.getUTCMinutes() === mi &&
      back.getUTCSeconds() === s &&
      back.getUTCMilliseconds() === ms;
    return ok ? epoch : null;
  }
  const dt = new Date(y, mo - 1, d, h, mi, s, ms);
  const ok =
    dt.getFullYear() === y &&
    dt.getMonth() === mo - 1 &&
    dt.getDate() === d &&
    dt.getHours() === h &&
    dt.getMinutes() === mi &&
    dt.getSeconds() === s &&
    dt.getMilliseconds() === ms;
  return ok ? dt.getTime() : null;
}

function parseDate(part: string, today: Date): { y: number; m: number; d: number } | null {
  let m = /^(\d{4})[-/.](\d{1,2})(?:[-/.](\d{1,2}))?$/.exec(part);
  if (m) {
    return makeDate(Number(m[1]), Number(m[2]), m[3] ? Number(m[3]) : 1);
  }
  // 16/11/2026 -> 日/月/年；其余（11/16/2026、11/12/2026）按 月/日/年
  m = /^(\d{1,2})[-/.](\d{1,2})[-/.](\d{4})$/.exec(part);
  if (m) {
    const a = Number(m[1]);
    const b = Number(m[2]);
    const y = Number(m[3]);
    if (a > 12 && b <= 12) {
      return makeDate(y, b, a);
    }
    return makeDate(y, a, b);
  }
  // 11-17 / 11月17日：缺年份就用今年
  m = /^(\d{1,2})[-/.](\d{1,2})$/.exec(part);
  if (m) {
    return makeDate(today.getFullYear(), Number(m[1]), Number(m[2]));
  }
  return null;
}

function makeDate(y: number, m: number, d: number): { y: number; m: number; d: number } | null {
  if (y < MIN_YEAR || y > MAX_YEAR || m < 1 || m > 12 || d < 1 || d > 31) {
    return null;
  }
  return { y: y, m: m, d: d };
}

function parseTimeOfDay(part: string): { h: number; mi: number; s: number; ms: number } | null {
  // 小数秒不限位数，只取前三位（纳秒/微秒精度的 ISO 串也能用）
  const m = /^(\d{1,2}):(\d{1,2})(?::(\d{1,2})(?:\.(\d+))?)?$/.exec(part);
  if (!m) {
    return null;
  }
  const h = Number(m[1]);
  const mi = Number(m[2]);
  const s = m[3] ? Number(m[3]) : 0;
  const ms = Number(((m[4] || '') + '00').slice(0, 3));
  if (h > 23 || mi > 59 || s > 59) {
    return null;
  }
  return { h: h, mi: mi, s: s, ms: ms };
}

function parseTime(raw: string, now: Date): number | null {
  if (raw === 'now' || raw === '现在' || raw === '当前' || raw === '此刻') {
    return now.getTime();
  }

  let text = raw;
  let shift: number | undefined;
  for (const pair of DAY_WORDS) {
    if (text.indexOf(pair[0]) >= 0) {
      shift = pair[1];
      text = text.split(pair[0]).join(' ').trim();
      break;
    }
  }

  const pm = stripWords(text, PM_WORDS);
  const am = stripWords(pm.text, AM_WORDS);
  const meridiem = pm.hit ? 2 : am.hit ? 1 : 0;
  const hadTimeUnit = /[时点分秒]/.test(raw) || raw.indexOf(':') >= 0;

  const zone = splitZone(translateUnits(am.text));
  const body = zone.text;

  if (!body) {
    // 只有「今天 / 明天」：当天 00:00
    if (shift === undefined) {
      return null;
    }
    const moved = new Date(now.getFullYear(), now.getMonth(), now.getDate() + shift);
    return buildMoment(
      moved.getFullYear(),
      moved.getMonth() + 1,
      moved.getDate(),
      0,
      0,
      0,
      0,
      zone.offsetMin
    );
  }

  const bare = /^(\d{1,2})$/.exec(body);
  let datePart = '';
  let timePart = '';
  let bareHour: number | undefined;
  const tIndex = body.indexOf('t');
  if (tIndex > 0) {
    datePart = body.slice(0, tIndex);
    timePart = body.slice(tIndex + 1);
  } else if (body.indexOf(' ') > 0) {
    const sp = body.indexOf(' ');
    datePart = body.slice(0, sp);
    timePart = body.slice(sp + 1);
  } else if (body.indexOf(':') >= 0) {
    timePart = body;
  } else if (bare && (hadTimeUnit || meridiem !== 0)) {
    // `下午3` / `3点`：没有分钟，按时段词或时间单位授权才认
    bareHour = Number(bare[1]);
  } else {
    datePart = body;
  }

  let y = now.getFullYear();
  let mo = now.getMonth() + 1;
  let d = now.getDate();
  if (shift !== undefined) {
    const moved = new Date(now.getFullYear(), now.getMonth(), now.getDate() + shift);
    y = moved.getFullYear();
    mo = moved.getMonth() + 1;
    d = moved.getDate();
  }
  if (datePart) {
    const date = parseDate(datePart, now);
    if (!date) {
      return null;
    }
    y = date.y;
    mo = date.m;
    d = date.d;
  }

  let h = 0;
  let mi = 0;
  let s = 0;
  let ms = 0;
  if (bareHour !== undefined) {
    h = bareHour;
  } else if (timePart) {
    const time = parseTimeOfDay(timePart);
    if (!time) {
      return null;
    }
    h = time.h;
    mi = time.mi;
    s = time.s;
    ms = time.ms;
  }

  if (meridiem === 2 && h < 12) {
    h += 12;
  } else if (meridiem === 1 && h === 12) {
    h = 0;
  } else if (meridiem !== 0 && h > 12) {
    // `下午13点` 这类自相矛盾的写法不猜
    return null;
  }

  return buildMoment(y, mo, d, h, mi, s, ms, zone.offsetMin);
}

// ---------------------------------------------------------------------------
// 时间戳 -> 毫秒
// ---------------------------------------------------------------------------

function parseStamp(raw: string): Stamp | null {
  const m = /^([+-]?)(\d+)(?:\.(\d{1,3}))?$/.exec(raw);
  if (!m) {
    return null;
  }
  const sign = m[1] === '-' ? -1 : 1;
  const digits = m[2];
  const frac = m[3] || '';
  if (digits.length > MAX_STAMP_DIGITS) {
    return null;
  }

  // 紧凑日期优先：8/12/14 位且各段合法时，用户显然是在输入时间
  if (frac === '' && (digits.length === 8 || digits.length === 12 || digits.length === 14)) {
    const c = /^(\d{4})(\d{2})(\d{2})(?:(\d{2})(\d{2})(?:(\d{2}))?)?$/.exec(digits);
    if (c) {
      const compact = buildMoment(
        Number(c[1]),
        Number(c[2]),
        Number(c[3]),
        c[4] ? Number(c[4]) : 0,
        c[5] ? Number(c[5]) : 0,
        c[6] ? Number(c[6]) : 0,
        0,
        undefined
      );
      if (compact !== null) {
        return { ms: compact, unit: 'time' };
      }
    }
  }

  const seconds = digits.length <= 11;
  const ms = seconds
    ? Math.round(sign * Number(digits + (frac ? '.' + frac : '')) * 1000)
    : sign * Number(digits);
  if (!isFinite(ms) || ms < -MAX_EPOCH_MS || ms > MAX_EPOCH_MS) {
    return null;
  }
  return { ms: ms, unit: seconds ? 's' : 'ms' };
}

// ---------------------------------------------------------------------------
// 输出
// ---------------------------------------------------------------------------

function usage(bad?: string): string {
  const head = bad ? `无法识别「${bad}」` : '没有输入内容';
  return `${head}。示例：ts 2026-11-17 14:30 · ts 1700000000 · ts 1700000000000`;
}

/// 输入是时间：给出秒级 / 毫秒级时间戳
function timeRows(ms: number): Row[] {
  const zone = format.zoneLabel(format.zoneOffset(ms));
  const local = format.local(ms, true);
  const secs = String(Math.floor(ms / 1000));
  const millis = String(ms);
  return [
    { title: secs, subtitle: `秒级时间戳 · ${local} · ${zone}`, copy: secs },
    { title: millis, subtitle: `毫秒级时间戳 · ${local} · ${zone}`, copy: millis }
  ];
}

/// 输入是时间戳：给出本地时间与 ISO 8601
function stampRows(ms: number, unit: string): Row[] {
  const offset = format.zoneOffset(ms);
  const zone = format.zoneLabel(offset);
  const local = format.local(ms, true);
  const iso = format.iso(ms, offset);
  return [
    {
      title: local,
      subtitle: `${unit === 'ms' ? '毫秒' : '秒'}时间戳 · ${zone} · 本机时区`,
      copy: local
    },
    { title: iso, subtitle: `ISO 8601 · ${zone}`, copy: iso }
  ];
}

function convert(input: string): Row[] | string {
  const raw = normalize(input);
  if (!raw) {
    return usage();
  }
  const stamp = parseStamp(raw);
  if (stamp) {
    return stamp.unit === 'time' ? timeRows(stamp.ms) : stampRows(stamp.ms, stamp.unit);
  }
  const ms = parseTime(raw, new Date());
  if (ms === null) {
    return usage(raw);
  }
  return timeRows(ms);
}

exports.onPluginEnter = (info: { code?: string; payload?: unknown }) => {
  const payload = info && info.payload !== undefined && info.payload !== null ? info.payload : '';
  const result = convert(String(payload));
  if (typeof result === 'string') {
    // 解析失败：只发通知、不产出结果 → 宿主收起搜索框
    efto.showNotification(result);
    return;
  }
  // 有输出：宿主把这几行内联到搜索框下方，不收起搜索框
  efto.output(result);
};
