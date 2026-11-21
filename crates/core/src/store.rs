//! 插件数据落盘。
//! 每个插件一个 JSON 文件，写入走原子替换，带配额保护。

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::error::{CoreError, Result};
use crate::fsutil;

/// 单插件键值上限 10MB。
pub const QUOTA_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct PluginStore {
    plugin_id: String,
    path: PathBuf,
    data: Map<String, Value>,
    dirty: bool,
}

impl PluginStore {
    pub fn file_path(plugin_id: &str) -> PathBuf {
        fsutil::data_root()
            .join("plugin-data")
            .join(format!("{}.json", fsutil::sanitize_key(plugin_id)))
    }

    pub fn open(plugin_id: &str) -> Self {
        let path = Self::file_path(plugin_id);
        let data = fsutil::read_to_string_opt(&path)
            .and_then(|t| serde_json::from_str::<Map<String, Value>>(&t).ok())
            .unwrap_or_default();
        Self {
            plugin_id: plugin_id.to_string(),
            path,
            data,
            dirty: false,
        }
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.data.get(key).cloned()
    }

    pub fn set(&mut self, key: &str, value: Value) -> Result<()> {
        let old = self.data.insert(key.to_string(), value);
        let size = self.size_bytes();
        if size > QUOTA_BYTES {
            match old {
                Some(prev) => {
                    self.data.insert(key.to_string(), prev);
                }
                None => {
                    self.data.remove(key);
                }
            }
            return Err(CoreError::InvalidManifest(format!(
                "插件数据超出配额（{} > {QUOTA_BYTES} 字节）",
                size
            )));
        }
        self.dirty = true;
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let had = self.data.remove(key).is_some();
        if had {
            self.dirty = true;
        }
        had
    }

    pub fn clear(&mut self) {
        if !self.data.is_empty() {
            self.data.clear();
            self.dirty = true;
        }
    }

    pub fn all(&self) -> Value {
        Value::Object(self.data.clone())
    }

    pub fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    pub fn size_bytes(&self) -> usize {
        serde_json::to_string(&self.data)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let json = serde_json::to_string(&self.data)?;
        fsutil::write_atomic(&self.path, json.as_bytes())?;
        self.dirty = false;
        Ok(())
    }
}

/// 宿主侧统一管理，插件退出/应用退出时批量 flush。
#[derive(Default)]
pub struct StoreManager {
    stores: HashMap<String, PluginStore>,
}

impl StoreManager {
    pub fn get_or_open(&mut self, plugin_id: &str) -> &mut PluginStore {
        self.stores
            .entry(plugin_id.to_string())
            .or_insert_with(|| PluginStore::open(plugin_id))
    }

    pub fn flush_all(&mut self) -> Result<()> {
        let mut first_err = None;
        for store in self.stores.values_mut() {
            if let Err(e) = store.flush() {
                tracing::error!("插件数据写入失败 {}: {e}", store.plugin_id());
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn drop_store(&mut self, plugin_id: &str) {
        if let Some(mut s) = self.stores.remove(plugin_id) {
            let _ = s.flush();
        }
    }

    pub fn len(&self) -> usize {
        self.stores.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stores.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_get_remove_roundtrip() {
        let id = format!("ut-store-{}", std::process::id());
        let s = PluginStore::open(&id);
        let path = s.path.clone();
        let _ = std::fs::remove_file(&path);
        let mut s = PluginStore::open(&id);
        assert!(s.get("k").is_none());
        s.set("k", json!({"a": 1})).unwrap();
        assert_eq!(s.get("k"), Some(json!({"a": 1})));
        assert!(s.is_dirty());
        s.flush().unwrap();
        assert!(!s.is_dirty());
        let reopened = PluginStore::open(&id);
        assert_eq!(reopened.get("k"), Some(json!({"a": 1})));
        assert!(reopened.size_bytes() > 0);
        assert_eq!(std::fs::remove_file(&path).is_ok(), true);
    }

    #[test]
    fn remove_reports_presence() {
        let mut s = PluginStore::open("ut-none");
        assert!(!s.remove("missing"));
        s.set("x", json!(1)).unwrap();
        assert!(s.remove("x"));
        assert!(s.get("x").is_none());
    }

    #[test]
    fn all_returns_object() {
        let mut s = PluginStore::open("ut-all");
        s.set("a", json!(1)).unwrap();
        s.set("b", json!("two")).unwrap();
        let all = s.all();
        assert_eq!(all["a"], json!(1));
        assert_eq!(all["b"], json!("two"));
        s.clear();
        assert_eq!(s.keys().len(), 0);
    }
}
