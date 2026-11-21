// 时间戳功能的独立交叉校验。
//
// 为什么要有这个文件：单元测试里 "Rust 解析 -> Rust 期望值" 是自证，
// 时区/历法算错的时候两边会一起错。这里换成 Node 的 Date（走操作系统时区库）
// 独立算一遍 epoch，再和 release/debug 二进制 `cli search` 的真实输出对齐：
//   1) 秒级、毫秒级两个数值必须与 Node 算的一致；
//   2) Rust 展示的墙上时间、星期必须与 Node 从同一个 epoch 反解出来的一致；
//   3) 显式时区（Z / +08:00）用 Date.parse 直接比对；
//   4) 一批 "绝不能出时间行" 的输入必须为空（不劫持应用搜索）；
//   5) 计算器让位/不让位两条都要验：2026-11-16 出时间行且不出 1,999，
//      11-16 仍然归计算器（时间解析器不该抢减法）。
//
// 用法：node scripts/check-time-probe.js [path\to\efto.exe]
'use strict';

const { execFileSync } = require('child_process');
const path = require('path');

const root = path.resolve(__dirname, '..');
// Windows 产物叫 efto.exe，macOS/Linux 叫 efto（对齐 scripts/verify.sh 与 verify.ps1）
const exeName = process.platform === 'win32' ? 'efto.exe' : 'efto';
const exe =
  process.argv[2] ||
  process.env.EFTO_EXE ||
  (process.env.CARGO_TARGET_DIR
    ? path.join(process.env.CARGO_TARGET_DIR, 'debug', exeName)
    : path.join(root, 'target', 'debug', exeName));

const pad = (n) => String(n).padStart(2, '0');
const WEEKDAYS = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

function search(query) {
  let out = '';
  try {
    out = execFileSync(exe, ['cli', 'search', query, '--json'], {
      encoding: 'utf8',
      maxBuffer: 8 << 20,
      // RUST_LOG 的日志走 stdout，会把 JSON 挤掉；探查时静音
      env: Object.assign({}, process.env, { RUST_LOG: 'off', EFTO_PLUGIN_DIRS: path.join(root, 'plugins') }),
    });
  } catch (e) {
    // 无结果时 CLI 以退出码 1 结束，stdout 里仍然是合法 JSON
    out = (e.stdout || '').toString();
  }
  const start = out.indexOf('[');
  const end = out.lastIndexOf(']');
  if (start < 0 || end <= start) return [];
  try {
    return JSON.parse(out.slice(start, end + 1));
  } catch (e) {
    throw new Error(`无法解析 cli search 输出（${e.message}）:\n${out.slice(0, 400)}`);
  }
}

let failed = 0;
function fail(msg) {
  failed++;
  console.log('  FAIL ' + msg);
}
function ok(msg) {
  console.log('  ok   ' + msg);
}

// epoch 行与系统时间格式行共用 ItemKind::Time（都走"单击即复制"那条通路），
// 所以按 kind 筛会把两种行混在一起 —— 必须按 source 区分。
function timeRows(hits) {
  return hits.filter((h) => h.kind === 'time' && h.source === 'time');
}
function clockRows(hits) {
  return hits.filter((h) => h.source === 'clock');
}

