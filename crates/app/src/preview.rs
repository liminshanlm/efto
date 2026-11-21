//! 搜索结果预览：判定目标类型、读取文本、生成前端可消费的 payload。
//!
//! 只在用户按住 Shift 选中某条结果时调用。明确认识的类型（网页 / 图片 / 视频 /
//! 文本 / 本地 html）给内容预览；其余本地文件与目录展示属性信息（kind=meta）；
//! 只有"不是本地目标"（插件动作、复制文本、不存在的路径、ftp 等）才返回 `None`。

use std::path::Path;

use efto_core::search::{HitPayload, SearchHit};
use serde::{Deserialize, Serialize};

/// 文本预览读取上限：超过就截断，避免把几百 MB 的日志塞进 WebView。
pub const MAX_TEXT: usize = 512 * 1024;

/// 预览窗口消费的扁平结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewPayload {
    /// text / image / video / web / html / meta
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    /// 文本内容（kind=text）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 本地文件路径（kind=image / video / 本地 html）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 网页地址（kind=web）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 属性信息（kind=meta）：名称 / 类型 / 位置 / 大小 / 时间…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<(String, String)>,
    /// 文本是否被截断
    #[serde(default)]
    pub truncated: bool,
}

impl PreviewPayload {
    fn base(kind: &str, hit: &SearchHit) -> Self {
        Self {
            kind: kind.to_string(),
            title: hit.title.clone(),
            subtitle: hit.subtitle.clone(),
            content: None,
            path: None,
            url: None,
            fields: Vec::new(),
            truncated: false,
        }
    }

    fn with_url(mut self, url: &str) -> Self {
        self.url = Some(url.to_string());
        self
    }

    fn with_path(mut self, path: &Path) -> Self {
        self.path = Some(path.to_string_lossy().to_string());
        self
    }
}

/// 最近一次展示的预览内容。预览页加载完后主动调 `preview_ready` 来取，
/// 补上"窗口刚建好、事件先发出去"的竞态。
static CURRENT: once_cell::sync::Lazy<std::sync::Mutex<Option<PreviewPayload>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

pub fn store(payload: PreviewPayload) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(payload);
}

pub fn current() -> Option<PreviewPayload> {
    CURRENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 生成预览内容；返回 `None` 表示这类目标不支持预览。
pub fn build(hit: &SearchHit) -> Option<PreviewPayload> {
    let raw = match &hit.payload {
        HitPayload::Open { path } => path.trim(),
        // 应用条目（开始菜单 / 桌面 / 目录扫描到的 exe）同样按本地路径判定：
        // exe 没有内联预览，最终落到属性信息；URI 类目标会被下面的协议判定挡掉。
        HitPayload::Launch { target } => target.path.trim(),
        // 插件 / 内置动作 / 复制文本都不预览
        _ => return None,
    };
    if raw.is_empty() {
        return None;
    }

    if let Some(url) = web_url(raw) {
        return Some(PreviewPayload::base("web", hit).with_url(&url));
    }
    // 其他协议（ftp / mailto / file:// …）不支持；盘符（C:）不算协议
    if has_foreign_scheme(raw) {
        return None;
    }

    let path = Path::new(raw);
    if !path.exists() {
        return None;
    }

    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if is_web_ext(&ext) {
            // 本地 html 走应用内预览窗口（iframe + asset 协议）：
            // 独立 webview 直接加载文件时没有我们的工具栏，关不掉也打不开。
            let url = url::Url::from_file_path(path).ok()?;
            return Some(PreviewPayload::base("html", hit).with_url(url.as_str()).with_path(path));
        }
        if is_image_ext(&ext) {
            return Some(PreviewPayload::base("image", hit).with_path(path));
        }
        if ext == "pdf" {
            // WebView2 自带 Edge 的 PDF 查看器，走 iframe 即可内嵌翻页
            return Some(PreviewPayload::base("pdf", hit).with_path(path));
        }
        if is_video_ext(&ext) {
            return Some(PreviewPayload::base("video", hit).with_path(path));
        }
        if is_text_ext(&ext) {
            // 后缀像文本但读不出（二进制 / 无权限）：继续往下走属性展示
            if let Some((content, truncated)) = read_text(path) {
                let mut p = PreviewPayload::base("text", hit).with_path(path);
                p.content = Some(content);
                p.truncated = truncated;
                return Some(p);
            }
        }
    }

    // 不支持内联预览的本地文件（exe/docx/zip…）与目录：展示属性信息
    Some(meta_payload(hit, path))
}

