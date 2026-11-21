//! 宿主侧文案（托盘、窗口标题）与语言解析。
//!
//! 词典与前端共用同一份 `frontend/locales/<tag>.json`，构建时嵌入二进制，
//! 避免页面上和托盘里出现两套翻译、各自漂移。

use std::collections::BTreeMap;

use efto_core::config::{Language, Settings};
use once_cell::sync::Lazy;

const ZH_CN: &str = include_str!("../../../frontend/locales/zh-CN.json");
const EN_US: &str = include_str!("../../../frontend/locales/en-US.json");

type Dict = BTreeMap<String, String>;

static DICTS: Lazy<(Dict, Dict)> = Lazy::new(|| (parse(ZH_CN), parse(EN_US)));

fn parse(src: &str) -> Dict {
    serde_json::from_str(src).unwrap_or_else(|e| panic!("locales 词典不是合法 JSON: {e}"))
}

/// 设置 + 系统语言 → 实际使用的语言代码（`zh-CN` / `en-US`）。
pub fn resolve(settings: &Settings) -> &'static str {
    match settings.language {
        Language::ZhCn => "zh-CN",
        Language::EnUs => "en-US",
        Language::Auto => {
            if system_is_chinese() {
                "zh-CN"
            } else {
                "en-US"
            }
        }
    }
}

/// 系统首选语言是否是中文。
///
/// 环境变量优先（CLI/测试可控），Windows 上再询问系统首选 UI 语言；
/// 其它平台没有可靠信号时按英文处理（与 `auto` 的"非中文回退英文"一致）。
pub fn system_is_chinese() -> bool {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return v.to_lowercase().starts_with("zh");
            }
        }
    }
    #[cfg(windows)]
    {
        // LANGID 的低 10 位是主语言 ID；0x04 = LANG_CHINESE
        let langid = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        return (langid & 0x03ff) == 0x04;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 取一条文案；缺失时回退中文，再缺失时返回 key（并记录一次日志）。
/// 不做键位装饰的 key：讲"热键录入语法"的文案必须保持 Tauri 语法原样
/// （存储/录入值就是 `Alt`/`Ctrl`），不能被翻译成平台叫法。
const NO_KEY_DECOR: &[&str] = &["settings.hotkey.desc"];

/// 平台键位叫法装饰：把 `Alt`/`Ctrl`/`Super`/`CmdOrCtrl` 换成 macOS 的
/// `Option`/`Control`/`Command`。只作用于展示文案，按**非字母数字段**切词，
/// 不会误伤 `Altitude` 这类普通词。与前端 i18n.js 的 decorateKeys 同规则
/// （词典共用、出口各装饰一次；改一侧对齐另一侧）。
fn decorate_keys(s: &str) -> String {
    if cfg!(windows) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for piece in s.split_inclusive(|c: char| !c.is_alphanumeric()) {
        let (word, tail) = match piece.char_indices().find(|(_, c)| !c.is_alphanumeric()) {
            Some((i, _)) => (&piece[..i], &piece[i..]),
            None => (piece, ""),
        };
        let word = match word {
            "CmdOrCtrl" => "Command",
            "Ctrl" => "Control",
            "Alt" => "Option",
            "Super" => "Command",
            _ => word,
        };
        out.push_str(word);
        out.push_str(tail);
    }
    out
}

pub fn tr(lang: &str, key: &str) -> String {
    let (zh, en) = &*DICTS;
    let primary = if lang.eq_ignore_ascii_case("en-US") { en } else { zh };
    let v = primary
        .get(key)
        .or_else(|| {
            if zh.contains_key(key) {
                tracing::warn!("i18n 词典缺少 key: {key}（语言 {lang}），已回退中文");
            } else {
                tracing::warn!("i18n 词典缺少 key: {key}");
            }
            zh.get(key)
        })
        .cloned()
        .unwrap_or_else(|| key.to_string());
    if NO_KEY_DECOR.contains(&key) {
        v
    } else {
        decorate_keys(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decorate_keys_rewrites_modifiers_by_word() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(decorate_keys("efto · Alt+Space 唤起"), "efto · Option+Space 唤起");
        assert_eq!(decorate_keys("Ctrl+Alt+1…7"), "Control+Option+1…7");
        // 非字母数字段之外的普通词不能误伤
        assert_eq!(decorate_keys("Altitude Altitude"), "Altitude Altitude");
        // CmdOrCtrl 必须整词先于 Ctrl 处理，不能变成 CmdOrControl
        assert_eq!(decorate_keys("CmdOrCtrl+S"), "Command+S");
    }

    #[test]
    fn hotkey_syntax_key_is_not_decorated() {
        let zh = tr("zh-CN", "settings.hotkey.desc");
        assert!(zh.contains("Alt+Space"), "语法说明必须保持 Tauri 语法: {zh}");
    }

    #[test]
    fn dictionaries_have_identical_keys() {
        let (zh, en) = &*DICTS;
        for k in zh.keys() {
            assert!(en.contains_key(k), "en-US 缺少 key: {k}");
        }
        for k in en.keys() {
            assert!(zh.contains_key(k), "zh-CN 缺少 key: {k}");
        }
    }

    #[test]
    fn tray_and_window_keys_exist() {
        for k in [
            "tray.show",
            "tray.settings",
            "tray.help",
            "tray.quit",
            "tray.tooltip",
            "window.settings",
            "window.clipboard",
            "window.help",
        ] {
            assert!(!tr("zh-CN", k).is_empty());
            assert!(!tr("en-US", k).is_empty());
        }
    }

    #[test]
    fn missing_key_falls_back_to_itself() {
        assert_eq!(tr("zh-CN", "no.such.key"), "no.such.key");
    }
}
