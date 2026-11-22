//! 便签纸：`~/.efto/notes.json`。
//!
//! 每次唤起工具新建一条便签，由宿主开一个置顶小窗；文本改动去抖后落盘。
//! 窗口关闭即删除该条便签；宿主退出时保留，下次启动按上次的位置恢复。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use efto_core::{fsutil, Result};

/// 单张便签。位置与尺寸是窗口物理像素，首次创建时由宿主给一个级联位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NoteFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    items: Vec<Note>,
}

#[derive(Debug, Default, Clone)]
pub struct NoteStore {
    items: Vec<Note>,
}

impl NoteStore {
    pub fn path() -> PathBuf {
        fsutil::data_root().join("notes.json")
    }

    pub fn load() -> Self {
        match fsutil::read_to_string_opt(&Self::path()) {
            None => Self::default(),
            Some(s) => match Self::from_json(&s) {
                Ok(store) => store,
                Err(e) => {
                    tracing::warn!("notes.json 解析失败({e})，忽略便签");
                    Self::default()
                }
            },
        }
    }

    pub fn from_json(raw: &str) -> Result<Self> {
        let file: NoteFile = serde_json::from_str(raw)?;
        Ok(Self { items: file.items })
    }

    pub fn to_json(&self) -> Result<String> {
        let file = NoteFile {
            version: 1,
            items: self.items.clone(),
        };
        Ok(serde_json::to_string_pretty(&file)?)
    }

    pub fn save(&self) {
        if let Err(e) = self.write() {
            tracing::warn!("保存便签失败: {e}");
        }
    }

    fn write(&self) -> Result<()> {
        fsutil::write_atomic(&Self::path(), self.to_json()?.as_bytes())
    }

    pub fn list(&self) -> &[Note] {
        &self.items
    }

    pub fn get(&self, id: &str) -> Option<&Note> {
        self.items.iter().find(|n| n.id == id)
    }

    /// 新建一条空便签。位置按现有数量级联，避免全部叠在同一个点。
    /// 不落盘，调用方决定何时 [`NoteStore::save`]。
    pub fn create(&mut self) -> Note {
        let id = new_id();
        let n = self.items.len() as i32;
        let note = Note {
            id,
            text: String::new(),
            x: Some(140 + (n % 8) * 34),
            y: Some(120 + (n % 8) * 34),
            width: Some(300),
            height: Some(240),
        };
        self.items.push(note.clone());
        note
    }

    pub fn set_text(&mut self, id: &str, text: String) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.text = text;
        }
    }

    pub fn set_geometry(&mut self, id: &str, x: i32, y: i32, width: u32, height: u32) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.x = Some(x);
            n.y = Some(y);
            n.width = Some(width);
            n.height = Some(height);
        }
    }

    /// 删除一条便签；返回是否真的删掉了。
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|n| n.id != id);
        before != self.items.len()
    }
}

fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> NoteStore {
        NoteStore {
            items: vec![Note {
                id: "a".into(),
                text: "第一行".into(),
                x: Some(10),
                y: Some(20),
                width: Some(300),
                height: Some(200),
            }],
        }
    }

    #[test]
    fn json_roundtrip_keeps_notes() {
        let store = sample();
        let back = NoteStore::from_json(&store.to_json().unwrap()).unwrap();
        assert_eq!(back.list(), store.list());
    }

    #[test]
    fn create_staggers_position_and_persists_in_memory() {
        let mut store = NoteStore::default();
        let a = store.create();
        let b = store.create();
        assert_ne!(a.id, b.id);
        assert_ne!((a.x, a.y), (b.x, b.y));
        assert_eq!(store.list().len(), 2);
        assert_eq!(a.width, Some(300));
    }

    #[test]
    fn text_geometry_and_remove_are_targeted() {
        let mut store = sample();
        store.set_text("a", "改过".into());
        store.set_geometry("a", 1, 2, 400, 260);
        let n = store.get("a").unwrap();
        assert_eq!(n.text, "改过");
        assert_eq!((n.x, n.y, n.width, n.height), (Some(1), Some(2), Some(400), Some(260)));
        assert!(store.remove("a"));
        assert!(!store.remove("a"));
        assert!(store.get("a").is_none());
    }
}
