//! 汉字 → 全拼 / 首字母。中文应用名（如"微信"）要能被 `wx`、`weixin` 搜到。

use pinyin::ToPinyin;

/// 返回 (全拼音串, 首字母串)。非汉字字符原样小写保留。
pub fn pinyin_of(text: &str) -> (String, String) {
    let mut full = String::with_capacity(text.len() * 2);
    let mut initials = String::with_capacity(text.len());
    let mut word_start = true;

    for c in text.chars() {
        if let Some(py) = c.to_pinyin() {
            let plain = py.plain();
            full.push_str(plain);
            if let Some(first) = plain.chars().next() {
                initials.push(first);
            }
            // 汉字之间不算词首，除了首字
            word_start = false;
        } else if c.is_alphanumeric() {
            let lc = c.to_ascii_lowercase();
            full.push(lc);
            if word_start {
                initials.push(lc);
            }
            word_start = false;
        } else {
            word_start = true;
        }
    }
    (full, initials)
}

/// 只保留字母数字并小写，用于"紧凑匹配"（`winrar` 命中 `Win RAR 7`）。
pub fn compact(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

/// 词首字母（英文场景）：`Microsoft Edge` → `me`。
pub fn word_initials(text: &str) -> String {
    let mut out = String::new();
    let mut word_start = true;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if word_start {
                out.push(c.to_ascii_lowercase());
            }
            word_start = false;
        } else {
            word_start = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_pinyin() {
        let (full, ini) = pinyin_of("微信");
        assert_eq!(full, "weixin");
        assert_eq!(ini, "wx");
    }

    #[test]
    fn mixed_cjk_ascii() {
        let (full, ini) = pinyin_of("钉钉DingTalk");
        assert!(full.starts_with("dingding"), "full={full}");
        assert!(ini.starts_with("dd"), "ini={ini}");
    }

    #[test]
    fn english_words() {
        assert_eq!(word_initials("Microsoft Edge"), "me");
        assert_eq!(compact("Win RAR 7 64-bit"), "winrar764bit");
        assert_eq!(pinyin_of("Notepad++").0, "notepad");
    }
}