// ---------- 1) 本机时区的各种写法 ----------
console.log('==> 本机时区：与 Node 独立计算的 epoch 对齐');
const now = new Date();
const localCases = [
  { input: '2026/11/16 15:03:22', ymd: [2026, 11, 16, 15, 3, 22, 0] },
  { input: '2026.11.16 15:03:22', ymd: [2026, 11, 16, 15, 3, 22, 0] },
  { input: '2026-11-16', ymd: [2026, 11, 16, 0, 0, 0, 0] },
  { input: '2026/11/16 15:03:22', ymd: [2026, 11, 16, 15, 3, 22, 0] },
  { input: '20261116 23:59:59', ymd: [2026, 11, 16, 23, 59, 59, 0] },
  { input: '2026-11-16T15:03:22', ymd: [2026, 11, 16, 15, 3, 22, 0] },
  { input: '2026年11月16日15时03分22秒', ymd: [2026, 11, 16, 15, 3, 22, 0] },
  { input: '2026年11月16日', ymd: [2026, 11, 16, 0, 0, 0, 0] },
  { input: '2026年11月16日下午3点', ymd: [2026, 11, 16, 15, 0, 0, 0] },
  { input: '2026-11-16 下午3点03分22秒', ymd: [2026, 11, 16, 15, 3, 22, 0] },
  { input: '2026-11-16 上午10点半', ymd: [2026, 11, 16, 10, 30, 0, 0] },
  { input: '2026-11-16 中午12点', ymd: [2026, 11, 16, 12, 0, 0, 0] },
  { input: '2026-11-16 凌晨12点半', ymd: [2026, 11, 16, 0, 30, 0, 0] },
  { input: '2026-11-16 15:03:20.250', ymd: [2026, 11, 16, 15, 3, 20, 250] },
  { input: '11月16日 08:30', ymd: [now.getFullYear(), 11, 16, 8, 30, 0, 0] },
  { input: '16/11 08:30', ymd: [now.getFullYear(), 11, 16, 8, 30, 0, 0] },
  { input: '11-16 15:00', ymd: [now.getFullYear(), 11, 16, 15, 0, 0, 0] },
  { input: '今天', ymd: [now.getFullYear(), now.getMonth() + 1, now.getDate(), 0, 0, 0, 0] },
  // 只有时间没有日期 -> 必须是"今天几点"。这一组是本机跑 release 二进制时抓到的
  // 真实回归（曾经落到今年 1 月 1 日），当时的用例表里恰好缺这一类。
  { input: '15:03', ymd: [now.getFullYear(), now.getMonth() + 1, now.getDate(), 15, 3, 0, 0] },
  { input: '23:59:59', ymd: [now.getFullYear(), now.getMonth() + 1, now.getDate(), 23, 59, 59, 0] },
  { input: '下午3点', ymd: [now.getFullYear(), now.getMonth() + 1, now.getDate(), 15, 0, 0, 0] },
  { input: '08:30', ymd: [now.getFullYear(), now.getMonth() + 1, now.getDate(), 8, 30, 0, 0] },
  {
    input: '明天 15:00',
    ymd: (function () {
      const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 15, 0, 0, 0);
      return [d.getFullYear(), d.getMonth() + 1, d.getDate(), 15, 0, 0, 0];
    })(),
  },
  {
    input: '昨天',
    ymd: (function () {
      const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
      return [d.getFullYear(), d.getMonth() + 1, d.getDate(), 0, 0, 0, 0];
    })(),
  },
];

for (const c of localCases) {
  const rows = timeRows(search(c.input));
  const label = JSON.stringify(c.input);
  if (rows.length !== 2) {
    fail(`${label} 期望 2 行（秒级+毫秒级），实际 ${rows.length}`);
    continue;
  }
  const [y, mo, d, h, mi, s, ms] = c.ymd;
  // Node 的 Date(y, mo-1, ...) 按本机时区解释，与 chrono::Local 是两条独立实现
  const expectMs = new Date(y, mo - 1, d, h, mi, s, ms).getTime();
  const gotSecs = Number(rows[0].title);
  const gotMs = Number(rows[1].title);
  if (gotMs !== expectMs) {
    fail(`${label} 毫秒级 ${gotMs} != Node ${expectMs}（差 ${(gotMs - expectMs) / 3600000} 小时）`);
    continue;
  }
  if (gotSecs !== Math.floor(expectMs / 1000)) {
    fail(`${label} 秒级 ${gotSecs} != ${Math.floor(expectMs / 1000)}`);
    continue;
  }
  if (rows[0].payload.text !== rows[0].title || rows[1].payload.text !== rows[1].title) {
    fail(`${label} 复制内容与显示内容不一致`);
    continue;
  }
  // 反解：Node 从 epoch 还原墙上时间与星期，必须和 Rust 展示的完全一致
  const back = new Date(gotMs);
  const nodeLocal =
    `${back.getFullYear()}-${pad(back.getMonth() + 1)}-${pad(back.getDate())} ` +
    `${pad(back.getHours())}:${pad(back.getMinutes())}:${pad(back.getSeconds())}`;
  const nodeWall = rows[0].payload.echo.local;
  if (!nodeWall.startsWith(nodeLocal)) {
    fail(`${label} 展示时间 "${nodeWall}" 与 Node 反解 "${nodeLocal}" 不一致`);
    continue;
  }
  if (!nodeWall.endsWith(WEEKDAYS[back.getDay()])) {
    fail(`${label} 星期标注错误：${nodeWall}（应为 ${WEEKDAYS[back.getDay()]}）`);
    continue;
  }
  ok(`${label} -> ${gotSecs} / ${gotMs}`);
}

