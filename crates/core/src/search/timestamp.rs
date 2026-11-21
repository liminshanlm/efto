//! 内置时间戳：输入是时间时，在结果列表里给出**秒级**与**毫秒级**两个 Unix 时间戳。
//!
//! 设计取舍（与 [`super::calc`] 同一套纪律）：
//! 1. **绝不劫持应用搜索**。整串必须被完整消费，任何残留字符都放弃。因此 `chrome`、
//!    `3.10 64-bit`、`1.2.3`、`C:\Program Files`、`2026`（光秃秃的年份）、
//!    `1700000000`（它本身就是时间戳，属于反向换算，交给插件/工具）都不出时间行。
//! 2. **两段式 `11-16` 只在有日期单位或后面跟了时间时才当日期**。因为 `2-3`、`11/16`
//!    同时是合法的减法/除法算式，那类输入已经归计算器。带年份的 `2026-11-16` 没有这种
//!    歧义（四位数当被减数没有实际意义），日期优先——计算器会主动让位，见
//!    [`super::calc::evaluate`]。
//! 3. **不猜歧义写法**。`11/12/2026` 既可能是 11 月 12 日也可能是 12 月 11 日，直接放弃。
//!    能靠"谁大于 12"消歧的（`16/11/2026`、`11/16/2026`）才接受。
//! 4. **时区规则明确**。输入不写时区就用本机时区（含历史 DST 规则）；写了 `Z`/`UTC`/
//!    `+08:00` 就按输入的来。DST 拨快造成的"不存在的本地时间"返回 None，回拨造成的
//!    重复时刻取较早的一次。
//! 5. **不 panic**。字段范围逐一校验，年份限制在 1000..=9999，不存在的日期
//!    （`2026-02-30`、`2026-04-31`）由 chrono 拒绝。

use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, SecondsFormat, TimeZone, Timelike};
use serde_json::json;

use super::fuzzy::MatchReason;
use super::{HitPayload, SearchHit};
use crate::ItemKind;

/// 输入长度上限：超过就当普通文本处理。
const MAX_INPUT_LEN: usize = 64;
/// 年份下限，避免把 `1.2.3`、`0001-01-01` 当日期。
const MIN_YEAR: i32 = 1000;
/// 年份上限。
const MAX_YEAR: i32 = 9999;

/// 一条解析成功的时间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeStamp {
    /// 秒级 Unix 时间戳
    pub secs: i64,
    /// 毫秒级 Unix 时间戳
    pub millis: i64,
    /// 墙上时间，如 `2026-11-16 15:03:22 周一`
    pub local: String,
    /// 带毫秒的墙上时间，如 `2026-11-16 15:03:22.500 周一`
    pub local_ms: String,
    /// 生效的时区，如 `UTC+08:00`
    pub zone: String,
    /// RFC3339 形式，如 `2026-11-16T15:03:22+08:00`
    pub iso: String,
    /// 输入里是否显式写了时区（副标题据此提示"本机时区"）
    pub explicit_tz: bool,
    /// 时间分量（供 UI / 交叉校验使用）
    pub parts: Components,
}

/// 解析出来的墙上时间分量（未定时区）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Components {
    /// 年
    pub year: i32,
    /// 月 1..=12
    pub month: u32,
    /// 日 1..=31
    pub day: u32,
    /// 时 0..=23
    pub hour: u32,
    /// 分 0..=59
    pub minute: u32,
    /// 秒 0..=59
    pub second: u32,
    /// 毫秒 0..=999
    pub millis: u32,
}

/// 尝试把输入当时间解析。不是时间就返回 None。
pub fn resolve(input: &str) -> Option<TimeStamp> {
    resolve_at(input, None, Local::now())
}

/// 同上，但"今天/现在"这类相对写法用调用方给的 `now` 解释。
/// 一屏之内系统时间格式行与时间戳行必须来自**同一个瞬间**，
/// 否则跨秒时会出现 `17:48:59` 和下一秒的 epoch 并排显示。
fn resolve_at(
    input: &str,
    fallback: Option<FixedOffset>,
    now: DateTime<Local>,
) -> Option<TimeStamp> {
    let normalized = normalize(input);
    if normalized.is_empty() || normalized.chars().count() > MAX_INPUT_LEN {
        return None;
    }
    let flags = Flags::scan(&normalized);
    let body = strip_meridiem(&normalized);
    if body.is_empty() {
        return None;
    }
    let ascii = translate_units(&body);
    parse_shape(&ascii, &flags).and_then(|p| finish(p, fallback, now))
}

/// 生成下拉框里的命中：秒级一行 + 毫秒级一行。解析失败就是空列表。
pub fn time_hits(query: &str) -> Vec<SearchHit> {
    time_hits_at(query, Local::now())
}

