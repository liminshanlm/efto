//! macOS 系统设置搜索源（`x-apple.systempreferences:`）。
//!
//! 结构与 [`super::win_settings`]（Windows 的 `ms-settings:`）对齐：
//! 整串以前缀开头才触发，不做普通包含匹配，避免日常输入被劫持（硬性约定 2）；
//! 冒号后为空时列出常用设置页；有内容时按 URI / 标题 / 别名过滤；
//! 列表里没有的 URI 仍给一条"打开"行，交给系统外壳判断。
//!
//! 平台：仅 macOS。其它平台上 [`is_query`] 恒 false、[`hits`] 恒空——
//! 设置页 URI 在别的平台是打不开的结果（宁可不出，不可乱出）。
//!
//! 页面 id 面向 macOS 13+（`com.apple.preference.*` 老 id 与 `*-Settings.extension`
//! 新 id 混用）；个别 id 在旧系统上可能落到设置首页，不影响"打开"语义。

use super::fuzzy::MatchReason;
use super::{HitPayload, SearchHit};
use crate::ItemKind;

pub const PREFIX: &str = "x-apple.systempreferences:";

/// (URI 后缀, 标题, 别名)。顺序即空查询时的展示顺序，常用的放前面。
const PAGES: &[(&str, &str, &[&str])] = &[
    ("com.apple.WiFiSettings", "无线局域网", &["wifi", "wi-fi", "wireless", "网络"]),
    ("com.apple.Network-Settings.extension", "网络", &["network", "internet"]),
    ("com.apple.BluetoothSettings", "蓝牙", &["bluetooth", "devices", "设备"]),
    (
        "com.apple.Displays-Settings.extension",
        "显示器",
        &["display", "monitor", "屏幕"],
    ),
    ("com.apple.Sound-Settings.extension", "声音", &["sound", "audio", "音频"]),
    (
        "com.apple.Notifications-Settings.extension",
        "通知",
        &["notifications", "通知"],
    ),
    ("com.apple.Focus-Settings.extension", "专注模式", &["focus", "勿扰", "dnd"]),
    (
        "com.apple.ControlCenter-Settings.extension",
        "控制中心",
        &["control center"],
    ),
    ("com.apple.Appearance-Settings.extension", "外观", &["appearance", "外观"]),
    (
        "com.apple.preference.dock",
        "桌面与程序坞",
        &["desktop", "dock", "桌面"],
    ),
    ("com.apple.preference.wallpaper", "墙纸", &["wallpaper", "壁纸"]),
    (
        "com.apple.preference.screensaver",
        "屏幕保护程序",
        &["screensaver", "屏保"],
    ),
    (
        "com.apple.Battery-Settings.extension",
        "电池",
        &["battery", "power", "电源"],
    ),
    ("com.apple.preference.datetime", "日期与时间", &["date", "time", "时间"]),
    (
        "com.apple.Localization-Settings.extension",
        "语言与地区",
        &["language", "region", "语言"],
    ),
    ("com.apple.preference.keyboard", "键盘", &["keyboard", "input", "输入"]),
    ("com.apple.preference.trackpad", "触控板", &["trackpad"]),
    ("com.apple.preference.mouse", "鼠标", &["mouse"]),
    (
        "com.apple.preference.printers",
        "打印机与扫描仪",
        &["printers", "scanners", "打印机"],
    ),
    (
        "com.apple.preference.users",
        "用户与群组",
        &["users", "accounts", "账户"],
    ),
    (
        "com.apple.settings.Storage",
        "储存空间",
        &["storage", "储存", "磁盘"],
    ),
    (
        "com.apple.Software-Update-Settings.extension",
        "软件更新",
        &["software update", "update", "更新"],
    ),
    (
        "com.apple.preference.security",
        "隐私与安全性",
        &["privacy", "security", "隐私", "安全"],
    ),
    (
        "com.apple.preference.security?Privacy_Accessibility",
        "隐私与安全性 · 辅助功能",
        &["accessibility", "辅助功能", "权限"],
    ),
    (
        "com.apple.preference.security?Privacy_ScreenCapture",
        "隐私与安全性 · 屏幕录制",
        &["screen recording", "屏幕录制", "截图权限"],
    ),
    (
        "com.apple.preference.security?Privacy_AllFiles",
        "隐私与安全性 · 完全磁盘访问",
        &["full disk access", "完全磁盘访问"],
    ),
    ("com.apple.preference.general", "通用", &["general", "通用"]),
    (
        "com.apple.Siri-Settings.extension",
        "Siri 与聚焦",
        &["siri", "spotlight", "聚焦"],
    ),
    ("com.apple.preference.dock?MenuBar", "菜单栏", &["menu bar", "menubar"]),
];