// ---------- 2) 显式时区 ----------
console.log('==> 显式时区：与 Date.parse 对齐');
const tzCases = [
  { input: '2026-11-16T15:03:22Z', ref: '2026-11-16T15:03:22Z' },
  { input: '2026-11-16 15:03:22+00:00', ref: '2026-11-16T15:03:22Z' },
  { input: '2026-11-16T15:03:22+08:00', ref: '2026-11-16T15:03:22+08:00' },
  { input: '2026-11-16 15:03:22+0800', ref: '2026-11-16T15:03:22+08:00' },
  { input: '2026-11-16 15:03:22-05:00', ref: '2026-11-16T15:03:22-05:00' },
  { input: '2026-11-16Z', ref: '2026-11-16T00:00:00Z' },
];
for (const c of tzCases) {
  const rows = timeRows(search(c.input));
  const label = JSON.stringify(c.input);
  if (rows.length !== 2) {
    fail(`${label} 期望 2 行，实际 ${rows.length}`);
    continue;
  }
  const expect = Date.parse(c.ref);
  if (Number(rows[1].title) !== expect) {
    fail(`${label} -> ${rows[1].title} != Date.parse ${expect}`);
    continue;
  }
  if (rows[0].payload.echo.zone !== 'UTC' + (c.ref.endsWith('Z') ? '+00:00' : c.ref.slice(19))) {
    fail(`${label} 时区标注 ${rows[0].payload.echo.zone} 不符合预期`);
    continue;
  }
  ok(`${label} -> ${rows[0].title}`);
}

// ---------- 3) 现在 ----------
console.log('==> 「现在」取当前时刻');
{
  const before = Date.now();
  const rows = timeRows(search('现在'));
  const after = Date.now();
  if (rows.length !== 2) {
    fail(`"现在" 期望 2 行，实际 ${rows.length}`);
  } else {
    const got = Number(rows[1].title);
    if (got < before - 5000 || got > after + 5000) {
      fail(`"现在" = ${got} 不在 [${before}, ${after}] 附近`);
    } else {
      ok(`"现在" -> ${rows[0].title} / ${rows[1].title}`);
    }
  }
}

// ---------- 4) 绝不劫持应用搜索 ----------
console.log('==> 不劫持：这些输入不得产生时间行');
const mustNot = [
  'chrome',
  'notepad',
  '3.10 64-bit',
  '1.2.3',
  '2026',
  '1700000000',
  '1700000000000',
  'C:\\Program Files',
  'www.example.com',
  'https://example.com/a',
  '2026-11-16-1',
  '2026-13-45',
  '2026-02-30',
  '10:0',
  '15:03 x',
  '半天',
  'tomorrowx',
  '时间戳',
];
const beforeMustNot = failed;
for (const q of mustNot) {
  const rows = timeRows(search(q));
  if (rows.length) {
    fail(`"${q}" 被时间解析器劫持：${rows.map((r) => r.title).join(' / ')}`);
  }
}
if (failed === beforeMustNot) ok('以上输入均未出现时间行');

// ---------- 5) 与计算器的边界 ----------
console.log('==> 计算器让位边界');
{
  const hits = search('2026-11-16');
  const calc = hits.filter((h) => h.kind === 'calc');
  if (calc.length) {
    fail(`"2026-11-16" 仍被当减法算出 ${calc[0].title}，计算器该让位`);
  } else {
    ok('"2026-11-16" 计算器已让位给日期');
  }
  const hits2 = search('11-16');
  const calc2 = hits2.filter((h) => h.kind === 'calc');
  if (!calc2.length || calc2[0].title !== '-5') {
    fail(`"11-16" 应仍归计算器得 -5，实际 ${calc2.map((c) => c.title).join(',') || '无'}`);
  } else {
    ok('"11-16" 仍归计算器（两段式没被抢）');
  }
  const hits3 = search('6*7');
  if (!hits3.some((h) => h.kind === 'calc' && h.title === '42')) {
    fail('"6*7" 计算器结果丢失');
  } else {
    ok('"6*7" 计算器正常');
  }
}

