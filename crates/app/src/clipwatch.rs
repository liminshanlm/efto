//! 剪贴板监听：轮询 `arboard` + Win32 文件列表，内容变化时写入历史。
//!
//! 不用 `AddClipboardFormatListener`（需要自建消息窗口），500ms 轮询实现更小、
//! 跨平台一致；代价是最多 0.5 秒延迟，见 docs/verification.md。
//! 优先级：文件列表 > 图片 > 文本（复制文件时系统常同时给出缩略图）。

use std::time::Duration;

use tauri::Manager;

use crate::state::Shared;

const POLL: Duration = Duration::from_millis(500);

pub fn start(app: tauri::AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("efto-clipwatch".into())
        .spawn(move || {
            let mut clipboard = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("剪贴板监听不可用: {e}");
                    return;
                }
            };
            // 启动时已有的内容不记（否则每次启动都往历史里塞一条旧内容）
            let mut last_text = clipboard.get_text().unwrap_or_default();
            let mut last_files = String::new();
            let mut last_image = String::new();
            let mut seq: u64 = 0;
            loop {
                std::thread::sleep(POLL);
                let now = efto_core::apps::indexer::unix_now();
                let data = app.state::<Shared>();

                let files = crate::winclip::read_files();
                if !files.is_empty() {
                    let signature = files.join("\u{1}");
                    if signature == last_files {
                        continue;
                    }
                    last_files = signature;
                    last_text.clear();
                    last_image.clear();
                    seq += 1;
                    let mut store = data.clipboard.lock();
                    store.push_files(format!("clip:{now}-{seq}"), files, now);
                    prune_and_save(&mut store, now);
                    continue;
                }

                if let Ok(text) = clipboard.get_text() {
                    if !text.trim().is_empty() && text != last_text {
                        last_text = text.clone();
                        last_files.clear();
                        last_image.clear();
                        seq += 1;
                        let mut store = data.clipboard.lock();
                        store.push_text(format!("clip:{now}-{seq}"), &text, now);
                        prune_and_save(&mut store, now);
                        continue;
                    }
                }

                let Ok(image) = clipboard.get_image() else {
                    continue;
                };
                let signature = image_signature(&image);
                if signature == last_image {
                    continue;
                }
                last_image = signature.clone();
                last_text.clear();
                last_files.clear();
                let Some(path) = save_png(&image, &signature) else {
                    continue;
                };
                seq += 1;
                let label = format!("{}×{}", image.width, image.height);
                let mut store = data.clipboard.lock();
                store.push_image(format!("clip:{now}-{seq}"), path, &label, now);
                prune_and_save(&mut store, now);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("剪贴板监听线程启动失败: {e}");
    }
}

/// 清掉过期历史，并删除被丢弃条目的图片文件。
fn prune_and_save(store: &mut efto_core::clipboard::ClipboardStore, now: i64) {
    for item in store.prune(now) {
        remove_image(&item);
    }
    if let Err(e) = store.save() {
        tracing::warn!("剪贴板历史保存失败: {e}");
    }
}

pub fn remove_image(item: &efto_core::clipboard::ClipItem) {
    if let Some(path) = item.image.as_deref() {
        let _ = std::fs::remove_file(path);
    }
}

fn image_dir() -> std::path::PathBuf {
    efto_core::fsutil::data_root().join("clipboard-images")
}

/// 采样哈希：整张 4K 图每 500ms 全量哈希代价太高，用尺寸 + 抽样字节足够去重。
fn image_signature(image: &arboard::ImageData<'_>) -> String {
    use std::hash::{Hash, Hasher};
    let bytes = image.bytes.as_ref();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    image.width.hash(&mut h);
    image.height.hash(&mut h);
    bytes.len().hash(&mut h);
    let step = (bytes.len() / 512).max(1);
    for b in bytes.iter().step_by(step) {
        b.hash(&mut h);
    }
    format!("{:x}-{}x{}", h.finish(), image.width, image.height)
}

fn save_png(image: &arboard::ImageData<'_>, signature: &str) -> Option<String> {
    let dir = image_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let path = dir.join(format!("{signature}.png"));
    let rgba = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.to_vec(),
    )?;
    if let Err(e) = rgba.save(&path) {
        tracing::warn!("剪贴板图片保存失败: {e}");
        return None;
    }
    Some(path.to_string_lossy().to_string())
}