/// 同 `time_hits`，但由调用方提供时刻（与系统时间行共用同一瞬间）。
pub fn time_hits_at(query: &str, now: DateTime<Local>) -> Vec<SearchHit> {
    let Some(ts) = resolve_at(query, None, now) else {
        return Vec::new();
    };
    let zone_note = if ts.explicit_tz {
        ts.zone.clone()
    } else {
        format!("{} · 本机时区", ts.zone)
    };
    let mut out = Vec::with_capacity(2);
    for (key, label, value, score, shown) in [
        ("sec", "秒级", ts.secs.to_string(), 690.0f32, ts.local.clone()),
        ("ms", "毫秒级", ts.millis.to_string(), 686.0f32, ts.local_ms.clone()),
    ] {
        out.push(
            SearchHit::new(
                ItemKind::Time,
                format!("time:{key}:{}", ts.secs),
                value.clone(),
                score,
                HitPayload::CopyText {
                    text: value,
                    echo: json!({
                        "kind": "timestamp",
                        "precision": key,
                        "input": query.trim(),
                        "local": shown,
                        "iso": ts.iso,
                        "zone": ts.zone,
                        "secs": ts.secs,
                        "millis": ts.millis,
                        "components": {
                            "year": ts.parts.year,
                            "month": ts.parts.month,
                            "day": ts.parts.day,
                            "hour": ts.parts.hour,
                            "minute": ts.parts.minute,
                            "second": ts.parts.second,
                            "millis": ts.parts.millis,
                        },
                    }),
                },
            )
            .subtitle(format!("{label} · {shown} · {zone_note}"))
            .reason(MatchReason::Exact)
            .source("time"),
        );
    }
    out
}

// ---------------------------------------------------------------------------
// 归一化
// ---------------------------------------------------------------------------

/// 全角转半角、统一标点、压掉多余空白、转小写。**保留**中文日期时间单位，
/// 让 [`translate_units`] 在翻译时顺便记录"是否出现过 日/时"这类授权信息。
fn normalize(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for ch in src.chars() {
        let mapped = match ch {
            '０'..='９' => char::from_u32(0x30 + (ch as u32 - 0xFF10)).unwrap_or(ch),
            'Ａ'..='Ｚ' => char::from_u32(0x41 + (ch as u32 - 0xFF21)).unwrap_or(ch),
            'ａ'..='ｚ' => char::from_u32(0x61 + (ch as u32 - 0xFF41)).unwrap_or(ch),
            '．' => '.',
            '：' => ':',
            // 全角连字符/斜杠/加号：`２０２６－１１－１６` 也要能拆开
            '－' | '‐' | '‑' | 'ー' => '-',
            '／' | '∕' => '/',
            '＋' => '+',
            '　' | '\u{2009}' | '\u{202F}' | '\u{00A0}' => ' ',
            '。' | '、' => ' ',
            c => c,
        };
        out.push(mapped);
    }
    let mut collapsed = String::with_capacity(out.len());
    let mut in_space = false;
    for ch in out.chars() {
        if ch == ' ' {
            if !in_space && !collapsed.is_empty() {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            in_space = false;
            collapsed.push(ch);
        }
    }
    collapsed.trim().to_lowercase()
}

/// 归一化后、翻译前扫一遍原文，记录授权信息。
#[derive(Debug, Clone, Copy, Default)]
struct Flags {
    /// 出现过 月/日/号 —— 允许 `11月16日` 这种没有年份的两段式
    had_day_unit: bool,
    /// 出现过 时/点/分/秒 —— 允许 `上午10点` 这种没有分钟的裸小时
    had_time_unit: bool,
    meridiem: Meridiem,
}

impl Flags {
    fn scan(s: &str) -> Self {
        let mut f = Flags {
            meridiem: meridiem_of(s),
            ..Default::default()
        };
        for c in s.chars() {
            match c {
                '月' | '日' | '号' => f.had_day_unit = true,
                '时' | '点' | '分' | '秒' => f.had_time_unit = true,
                _ => {}
            }
        }
        f
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Meridiem {
    #[default]
    None,
    Am,
    Pm,
}

/// 中午/正午 归到 PM 侧：`中午12点` 是 12:00 而不是 00:00，
/// `中午1点` 是 13:00。凌晨/早上/上午 才是 AM（12 点变 0 点）。
const PM_WORDS: &[&str] = &["下午", "晚上", "夜里", "深夜", "傍晚", "中午", "正午", "pm"];
const AM_WORDS: &[&str] = &["上午", "早上", "凌晨", "am"];

fn meridiem_of(s: &str) -> Meridiem {
    if PM_WORDS.iter().any(|k| s.contains(k)) {
        Meridiem::Pm
    } else if AM_WORDS.iter().any(|k| s.contains(k)) {
        Meridiem::Am
    } else {
        Meridiem::None
    }
}

/// 去掉时段词（它们的语义由 [`Flags::meridiem`] 承载）。
fn strip_meridiem(s: &str) -> String {
    let mut out = s.to_string();
    for k in PM_WORDS.iter().chain(AM_WORDS.iter()) {
        out = out.replace(k, " ");
    }
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim().to_string()
}

/// 中文日期时间单位 -> 通用 ASCII 形状。
///
/// `2026年11月16日15时03分22秒` -> `2026-11-16 15:03:22`
/// `11月16日`                  -> `11-16`
/// `上午10点半`                -> `10:30`（时段词已被剥掉）
///
/// 关键点：`日/号` 翻译成**空格**而不是删除，否则
/// `2026年11月16日15时` 会黏成 `2026-11-1615:00`。
fn translate_units(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '年' | '月' => out.push('-'),
            '日' | '号' => out.push(' '),
            '时' | '点' | '分' => out.push(':'),
            '秒' => out.push(' '),
            '半' => {
                // 只有紧跟在冒号后的"半"才是"半分钟"，即 30 分；
                // 其他位置出现"半"说明这不是时间表达式，用不可能匹配的字符顶掉。
                if out.ends_with(':') {
                    out.push_str("30");
                } else {
                    out.push('×');
                }
            }
            c => out.push(c),
        }
    }
    // `15时03分` 翻译后是 `15:03:`，`10点` 是 `10:` —— 结尾冒号无害，去掉
    let trimmed = out.trim().trim_end_matches(':').trim().to_string();
    // 冒号后紧跟空格（`15: 03` 这类手滑）不算时间
    trimmed
}

// ---------------------------------------------------------------------------
// 结构解析
// ---------------------------------------------------------------------------

/// 一组还没定时区的墙上时间分量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Parts {
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
    hour: Option<u32>,
    minute: Option<u32>,
    second: Option<u32>,
    millis: Option<u32>,
    /// 输入里显式给出的 UTC 偏移（分钟）
    offset_min: Option<i32>,
    meridiem: Meridiem,
    /// 相对今天的日偏移（今天/明天/前天/后天）
    day_shift: Option<i64>,
    /// 直接取当前时刻（现在 / now）
    use_now: bool,
}