// ---------- 3.5) 系统时间的多种格式 ----------
// 逐条拿 Node 的渲染结果比。星期名/月份名用 ICU 的 toLocaleDateString 取，
// 而不是照抄 Rust 里的表 —— 否则又是 Rust 自证。
console.log('==> 系统时间格式：与 Node/ICU 的渲染逐条对齐');
{
  const EN_WD = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const EN_MO = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const CN_WD = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

  const hits = search('now');
  const clock = clockRows(hits);
  const epoch = timeRows(hits);
  if (clock.length !== 8) {
    fail(`"now" 期望 8 行系统时间，实际 ${clock.length}`);
  } else if (epoch.length !== 2) {
    fail(`"now" 期望 2 行时间戳，实际 ${epoch.length}`);
  } else {
    // 瞬间从我们自己的 ISO 行反解（Node 解析无时区 ISO 串按本地处理），
    // 这样比较的是"同一屏内部是否自洽"，不受两次取时刻的跨秒抖动影响
    const d = new Date(clock[0].title.replace(' ', 'T'));
    if (Number.isNaN(d.getTime())) fail(`ISO 行无法被 Node 解析：${clock[0].title}`);
    else {
      const secs = Number(epoch[0].title);
      if (Math.abs(d.getTime() / 1000 - secs) > 2) {
        fail(`格式行与时间戳行不是同一瞬间：${d.getTime() / 1000} vs ${secs}`);
      } else {
        const offMin = -d.getTimezoneOffset();
        const sign = offMin < 0 ? '-' : '+';
        const abs = Math.abs(offMin);
        const oh = pad(Math.floor(abs / 60));
        const om = pad(abs % 60);
        const expect = [
          `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`,
          `${d.getFullYear()}/${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`,
          `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
          `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`,
          `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}${sign}${oh}:${om}`,
          `${EN_WD[d.getDay()]}, ${pad(d.getDate())} ${EN_MO[d.getMonth()]} ${d.getFullYear()} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())} ${sign}${oh}${om}`,
          `${d.getFullYear()}年${pad(d.getMonth() + 1)}月${pad(d.getDate())}日 ${pad(d.getHours())}时${pad(d.getMinutes())}分${pad(d.getSeconds())}秒 ${CN_WD[d.getDay()]}`,
          `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}_${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`,
        ];
        let bad = 0;
        expect.forEach((e, i) => {
          if (clock[i].title !== e) {
            fail(`格式 ${i} 不一致：我们 "${clock[i].title}" vs Node "${e}"`);
            bad++;
          }
          if (clock[i].payload.text !== clock[i].title) {
            fail(`格式 ${i} 复制内容与显示内容不一致`);
            bad++;
          }
        });
        // ICU 独立复核星期/月份缩写（不依赖上面那份手写表）
        const rfc = clock[5].title;
        const cn = clock[6].title;
        if (!rfc.startsWith(d.toLocaleDateString('en-US', { weekday: 'short' }) + ', ')) {
          fail(`RFC1123 星期与 ICU 不一致：${rfc}`);
          bad++;
        }
        if (!rfc.includes(' ' + d.toLocaleDateString('en-US', { month: 'short' }) + ' ')) {
          fail(`RFC1123 月份与 ICU 不一致：${rfc}`);
          bad++;
        }
        if (!cn.endsWith(d.toLocaleDateString('zh-CN', { weekday: 'short' }))) {
          fail(`中文星期与 ICU 不一致：${cn}`);
          bad++;
        }
        if (!bad) ok(`8 种格式逐条与 Node/ICU 一致（${clock[0].title} / ${rfc}）`);
      }
    }
  }

  // date 只出格式、不出 epoch；且绝不能吃掉包含这些词的其它输入
  const dateRows = clockRows(search('date'));
  if (dateRows.length !== 8) fail(`"date" 期望 8 行，实际 ${dateRows.length}`);
  if (timeRows(search('date')).length) fail('"date" 不该出时间戳行');
  for (const q of ['update', 'dated', 'datetime', 'date 2026-11-16', 'runtime', '时间戳', 'timezone', 'noweb', 'update time', 'chrome', '今天 15:00']) {
    const rows = clockRows(search(q));
    if (rows.length) fail(`"${q}" 被系统时间劫持：${rows.map((r) => r.title).join(' / ')}`);
  }
}

console.log('');
if (failed) {
  console.log(`TIME PROBE: ${failed} CHECK(S) FAILED`);
  process.exit(1);
}
console.log('TIME PROBE: ALL CHECKS PASSED');
