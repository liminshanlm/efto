//! 剪贴板历史，落盘 `~/.efto/clipboard.json`。
//!
//! 记录文本、图片（只存图片文件路径，PNG 由宿主写入）与文件列表。
//! 置顶条目不参与数量上限与 30 天清理；其余按最近使用排序。
//! 只做数据与持久化，监听与 UI 都在宿主侧。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{fsutil, Result};

const MAX_ITEMS: usize = 200;
const KEEP_DAYS: i64 = 30;
const DAY_SECS: i64 = 86_400;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    #[default]
    Text,
    Image,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipItem {
    pub id: String,
    #[serde(default)]
    pub kind: ClipKind,
    /// 搜索与纯文本粘贴用的文本：文本条目是原文；文件条目是路径换行拼接；
    /// 图片条目是尺寸标签（如 `1920×1080`）。
    pub text: String,
    pub ts: i64,
    #[serde(default)]
    pub pinned: bool,
    /// 图片文件路径（PNG），图片条目才有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// 文件列表，文件条目才有。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

impl ClipItem {
    pub fn is_text(&self) -> bool {
        self.kind == ClipKind::Text
    }

    fn same_content(&self, other: &ClipItem) -> bool {
        if self.kind != other.kind {
            return false;
        }
        match self.kind {
            // Windows 路径大小写不敏感，`C:\a.txt` 与 `c:\a.txt` 视为同一份文件。
            ClipKind::Files => {
                self.files.len() == other.files.len()
                    && self
                        .files
                        .iter()
                        .zip(&other.files)
                        .all(|(a, b)| a.eq_ignore_ascii_case(b))
            }
            _ => self.text == other.text && self.image == other.image,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ClipFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    items: Vec<ClipItem>,
}

#[derive(Debug, Default, Clone)]
pub struct ClipboardStore {
    items: Vec<ClipItem>,
}

impl ClipboardStore {
    pub fn path() -> PathBuf {
        fsutil::data_root().join("clipboard.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match fsutil::read_to_string_opt(&path) {
            None => Self::default(),
            Some(s) => match serde_json::from_str::<ClipFile>(&s) {
                Ok(f) => Self { items: f.items },
                Err(e) => {
                    tracing::warn!("clipboard.json 解析失败({e})，忽略历史");
                    Self::default()
                }
            },
        }
    }

    pub fn save(&self) -> Result<()> {
        let file = ClipFile {
            version: 2,
            items: self.items.clone(),
        };
        let json = serde_json::to_string(&file)?;
        fsutil::write_atomic(&Self::path(), json.as_bytes())
    }

    /// 记一条文本：空白忽略，重复内容移到最前。返回是否是新内容。
    pub fn push_text(&mut self, id: impl Into<String>, text: &str, now: i64) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        self.insert(ClipItem {
            id: id.into(),
            kind: ClipKind::Text,
            text: text.to_string(),
            ts: now,
            pinned: false,
            image: None,
            files: Vec::new(),
        })
    }

    /// 记一条图片：`label` 是尺寸等展示/搜索文本，`image` 是宿主写好的 PNG 路径。
    pub fn push_image(
        &mut self,
        id: impl Into<String>,
        image: impl Into<String>,
        label: &str,
        now: i64,
    ) -> bool {
        self.insert(ClipItem {
            id: id.into(),
            kind: ClipKind::Image,
            text: label.to_string(),
            ts: now,
            pinned: false,
            image: Some(image.into()),
            files: Vec::new(),
        })
    }

    /// 记一条文件列表：路径去空去重，顺序保留。
    pub fn push_files(&mut self, id: impl Into<String>, files: Vec<String>, now: i64) -> bool {
        let mut seen = std::collections::HashSet::new();
        let files: Vec<String> = files
            .into_iter()
            .filter(|p| !p.trim().is_empty())
            .filter(|p| seen.insert(p.to_lowercase()))
            .collect();
        if files.is_empty() {
            return false;
        }
        self.insert(ClipItem {
            id: id.into(),
            kind: ClipKind::Files,
            text: files.join("\n"),
            ts: now,
            pinned: false,
            image: None,
            files,
        })
    }

    fn insert(&mut self, item: ClipItem) -> bool {
        let existed = match self.items.iter().position(|i| i.same_content(&item)) {
            Some(pos) => {
                // 重复内容移到最前并刷新时间；用户手工置顶的状态保留。
                self.items.remove(pos);
                true
            }
            None => false,
        };
        self.items.insert(0, item);
        self.enforce_cap();
        !existed
    }

    /// 只裁掉未置顶的多余条目；置顶条目不设上限。
    fn enforce_cap(&mut self) {
        let mut unpinned = 0usize;
        self.items.retain(|i| {
            if i.pinned {
                return true;
            }
            unpinned += 1;
            unpinned <= MAX_ITEMS
        });
    }

    /// 丢弃超过 30 天的未置顶历史，返回被丢弃的条目（宿主用来删除图片文件）。
    pub fn prune(&mut self, now: i64) -> Vec<ClipItem> {
        let mut dropped = Vec::new();
        self.items.retain(|i| {
            let keep = i.pinned || now.saturating_sub(i.ts) <= KEEP_DAYS * DAY_SECS;
            if !keep {
                dropped.push(i.clone());
            }
            keep
        });
        dropped
    }

    /// 按关键词过滤；置顶条目恒在最前，其余按时间倒序。
    pub fn search(&self, query: &str) -> Vec<ClipItem> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<ClipItem> = self
            .items
            .iter()
            .filter(|i| q.is_empty() || i.text.to_lowercase().contains(&q))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.ts.cmp(&a.ts)));
        out
    }

    /// 置顶 / 取消置顶，返回新状态。
    pub fn toggle_pin(&mut self, id: &str) -> Option<bool> {
        let item = self.items.iter_mut().find(|i| i.id == id)?;
        item.pinned = !item.pinned;
        Some(item.pinned)
    }

    pub fn remove(&mut self, id: &str) -> Option<ClipItem> {
        let pos = self.items.iter().position(|i| i.id == id)?;
        Some(self.items.remove(pos))
    }

    pub fn clear(&mut self) -> Vec<ClipItem> {
        std::mem::take(&mut self.items)
    }

    pub fn items(&self) -> &[ClipItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn pinned_count(&self) -> usize {
        self.items.iter().filter(|i| i.pinned).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_dedupes_and_moves_to_front() {
        let mut s = ClipboardStore::default();
        assert!(s.push_text("clip:1", "hello", 100));
        assert!(s.push_text("clip:2", "world", 200));
        assert!(!s.push_text("clip:3", "hello", 300));
        let texts: Vec<&str> = s.items().iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, vec!["hello", "world"]);
        assert_eq!(s.items()[0].ts, 300, "重复内容应刷新为最近时间");
    }

    #[test]
    fn blank_text_is_ignored() {
        let mut s = ClipboardStore::default();
        assert!(!s.push_text("clip:1", "   \n\t", 100));
        assert!(s.is_empty());
    }

    #[test]
    fn push_is_capped_but_pinned_survive() {
        let mut s = ClipboardStore::default();
        for i in 0..(MAX_ITEMS + 3) {
            s.push_text(format!("clip:{i}"), &format!("t{i}"), i as i64);
        }
        assert_eq!(s.len(), MAX_ITEMS);
        assert_eq!(s.items()[0].text, format!("t{}", MAX_ITEMS + 2));

        let oldest = s.items().last().unwrap().id.clone();
        assert_eq!(s.toggle_pin(&oldest), Some(true));
        s.push_text("clip:new", "newest", 10_000);
        assert_eq!(s.len(), MAX_ITEMS + 1, "置顶条目不吃数量上限");
        assert!(s.items().iter().any(|i| i.id == oldest));
    }

    #[test]
    fn prune_drops_expired_but_keeps_pinned() {
        let mut s = ClipboardStore::default();
        s.push_text("clip:old", "old", 0);
        s.push_text("clip:pin", "pin", 0);
        s.push_text("clip:new", "new", KEEP_DAYS * DAY_SECS);
        s.toggle_pin("clip:pin");
        let dropped = s.prune(KEEP_DAYS * DAY_SECS + 1);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].id, "clip:old");
        let ids: Vec<&str> = s.items().iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["clip:new", "clip:pin"]);
    }

    #[test]
    fn search_and_remove() {
        let mut s = ClipboardStore::default();
        s.push_text("clip:1", "Hello World", 1);
        s.push_text("clip:2", "other", 2);
        assert_eq!(s.search("hello").len(), 1);
        assert_eq!(s.search("  ").len(), 2);
        s.toggle_pin("clip:1");
        let hits = s.search("");
        assert_eq!(hits[0].id, "clip:1", "置顶条目应排在最前");
        assert!(s.remove("clip:1").is_some());
        assert!(s.remove("clip:1").is_none());
        assert_eq!(s.len(), 1);
        assert_eq!(s.clear().len(), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn files_dedupe_and_join() {
        let mut s = ClipboardStore::default();
        let files = vec!["C:\\a.txt".to_string(), "C:\\A.txt".to_string(), " ".into()];
        assert!(s.push_files("clip:f1", files, 10));
        assert_eq!(s.items()[0].files, vec!["C:\\a.txt".to_string()]);
        assert_eq!(s.items()[0].text, "C:\\a.txt");
        assert!(!s.push_files("clip:f2", vec!["c:\\a.txt".into()], 20));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn image_push_and_kind_default() {
        let mut s = ClipboardStore::default();
        assert!(s.push_image("clip:i1", "C:\\img.png", "640×480", 5));
        assert_eq!(s.items()[0].kind, ClipKind::Image);
        assert_eq!(s.items()[0].image.as_deref(), Some("C:\\img.png"));

        // v1 的旧数据没有 kind 字段，反序列化后按文本处理。
        let old = r#"{"version":1,"items":[{"id":"clip:1","text":"legacy","ts":1}]}"#;
        let parsed: ClipFile = serde_json::from_str(old).unwrap();
        assert_eq!(parsed.items[0].kind, ClipKind::Text);
        assert!(!parsed.items[0].pinned);
    }
}
