//! 归一化匹配文本 + 搜索结果模型 + 输入直连（路径/URL）。

pub mod calc;
pub mod clock;
pub mod command;
#[cfg(windows)]
pub mod everything;
pub mod fuzzy;
pub mod pinyin;
pub mod service;
pub mod timestamp;
#[cfg(windows)]
pub mod win_settings;

pub use calc::{evaluate as evaluate_expression, format_number};
pub use clock::{clock_hits, is_clock_query};
#[cfg(windows)]
pub use everything::hits as everything_hits;
pub use fuzzy::{MatchKey, MatchReason};
pub use service::{merge_hits, BuiltinAction, DefaultPanel, SearchService};
pub use timestamp::{resolve as resolve_time, time_hits, TimeStamp};

use serde::{Deserialize, Serialize};

use crate::apps::model::LaunchTarget;
use crate::ItemKind;

/// 一条记录的全部可匹配文本（主名 + 别名）。派生数据，不入库，重建索引时生成。
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct SearchText {
    keys: Vec<MatchKey>,
}

impl SearchText {
    pub fn build<S: AsRef<str>>(texts: &[S]) -> Self {
        let mut keys: Vec<MatchKey> = Vec::with_capacity(texts.len());
        let mut seen: Vec<String> = Vec::with_capacity(texts.len());
        for t in texts {
            let t = t.as_ref();
            if t.trim().is_empty() {
                continue;
            }
            let low = t.to_lowercase();
            if seen.iter().any(|s| s == &low) {
                continue;
            }
            seen.push(low);
            keys.push(MatchKey::from(t));
        }
        Self { keys }
    }

    pub fn best(&self, query: &str) -> Option<(f32, MatchReason)> {
        if query.trim().is_empty() {
            return None;
        }
        let q = query.trim().to_lowercase();
        let qc = pinyin::compact(&q);
        let mut best: Option<(f32, MatchReason)> = None;
        for (i, k) in self.keys.iter().enumerate() {
            if let Some((sc, reason)) = fuzzy::score_key(&q, &qc, k) {
                // 主名（第 0 个 key）权重略高于别名
                let weight = if i == 0 { 1.0 } else { 0.9 };
                let sc = sc * weight;
                if best.as_ref().map(|(s, _)| sc > *s).unwrap_or(true) {
                    best = Some((sc, reason));
                }
            }
        }
        best
    }

    pub fn keys(&self) -> &[MatchKey] {
        &self.keys
    }
}

/// 命中后要让宿主做什么。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HitPayload {
    /// 启动应用/快捷方式/URI
    Launch { target: LaunchTarget },
    /// 唤起插件（main 入口 + cmd 关键字）
    Plugin {
        plugin_id: String,
        cmd: String,
        feature: String,
    },
    /// 内置动作
    Action { name: String },
    /// 直接打开外部目标
    Open { path: String },
    /// 把文本复制到剪贴板（计算器结果）。`echo` 是给 UI 展示的结构化数据。
    CopyText { text: String, echo: serde_json::Value },
    /// 搜索框命令执行（`>` 前缀）。正文由宿主交给 [`crate::exec::run_command`]。
    RunCommand { command: String },
    /// 片段：`text` 是已展开占位符的正文，宿主复制并粘贴回前台窗口。
    Snippet { id: String, text: String },
    /// 网页搜索：`url` 已经完成查询编码，宿主直接用浏览器打开。
    WebSearch { url: String },
    /// 系统命令：`action` 取 [`crate::system::SystemAction::id`]。
    SystemAction { action: String },
}

/// 前端唯一需要的结果结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub kind: ItemKind,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub score: f32,
    /// 命中方式（调试用，前端可忽略）
    pub reason: String,
    pub icon: Option<String>,
    /// 来源标签：start_menu / registry / dir_scan / plugin:<id> / action
    pub source: String,
    /// 默认面板分组（`recent` / `tools`），普通搜索结果不带该字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 常用工具的原始条目（`app:...` / `plugin:...` / 内置 id），供前端"移除"用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub payload: HitPayload,
}