/// 文件属性信息（Windows「属性」对话框里最常用的几项）。
fn meta_payload(hit: &SearchHit, path: &Path) -> PreviewPayload {
    let meta = std::fs::metadata(path).ok();
    let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut fields: Vec<(String, String)> = Vec::new();
    fields.push(("名称".into(), file_name(path)));
    fields.push((
        "类型".into(),
        if is_dir {
            "文件夹".to_string()
        } else if ext.is_empty() {
            "文件".to_string()
        } else {
            format!("{} 文件", ext.to_ascii_uppercase())
        },
    ));
    fields.push((
        "位置".into(),
        path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    ));
    if let Some(m) = meta.as_ref() {
        if !is_dir {
            fields.push((
                "大小".into(),
                format!("{} ({} 字节)", human_size(m.len()), m.len()),
            ));
        }
        if let Some(t) = m.modified().ok().map(fmt_time) {
            fields.push(("修改时间".into(), t));
        }
        if let Some(t) = m.created().ok().map(fmt_time) {
            fields.push(("创建时间".into(), t));
        }
        if let Some(t) = m.accessed().ok().map(fmt_time) {
            fields.push(("访问时间".into(), t));
        }
        let mut attrs = Vec::new();
        if m.permissions().readonly() {
            attrs.push("只读");
        }
        if !attrs.is_empty() {
            fields.push(("属性".into(), attrs.join("、")));
        }
    }

    let mut p = PreviewPayload::base("meta", hit).with_path(path);
    p.fields = fields;
    p
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

fn fmt_time(t: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// http(s) / www. 开头的直接当网页；返回规范化后的地址。
fn web_url(s: &str) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(s.to_string());
    }
    if lower.starts_with("www.") {
        return Some(format!("https://{s}"));
    }
    None
}

fn has_foreign_scheme(s: &str) -> bool {
    let Some(i) = s.find(':') else { return false };
    let head = &s[..i];
    // 单字母是 Windows 盘符（C:\），不是协议
    if head.len() < 2 {
        return false;
    }
    head.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

fn is_web_ext(ext: &str) -> bool {
    matches!(ext, "html" | "htm")
}

fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "avif" | "tif" | "tiff"
    )
}

fn is_video_ext(ext: &str) -> bool {
    matches!(ext, "mp4" | "m4v" | "webm" | "ogv" | "ogg" | "mov")
}

fn is_text_ext(ext: &str) -> bool {
    matches!(
        ext,
        "txt"
            | "md"
            | "markdown"
            | "log"
            | "json"
            | "jsonc"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "jsx"
            | "css"
            | "scss"
            | "less"
            | "xml"
            | "yml"
            | "yaml"
            | "toml"
            | "ini"
            | "cfg"
            | "conf"
            | "env"
            | "rs"
            | "go"
            | "py"
            | "java"
            | "kt"
            | "kts"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "cs"
            | "php"
            | "rb"
            | "sh"
            | "bash"
            | "ps1"
            | "bat"
            | "cmd"
            | "sql"
            | "csv"
            | "tsv"
            | "gradle"
    )
}

/// 读文本文件：最多 [`MAX_TEXT`] 字节，识别 BOM，UTF-8 失败时按系统 ANSI
/// 代码页回退（中文系统上是 GBK）。含 NUL 视为二进制。
fn read_text(path: &Path) -> Option<(String, bool)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.by_ref()
        .take((MAX_TEXT + 1) as u64)
        .read_to_end(&mut buf)
        .ok()?;
    let truncated = buf.len() > MAX_TEXT;
    buf.truncate(MAX_TEXT);
    let text = decode(&buf)?;
    Some((text, truncated))
}

fn decode(buf: &[u8]) -> Option<String> {
    if let Some(rest) = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    if let Some(rest) = buf.strip_prefix(&[0xFF, 0xFE]) {
        return Some(utf16(rest, true));
    }
    if let Some(rest) = buf.strip_prefix(&[0xFE, 0xFF]) {
        return Some(utf16(rest, false));
    }
    if buf.contains(&0) {
        return None;
    }
    match std::str::from_utf8(buf) {
        Ok(s) => Some(s.to_string()),
        Err(_) => Some(ansi_or_lossy(buf)),
    }
}

fn utf16(bytes: &[u8], le: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(windows)]
fn ansi_or_lossy(buf: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};
    if buf.is_empty() {
        return String::new();
    }
    unsafe {
        let len = MultiByteToWideChar(
            CP_ACP,
            0,
            buf.as_ptr(),
            buf.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if len <= 0 {
            return String::from_utf8_lossy(buf).into_owned();
        }
        let mut out = vec![0u16; len as usize];
        let n = MultiByteToWideChar(
            CP_ACP,
            0,
            buf.as_ptr(),
            buf.len() as i32,
            out.as_mut_ptr(),
            len,
        );
        if n <= 0 {
            return String::from_utf8_lossy(buf).into_owned();
        }
        String::from_utf16_lossy(&out[..n as usize])
    }
}