/// 顶层：关键字 -> 日期/时间形状。
fn parse_shape(s: &str, flags: &Flags) -> Option<Parts> {
    let (kw, rest) = split_day_keyword(s);
    if kw == Some(DayKeyword::Now) {
        // `现在 15:00` 语义不明，宁可不给结果
        if !rest.is_empty() {
            return None;
        }
        return Some(Parts {
            use_now: true,
            meridiem: flags.meridiem,
            ..Default::default()
        });
    }

    let mut parts = if rest.is_empty() {
        // 只有 `今天` / `明天`：当天 00:00
        Parts::default()
    } else {
        parse_datetime(rest, flags)?
    };

    if kw.is_some() {
        // `明天 2026-11-16` 自相矛盾，放弃而不是覆盖
        if parts.year.is_some() || parts.month.is_some() || parts.day.is_some() {
            return None;
        }
        parts.day_shift = match kw {
            Some(DayKeyword::Shift(d)) => Some(d),
            _ => None,
        };
    }
    // 只有时段词没有时间（`下午`）没有意义
    if flags.meridiem != Meridiem::None && parts.hour.is_none() {
        return None;
    }
    // 时段词在 strip_meridiem 里被吃掉了，这里补回给 finish 做 12 小时制换算。
    // 漏掉这一步的后果是 `下午3点` 静默变成凌晨 3 点——数值差 12 小时且不报错。
    parts.meridiem = flags.meridiem;
    Some(parts)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DayKeyword {
    Now,
    Shift(i64),
}

/// 识别开头的 现在/今天/明天/…，返回 (关键字, 剩余串)。
fn split_day_keyword(s: &str) -> (Option<DayKeyword>, &str) {
    // 长词优先，`现在时间` 不该被 `现在` 抢走后留下"时间"这种垃圾尾巴
    const TABLE: &[(&str, DayKeyword)] = &[
        ("现在时间", DayKeyword::Now),
        ("当前时间", DayKeyword::Now),
        ("此刻", DayKeyword::Now),
        ("现在", DayKeyword::Now),
        ("当前", DayKeyword::Now),
        ("今天", DayKeyword::Shift(0)),
        ("今日", DayKeyword::Shift(0)),
        ("昨天", DayKeyword::Shift(-1)),
        ("昨日", DayKeyword::Shift(-1)),
        ("明天", DayKeyword::Shift(1)),
        ("明日", DayKeyword::Shift(1)),
        ("前天", DayKeyword::Shift(-2)),
        ("后天", DayKeyword::Shift(2)),
        ("now", DayKeyword::Now),
        ("today", DayKeyword::Shift(0)),
        ("yesterday", DayKeyword::Shift(-1)),
        ("tomorrow", DayKeyword::Shift(1)),
    ];
    for (k, v) in TABLE {
        if let Some(tail) = s.strip_prefix(k) {
            let tail = tail.trim_start();
            // 英文关键字必须是完整单词：`tomorrowx` 不该被切开
            if let Some(first) = tail.chars().next() {
                if k.is_ascii() && (first.is_ascii_alphanumeric() || first == '-' || first == '.') {
                    return (None, s);
                }
            }
            return (Some(*v), tail);
        }
    }
    (None, s)
}

/// 解析 `[日期] [时间] [时区]`，接受 `2026-11-16T15:03:22+08:00` 连写。
fn parse_datetime(s: &str, flags: &Flags) -> Option<Parts> {
    let tokens = split_tokens(s);
    if tokens.is_empty() {
        return None;
    }
    // 有时间段跟在后面，才允许 `11-16` 这种两段式日期（`11-16 15:00` 显然不是减法）
    let has_time_token = tokens.iter().any(|t| looks_like_time(t));
    let allow_short_date = flags.had_day_unit || has_time_token;
    // `下午3点` 归一化后是 `3`：没有中文单位、也没有冒号，
    // 但时段词已经证明这是小时，否则 `3pm` / `下午3点` 就废了
    let bare_hour_ok = flags.had_time_unit || flags.meridiem != Meridiem::None;

    let mut p = Parts::default();
    let mut saw_date = false;
    let mut saw_time = false;

    for token in &tokens {
        if looks_like_time(token) {
            if saw_time {
                return None;
            }
            let (clock, tz) = split_tz_suffix(token);
            fill_time(&mut p, clock)?;
            if let Some(m) = tz {
                p.offset_min = Some(m);
            }
            saw_time = true;
        } else if looks_like_date(token) {
            if saw_date {
                return None;
            }
            let (date_text, tz) = split_z_suffix(token);
            fill_date(&mut p, date_text, allow_short_date)?;
            if let Some(m) = tz {
                p.offset_min = Some(m);
            }
            saw_date = true;
        } else if is_tz_token(token) {
            p.offset_min = Some(parse_offset(token)?);
        } else if bare_hour_ok && !saw_time && token.len() <= 2 && is_digits(token) {
            // `上午10点` -> `10`：只有确认出现过 时/点/分/秒 或时段词才敢当小时
            p.hour = Some(token.parse().ok()?);
            p.minute = Some(0);
            p.second = Some(0);
            saw_time = true;
        } else {
            return None; // 有任何解释不了的片段就整体放弃
        }
    }

    if !saw_date && !saw_time {
        return None;
    }
    // 光秃秃的年份没有信息量，还会劫持搜索（`2026` 不是时间）
    if !saw_time && p.month.is_none() {
        return None;
    }
    Some(p)
}

/// 按空格切 token，并把 ISO 的 `2026-11-16T15:03:22` 拆成两个 token。
/// 拆开后时区后缀仍由时间 token 自己携带，不会被丢掉。
fn split_tokens(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in s.split(' ').filter(|t| !t.is_empty()) {
        if let Some(idx) = raw.find('t') {
            let (l, r) = (&raw[..idx], &raw[idx + 1..]);
            if !l.is_empty() && !r.is_empty() && looks_like_date(l) && looks_like_time(r) {
                out.push(l.to_string());
                out.push(r.to_string());
                continue;
            }
        }
        out.push(raw.to_string());
    }
    out
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// `15:03` / `06:13:20` / `06:13:20.250`，可带 `Z` 或 `+08:00` 尾巴。
fn looks_like_time(t: &str) -> bool {
    let (body, _) = split_tz_suffix(t);
    let seg: Vec<&str> = body.split(':').collect();
    if seg.len() < 2 || seg.len() > 3 {
        return false;
    }
    if seg[0].len() > 2 || !is_digits(seg[0]) {
        return false;
    }
    if seg[1].len() != 2 || !is_digits(seg[1]) {
        return false;
    }
    if let Some(sec) = seg.get(2) {
        let mut it = sec.split(['.', ',']);
        let whole = it.next().unwrap_or("");
        let frac = it.next().unwrap_or("");
        if it.next().is_some() || whole.len() != 2 || !is_digits(whole) {
            return false;
        }
        if !frac.is_empty() && (!is_digits(frac) || frac.len() > 6) {
            return false;
        }
    }
    true
}

/// `2026-11-16` / `2026/11` / `11-16` / `20261116` / `16/11/2026`，可带 `Z` 尾巴。
fn looks_like_date(t: &str) -> bool {
    let (body, _) = split_z_suffix(t);
    if is_digits(body) {
        return body.len() == 8; // 10/13 位是时间戳本身，绝不当日期
    }
    let seg: Vec<&str> = body.split(['-', '/', '.']).collect();
    if seg.is_empty() || seg.len() > 3 {
        return false;
    }
    seg.iter().all(|p| is_digits(p) && !p.is_empty() && p.len() <= 4) && body.len() >= 5
}

/// 单独的时区 token：`UTC` / `GMT` / `Z` / `+08:00` / `+0800` / `+08`。
fn is_tz_token(t: &str) -> bool {
    matches!(t, "z" | "utc" | "gmt") || parse_offset(t).is_some()
}

/// 日期 token 结尾的 `Z`（`2026-11-16Z` = 当天 UTC 零点）。
fn split_z_suffix(t: &str) -> (&str, Option<i32>) {
    if let Some(stripped) = t.strip_suffix('z') {
        if is_digits(stripped) && stripped.len() == 8 {
            return (stripped, Some(0));
        }
        let seg: Vec<&str> = stripped.split(['-', '/', '.']).collect();
        if seg.len() >= 2 && seg.iter().all(|p| is_digits(p) && !p.is_empty()) {
            return (stripped, Some(0));
        }
    }
    (t, None)
}

/// 剥掉时间串结尾的 `Z` / `+08:00` / `-0500`，返回 (剩余, 偏移分钟)。
fn split_tz_suffix(t: &str) -> (&str, Option<i32>) {
    if let Some(stripped) = t.strip_suffix('z') {
        if stripped.contains(':') {
            return (stripped, Some(0));
        }
    }
    if let Some(pos) = t.rfind(['+', '-']) {
        let head = &t[..pos];
        // 必须已经有一段像时间的内容（含冒号），否则 `2026-11-16-1` 会被误切
        if head.contains(':') {
            if let Some(min) = parse_offset(&t[pos..]) {
                return (head, Some(min));
            }
        }
    }
    (t, None)
}

/// `+08:00` / `+0800` / `+08` / `utc` -> 分钟。
fn parse_offset(t: &str) -> Option<i32> {
    let lower = t.to_lowercase();
    if matches!(lower.as_str(), "z" | "utc" | "gmt") {
        return Some(0);
    }
    // 必须先确认符号再切字节：`split_at(1)` 落在多字节字符中间会直接 panic，
    // 而任意中文查询（`微信`）都会走进这个函数
    if !lower.starts_with(['+', '-']) {
        return None;
    }
    let (sign, body) = lower.split_at(1);
    let sign_mul = if sign == "+" { 1 } else { -1 };
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return None;
    }
    if body.contains(':') {
        let mut it = body.split(':');
        let h: i32 = it.next()?.parse().ok()?;
        let m: i32 = it.next()?.parse().ok()?;
        if it.next().is_some() || h > 23 || m > 59 {
            return None;
        }
        return Some(sign_mul * (h * 60 + m));
    }
    match body.len() {
        2 => {
            let h: i32 = body.parse().ok()?;
            if h > 23 {
                return None;
            }
            Some(sign_mul * h * 60)
        }
        4 => {
            let h: i32 = body[..2].parse().ok()?;
            let m: i32 = body[2..].parse().ok()?;
            if h > 23 || m > 59 {
                return None;
            }
            Some(sign_mul * (h * 60 + m))
        }
        _ => None,
    }
}

/// 解析日期分量。`allow_short` 控制是否接受没有年份的两段式（`11-16`）。
fn fill_date(p: &mut Parts, t: &str, allow_short: bool) -> Option<()> {
    if is_digits(t) {
        if t.len() != 8 {
            return None;
        }
        p.year = Some(t[..4].parse().ok()?);
        p.month = Some(t[4..6].parse().ok()?);
        p.day = Some(t[6..8].parse().ok()?);
        return Some(());
    }
    let seg: Vec<&str> = t.split(['-', '/', '.']).collect();
    match seg.len() {
        2 => {
            let a = parse_num(seg[0])?;
            let b = parse_num(seg[1])?;
            if seg[0].len() == 4 {
                p.year = Some(a as i32);
                p.month = Some(b);
                p.day = Some(1);
            } else if allow_short {
                // 只给了两段：谁大于 12 谁就是日（`16/11` -> 11 月 16 日）
                if a > 12 && b <= 12 {
                    p.day = Some(a);
                    p.month = Some(b);
                } else if b > 12 && a <= 12 {
                    p.month = Some(a);
                    p.day = Some(b);
                } else {
                    p.month = Some(a);
                    p.day = Some(b);
                }
            } else {
                return None;
            }
            Some(())
        }
        3 => {
            let a = parse_num(seg[0])?;
            let b = parse_num(seg[1])?;
            let c = parse_num(seg[2])?;
            if seg[0].len() == 4 {
                p.year = Some(a as i32);
                p.month = Some(b);
                p.day = Some(c);
            } else if seg[2].len() == 4 {
                // 年末写法：靠"谁大于 12"消歧，两边都 <=12 就放弃，不猜
                if a > 12 && b <= 12 {
                    p.day = Some(a);
                    p.month = Some(b);
                } else if b > 12 && a <= 12 {
                    p.month = Some(a);
                    p.day = Some(b);
                } else {
                    return None;
                }
                p.year = Some(c as i32);
            } else {
                return None;
            }
            Some(())
        }
        _ => None,
    }
}

fn parse_num(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 4 || !is_digits(s) {
        return None;
    }
    s.parse().ok()
}

/// 解析 `h:m[:s[.frac]]`。范围校验留给 [`finish`]。
fn fill_time(p: &mut Parts, t: &str) -> Option<()> {
    let (body, _) = split_tz_suffix(t);
    let seg: Vec<&str> = body.split(':').collect();
    if seg.is_empty() || seg.len() > 3 {
        return None;
    }
    let h: u32 = seg[0].parse().ok()?;
    let m: u32 = seg.get(1).and_then(|v| v.parse().ok())?;
    let (s, frac) = match seg.get(2) {
        Some(rest) => match rest.split_once(['.', ',']) {
            Some((sec, f)) => {
                let mut digits: String = f.chars().take(3).collect();
                while digits.len() < 3 {
                    digits.push('0');
                }
                (sec.parse().ok()?, digits.parse::<u32>().ok()?)
            }
            None => (rest.parse().ok()?, 0),
        },
        None => (0, 0),
    };
    p.hour = Some(h);
    p.minute = Some(m);
    p.second = Some(s);
    p.millis = Some(frac);
    Some(())
}

// ---------------------------------------------------------------------------
// 校验 + 换算
// ---------------------------------------------------------------------------

fn finish(mut p: Parts, fallback: Option<FixedOffset>, now: DateTime<Local>) -> Option<TimeStamp> {
    if p.use_now {
        let now = now.fixed_offset();
        let parts = Components {
            year: now.year(),
            month: now.month(),
            day: now.day(),
            hour: now.hour(),
            minute: now.minute(),
            second: now.second(),
            millis: now.timestamp_subsec_millis(),
        };
        return Some(stamp_from(now, false, parts));
    }

    // 12 小时制修正
    if let Some(h) = p.hour {
        match p.meridiem {
            Meridiem::Pm => {
                if !(1..=12).contains(&h) {
                    return None;
                }
                p.hour = Some(if h < 12 { h + 12 } else { h });
            }
            Meridiem::Am => {
                if !(0..=12).contains(&h) {
                    return None;
                }
                p.hour = Some(if h == 12 { 0 } else { h });
            }
            Meridiem::None => {}
        }
    }

    let today = now.date_naive();
    let base = match p.day_shift {
        Some(d) => today.checked_add_signed(chrono::Duration::days(d))?,
        None => today,
    };

    let year = match p.year {
        Some(y) if (MIN_YEAR..=MAX_YEAR).contains(&y) => y,
        Some(_) => return None,
        None => base.year(),
    };
    // 日期缺省规则：完全没解析到日期分量时，用 base（今天，或"明天"这类关键字指定的那天）。
    // `15:03` / `下午3点` 是"今天几点"，不是"今年 1 月 1 日 15:03"——
    // 早先这里只看了 day_shift，导致纯时间落到 01-01。
    let no_date_at_all = p.year.is_none() && p.month.is_none() && p.day.is_none();
    let from_base = p.day_shift.is_some() || no_date_at_all;
    let month = p.month.unwrap_or(if from_base { base.month() } else { 1 });
    let day = p.day.unwrap_or(if from_base { base.day() } else { 1 });
    let hour = p.hour.unwrap_or(0);
    let minute = p.minute.unwrap_or(0);
    let second = p.second.unwrap_or(0);
    let millis = p.millis.unwrap_or(0);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 || millis > 999 {
        return None;
    }
    // 只有日关键字（今天/明天）时，日期由偏移决定
    if p.day_shift.is_some() && (p.year.is_some() || p.month.is_some() || p.day.is_some()) {
        return None;
    }

    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_milli_opt(hour, minute, second, millis))?;

    let explicit_tz = p.offset_min.is_some();
    // 优先级：输入显式时区 > 调用方给定的兜底时区（仅测试）> 本机时区
    // east_opt 接收的是**秒**，而 offset_min 存的是分钟
    let given = p
        .offset_min
        .and_then(|m| FixedOffset::east_opt(m * 60))
        .or(fallback);
    let dt = match given {
        Some(tz) => tz.from_local_datetime(&naive).single()?,
        // 本机时区：DST 拨快造成的"不存在的本地时间"给不出结果；
        // 回拨造成的重复时刻取较早的一次
        None => Local.from_local_datetime(&naive).earliest()?.fixed_offset(),
    };
    Some(stamp_from(
        dt,
        explicit_tz,
        Components {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millis,
        },
    ))
}