/// 输入是否属于本源。
pub fn is_query(query: &str) -> bool {
    cfg!(target_os = "macos") && query.trim().to_ascii_lowercase().starts_with(PREFIX)
}

/// 生成设置页结果。空后缀列出常用页，未知 URI 保留一条"打开"行。
pub fn hits(query: &str, limit: usize) -> Vec<SearchHit> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    hits_impl(query, limit)
}

fn hits_impl(query: &str, limit: usize) -> Vec<SearchHit> {
    let q = query.trim();
    let lower = q.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix(PREFIX) else {
        return Vec::new();
    };
    let rest = rest.trim();
    let limit = limit.max(1);

    if rest.is_empty() {
        return PAGES
            .iter()
            .take(limit)
            .enumerate()
            .map(|(i, page)| hit(page, 700.0 - i as f32 * 0.1, MatchReason::Prefix))
            .collect();
    }

    let mut out: Vec<SearchHit> = Vec::new();
    for (i, page) in PAGES.iter().enumerate() {
        let (id, title, aliases) = *page;
        let title_lower = title.to_lowercase();
        let score = if id == rest {
            900.0
        } else if id.starts_with(rest) {
            820.0 - i as f32 * 0.05
        } else if id.contains(rest) {
            760.0
        } else if title_lower.contains(rest)
            || aliases.iter().any(|a| a.to_lowercase().contains(rest))
        {
            720.0
        } else {
            continue;
        };
        let reason = if id == rest {
            MatchReason::Exact
        } else if id.starts_with(rest) {
            MatchReason::Prefix
        } else {
            MatchReason::Substring
        };
        out.push(hit(page, score, reason));
    }

    if out.is_empty() {
        // 表里没收录的页面也要能打开：把整串交给系统外壳判断。
        out.push(
            SearchHit::new(
                ItemKind::External,
                format!("uri:{q}"),
                format!("打开  {q}"),
                650.0,
                HitPayload::Open {
                    path: q.to_string(),
                },
            )
            .subtitle("系统设置")
            .reason(MatchReason::Substring)
            .source("settings"),
        );
    }

    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(limit);
    out
}

fn hit(
    page: &(&'static str, &'static str, &'static [&'static str]),
    score: f32,
    reason: MatchReason,
) -> SearchHit {
    let (id, title, _) = *page;
    let uri = format!("{PREFIX}{id}");
    SearchHit::new(
        ItemKind::External,
        format!("uri:{uri}"),
        title,
        score,
        HitPayload::Open { path: uri.clone() },
    )
    .subtitle(format!("系统设置 · {uri}"))
    .reason(reason)
    .source("settings")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_trigger_is_platform_gated() {
        let q = format!("{PREFIX}com.apple.preference.keyboard");
        if cfg!(target_os = "macos") {
            assert!(is_query(&q));
        } else {
            assert!(!is_query(&q), "非 macOS 不得触发系统设置源");
        }
    }

    /// 反例（硬性约定 2）：普通输入与缺前缀的输入一律不触发。
    #[test]
    fn normal_input_does_not_trigger() {
        for q in ["显示", "settings", "systempreferences", "x-apple", ""] {
            assert!(!is_query(q), "{q:?} 不该触发");
        }
    }

    #[test]
    fn empty_suffix_lists_common_pages() {
        let hits = hits_impl(PREFIX, 20);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.source == "settings"));
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score), "分数递减");
    }

    #[test]
    fn full_uri_hits_exactly() {
        let hits = hits_impl(&format!("{PREFIX}com.apple.preference.keyboard"), 10);
        assert!(hits.iter().any(|h| {
            h.payload == HitPayload::Open {
                path: format!("{PREFIX}com.apple.preference.keyboard"),
            }
        }));
    }

    #[test]
    fn alias_filters_and_unknown_still_opens() {
        let hits = hits_impl(&format!("{PREFIX}蓝牙"), 10);
        assert!(hits.iter().any(|h| h.source == "settings"));

        let hits = hits_impl(&format!("{PREFIX}com.apple.not.a.real.pane"), 10);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].title.starts_with("打开"), "{}", hits[0].title);
    }

    /// 反例（硬性约定 2）：非 macOS 上 hits 必须恒空——不给出打不开的结果。
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn hits_empty_off_macos() {
        assert!(hits(PREFIX, 20).is_empty());
    }
}
