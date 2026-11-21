//! Windows 系统设置页（`ms-settings:`）搜索源。
//!
//! 整串以 `ms-settings:` 开头才触发，不做普通包含匹配，避免日常输入被劫持。
//! 冒号后为空时列出常用设置页；有内容时按 URI / 标题 / 别名过滤；
//! 列表里没有的 URI 仍给一条"打开"行，交给系统外壳判断。

use super::fuzzy::MatchReason;
use super::{HitPayload, SearchHit};
use crate::ItemKind;

pub const PREFIX: &str = "ms-settings:";

/// (URI 后缀, 标题, 别名)。顺序即空查询时的展示顺序，常用的放前面。
const PAGES: &[(&str, &str, &[&str])] = &[
    ("display", "显示", &["display", "monitor"]),
    ("bluetooth", "蓝牙和其他设备", &["bluetooth", "devices"]),
    ("network", "网络和 Internet", &["network", "internet"]),
    ("network-wifi", "Wi-Fi", &["wifi", "wireless"]),
    ("network-ethernet", "以太网", &["ethernet"]),
    ("network-vpn", "VPN", &["vpn"]),
    ("network-proxy", "代理", &["proxy"]),
    ("network-airplanemode", "飞行模式", &["airplane mode"]),
    ("network-mobilehotspot", "移动热点", &["mobile hotspot", "hotspot"]),
    ("windowsupdate", "Windows 更新", &["windows update", "update"]),
    ("appsfeatures", "应用和功能", &["apps", "installed apps"]),
    ("defaultapps", "默认应用", &["default apps"]),
    ("startupapps", "启动应用", &["startup"]),
    ("optionalfeatures", "可选功能", &["optional features"]),
    ("personalization", "个性化", &["personalization"]),
    ("personalization-background", "背景", &["background", "wallpaper"]),
    ("personalization-colors", "颜色", &["colors", "accent"]),
    ("lockscreen", "锁屏界面", &["lock screen"]),
    ("themes", "主题", &["themes"]),
    ("taskbar", "任务栏", &["taskbar"]),
    ("start", "开始", &["start menu"]),
    ("fonts", "字体", &["fonts"]),
    ("sounds", "声音", &["sound", "audio"]),
    ("notifications", "通知", &["notifications"]),
    ("focusassist", "专注助手", &["focus assist", "quiet hours"]),
    ("battery", "电源和电池", &["battery", "power"]),
    ("powersleep", "电源和睡眠", &["power", "sleep"]),
    ("batterysaver", "节电模式", &["battery saver"]),
    ("storage", "存储", &["storage"]),
    ("storagesense", "存储感知", &["storage sense"]),
    ("printers", "打印机和扫描仪", &["printers", "scanners"]),
    ("connecteddevices", "已连接的设备", &["connected devices"]),
    ("mouse", "鼠标", &["mouse"]),
    ("devices-touchpad", "触摸板", &["touchpad"]),
    ("pen", "笔和 Windows Ink", &["pen", "ink"]),
    ("typing", "输入", &["typing", "input"]),
    ("usb", "USB", &["usb"]),
    ("dateandtime", "日期和时间", &["date", "time"]),
    ("language", "语言和区域", &["language", "region"]),
    ("region", "区域", &["region"]),
    ("keyboard", "键盘", &["keyboard"]),
    ("speech", "语音", &["speech", "voice"]),
    ("privacy", "隐私和安全性", &["privacy", "security"]),
    ("privacy-location", "定位", &["location"]),
    ("privacy-camera", "相机", &["camera"]),
    ("privacy-microphone", "麦克风", &["microphone", "mic"]),
    ("privacy-speech", "语音", &["speech"]),
    ("privacy-activity", "活动历史记录", &["activity history"]),
    ("privacy-accountinfo", "账户信息", &["account info"]),
    ("privacy-feedback", "反馈和诊断", &["feedback", "diagnostics"]),
    ("privacy-notifications", "通知", &["notifications"]),
    ("privacy-backgroundapps", "后台应用", &["background apps"]),
    ("privacy-general", "常规", &["general"]),
    ("accounts", "账户", &["accounts"]),
    ("yourinfo", "账户信息", &["your info"]),
    ("emailandaccounts", "电子邮件和账户", &["email", "accounts"]),
    ("signinoptions", "登录选项", &["sign-in options", "login"]),
    ("otherusers", "其他用户", &["other users"]),
    ("sync", "同步你的设置", &["sync"]),
    ("gaming-gamebar", "游戏栏", &["game bar"]),
    ("gaming-gamemode", "游戏模式", &["game mode"]),
    ("gaming-captures", "捕获", &["captures"]),
    ("easeofaccess", "辅助功能", &["accessibility"]),
    ("easeofaccess-text", "文本大小", &["text size"]),
    ("easeofaccess-magnifier", "放大镜", &["magnifier"]),
    ("easeofaccess-colorfilter", "颜色滤镜", &["color filter"]),
    ("easeofaccess-narrator", "讲述人", &["narrator"]),
    ("easeofaccess-audio", "音频", &["audio"]),
    ("easeofaccess-keyboard", "键盘", &["keyboard"]),
    ("easeofaccess-mouse", "鼠标指针", &["mouse pointer"]),
    ("search", "搜索", &["search"]),
    ("multitasking", "多任务处理", &["multitasking"]),
    ("about", "关于", &["about"]),
    ("system", "系统", &["system"]),
    ("recovery", "恢复", &["recovery"]),
    ("activation", "激活", &["activation"]),
    ("findmydevice", "查找我的设备", &["find my device"]),
    ("crossdevice", "共享体验", &["cross device", "shared experiences"]),
    ("maps", "离线地图", &["maps"]),
    ("manageappaliases", "应用别名", &["app aliases"]),
    ("windowsdefender", "Windows 安全中心", &["windows security", "defender"]),
    ("troubleshoot", "疑难解答", &["troubleshoot"]),
    ("developers", "开发者选项", &["developer", "developers"]),
    ("remotedesktop", "远程桌面", &["remote desktop"]),
    ("backup", "备份", &["backup"]),
    ("delivery-optimization", "传递优化", &["delivery optimization"]),
];

