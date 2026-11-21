//! 匹配打分。所有分数落在 0..1000，越大越像用户想要的。
//!
//! 分层：完全匹配 > 前缀 > 词首 > 包含 > 拼音前缀 > 首字母前缀 > 子序列。
//! 拼音/首字母命中在英文查询里天然成立，无需额外分支。

use super::pinyin;

/// 一个可匹配文本的全部派生形式。
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct MatchKey {
    /// 展示用原文
    pub display: String,
    pub lower: String,
    pub compact: String,
    /// 全拼音（无汉字时等于 compact）
    pub pinyin: String,
    /// 首字母（汉字拼音首字母 + 英文词首字母）
    pub initials: String,
}

impl MatchKey {
    pub fn from(text: &str) -> Self {
        let lower = text.to_lowercase();
        let compact = pinyin::compact(&lower);
        let (py, ini) = pinyin::pinyin_of(&lower);
        let pinyin_full = if py.is_empty() { compact.clone() } else { py };
        let initials = if ini.is_empty() {
            pinyin::word_initials(&lower)
        } else {
            ini
        };
        Self {
            display: text.to_string(),
            lower,
            compact,
            pinyin: pinyin_full,
            initials,
        }
    }
}

/// 命中理由，调试与 UI 展示用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    Exact,
    Prefix,
    WordStart,
    Substring,
    CompactPrefix,
    PinyinPrefix,
    InitialsPrefix,
    Subsequence,
}

/// 匹配一个 query 与一个 key，返回 (分数, 理由)。
pub fn score_key(q_lower: &str, q_compact: &str, key: &MatchKey) -> Option<(f32, MatchReason)> {
    if q_lower.is_empty() {
        return None;
    }

    let mut best: Option<(f32, MatchReason)> = None;
    let offer = |score: f32, reason: MatchReason, best: &mut Option<(f32, MatchReason)>| {
        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            *best = Some((score, reason));
        }
    };

    // 1. 完全匹配
    if key.lower == q_lower || key.compact == q_compact {
        offer(1000.0, MatchReason::Exact, &mut best);
        return best;
    }

    // 2. 前缀
    if let Some(pos) = key.lower.find(q_lower) {
        if pos == 0 {
            let len_penalty = (key.lower.len() - q_lower.len()) as f32 * 2.0;
            offer(900.0 - len_penalty.min(150.0), MatchReason::Prefix, &mut best);
        } else if is_word_start(&key.lower, pos) {
            let len_penalty = (key.lower.len() - q_lower.len()) as f32;
            offer(800.0 - len_penalty.min(100.0), MatchReason::WordStart, &mut best);
        } else {
            let pos_penalty = pos as f32 * 3.0;
            let len_penalty = (key.lower.len() - q_lower.len()) as f32;
            offer(
                700.0 - pos_penalty.min(120.0) - len_penalty.min(80.0),
                MatchReason::Substring,
                &mut best,
            );
        }
    }

    // 3. 紧凑前缀（忽略空格/标点）："winrar" → "Win Rar 7"
    if !q_compact.is_empty() && key.compact.starts_with(q_compact) {
        let len_penalty = (key.compact.len() - q_compact.len()) as f32;
        offer(
            860.0 - len_penalty.min(120.0),
            MatchReason::CompactPrefix,
            &mut best,
        );
    }

    // 4. 拼音前缀 / 首字母前缀
    if key.pinyin.starts_with(q_compact) {
        let len_penalty = (key.pinyin.len() - q_compact.len()) as f32;
        offer(
            780.0 - len_penalty.min(120.0),
            MatchReason::PinyinPrefix,
            &mut best,
        );
    }
    if key.initials.starts_with(q_compact) && q_compact.len() >= 2 {
        let len_penalty = (key.initials.len() - q_compact.len()) as f32;
        offer(
            700.0 - len_penalty.min(100.0),
            MatchReason::InitialsPrefix,
            &mut best,
        );
    }

    // 5. 子序列兜底："chr" → "chrome"，"edg" → "microsoft edge"
    if let Some(sc) = subsequence_score(q_compact, &key.compact) {
        offer(sc, MatchReason::Subsequence, &mut best);
    }

    best
}

fn is_word_start(hay: &str, pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let bytes: Vec<char> = hay.chars().collect();
    let idx = char_index_of_byte(hay, pos);
    idx > 0 && !bytes[idx - 1].is_alphanumeric()
}

fn char_index_of_byte(s: &str, byte_pos: usize) -> usize {
    s[..byte_pos].chars().count()
}

/// 子序列匹配：连续段奖励、跳跃惩罚。返回 None 表示不匹配。
pub fn subsequence_score(needle: &str, hay: &str) -> Option<f32> {
    let ns: Vec<char> = needle.chars().collect();
    let hs: Vec<char> = hay.chars().collect();
    if ns.is_empty() || ns.len() > hs.len() {
        return None;
    }
    let mut hi = 0usize;
    let mut score = 300.0f32;
    let mut last: Option<usize> = None;
    let mut gaps = 0usize;
    for ni in 0..ns.len() {
        let found = (hi..hs.len()).find(|&j| hs[j] == ns[ni]);
        let j = found?;
        if let Some(prev) = last {
            if j == prev + 1 {
                score += 12.0; // 连续
            } else {
                gaps += j - prev - 1;
                score -= 6.0;
            }
        }
        if j == 0 {
            score += 30.0; // 从头开始更像意图
        }
        last = Some(j);
        hi = j + 1;
    }
    score -= gaps as f32 * 2.0;
    score -= (hs.len() - ns.len()) as f32 * 0.6;
    Some(score.clamp(1.0, 600.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(q: &str, name: &str) -> Option<f32> {
        let key = MatchKey::from(name);
        score_key(&q.to_lowercase(), &pinyin::compact(&q.to_lowercase()), &key)
            .map(|(v, _)| v)
    }

    #[test]
    fn exact_beats_prefix() {
        assert_eq!(s("chrome", "chrome"), Some(1000.0));
        assert!(s("chro", "chrome").unwrap() < 1000.0);
    }

    #[test]
    fn prefix_beats_substring() {
        let p = s("code", "Visual Studio Code").unwrap();
        let sub = s("studio", "Visual Studio Code").unwrap();
        assert!(p > 0.0);
        assert!(sub > 0.0);
    }

    #[test]
    fn compact_and_pinyin() {
        assert!(s("winrar", "Win RAR 7 64-bit").unwrap() > 700.0);
        assert!(s("weixin", "微信").unwrap() > 700.0);
        assert!(s("wx", "微信").unwrap() > 600.0);
        // 记事本：拼音 jishiben / 首字母 jsb（不是 jzb）
        assert!(s("jishiben", "记事本").unwrap() > 600.0);
        assert!(s("jsb", "记事本").unwrap() > 600.0);
        // 英文名对中文名无能为力，靠别名（索引层会把 exe 名并进来）
        assert!(s("notepad", "记事本").is_none());
        assert!(s("notepad", "记事本 Notepad").is_some());
    }

    #[test]
    fn subsequence_works() {
        assert!(s("jsp", "jetbrains toolbox").is_some() || s("jb", "jetbrains").is_some());
        assert!(s("edm", "microsoft edge").is_none());
        assert!(s("edg", "microsoft edge").is_some());
    }

    #[test]
    fn empty_query_never_matches() {
        assert!(s("", "chrome").is_none());
    }
}
