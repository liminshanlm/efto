//! 自定义协议 `efto://`：把插件目录作为静态站点暴露给插件 UI 页面。
//!
//! 平台差异（已在 tauri/wry 源码核实）：Windows 上 WebView2 不支持任意 scheme，
//! 页面实际请求 `http://efto.localhost/...`，wry 拦截后还原成 `efto://localhost/...`
//! 再交给这里，所以 handler 只需按 `/{plugin_id}/{相对路径}` 解析。

use std::io::Read;
use std::path::PathBuf;

use tauri::{UriSchemeResponder, http::Response};
use efto_core::fsutil;

use crate::state::Shared;

/// 插件 UI 使用的 scheme。
pub const EFTO_SCHEME: &str = "efto";

pub fn handle(data: Shared, request: tauri::http::Request<Vec<u8>>, responder: UriSchemeResponder) {
    let path = request.uri().path().to_string();
    let result = serve(&data, &request);
    // 白屏排查的第一现场：请求到底来没来、我们回了什么状态码
    tracing::debug!("协议 {} {} -> {}", request.method(), path, result.status());
    responder.respond(result);
}

fn serve(
    data: &Shared,
    request: &tauri::http::Request<Vec<u8>>,
) -> Response<std::borrow::Cow<'static, [u8]>> {
    if request.method() != "GET" && request.method() != "HEAD" {
        return response(405, "text/plain; charset=utf-8", b"method not allowed".to_vec());
    }
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let decoded = percent_decode(&path);
    let mut parts = decoded.trim_start_matches('/').splitn(2, '/');
    let plugin_id = parts.next().unwrap_or("");
    let rel = parts.next().unwrap_or("");
    if plugin_id.is_empty() {
        return response(404, "text/plain; charset=utf-8", b"missing plugin id".to_vec());
    }
    let root = match data.plugin_root(plugin_id) {
        Some(r) => r,
        None => return response(404, "text/plain; charset=utf-8", b"unknown plugin".to_vec()),
    };
    let file = match resolve_in_root(&root, rel) {
        Ok(f) => f,
        Err(code) => return response(code, "text/plain; charset=utf-8", b"path rejected".to_vec()),
    };
    match std::fs::read(&file) {
        Ok(bytes) => response(200, mime_for(&file), bytes),
        Err(e) => {
            tracing::debug!("协议读取失败 {}: {e}", file.display());
            response(404, "text/plain; charset=utf-8", b"not found".to_vec())
        }
    }
}

/// 拼路径 + 越界检查。
/// 先做词法检查（能给出"越界"这种准确原因），再用 canonicalize 挡符号链接逃逸。
fn resolve_in_root(root: &std::path::Path, rel: &str) -> Result<PathBuf, u16> {
    let rel = rel.replace('\\', "/");
    let rel = rel.trim_start_matches('/');
    if escapes_root(&rel) {
        tracing::warn!("插件协议路径包含越界的 .. : {rel}");
        return Err(403);
    }
    let candidate = if rel.is_empty() {
        root.join("index.html")
    } else {
        root.join(rel)
    };
    // 目录请求补 index.html
    let final_path = if candidate.is_dir() {
        candidate.join("index.html")
    } else {
        candidate
    };
    let canonical = final_path.canonicalize().map_err(|_| 404u16)?;
    let root_canon = root.canonicalize().map_err(|_| 500u16)?;
    if !canonical.starts_with(&root_canon) {
        tracing::warn!("插件协议路径越界被拒绝: {}", canonical.display());
        return Err(403);
    }
    if !canonical.is_file() {
        return Err(404);
    }
    Ok(canonical)
}

/// 词法判断：`a/../..` 之类是否会把请求带到 root 之外。
fn escapes_root(rel: &str) -> bool {
    let mut depth = 0i32;
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            _ => depth += 1,
        }
    }
    false
}

fn response(status: u16, content_type: &str, body: Vec<u8>) -> Response<std::borrow::Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(tauri::http::header::CONTENT_TYPE, content_type)
        // 自定义协议不参与同源策略，插件页面 fetch 自身资源时需要它
        .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header("Cache-Control", "no-cache")
        .body(std::borrow::Cow::Owned(body))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(500)
                .body(std::borrow::Cow::Owned(b"response build failed".to_vec()))
                .expect("静态响应构建失败")
        })
}

fn mime_for(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

/// 最小百分号解码（避免为几个字符引入 &str 扩展 trait 的歧义）。
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 读取整个响应体（给测试与将来的离线打包用）。
#[allow(dead_code)]
fn read_all(mut r: impl Read) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf)?;
    Ok(buf)
}

/// 数据目录相对路径（用于把插件数据与代码分开）。
#[allow(dead_code)]
pub fn data_dir_for(plugin_id: &str) -> PathBuf {
    fsutil::data_root()
        .join("plugin-data")
        .join(format!("{}.json", fsutil::sanitize_key(plugin_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_basic() {
        assert_eq!(percent_decode("/a%20b/c.txt"), "/a b/c.txt");
        assert_eq!(percent_decode("/demo/index.html"), "/demo/index.html");
        assert_eq!(percent_decode("%E4%B8%AD"), "中");
        assert_eq!(percent_decode("100%broken"), "100%broken");
    }

    #[test]
    fn mime_mapping() {
        assert!(mime_for(std::path::Path::new("a.html")).starts_with("text/html"));
        assert!(mime_for(std::path::Path::new("a.JS")).starts_with("text/javascript"));
        assert_eq!(mime_for(std::path::Path::new("a.svg")), "image/svg+xml");
        assert_eq!(mime_for(std::path::Path::new("a.unknown")), "application/octet-stream");
    }

    #[test]
    fn path_resolution_blocks_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("index.html"), "<p>hi").unwrap();
        std::fs::write(root.join("sub").join("a.css"), "x").unwrap();

        let ok = resolve_in_root(&root, "").unwrap();
        assert!(ok.to_string_lossy().ends_with("index.html"));
        assert!(resolve_in_root(&root, "sub/a.css").is_ok());
        assert_eq!(resolve_in_root(&root, "../../etc/hosts"), Err(403));
        assert_eq!(resolve_in_root(&root, "nope.js"), Err(404));
        assert_eq!(resolve_in_root(&root, "sub"), Err(404)); // 目录里没有 index.html
    }

    #[test]
    fn dir_with_index_resolves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("page")).unwrap();
        std::fs::write(dir.path().join("page").join("index.html"), "x").unwrap();
        assert!(resolve_in_root(dir.path(), "page").is_ok());
    }

    #[test]
    fn lexical_escape_detection() {
        assert!(escapes_root("../../etc/passwd"));
        assert!(escapes_root("a/../../x"));
        assert!(!escapes_root("a/../b.js"));
        assert!(!escapes_root("./logo.png"));
        assert!(!escapes_root("sub/deep/file.css"));
    }

    #[test]
    fn internal_dotdot_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("x.css"), "y").unwrap();
        assert!(resolve_in_root(dir.path(), "a/../x.css").is_ok());
    }
}
