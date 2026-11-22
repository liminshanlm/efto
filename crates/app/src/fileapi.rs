//! 插件 UI 的受限文本文件读写。
//!
//! 为什么需要授权：插件 main 没有 `fs`，UI 只拿得到「选文件 / 拖文件」的路径，
//! 读不到内容。这里只放行**用户亲自选择或拖入**的路径，避免插件页面凭一个路径
//! 字符串读写任意磁盘文件。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::state::Shared;

/// 单文件上限：编辑器场景够用，避免把超大文件读进 webview 或一次性写盘。
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 每个插件已授权的路径（canonical 形式，键是插件 id）。
#[derive(Default)]
pub struct FileGrants {
    inner: Mutex<HashMap<String, HashSet<PathBuf>>>,
}

impl FileGrants {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录用户主动选择 / 拖入的路径。不存在的路径直接忽略（拖入时可能已被删掉）。
    pub fn grant(&self, plugin: &str, paths: impl IntoIterator<Item = PathBuf>) {
        let mut map = self.inner.lock();
        let slot = map.entry(plugin.to_string()).or_default();
        for p in paths {
            if let Ok(c) = p.canonicalize() {
                slot.insert(c);
            }
        }
    }

    /// 已授权时返回 canonical 路径，未授权返回 None。
    pub fn granted_path(&self, plugin: &str, path: &Path) -> Option<PathBuf> {
        let canon = path.canonicalize().ok()?;
        let map = self.inner.lock();
        map.get(plugin)
            .filter(|set| set.contains(&canon))
            .map(|_| canon.clone())
    }
}

#[derive(Debug, Serialize)]
pub struct ReadResult {
    pub path: String,
    pub content: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct WriteResult {
    pub path: String,
    pub bytes: u64,
}

/// 去 UTF-8 BOM：编辑器里不该看到一个不可见的 `` 字符。
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

pub fn read_text(grants: &FileGrants, plugin: &str, path: &Path) -> Result<ReadResult, String> {
    let canon = grants
        .granted_path(plugin, path)
        .ok_or_else(|| "只能读取你选择或拖入的文件".to_string())?;
    let meta = std::fs::metadata(&canon).map_err(|e| format!("读取失败：{e}"))?;
    if !meta.is_file() {
        return Err("只能打开文件，不能打开目录".into());
    }
    if meta.len() > MAX_BYTES {
        return Err(format!(
            "文件超过 {} MiB，无法在编辑器里打开",
            MAX_BYTES / 1024 / 1024
        ));
    }
    let bytes = std::fs::read(&canon).map_err(|e| format!("读取失败：{e}"))?;
    let content = String::from_utf8(bytes).map_err(|_| "不是 UTF-8 文本，无法编辑".to_string())?;
    Ok(ReadResult {
        path: canon.to_string_lossy().into(),
        content: strip_bom(&content).to_string(),
        bytes: meta.len(),
    })
}

pub fn write_text(
    grants: &FileGrants,
    plugin: &str,
    path: &Path,
    text: &str,
) -> Result<WriteResult, String> {
    let canon = grants
        .granted_path(plugin, path)
        .ok_or_else(|| "只能保存你选择或拖入的文件".to_string())?;
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("内容超过 {} MiB，无法保存", MAX_BYTES / 1024 / 1024));
    }
    efto_core::fsutil::write_atomic(&canon, text.as_bytes())
        .map_err(|e| format!("保存失败：{e}"))?;
    Ok(WriteResult {
        path: canon.to_string_lossy().into(),
        bytes: text.len() as u64,
    })
}

#[tauri::command]
pub fn plugin_file_read(
    path: String,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<ReadResult, String> {
    let plugin = crate::ui::plugin_of(&app, &window)?;
    let data = app.state::<Shared>();
    read_text(&data.file_grants, &plugin, Path::new(&path))
}

#[tauri::command]
pub fn plugin_file_write(
    path: String,
    text: String,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<WriteResult, String> {
    let plugin = crate::ui::plugin_of(&app, &window)?;
    let data = app.state::<Shared>();
    write_text(&data.file_grants, &plugin, Path::new(&path), &text)
}

/// 另存为：路径由用户在原生保存框里挑，因此不需要预先授权；
/// 保存成功后把该路径加进授权，后续 Ctrl+S 可以直接写回。
#[tauri::command]
pub async fn plugin_file_save_as(
    name: String,
    text: String,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<String>, String> {
    let plugin = crate::ui::plugin_of(&app, &window)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("内容超过 {} MiB，无法保存", MAX_BYTES / 1024 / 1024));
    }
    let file_name = sanitize_file_name(&name);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_title("保存文件")
            .set_file_name(&file_name)
            .add_filter("Markdown", &["md", "markdown"])
            .add_filter("HTML", &["html", "htm"])
            .add_filter("文本", &["txt"])
            .set_parent(&window)
            .save_file()
    })
    .await
    .map_err(|e| format!("保存对话框失败：{e}"))?;
    let Some(path) = picked else {
        return Ok(None);
    };
    efto_core::fsutil::write_atomic(&path, text.as_bytes())
        .map_err(|e| format!("保存失败：{e}"))?;
    let data = app.state::<Shared>();
    data.file_grants.grant(&plugin, [path.clone()]);
    Ok(Some(path.to_string_lossy().into()))
}

/// 文件名里的路径分隔符会让保存框把它当目录，统一兜成纯文件名。
fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = base
        .chars()
        .map(|c| if "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    if cleaned.is_empty() {
        "untitled.md".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn ungranted_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_fixture(dir.path(), "a.md", b"# hi");
        let grants = FileGrants::new();
        assert!(read_text(&grants, "x", &p).unwrap_err().contains("只能读取"));
        assert!(write_text(&grants, "x", &p, "no").is_err());
        // 别的插件拿到同一个路径也不行
        grants.grant("x", [p.clone()]);
        assert!(read_text(&grants, "y", &p).is_err());
        assert_eq!(read_text(&grants, "x", &p).unwrap().content, "# hi");
    }

    #[test]
    fn write_then_read_roundtrip_and_bom_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_fixture(dir.path(), "b.md", b"old");
        let grants = FileGrants::new();
        grants.grant("editor", [p.clone()]);
        write_text(&grants, "editor", &p, "# 新标题").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "# 新标题");

        std::fs::write(&p, b"\xEF\xBB\xBF# bom").unwrap();
        assert_eq!(read_text(&grants, "editor", &p).unwrap().content, "# bom");
    }

    #[test]
    fn oversize_and_non_utf8_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let big = write_fixture(dir.path(), "big.md", b"");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        let bad = write_fixture(dir.path(), "bad.md", &[0xff, 0xfe, 0x00]);
        let grants = FileGrants::new();
        grants.grant("editor", [big.clone(), bad.clone()]);
        assert!(read_text(&grants, "editor", &big).unwrap_err().contains("MiB"));
        assert!(read_text(&grants, "editor", &bad).unwrap_err().contains("UTF-8"));
        let text = "a".repeat(MAX_BYTES as usize + 1);
        assert!(write_text(&grants, "editor", &bad, &text).unwrap_err().contains("MiB"));
    }

    #[test]
    fn save_as_file_name_is_sanitized() {
        assert_eq!(sanitize_file_name("D:\\x\\报告.md"), "报告.md");
        assert_eq!(sanitize_file_name("a/b:c.md"), "b_c.md");
        assert_eq!(sanitize_file_name("   "), "untitled.md");
    }
}
