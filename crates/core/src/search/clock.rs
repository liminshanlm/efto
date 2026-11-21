//! 系统时间：输入 `date` / `now` / `时间` 这类关键字时，把当前系统时间按
//! 多种常用格式列出来，单击即复制（和计算器、时间戳一样的交互）。
//!
//! 设计取舍：
//! 1. **触发必须整串相等**。`date` 是关键字，但 `update`、`dated`、`date 2026-11-16`、
//!    `datetime` 都不是——那些是用户在搜应用或在算时间戳，抢过来就是劫持搜索。
//!    归一化只做全角转半角 + 小写 + trim，不做任何"包含"匹配。
//! 2. **一次查询一个时刻**。八种格式必须描述同一瞬间，否则 `17:48:59` 和
//!    `17:49:00` 会出现在同一屏上（跨秒时真实存在）。所以时刻由调用方传入，
//!    本模块内部绝不各自 `Local::now()`。
//! 3. **复制内容就是看到的字符串**（`HitPayload::CopyText`），`ItemKind::Time`
//!    复用计算器那条"单击即复制"的通路，不再新增交互类型。
//! 4. 格式集合按"抄下来就能用"选：日志/配置里最常见的 ISO 本地、Windows 斜杠、
//!    纯日期、纯时间、RFC3339（带时区偏移）、RFC1123（HTTP Date 头）、中文、
//!    文件名安全（无分隔符）。

use chrono::{DateTime, Datelike, Local, SecondsFormat, Timelike};
use serde_json::json;

use super::fuzzy::MatchReason;
use super::{HitPayload, SearchHit};
use crate::ItemKind;

/// 整串等于这些词才触发（大小写、全角、首尾空白不敏感）。
const TRIGGERS: &[&str] = &[
    "date", "now", "time", "today", "日期", "时间", "现在", "当前", "系统时间", "当前时间", "今天",
];

/// 一条格式：名字（副标题里显示，说明"这是哪种格式"）+ 渲染函数。
struct Spec {
    label: &'static str,
    render: fn(&DateTime<Local>) -> String,
}

const WEEKDAYS_CN: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
const WEEKDAYS_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const SPECS: &[Spec] = &[
    Spec {
        label: "ISO 本地",
        render: |d| d.format("%Y-%m-%d %H:%M:%S").to_string(),
    },
    Spec {
        label: "Windows 中文",
        render: |d| d.format("%Y/%m/%d %H:%M").to_string(),
    },
    Spec {
        label: "仅日期",
        render: |d| d.format("%Y-%m-%d").to_string(),
    },
    Spec {
        label: "仅时间",
        render: |d| d.format("%H:%M:%S").to_string(),
    },
    Spec {
        label: "RFC3339 带时区",
        render: |d| d.to_rfc3339_opts(SecondsFormat::Secs, false),
    },
    Spec {
        // HTTP `Date:` 头 / `Date()` 的默认形态。chrono 的 %a/%b 依赖 locale，
        // 这里自己按 RFC1123 的固定英文表拼，保证任何机器上都一致。
        label: "RFC1123 / HTTP",
        render: |d| {
            let off = d.offset().local_minus_utc();
            let sign = if off < 0 { '-' } else { '+' };
            let a = off.abs();
            format!(
                "{}, {:02} {} {} {:02}:{:02}:{:02} {}{:02}{:02}",
                WEEKDAYS_EN[d.weekday().num_days_from_sunday() as usize],
                d.day(),
                MONTHS_EN[d.month0() as usize],
                d.year(),
                d.hour(),
                d.minute(),
                d.second(),
                sign,
                a / 3600,
                (a % 3600) / 60
            )
        },
    },
    Spec {
        label: "中文",
        render: |d| {
            format!(
                "{}年{:02}月{:02}日 {:02}时{:02}分{:02}秒 {}",
                d.year(),
                d.month(),
                d.day(),
                d.hour(),
                d.minute(),
                d.second(),
                WEEKDAYS_CN[d.weekday().num_days_from_sunday() as usize]
            )
        },
    },
    Spec {
        label: "文件名安全",
        render: |d| d.format("%Y%m%d_%H%M%S").to_string(),
    },
];

/// 输入是否是"给我看系统时间"的关键字。
pub fn is_clock_query(query: &str) -> bool {
    let q = normalize(query);
    TRIGGERS.iter().any(|t| *t == q)
}

/// 生成结果行。`now` 由调用方提供，保证同一屏内所有格式指向同一瞬间。
/// 分数从 700 递减，排在时间戳（690/686）之前、应用精确匹配（≈1000）之后。
pub fn clock_hits(query: &str, now: DateTime<Local>) -> Vec<SearchHit> {
    if !is_clock_query(query) {
        return Vec::new();
    }
    let zone = zone_label(&now);
    SPECS.iter().enumerate().map(|(i, s)| {
        let text = (s.render)(&now);
        SearchHit::new(
            ItemKind::Time,
            // id 里用序号而不是中文标签：它会进 JSON / 使用记录，纯 ASCII 更稳
            format!("clock:{i}:{}", now.timestamp()),
            text.clone(),
            700.0 - i as f32,
            HitPayload::CopyText {
                text,
                echo: json!({
                    "kind": "clock",
                    "format": s.label,
                    "input": query.trim(),
                    "secs": now.timestamp(),
                    "millis": now.timestamp_millis(),
                    "zone": zone,
                }),
            },
        )
        .subtitle(format!("{} · {}", s.label, zone))
        .reason(MatchReason::Exact)
        .source("clock")
    })
    .collect()
}