impl SearchHit {
    pub fn new(
        kind: ItemKind,
        id: impl Into<String>,
        title: impl Into<String>,
        score: f32,
        payload: HitPayload,
    ) -> Self {
        Self {
            kind,
            id: id.into(),
            title: title.into(),
            subtitle: String::new(),
            score,
            reason: String::new(),
            icon: None,
            source: String::new(),
            group: None,
            pin: None,
            payload,
        }
    }

    pub fn subtitle(mut self, s: impl Into<String>) -> Self {
        self.subtitle = s.into();
        self
    }
    pub fn reason(mut self, r: MatchReason) -> Self {
        self.reason = match r {
            MatchReason::Exact => "exact",
            MatchReason::Prefix => "prefix",
            MatchReason::WordStart => "word_start",
            MatchReason::Substring => "substring",
            MatchReason::CompactPrefix => "compact_prefix",
            MatchReason::PinyinPrefix => "pinyin_prefix",
            MatchReason::InitialsPrefix => "initials",
            MatchReason::Subsequence => "subsequence",
        }
        .to_string();
        self
    }
    pub fn icon(mut self, i: Option<String>) -> Self {
        self.icon = i;
        self
    }
    pub fn source(mut self, s: impl Into<String>) -> Self {
        self.source = s.into();
        self
    }
    pub fn group(mut self, g: impl Into<String>) -> Self {
        self.group = Some(g.into());
        self
    }
    pub fn pin(mut self, p: impl Into<String>) -> Self {
        self.pin = Some(p.into());
        self
    }
}

/// 用户输入直接就是 URL / 绝对路径时，给出"打开"项。
pub fn external_hit(query: &str) -> Option<SearchHit> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let lower = q.to_ascii_lowercase();

    // URL
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || (lower.starts_with("www.") && !lower.contains(' '))
        || (lower.contains("://"))
    {
        return Some(
            SearchHit::new(
                ItemKind::External,
                format!("url:{q}"),
                format!("在浏览器打开  {q}"),
                620.0,
                HitPayload::Open { path: q.to_string() },
            )
            .subtitle("URL")
            .source("url"),
        );
    }

    // ms-settings: 等协议
    if let Some(idx) = lower.find(':') {
        let scheme = &q[..idx];
        if scheme.len() >= 2
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '+')
            && !matches!(scheme.chars().next(), Some(c) if c.is_ascii_alphabetic() && idx == 1)
            && q[idx + 1..].starts_with("//")
        {
            return Some(
                SearchHit::new(
                    ItemKind::External,
                    format!("uri:{q}"),
                    format!("打开  {q}"),
                    600.0,
                    HitPayload::Open { path: q.to_string() },
                )
                .subtitle("URI")
                .source("uri"),
            );
        }
    }

    // 已存在的文件/目录
    let candidate = expand_tilde(q);
    let path = std::path::Path::new(&candidate);
    if (candidate.contains(':') || candidate.starts_with("\\\\") || path.is_absolute())
        && path.exists()
    {
        let is_dir = path.is_dir();
        let shown = candidate.clone();
        let title = if is_dir {
            "打开文件夹".to_string()
        } else {
            format!(
                "打开文件  {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        };
        return Some(
            SearchHit::new(
                ItemKind::External,
                format!("path:{shown}"),
                title,
                610.0,
                HitPayload::Open { path: candidate },
            )
            .subtitle(shown)
            .source("path"),
        );
    }

    None
}

fn expand_tilde(s: &str) -> String {
    if s == "~" || s.starts_with("~/") || s.starts_with("\\~\\") {
        if let Some(home) = dirs::home_dir() {
            return s.replacen("~", &home.to_string_lossy(), 1);
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedups_case_insensitive_keys() {
        let st = SearchText::build(&["Chrome", "chrome", "Chrome", ""]);
        assert_eq!(st.keys().len(), 1);
    }

    #[test]
    fn external_url_detected() {
        let h = external_hit("https://example.com/a").unwrap();
        assert!(matches!(h.payload, HitPayload::Open { .. }));
        assert!(h.title.contains("example.com"));
        assert!(external_hit("chrome").is_none());
        assert!(external_hit("www.example.com").is_some());
    }

    #[test]
    fn external_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let q = dir.path().to_string_lossy().to_string();
        let h = external_hit(&q).unwrap();
        assert_eq!(h.title, "打开文件夹");
    }
}
