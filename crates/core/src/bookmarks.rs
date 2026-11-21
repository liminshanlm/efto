//! 浏览器收藏夹：读取 Chrome / Edge 各 Profile 的 `Bookmarks` JSON，供搜索框检索。
//!
//! 约定：
//! - 只在设置里打开 `enable_bookmarks` 后才参与搜索（是否调用由宿主决定）；
//! - 只收录 `http` / `https` 书签，跳过 `javascript:` 等不能安全打开的目标；
//! - 按文件 mtime 缓存：收藏夹改动后，下一次搜索自动重建，不需要重启。

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use serde_json::Value;

use crate::search::{HitPayload, SearchHit, SearchText};
use crate::ItemKind;

/// 一条收藏的书签。
#[derive(Debug, Clone)]
struct Bookmark {
    title: String,
    url: String,
    browser: &'static str,
    /// 所在文件夹路径，`书签栏 / 工作` 这种形式。
    folder: String,
    /// 预先算好的匹配文本（标题 + 主机名 + 完整 URL），避免每次按键都重建拼音。
    search: SearchText,
}

/// 收藏夹快照。宿主放在锁里，搜索前调用 [`Self::refresh`]。
#[derive(Debug, Default)]
pub struct BookmarkStore {
    entries: Vec<Bookmark>,
    /// 已解析文件的路径与 mtime，用来判断是否需要重建。
    sources: Vec<(PathBuf, SystemTime)>,
}

impl BookmarkStore {
    /// 扫描 Chrome / Edge 的 User Data 目录；文件没变化时直接返回。
    pub fn refresh(&mut self) {
        let mut found = discover();
        let mut signatures = Vec::with_capacity(found.len());
        for (path, browser) in found.drain(..) {
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    signatures.push((path, mtime, browser));
                }
            }
        }
        let unchanged = signatures.len() == self.sources.len()
            && signatures
                .iter()
                .zip(self.sources.iter())
                .all(|((p, t, _), (sp, st))| p == sp && t == st);
        if unchanged {
            return;
        }

        let mut entries = Vec::new();
        for (path, _, browser) in &signatures {
            match std::fs::read_to_string(path) {
                Ok(text) => match parse(&text, browser) {
                    Ok(mut list) => entries.append(&mut list),
                    Err(e) => tracing::warn!("收藏夹解析失败 {}: {e}", path.display()),
                },
                Err(e) => tracing::warn!("读取收藏夹失败 {}: {e}", path.display()),
            }
        }
        tracing::info!("收藏夹载入 {} 条（{} 个 Profile）", entries.len(), signatures.len());
        self.entries = entries;
        self.sources = signatures.into_iter().map(|(p, t, _)| (p, t)).collect();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按标题 / 主机名 / URL 模糊匹配；同一 URL 只保留得分最高的一条。
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let mut hits: Vec<SearchHit> = Vec::new();
        for b in &self.entries {
            let Some((base, reason)) = b.search.best(query) else {
                continue;
            };
            let subtitle = if b.folder.is_empty() {
                b.url.clone()
            } else {
                format!("{} · {}", b.folder, b.url)
            };
            hits.push(
                SearchHit::new(
                    ItemKind::Bookmark,
                    format!("bookmark:{}", b.url),
                    &b.title,
                    // 收藏夹是补充源：同等匹配下让应用 / 插件 / 内置动作排在前面
                    base * 0.8,
                    HitPayload::Open { path: b.url.clone() },
                )
                .subtitle(subtitle)
                .reason(reason)
                .source(format!("bookmark:{}", b.browser)),
            );
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut seen = HashSet::new();
        hits.into_iter()
            .filter(|h| match &h.payload {
                HitPayload::Open { path } => seen.insert(path.clone()),
                _ => true,
            })
            .take(limit)
            .collect()
    }
}