#[cfg(not(windows))]
fn ansi_or_lossy(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use efto_core::ItemKind;

    fn hit(payload: HitPayload) -> SearchHit {
        SearchHit {
            kind: ItemKind::File,
            id: "t".into(),
            title: "t".into(),
            subtitle: String::new(),
            score: 0.0,
            reason: String::new(),
            icon: None,
            source: "everything".into(),
            group: None,
            pin: None,
            payload,
        }
    }

    fn open(path: &str) -> SearchHit {
        hit(HitPayload::Open {
            path: path.to_string(),
        })
    }

    #[test]
    fn urls_are_web() {
        assert_eq!(web_url("http://a.com").as_deref(), Some("http://a.com"));
        assert_eq!(web_url("HTTPS://a.com").as_deref(), Some("HTTPS://a.com"));
        assert_eq!(web_url("www.a.com").as_deref(), Some("https://www.a.com"));
        assert_eq!(web_url("C:\\Windows"), None);
    }

    #[test]
    fn foreign_scheme_rejected() {
        assert!(has_foreign_scheme("ftp://x"));
        assert!(has_foreign_scheme("mailto:a@b.c"));
        assert!(!has_foreign_scheme("C:\\Windows"));
        assert!(!has_foreign_scheme("C:/Windows"));
        assert!(!has_foreign_scheme("\\\\server\\share"));
    }

    #[test]
    fn web_and_unknown_targets() {
        let p = build(&open("www.example.com")).expect("URL 应可预览");
        assert_eq!(p.kind, "web");
        assert_eq!(p.url.as_deref(), Some("https://www.example.com"));

        // 不存在的路径、其它协议都不弹预览
        assert!(build(&open("Z:\\no\\such\\file.txt")).is_none());
        assert!(build(&open("ftp://example.com/a.txt")).is_none());

        // 目录展示属性信息
        let dir = tempfile::tempdir().unwrap();
        let meta = build(&open(dir.path().to_str().unwrap())).expect("目录应展示属性");
        assert_eq!(meta.kind, "meta");
        assert!(meta.fields.iter().any(|(k, v)| k == "类型" && v == "文件夹"));
    }

    #[test]
    fn unsupported_payloads_never_preview() {
        assert!(build(&hit(HitPayload::CopyText {
            text: "1".into(),
            echo: serde_json::Value::Null,
        }))
        .is_none());
        assert!(build(&hit(HitPayload::Action {
            name: "settings".into()
        }))
        .is_none());
    }

    #[test]
    fn text_is_read_and_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, "你好，efto").unwrap();
        let payload = build(&open(p.to_str().unwrap())).expect("文本应可预览");
        assert_eq!(payload.kind, "text");
        assert_eq!(payload.content.as_deref(), Some("你好，efto"));
        assert!(!payload.truncated);

        let big = dir.path().join("big.log");
        std::fs::write(&big, "x".repeat(MAX_TEXT + 10)).unwrap();
        let payload = build(&open(big.to_str().unwrap())).unwrap();
        assert!(payload.truncated);
        assert_eq!(payload.content.unwrap().len(), MAX_TEXT);
    }

    #[test]
    fn binary_text_ext_falls_back_to_meta() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bin.txt");
        std::fs::write(&p, [0x00, 0x01, 0x02, 0x00]).unwrap();
        let payload = build(&open(p.to_str().unwrap())).expect("二进制文本应展示属性");
        assert_eq!(payload.kind, "meta");
    }

    #[test]
    fn pdf_is_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        std::fs::write(&p, b"%PDF-1.4").unwrap();
        let payload = build(&open(p.to_str().unwrap())).unwrap();
        assert_eq!(payload.kind, "pdf");
    }

    #[test]
    fn app_launch_target_shows_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("tool.exe");
        std::fs::write(&exe, vec![0u8; 16]).unwrap();
        let hit = hit(HitPayload::Launch {
            target: efto_core::apps::model::LaunchTarget::exe(exe.to_str().unwrap()),
        });
        let payload = build(&hit).expect("exe 应用应展示属性");
        assert_eq!(payload.kind, "meta");
        assert!(payload.fields.iter().any(|(k, v)| k == "类型" && v == "EXE 文件"));
    }

    #[test]
    fn other_files_show_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("setup.exe");
        std::fs::write(&p, vec![0u8; 2048]).unwrap();
        let payload = build(&open(p.to_str().unwrap())).expect("exe 应展示属性");
        assert_eq!(payload.kind, "meta");
        let field = |name: &str| {
            payload
                .fields
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(field("类型"), "EXE 文件");
        assert_eq!(field("名称"), "setup.exe");
        assert!(field("大小").contains("2.0 KB"), "大小: {}", field("大小"));
        assert!(field("修改时间").contains(':'));
    }

    #[test]
    fn image_and_video_detected() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("a.PNG");
        std::fs::write(&img, b"png").unwrap();
        assert_eq!(build(&open(img.to_str().unwrap())).unwrap().kind, "image");
        let vid = dir.path().join("b.mp4");
        std::fs::write(&vid, b"mp4").unwrap();
        assert_eq!(build(&open(vid.to_str().unwrap())).unwrap().kind, "video");
    }

    #[test]
    fn local_html_is_html() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("index.html");
        std::fs::write(&p, "<html></html>").unwrap();
        let payload = build(&open(p.to_str().unwrap())).unwrap();
        assert_eq!(payload.kind, "html");
        assert!(payload.url.unwrap().starts_with("file:///"));
    }

    #[test]
    fn utf16_bom_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("u16.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for u in "abc".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&p, bytes).unwrap();
        let payload = build(&open(p.to_str().unwrap())).unwrap();
        assert_eq!(payload.content.as_deref(), Some("abc"));
    }
}
