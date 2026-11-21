//! 使用频次存储：`~/.efto/usage.json`。
//! 记录 hit id → (次数, 最近使用时间)，用于排序加权与"最近使用"面板。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::fsutil;

pub const DAY_SECONDS: i64 = 86_400;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub last_ts: i64,
    /// 衰减后的权重，读取时用；写入由 `record` 维护。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UsageFile {
    version: u32,
    items: BTreeMap<String, UsageRecord>,
}

#[derive(Debug, Default, Clone)]
pub struct UsageStore {
    items: BTreeMap<String, UsageRecord>,
}

impl UsageStore {
    pub fn path() -> PathBuf {
        fsutil::data_root().join("usage.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        let text = match fsutil::read_to_string_opt(&path) {
            Some(t) => t,
            None => return Self::default(),
        };
        match serde_json::from_str::<UsageFile>(&text) {
            Ok(f) => Self { items: f.items },
            Err(e) => {
                tracing::warn!("usage.json 解析失败({e})，从空开始");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let file = UsageFile {
            version: 1,
            items: self.items.clone(),
        };
        fsutil::write_atomic(&Self::path(), serde_json::to_string_pretty(&file)?.as_bytes())
    }

    pub fn get(&self, id: &str) -> Option<&UsageRecord> {
        self.items.get(id)
    }

    /// 记一次使用。同一 id 30 秒内的重复点击只算一次，避免连点刷权重。
    pub fn record(&mut self, id: &str, now: i64) -> &UsageRecord {
        let rec = self.items.entry(id.to_string()).or_default();
        if now - rec.last_ts < 30 {
            rec.last_ts = now;
            return rec;
        }
        rec.count = rec.count.saturating_add(1);
        rec.last_ts = now;
        rec
    }

    /// 清理：90 天没用过且只用过 1 次的条目丢弃，文件不会无限膨胀。
    pub fn prune(&mut self, now: i64, keep_days: i64) -> usize {
        let cutoff = now - keep_days * DAY_SECONDS;
        let before = self.items.len();
        self.items
            .retain(|_, r| r.count > 1 || r.last_ts >= cutoff);
        before - self.items.len()
    }

    pub fn top(&self, n: usize) -> Vec<(String, UsageRecord)> {
        let mut v: Vec<(String, UsageRecord)> =
            self.items.iter().map(|(k, r)| (k.clone(), r.clone())).collect();
        v.sort_by(|a, b| b.1.last_ts.cmp(&a.1.last_ts));
        v.truncate(n);
        v
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

    #[test]
    fn record_counts_and_dedups_rapid_clicks() {
        let mut s = UsageStore::default();
        assert_eq!(s.record("x", 1000).count, 1);
        assert_eq!(s.record("x", 1005).count, 1, "30 秒内重复不计次");
        assert_eq!(s.record("x", 2000).count, 2);
        assert_eq!(s.get("x").unwrap().last_ts, 2000);
    }

    #[test]
    fn prune_drops_stale_singles() {
        let now = 1_700_000_000_i64; // 真实量级的 unix 时间戳
        let mut s = UsageStore::default();
        s.record("fresh", now);
        s.record("stale", now - 200 * DAY_SECONDS);
        s.record("heavy", now - 200 * DAY_SECONDS);
        s.record("heavy", now - 100 * DAY_SECONDS);
        let removed = s.prune(now, 90);
        assert_eq!(removed, 1, "只用过一次且 90 天未用的应被丢弃");
        assert!(s.get("fresh").is_some());
        assert!(s.get("heavy").is_some(), "高频项即使久远也保留");
        assert!(s.get("stale").is_none());
    }

    #[test]
    fn top_orders_by_recent() {
        let mut s = UsageStore::default();
        s.record("a", 100);
        s.record("b", 500);
        s.record("c", 300);
        let top = s.top(2);
        assert_eq!(top[0].0, "b");
        assert_eq!(top[1].0, "c");
    }
}