fn stamp_from(dt: DateTime<FixedOffset>, explicit_tz: bool, parts: Components) -> TimeStamp {
    const WEEKDAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
    let weekday = WEEKDAYS[dt.weekday().num_days_from_monday() as usize];
    let secs = dt.timestamp();
    let millis = dt.timestamp_millis();
    let offset = dt.offset().local_minus_utc();
    let sign = if offset < 0 { '-' } else { '+' };
    let a = offset.abs();
    let zone = format!("UTC{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60);
    let base = dt.format("%Y-%m-%d %H:%M:%S").to_string();
    TimeStamp {
        secs,
        millis,
        local: format!("{base} {weekday}"),
        local_ms: format!("{base}.{:03} {weekday}", millis.rem_euclid(1000)),
        zone,
        iso: dt.to_rfc3339_opts(SecondsFormat::AutoSi, false),
        explicit_tz,
        parts,
    }
}

/// 计算器用它判断要不要让位：整串确实是时间时，
/// 就不再当算式（否则 `2026-11-16` 会顺手算出 1,999）。
pub fn looks_like_time_input(input: &str) -> bool {
    input.chars().any(|c| c.is_ascii_digit()) && resolve(input).is_some()
}

/// 测试与交叉校验入口：强制用给定时区解释没有显式时区的输入。
#[cfg(test)]
fn resolve_utc(input: &str) -> Option<TimeStamp> {
    let utc = FixedOffset::east_opt(0).expect("UTC 恒合法");
    resolve_at(input, Some(utc), Local::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 所有期望值都来自独立实现（Node 的 `Date`），不是本模块自己算的。
    fn secs(s: &str) -> Option<i64> {
        resolve_utc(s).map(|t| t.secs)
    }

    #[test]
    fn known_epoch_values() {
        assert_eq!(secs("1970-01-01"), Some(0));
        assert_eq!(secs("2000-01-01 00:00:00"), Some(946_684_800));
        assert_eq!(secs("2026-01-01"), Some(1_767_225_600));
        assert_eq!(secs("2026-11-16 15:03:22"), Some(1_794_841_402));
        assert_eq!(secs("2026-11"), Some(1_793_491_200), "年月 -> 1 号 00:00");
        assert_eq!(secs("2024-02-29"), Some(1_709_164_800), "闰日");
        // 1700000000 是北京时间 2023-11-15 06:13:20，即 UTC 2023-11-14 22:13:20
        assert_eq!(secs("2023-11-14 22:13:20"), Some(1_700_000_000));
        assert_eq!(secs("2023-11-15 06:13:20+08:00"), Some(1_700_000_000));
    }

    #[test]
    fn separators_and_t_form_are_interchangeable() {
        let want = Some(1_794_841_402);
        for s in [
            "2026-11-16 15:03:22",
            "2026/11/16 15:03:22",
            "2026.11.16 15:03:22",
            "20261116 15:03:22",
            "2026-11-16T15:03:22",
            "2026-11-16 15:03:22",
        ] {
            assert_eq!(secs(s), want, "{s}");
        }
        // 空格可省（紧挨着写），因为 T 之外的连写会被切成两个 token
        assert_eq!(secs("2026-11-16T15:03:22Z"), want);
    }

    #[test]
    fn chinese_units() {
        assert_eq!(secs("2026年11月16日15时03分22秒"), Some(1_794_841_402));
        assert_eq!(secs("2026年11月16日 15:03:22"), Some(1_794_841_402));
        assert_eq!(secs("2026年11月16日"), Some(1_794_787_200));
        assert_eq!(secs("2023年11月15日6时13分20秒+08:00"), Some(1_700_000_000));
        // 两段式：中文单位授权
        assert!(resolve_utc("11月16日").is_some());
    }

    #[test]
    fn meridiem() {
        assert_eq!(secs("2026-11-16 下午3点"), secs("2026-11-16 15:00:00"));
        assert_eq!(secs("2026-11-16 上午10点半"), secs("2026-11-16 10:30:00"));
        assert_eq!(secs("2026-11-16 中午12点"), secs("2026-11-16 12:00:00"));
        assert_eq!(secs("2026-11-16 凌晨12点半"), secs("2026-11-16 00:30:00"));
        assert_eq!(secs("2026-11-16 3pm"), secs("2026-11-16 15:00:00"));
        assert!(resolve_utc("2026-11-16 下午15点").is_none(), "下午15点 自相矛盾");
        assert!(resolve_utc("下午").is_none(), "只有时段词不构成时间");
    }

    #[test]
    fn short_date_needs_authorization() {
        // 计算器已经占了 `11-16`（减法）；只有带日期单位或后面跟了时间才当日期
        assert!(resolve("11-16").is_none());
        assert!(resolve("11-16 15:00").is_some());
        assert!(resolve("11月16日").is_some());
        assert!(resolve("16/11 08:00").is_some());
    }

    #[test]
    fn explicit_offset_wins_over_fallback() {
        assert_eq!(secs("2023-11-15 06:13:20"), Some(1_700_028_800));
        assert_eq!(secs("2023-11-15T06:13:20Z"), Some(1_700_028_800));
        assert_eq!(secs("2023-11-15 06:13:20 UTC"), Some(1_700_028_800));
        assert_eq!(secs("2023-11-15 06:13:20+0800"), Some(1_700_000_000));
        assert_eq!(secs("2023-11-15 06:13:20-05:00"), Some(1_700_046_800));
    }

    #[test]
    fn fractional_seconds_only_affect_millis() {
        let t = resolve_utc("2023-11-15 06:13:20.250").unwrap();
        assert_eq!(t.secs, 1_700_028_800);
        assert_eq!(t.millis, 1_700_028_800_250);
        assert!(t.local_ms.contains(".250"), "{}", t.local_ms);
        assert_eq!(resolve_utc("2023-11-15 06:13:20,5").unwrap().millis, 1_700_028_800_500);
    }

    #[test]
    fn invalid_calendar_values_are_refused() {
        for s in [
            "2026-02-30",
            "2026-02-29",
            "2026-13-01",
            "2026-04-31",
            "2026-11-16 24:00",
            "2026-11-16 12:60",
            "2026-11-16 12:00:60",
            "2026-00-10",
            "2026-11-00",
            "0001-01-01",
        ] {
            assert!(resolve_utc(s).is_none(), "不该接受 {s}");
        }
    }

    #[test]
    fn day_words() {
        let today = Local::now().date_naive();
        let day_of = |s: &str| resolve(s).unwrap().local.clone();
        let expect = |shift: i64, hm: &str| {
            let d = today + chrono::Duration::days(shift);
            format!("{} {hm}", d.format("%Y-%m-%d"))
        };
        assert!(day_of("今天").starts_with(&expect(0, "00:00:00")), "{}", day_of("今天"));
        assert!(day_of("明天 15:00").starts_with(&expect(1, "15:00:00")), "{}", day_of("明天 15:00"));
        assert!(day_of("昨天").starts_with(&expect(-1, "00:00:00")));
        assert!(day_of("前天 08:30").starts_with(&expect(-2, "08:30:00")));
        assert!(day_of("后天").starts_with(&expect(2, "00:00:00")));
        assert!(day_of("now").starts_with(&expect(0, "")), "{}", day_of("now"));
        // 关键字 + 显式日期 = 自相矛盾
        assert!(resolve("明天 2026-11-16").is_none());
        // 现在：接近当前时刻（独立交叉校验在 scripts/check-time-probe.js 里做）
        let now = resolve("现在").unwrap();
        let wall = chrono::Utc::now().timestamp();
        assert!((now.secs - wall).abs() <= 5, "现在 应当接近当前时刻");
        assert!(resolve("now 15:00").is_none(), "现在 后面不接时间");
    }

    #[test]
    fn time_only_means_today() {
        // 这是本机跑 release 二进制时抓到的真实回归：纯时间一度落到 01-01
        let today = Local::now().date_naive();
        let y = today.format("%Y-%m-%d").to_string();
        for q in ["15:03", "15:03:22", "08:30", "下午3点", "10点半", "23:59:59"] {
            let t = resolve(q).unwrap_or_else(|| panic!("{q} 应该解析成功"));
            assert!(t.local.starts_with(&y), "{q} -> {}，应以今天为日期", t.local);
        }
        // 部分日期仍然按"缺的那一项补 1"，不受上面规则影响
        assert_eq!(
            resolve_utc("2026-11").map(|t| t.local),
            Some("2026-11-01 00:00:00 周日".to_string()),
            "只给年月 -> 1 号，而不是今天"
        );
        assert!(resolve_utc("2026").is_none(), "光秃秃的年份不出结果");
    }

    #[test]
    fn never_hijacks_plain_queries() {
        for s in [
            "chrome",
            "notepad",
            "1700000000",
            "1700000000000",
            "2026",
            "3.10 64-bit",
            "1.2.3",
            "C:\\Program Files",
            "www.example.com",
            "https://a.b/c",
            "6*7",
            "1+1",
            "11-16",
            "2-3",
            "10:0",
            "2026-11-16-1",
            "202601",
            "9999999",
            "12345678",
            "15:03 x",
            "tomorrowx",
            "noweb",
            "",
            "   ",
            "时间戳",
            "ts",
            "半天",
        ] {
            assert!(time_hits(s).is_empty(), "不该出现时间行: {s:?}");
        }
    }

    #[test]
    fn ambiguous_day_month_year_is_refused() {
        assert!(resolve_utc("11/12/2026").is_none(), "两边都 <=12 时不猜");
        assert_eq!(
            resolve_utc("16/11/2026").map(|t| t.local),
            Some("2026-11-16 00:00:00 周一".to_string())
        );
        assert_eq!(
            resolve_utc("11/16/2026").map(|t| t.local),
            Some("2026-11-16 00:00:00 周一".to_string())
        );
    }

    #[test]
    fn two_rows_second_then_millisecond() {
        // 带 Z 的输入不受本机时区影响，可以直接断言具体数字（值来自 Node）
        let hits = time_hits("2023-11-15 06:13:20Z");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].kind, ItemKind::Time);
        assert_eq!(hits[0].title, "1700028800");
        assert_eq!(hits[1].title, "1700028800000");
        assert!(hits[0].subtitle.contains("秒级"));
        assert!(hits[1].subtitle.contains("毫秒级"));
        assert!(hits[1].subtitle.contains("毫秒级 · 2023-11-15 06:13:20.000 周三"), "{}", hits[1].subtitle);
        assert!(hits[0].score > hits[1].score, "秒级排在前面");
        for h in &hits {
            assert_eq!(h.source, "time");
            match &h.payload {
                HitPayload::CopyText { text, echo } => {
                    assert_eq!(text, &h.title, "所见即所复制");
                    assert_eq!(echo["millis"], json!(1_700_028_800_000i64));
                    assert_eq!(echo["components"]["hour"], json!(6));
                    assert_eq!(echo["zone"], json!("UTC+00:00"));
                }
                other => panic!("wrong payload {other:?}"),
            }
        }
    }

    #[test]
    fn rows_are_consistent_with_the_wall_time() {
        // 本机时区路径：不断言绝对值（那取决于机器时区），只断言两行自洽
        let hits = time_hits("2023-11-15 06:13:20");
        assert_eq!(hits.len(), 2);
        let s: i64 = hits[0].title.parse().unwrap();
        let ms: i64 = hits[1].title.parse().unwrap();
        assert_eq!(ms, s * 1000, "毫秒行必须是秒行的 1000 倍");
        assert_eq!(hits[0].title.len(), 10);
        assert_eq!(hits[1].title.len(), 13);
        // 展示的墙上时间与输入一致（时区无关的部分）
        assert!(hits[0].subtitle.contains("2023-11-15 06:13:20"), "{}", hits[0].subtitle);
        assert!(hits[0].subtitle.contains("本机时区"), "{}", hits[0].subtitle);
    }

    #[test]
    fn fullwidth_digits_accepted() {
        assert_eq!(secs("２０２６－１１－１６ １５:０３:２２"), Some(1_794_841_402));
    }

    #[test]
    fn weekday_names_match_the_calendar() {
        // 2026-11-16 是周一（Node: new Date("2026-11-16").toLocaleDateString('zh-CN',{weekday})）
        assert_eq!(resolve_utc("2026-11-16").unwrap().local, "2026-11-16 00:00:00 周一");
        assert!(resolve_utc("2026-11-17").unwrap().local.ends_with("周二"));
        assert!(resolve_utc("2026-11-22").unwrap().local.ends_with("周日"));
    }

    #[test]
    fn no_panic_on_garbage() {
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let alphabet: Vec<char> =
            "0123456789:-/.+,TZYz年月日号时分秒点半天上午下午pmUTC ".chars().collect();
        for _ in 0..5000 {
            let mut s = String::new();
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let len = 1 + (rng >> 33) as usize % 30;
            for _ in 0..len {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.push(alphabet[(rng >> 32) as usize % alphabet.len()]);
            }
            let _ = resolve(&s);
        }
    }
}