/// 输入是否属于 `ms-settings:` 源。
pub fn is_query(query: &str) -> bool {
    query.trim().to_ascii_lowercase().starts_with(PREFIX)
}

/// 生成设置页结果。空后缀列出常用页，未知 URI 保留一条"打开"行。
pub fn hits(query: &str, limit: usize) -> Vec<SearchHit> {
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
        // 蓝图里没收录的页面也要能打开：把整串交给 shell。
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
    fn empty_suffix_lists_common_pages() {
        let hits = hits("ms-settings:", 100);
        assert!(hits.len() > 20, "{hits:?}");
        assert_eq!(hits[0].title, "显示");
        assert_eq!(hits[0].source, "settings");
        assert!(hits.iter().all(|h| match &h.payload {
            HitPayload::Open { path } => path.starts_with(PREFIX),
            other => panic!("设置页必须是 Open：{other:?}"),
        }));
    }

    #[test]
    fn exact_page_opens_directly() {
        let hits = hits("ms-settings:bluetooth", 20);
        assert_eq!(hits[0].title, "蓝牙和其他设备");
        assert_eq!(
            hits[0].payload,
            HitPayload::Open {
                path: "ms-settings:bluetooth".into()
            }
        );
    }

    #[test]
    fn partial_suffix_filters_list() {
        let hits = hits("ms-settings:blue", 20);
        assert!(hits.iter().any(|h| h.title == "蓝牙和其他设备"), "{hits:?}");
        assert!(hits.iter().all(|h| h.title != "显示"), "{hits:?}");
    }

    #[test]
    fn unknown_page_still_offers_to_open() {
        let hits = hits("ms-settings:not-a-real-page", 20);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(
            hits[0].payload,
            HitPayload::Open {
                path: "ms-settings:not-a-real-page".into()
            }
        );
    }

    #[test]
    fn ordinary_input_never_triggers() {
        for q in ["display", "设置", "ms-settings", "ms settings:display"] {
            assert!(hits(q, 20).is_empty(), "{q:?} 不该出设置页");
        }
    }
}