/// `%LOCALAPPDATA%` 下两个浏览器的 User Data 根目录。
fn user_data_roots() -> Vec<(PathBuf, &'static str)> {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        (local.join("Google").join("Chrome").join("User Data"), "chrome"),
        (local.join("Microsoft").join("Edge").join("User Data"), "edge"),
    ]
}

/// 找出每个 Profile 目录下的 `Bookmarks` 文件。
fn discover() -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    for (root, browser) in user_data_roots() {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in rd.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let file = dir.join("Bookmarks");
            if file.is_file() {
                out.push((file, browser));
            }
        }
    }
    out
}

fn parse(text: &str, browser: &'static str) -> Result<Vec<Bookmark>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("JSON 解析失败: {e}"))?;
    let roots = root.get("roots").ok_or("缺少 roots")?;
    let mut out = Vec::new();
    for key in ["bookmark_bar", "other", "synced"] {
        if let Some(node) = roots.get(key) {
            walk(node, "", browser, &mut out, 0);
        }
    }
    Ok(out)
}

fn walk(
    node: &Value,
    folder: &str,
    browser: &'static str,
    out: &mut Vec<Bookmark>,
    depth: usize,
) {
    // 收藏夹理论上不会嵌套这么深，防止损坏文件把递归撑爆
    if depth > 32 {
        return;
    }
    match node.get("type").and_then(|v| v.as_str()) {
        Some("url") => {
            let url = node.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
            if !is_searchable_url(url) {
                return;
            }
            let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
            let title = if name.is_empty() { url.to_string() } else { name.to_string() };
            let host = host_of(url);
            let search = SearchText::build(&[title.as_str(), host.as_str(), url]);
            out.push(Bookmark {
                title,
                url: url.to_string(),
                browser,
                folder: folder.to_string(),
                search,
            });
        }
        Some("folder") => {
            let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
            let next = if folder.is_empty() {
                name.to_string()
            } else {
                format!("{folder} / {name}")
            };
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                for child in children {
                    walk(child, &next, browser, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

fn is_searchable_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "roots": {
        "bookmark_bar": {
          "type": "folder",
          "name": "书签栏",
          "children": [
            { "type": "url", "name": "GitHub", "url": "https://github.com/" },
            { "type": "folder", "name": "工作", "children": [
              { "type": "url", "name": "内网", "url": "http://intra.example/docs" }
            ]}
          ]
        },
        "other": {
          "type": "folder",
          "name": "其他收藏",
          "children": [
            { "type": "url", "name": "坏链接", "url": "javascript:alert(1)" }
          ]
        }
      }
    }"#;

    fn store() -> BookmarkStore {
        BookmarkStore {
            entries: parse(SAMPLE, "chrome").unwrap(),
            sources: vec![],
        }
    }

    #[test]
    fn parse_nested_folders_and_skip_unsafe_scheme() {
        let entries = parse(SAMPLE, "edge").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|b| b.url.starts_with("http")));
        assert_eq!(entries[1].folder, "书签栏 / 工作");
        assert_eq!(entries[0].browser, "edge");
    }

    #[test]
    fn search_matches_title_and_host() {
        let s = store();
        let hits = s.search("github", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "GitHub");
        assert_eq!(hits[0].kind, ItemKind::Bookmark);
        assert!(matches!(hits[0].payload, HitPayload::Open { .. }));

        // 网址 / 主机名也能命中
        assert_eq!(s.search("intra.example", 10)[0].title, "内网");
        // 危险 scheme 不参与
        assert!(s.search("alert", 10).is_empty());
        // 空查询不返回（与其它搜索源一致，默认面板由宿主提供）
        assert!(s.search("  ", 10).is_empty());
    }

    #[test]
    fn search_dedupes_same_url() {
        let mut s = store();
        let mut dup = s.entries[0].clone();
        dup.browser = "edge";
        s.entries.push(dup);
        let hits = s.search("github", 10);
        assert_eq!(hits.len(), 1);
    }
}
