//! 最近执行 / 打开 / 使用的条目快照，用于搜索框空查询的第一行。
//!
//! 存 `SearchHit` 快照而不是只存 id：插件与内置指令的执行结果无法仅凭 id 重建，
//! 应用命中也能直接从快照里的 `LaunchTarget` 执行。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::search::SearchHit;
use crate::Result;

/// 最多保留多少条；默认面板只取前 6 条。
const MAX_ITEMS: usize = 20;

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    items: Vec<SearchHit>,
}

#[derive(Debug, Default, Clone)]
pub struct RecentStore {
    items: Vec<SearchHit>,
}

impl RecentStore {
    pub fn path() -> PathBuf {
        fsutil::data_root().join("recent.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match fsutil::read_to_string_opt(&path) {
            None => Self::default(),
            Some(s) => match serde_json::from_str::<RecentFile>(&s) {
                Ok(f) => Self { items: f.items },
                Err(e) => {
                    tracing::warn!("recent.json 解析失败({e})，忽略历史");
                    Self::default()
                }
            },
        }
    }

    pub fn save(&self) -> Result<()> {
        let file = RecentFile {
            version: 1,
            items: self.items.clone(),
        };
        let json = serde_json::to_string(&file)?;
        fsutil::write_atomic(&Self::path(), json.as_bytes())
    }

    /// 记录一次执行：同 id 前插去重，最多 20 条。
    pub fn record(&mut self, hit: &SearchHit) {
        self.items.retain(|h| h.id != hit.id);
        let mut hit = hit.clone();
        // 分组只在默认面板里临时加，历史里不固化成 "recent"
        hit.group = None;
        self.items.insert(0, hit);
        self.items.truncate(MAX_ITEMS);
    }

    pub fn items(&self) -> &[SearchHit] {
        &self.items
    }

    /// 清空历史（面板上的"清除"按钮）。
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// 取最近 `n` 条，并用 `keep` 过滤失效项（插件已卸载等）。
    pub fn recent(&self, n: usize, keep: impl Fn(&SearchHit) -> bool) -> Vec<SearchHit> {
        self.items.iter().filter(|h| keep(h)).take(n).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::HitPayload;
    use crate::ItemKind;

    fn hit(id: &str) -> SearchHit {
        SearchHit::new(
            ItemKind::Action,
            id,
            id,
            0.0,
            HitPayload::Action { name: "screenshot".into() },
        )
    }

    #[test]
    fn record_dedupes_and_moves_to_front() {
        let mut store = RecentStore::default();
        store.record(&hit("action:screenshot"));
        store.record(&hit("action:clipboard"));
        store.record(&hit("action:screenshot"));
        let ids: Vec<&str> = store.items().iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["action:screenshot", "action:clipboard"]);
    }

    #[test]
    fn record_is_capped() {
        let mut store = RecentStore::default();
        for i in 0..(MAX_ITEMS + 5) {
            store.record(&hit(&format!("app:{i}")));
        }
        assert_eq!(store.len(), MAX_ITEMS);
        assert_eq!(store.items()[0].id, format!("app:{}", MAX_ITEMS + 4));
    }

    #[test]
    fn clear_removes_all_items() {
        let mut store = RecentStore::default();
        store.record(&hit("action:screenshot"));
        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn recent_filters_and_limits() {
        let mut store = RecentStore::default();
        store.record(&hit("action:screenshot"));
        store.record(&hit("app:gone"));
        let kept = store.recent(6, |h| h.id != "app:gone");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "action:screenshot");
        assert!(store.recent(6, |_| false).is_empty());
    }
}
