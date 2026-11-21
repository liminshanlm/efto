//! 自定义网页搜索：`g 关键词` 这类前缀直达搜索引擎，本地无结果时兜底。
//!
//! 判定严格：关键词与输入的第一段**整段相等**（忽略大小写），且后面必须有空白分隔的正文。
//! 单独输入 `g` 不会执行搜索，只给一条"用法提示"行（[`hint`]），避免把普通搜索劫持成网页搜索。

use serde::{Deserialize, Serialize};

use crate::search::{MatchReason, SearchHit, HitPayload};
use crate::ItemKind;

/// 一个可配置的搜索引擎。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebSearch {
    /// 触发词，不能含空白。
    pub keyword: String,
    /// 展示名。
    pub name: String,
    /// 跳转模板，必须含 `{q}`；查询正文会被百分号编码后替换进去。
    pub url: String,
}

impl WebSearch {
    pub fn url_for(&self, query: &str) -> String {
        self.url.replace("{q}", &encode(query))
    }
}

/// 默认引擎：中文环境优先百度 / Bing，另外给 GitHub 与维基百科。
pub fn default_engines() -> Vec<WebSearch> {
    vec![
        WebSearch {
            keyword: "b".into(),
            name: "百度".into(),
            url: "https://www.baidu.com/s?wd={q}".into(),
        },
        WebSearch {
            keyword: "g".into(),
            name: "Bing".into(),
            url: "https://www.bing.com/search?q={q}".into(),
        },
        WebSearch {
            keyword: "gh".into(),
            name: "GitHub".into(),
            url: "https://github.com/search?q={q}".into(),
        },
        WebSearch {
            keyword: "wiki".into(),
            name: "维基百科".into(),
            url: "https://zh.wikipedia.org/w/index.php?search={q}".into(),
        },
    ]
}

/// 收敛设置里的引擎列表：去空白、丢非法项、关键词大小写不敏感去重。
pub fn normalize_engines(items: impl IntoIterator<Item = WebSearch>) -> Vec<WebSearch> {
    let mut out: Vec<WebSearch> = Vec::new();
    for mut e in items {
        e.keyword = e.keyword.trim().to_string();
        e.name = e.name.trim().to_string();
        e.url = e.url.trim().to_string();
        if e.keyword.is_empty()
            || e.keyword.chars().any(char::is_whitespace)
            || e.name.is_empty()
            || !valid_url(&e.url)
        {
            continue;
        }
        if out
            .iter()
            .any(|x| x.keyword.eq_ignore_ascii_case(&e.keyword))
        {
            continue;
        }
        if e.name.is_empty() {
            e.name = e.keyword.clone();
        }
        out.push(e);
        if out.len() == 20 {
            break;
        }
    }
    out
}

fn valid_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && url.contains("{q}")
}

/// 解析 `关键词 正文`。没有空白分隔的正文时返回 `None`（单独输入触发词不算）。
pub fn parse<'a>(query: &str, engines: &'a [WebSearch]) -> Option<(&'a WebSearch, String)> {
    let q = query.trim();
    let (head, rest) = q.split_once(char::is_whitespace)?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    engines
        .iter()
        .find(|e| e.keyword.eq_ignore_ascii_case(head))
        .map(|e| (e, rest.to_string()))
}

/// 空正文提示行的动作名；宿主只回一句提示，不发请求。
pub const HINT_ACTION: &str = "websearch_hint";

/// 只输入触发词时的用法提示行（`g`、`wiki` 这类整串）。
///
/// 与 [`parse`] 互补：`parse` 要求"关键词 + 正文"才执行搜索，这里只认"整串等于关键词"，
/// 分数压在下面前缀匹配的应用之下，只做提示、不劫持应用搜索。
pub fn hint(query: &str, engines: &[WebSearch]) -> Option<SearchHit> {
    let q = query.trim();
    let engine = engines
        .iter()
        .find(|e| e.keyword.eq_ignore_ascii_case(q))?;
    let keyword = &engine.keyword;
    Some(
        SearchHit::new(
            ItemKind::External,
            format!("websearch:hint:{keyword}"),
            format!("用 {} 搜索…", engine.name),
            850.0,
            HitPayload::Action {
                name: HINT_ACTION.into(),
            },
        )
        .subtitle(format!(
            "网页搜索 · 在 {keyword} 后加空格和内容，例如 {keyword} rust"
        ))
        .reason(MatchReason::Exact)
        .source("websearch"),
    )
}

