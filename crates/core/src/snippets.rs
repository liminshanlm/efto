//! 片段（Snippets）：`~/.efto/snippets.json`。
//!
//! 触发方式是"关键词精确匹配"：搜索框输入完整关键词时给出片段，
//! 回车把展开后的正文粘贴回唤起前的窗口。不做系统级逐字文本扩展。
//! 只做数据与展开逻辑，设置页与粘贴在宿主侧。

use std::path::PathBuf;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::{fsutil, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    #[serde(default)]
    pub id: String,
    /// 触发关键词：不含空白，大小写不敏感，全库唯一。
    pub keyword: String,
    /// 展示名，留空时用 keyword。
    #[serde(default)]
    pub title: String,
    /// 展开后的正文，支持 `{date}` / `{time}` / `{datetime}`。
    pub body: String,
}

impl Snippet {
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.keyword
        } else {
            &self.title
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SnippetFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    items: Vec<Snippet>,
}

#[derive(Debug, Default, Clone)]
pub struct SnippetStore {
    items: Vec<Snippet>,
}

impl SnippetStore {
    pub fn path() -> PathBuf {
        fsutil::data_root().join("snippets.json")
    }

    pub fn load() -> Self {
        match fsutil::read_to_string_opt(&Self::path()) {
            None => Self::default(),
            Some(s) => match Self::from_json(&s) {
                Ok(store) => store,
                Err(e) => {
                    tracing::warn!("snippets.json 解析失败({e})，忽略片段");
                    Self::default()
                }
            },
        }
    }

    pub fn from_json(raw: &str) -> Result<Self> {
        let file: SnippetFile = serde_json::from_str(raw)?;
        Ok(Self { items: file.items })
    }

    pub fn to_json(&self) -> Result<String> {
        let file = SnippetFile {
            version: 1,
            items: self.items.clone(),
        };
        Ok(serde_json::to_string(&file)?)
    }

    pub fn save(&self) -> Result<()> {
        fsutil::write_atomic(&Self::path(), self.to_json()?.as_bytes())
    }

    /// 整体替换（设置页保存）。空白项会被拒绝，关键词重复会报错。
    pub fn replace_all(&mut self, items: Vec<Snippet>) -> std::result::Result<(), String> {
        let mut cleaned = Vec::with_capacity(items.len());
        let mut seen = std::collections::HashSet::new();
        for (idx, mut item) in items.into_iter().enumerate() {
            let keyword = item.keyword.trim().to_string();
            if keyword.is_empty() {
                return Err(format!("第 {} 个片段缺少关键词", idx + 1));
            }
            if keyword.chars().any(char::is_whitespace) {
                return Err(format!("关键词「{keyword}」不能包含空格"));
            }
            if item.body.trim().is_empty() {
                return Err(format!("片段「{keyword}」正文为空"));
            }
            if !seen.insert(keyword.to_lowercase()) {
                return Err(format!("关键词「{keyword}」重复"));
            }
            if item.id.trim().is_empty() {
                item.id = format!("snippet-{}", idx + 1);
            }
            item.keyword = keyword;
            item.title = item.title.trim().to_string();
            cleaned.push(item);
        }
        self.items = cleaned;
        Ok(())
    }

    /// 关键词精确匹配（去除首尾空白、大小写不敏感），返回展开后的正文。
    pub fn matches(&self, query: &str, now: DateTime<Local>) -> Vec<(Snippet, String)> {
        let q = query.trim();
        if q.is_empty() {
            return Vec::new();
        }
        self.items
            .iter()
            .filter(|s| s.keyword.eq_ignore_ascii_case(q))
            .map(|s| (s.clone(), expand(&s.body, now)))
            .collect()
    }

    pub fn items(&self) -> &[Snippet] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// 展开占位符。只做本地时间相关的替换，不触碰剪贴板等宿主能力。
pub fn expand(body: &str, now: DateTime<Local>) -> String {
    body.replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H:%M:%S").to_string())
        .replace("{datetime}", &now.format("%Y-%m-%d %H:%M:%S").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(ts: i64) -> DateTime<Local> {
        Local.timestamp_opt(ts, 0).single().expect("valid timestamp")
    }

    #[test]
    fn only_exact_keyword_triggers() {
        let mut s = SnippetStore::default();
        s.replace_all(vec![Snippet {
            id: String::new(),
            keyword: "sig".into(),
            title: "签名".into(),
            body: "Best regards".into(),
        }])
        .unwrap();
        let now = at(0);
        assert_eq!(s.matches("sig", now).len(), 1);
        assert_eq!(s.matches(" SIG ", now).len(), 1);
        assert!(s.matches("si", now).is_empty(), "前缀不能触发");
        assert!(s.matches("sig extra", now).is_empty(), "多余输入不能触发");
        assert!(s.matches("signature", now).is_empty());
    }

    #[test]
    fn expand_placeholders() {
        let now = Local.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let out = expand("{date} {time} {datetime}", now);
        assert!(out.starts_with("2023-11-1"));
        assert!(out.contains(':'));
        assert!(!out.contains('{'));
    }

    #[test]
    fn replace_all_validates() {
        let mut s = SnippetStore::default();
        let bad = |keyword: &str, body: &str| Snippet {
            id: String::new(),
            keyword: keyword.into(),
            title: String::new(),
            body: body.into(),
        };
        assert!(s.replace_all(vec![bad("", "x")]).is_err());
        assert!(s.replace_all(vec![bad("a b", "x")]).is_err());
        assert!(s.replace_all(vec![bad("a", "  ")]).is_err());
        assert!(s.replace_all(vec![bad("a", "x"), bad("A", "y")]).is_err());
        s.replace_all(vec![bad("a", "x")]).unwrap();
        assert_eq!(s.items()[0].id, "snippet-1");
    }

    #[test]
    fn json_roundtrip() {
        let mut s = SnippetStore::default();
        s.replace_all(vec![Snippet {
            id: String::new(),
            keyword: "addr".into(),
            title: "地址".into(),
            body: "上海市".into(),
        }])
        .unwrap();
        let raw = s.to_json().unwrap();
        let back = SnippetStore::from_json(&raw).unwrap();
        assert_eq!(back.items(), s.items());
    }
}