/// `UTC+08:00 · Asia/Shanghai` 这种能一眼看清"是哪套时区算出来的"的标注。
fn zone_label(d: &DateTime<Local>) -> String {
    let off = d.offset().local_minus_utc();
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.abs();
    let tz = std::env::var("TZ").unwrap_or_default();
    let name = if tz.is_empty() {
        String::new()
    } else {
        format!(" · {tz}")
    };
    format!("UTC{sign}{:02}:{:02}{name}", a / 3600, (a % 3600) / 60)
}

/// 全角转半角、小写、去首尾空白。刻意不做任何"包含"或前缀匹配。
fn normalize(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for ch in src.chars() {
        let mapped = match ch {
            '０'..='９' => char::from_u32(0x30 + (ch as u32 - 0xFF10)).unwrap_or(ch),
            'Ａ'..='Ｚ' => char::from_u32(0x41 + (ch as u32 - 0xFF21)).unwrap_or(ch),
            'ａ'..='ｚ' => char::from_u32(0x61 + (ch as u32 - 0xFF41)).unwrap_or(ch),
            '　' | '\u{2009}' | '\u{202F}' | '\u{00A0}' => ' ',
            c => c,
        };
        out.push(mapped);
    }
    out.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 固定墙上时间：2026-11-16 17:48:12（本地）。用**墙上时间**构造，
    /// 这样断言的是格式渲染，不依赖机器时区算出的 epoch。
    fn fixed() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 16, 17, 48, 12).unwrap()
    }

    fn titles(query: &str) -> Vec<String> {
        clock_hits(query, fixed()).into_iter().map(|h| h.title).collect()
    }

    #[test]
    fn every_format_renders_exactly() {
        let t = titles("date");
        assert_eq!(t.len(), SPECS.len(), "八种格式都要出现");
        // 前四种与机器时区无关，可以逐字断言
        assert_eq!(t[0], "2026-11-16 17:48:12");
        assert_eq!(t[1], "2026/11/16 17:48");
        assert_eq!(t[2], "2026-11-16");
        assert_eq!(t[3], "17:48:12");
        assert_eq!(t[7], "20261116_174812");
        // 中文行的星期不依赖时区：2026-11-16 是周一
        assert!(t[6].ends_with(" 周一"), "{}", t[6]);
        assert!(t[6].starts_with("2026年11月16日 17时48分12秒"), "{}", t[6]);
        // RFC3339 / RFC1123 带偏移，机器相关，只断言形状与本地时间部分
        assert!(t[4].starts_with("2026-11-16T17:48:12"), "{}", t[4]);
        assert!(t[4].ends_with(offset_rfc3339_tail().as_str()), "尾部应是本机偏移：{}", t[4]);
        assert!(t[5].starts_with("Mon, 16 Nov 2026 17:48:12 "), "{}", t[5]);
        assert!(t[5].ends_with(offset_compact().as_str()), "{}", t[5]);
    }

    fn offset_rfc3339_tail() -> String {
        let off = fixed().offset().local_minus_utc();
        let sign = if off < 0 { '-' } else { '+' };
        let a = off.abs();
        format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60)
    }

    fn offset_compact() -> String {
        offset_rfc3339_tail().replace(':', "")
    }

    #[test]
    fn all_formats_describe_the_same_instant() {
        // 跨秒 bug 的守卫：八行必须来自同一个时刻（时刻由调用方传入，不在各自行里 now()）
        let hits = clock_hits("now", fixed());
        let secs: Vec<i64> = hits
            .iter()
            .map(|h| match &h.payload {
                HitPayload::CopyText { echo, .. } => echo["secs"].as_i64().unwrap(),
                _ => panic!("clock 行必须是 CopyText"),
            })
            .collect();
        assert!(secs.windows(2).all(|w| w[0] == w[1]), "同一屏内时刻必须一致：{secs:?}");
        assert_eq!(secs[0], fixed().timestamp());
    }

    #[test]
    fn triggers_are_exact() {
        for q in ["date", "DATE", " now ", "ｎｏｗ", "time", "Today", "时间", "日期", "现在", "当前", "系统时间", "今天"] {
            assert_eq!(titles(q).len(), SPECS.len(), "{q:?} 应触发出系统时间");
        }
        // 这些绝不能触发：包含关键字但意图是别的
        for q in [
            "update",
            "dated",
            "datetime",
            "date 2026-11-16",
            "now(){}",
            "calendar",
            "runtime",
            "timer",
            "时间戳",
            "现在几点",
            "timezone",
            "chrome",
            "",
            "   ",
        ] {
            assert!(titles(q).is_empty(), "{q:?} 不该触发系统时间行");
        }
    }

    #[test]
    fn rows_are_click_to_copy_shaped() {
        let hits = clock_hits("date", fixed());
        for h in &hits {
            assert_eq!(h.kind, ItemKind::Time, "复用计算器的单击即复制通路");
            assert_eq!(h.source, "clock");
            match &h.payload {
                HitPayload::CopyText { text, echo } => {
                    assert_eq!(text, &h.title, "所见即所复制");
                    assert_eq!(echo["kind"], json!("clock"));
                }
                other => panic!("wrong payload {other:?}"),
            }
        }
        // 分数递减 => 列表顺序 = SPECS 顺序，ISO 本地在最前
        assert!(hits[0].score > hits[1].score);
        assert_eq!(hits[0].title, "2026-11-16 17:48:12");
    }

    #[test]
    fn ordering_puts_formats_before_timestamps() {
        // clock 最低分 700-7=693 必须高于时间戳秒级 690，否则用户要的格式被压在下面
        let hits = clock_hits("now", fixed());
        assert!(hits.last().unwrap().score > 690.0);
    }
}