/// 兜底引擎：优先用户指定的关键词，找不到就退回第一个。
pub fn fallback_engine<'a>(engines: &'a [WebSearch], keyword: &str) -> Option<&'a WebSearch> {
    engines
        .iter()
        .find(|e| e.keyword.eq_ignore_ascii_case(keyword))
        .or_else(|| engines.first())
}

/// 生成一条“用某引擎搜索”的结果。`fallback` 决定分数与副标题。
pub fn hit(engine: &WebSearch, query: &str, fallback: bool) -> SearchHit {
    let brief: String = query.chars().take(40).collect();
    let url = engine.url_for(query);
    SearchHit::new(
        ItemKind::External,
        format!("websearch:{}:{query}", engine.keyword),
        format!("用 {} 搜索「{brief}」", engine.name),
        if fallback { 300.0 } else { 780.0 },
        HitPayload::WebSearch { url },
    )
    .subtitle(if fallback {
        "无本地结果 · 回车在浏览器搜索".to_string()
    } else {
        format!("网页搜索 · {}", engine.name)
    })
    .reason(MatchReason::Exact)
    .source("websearch")
}

/// 最小百分号编码：保留 RFC 3986 unreserved，其余按 UTF-8 字节编码。
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_requires_whitespace_and_body() {
        let engines = default_engines();
        assert!(parse("g", &engines).is_none());
        assert!(parse("g   ", &engines).is_none());
        assert!(parse("g rust", &engines).is_some());
        assert!(parse("G rust", &engines).is_some());
        // 前缀但关键词不完整：不触发
        assert!(parse("ghx rust", &engines).is_none());
        let (e, q) = parse("wiki 上海", &engines).unwrap();
        assert_eq!(e.keyword, "wiki");
        assert_eq!(q, "上海");
    }

    #[test]
    fn bare_keyword_gets_usage_hint() {
        let engines = default_engines();
        let h = hint("g", &engines).expect("bare keyword should hint");
        assert_eq!(h.kind, ItemKind::External);
        assert!(h.title.contains("Bing"), "{}", h.title);
        assert_eq!(h.source, "websearch");
        assert!(matches!(h.payload, HitPayload::Action { .. }));
        // 大小写不敏感、首尾空白无害
        assert!(hint("  G ", &engines).is_some());
        // 不是关键词不提示；带正文时交给 parse
        assert!(hint("gx", &engines).is_none());
        assert!(hint("g rust", &engines).is_none());
    }

    #[test]
    fn encodes_query_bytes() {
        let e = &default_engines()[0];
        let url = e.url_for("a b&c/中文");
        assert!(url.starts_with("https://www.baidu.com/s?wd="));
        assert!(url.contains("a%20b%26c%2F%E4%B8%AD%E6%96%87"));
    }

    #[test]
    fn normalizes_engines() {
        let got = normalize_engines(vec![
            WebSearch {
                keyword: " g ".into(),
                name: "Bing".into(),
                url: "https://www.bing.com/search?q={q}".into(),
            },
            WebSearch {
                keyword: "G".into(),
                name: "dup".into(),
                url: "https://x/{q}".into(),
            },
            WebSearch {
                keyword: "bad key".into(),
                name: "x".into(),
                url: "https://x/{q}".into(),
            },
            WebSearch {
                keyword: "noq".into(),
                name: "x".into(),
                url: "https://x/".into(),
            },
            WebSearch {
                keyword: "ftp".into(),
                name: "x".into(),
                url: "ftp://x/{q}".into(),
            },
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].keyword, "g");
    }

    #[test]
    fn fallback_prefers_configured_engine() {
        let engines = default_engines();
        assert_eq!(
            fallback_engine(&engines, "wiki").map(|e| e.keyword.as_str()),
            Some("wiki")
        );
        assert_eq!(
            fallback_engine(&engines, "nope").map(|e| e.keyword.as_str()),
            Some("b")
        );
        assert!(fallback_engine(&[], "g").is_none());
    }

    #[test]
    fn hit_scores_rank_keyword_above_fallback() {
        let e = &default_engines()[1];
        assert!(hit(e, "rust", false).score > hit(e, "rust", true).score);
        assert_eq!(hit(e, "rust", false).source, "websearch");
    }
}
