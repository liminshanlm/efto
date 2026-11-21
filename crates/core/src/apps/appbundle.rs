//! macOS 应用发现：扫 `*.app` bundle，读 Info.plist。
//!
//! 边界（与 Windows 侧 startmenu/registry 对应）：
//! 1. **`.app` 是叶子**：只登记 bundle 本身，不进 `Contents/MacOS` 找 helper。
//! 2. 显示名 `CFBundleDisplayName` > `CFBundleName` > 目录名去 `.app` 后缀。
//! 3. `CFBundleIdentifier`（如 `com.google.Chrome`）进别名，可以用 bundle id 搜到。
//! 4. 启动目标是 `.app` 目录本身（`TargetKind::AppBundle`），由 launch 交给 `open`。
//!
//! 只在 macOS 上编译（Windows 用 startmenu/registry，见 docs/macos-support-plan.md）。

use std::fs;
use std::path::{Path, PathBuf};

use super::model::{AppEntry, AppSource, LaunchTarget, TargetKind};
use super::startmenu::IgnoreMatcher;

/// bundle 扫描的最大深度：`/Applications/Utilities/Foo.app` 足够，再深基本是插件目录。
pub const MAX_DEPTH: usize = 4;

/// 平台默认应用目录。
pub fn app_dirs() -> Vec<PathBuf> {
    let mut out = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        out.push(home.join("Applications"));
    }
    out.retain(|p| p.is_dir());
    out.dedup();
    out
}

pub fn collect(dirs: &[PathBuf], ignore: &IgnoreMatcher) -> Vec<AppEntry> {
    collect_limited(dirs, ignore, MAX_DEPTH)
}

pub fn collect_limited(dirs: &[PathBuf], ignore: &IgnoreMatcher, max_depth: usize) -> Vec<AppEntry> {
    let mut out = Vec::new();
    for root in dirs {
        // 栈式遍历：遇到 *.app 登记后不再下探（filter_entry 做不到"登记但不递归"）
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(read) = fs::read_dir(&dir) else { continue };
            for entry in read.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with('.') || ignore.is_ignored(&path) {
                    continue;
                }
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir && name.ends_with(".app") {
                    if let Some(e) = bundle_entry(&path) {
                        out.push(e);
                    }
                    continue;
                }
                if is_dir && depth < max_depth {
                    stack.push((path, depth + 1));
                }
            }
        }
    }
    out
}

/// 解析一个 `.app` bundle。Info.plist 缺失或读不了时退回目录名，不丢条目。
fn bundle_entry(app: &Path) -> Option<AppEntry> {
    if !app.is_dir() {
        return None;
    }
    let stem = app.file_stem()?.to_string_lossy().to_string();
    let plist_path = app.join("Contents").join("Info.plist");
    let info = plist::Value::from_file(&plist_path).ok();
    let dict = info.as_ref().and_then(|v| v.as_dictionary());

    let name = dict
        .and_then(|d| {
            d.get("CFBundleDisplayName")
                .or_else(|| d.get("CFBundleName"))
        })
        .and_then(|v| v.as_string())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(stem);

    let mut e = AppEntry::new(
        name,
        LaunchTarget {
            path: app.to_string_lossy().to_string(),
            args: None,
            working_dir: None,
            kind: TargetKind::AppBundle,
        },
        AppSource::AppBundle,
    );
    e.icon = Some(e.target.path.clone());
    if let Some(bid) = dict
        .and_then(|d| d.get("CFBundleIdentifier"))
        .and_then(|v| v.as_string())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        e.aliases.push(bid);
    }
    e.finalize();
    Some(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个假 .app：Info.plist 可选，helper 目录结构照真机摆。
    fn fake_app(root: &Path, dir_name: &str, plist: Option<&str>, helper: bool) -> PathBuf {
        let app = root.join(dir_name);
        fs::create_dir_all(app.join("Contents").join("MacOS")).unwrap();
        if let Some(p) = plist {
            fs::write(app.join("Contents").join("Info.plist"), p).unwrap();
        }
        if helper {
            fs::write(
                app.join("Contents").join("MacOS").join("crashpad_handler"),
                b"\0".repeat(128),
            )
            .unwrap();
        }
        app
    }

    const CHROME_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleDisplayName</key><string>Google Chrome</string>
  <key>CFBundleName</key><string>Chrome</string>
  <key>CFBundleIdentifier</key><string>com.google.Chrome</string>
</dict></plist>"#;

    #[test]
    fn collects_bundle_with_plist_name_and_bundle_id_alias() {
        let dir = tempfile::tempdir().unwrap();
        let app = fake_app(dir.path(), "Google Chrome.app", Some(CHROME_PLIST), true);
        let entries = collect(&[dir.path().to_path_buf()], &IgnoreMatcher::default());
        assert_eq!(entries.len(), 1, "{entries:?}");
        let e = &entries[0];
        assert_eq!(e.name, "Google Chrome");
        assert_eq!(e.target.kind, TargetKind::AppBundle);
        assert_eq!(e.target.path, app.to_string_lossy());
        assert!(e.aliases.iter().any(|a| a == "com.google.Chrome"), "{:?}", e.aliases);
    }

    #[test]
    fn name_falls_back_to_dir_stem_without_plist() {
        let dir = tempfile::tempdir().unwrap();
        fake_app(dir.path(), "SomeTool.app", None, false);
        let entries = collect(&[dir.path().to_path_buf()], &IgnoreMatcher::default());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "SomeTool");
    }

    /// 反例（硬性约定 2）：bundle 内部的 helper 不能被当成应用登记——
    /// `.app` 是叶子，不递归进 Contents/MacOS。裸可执行文件也不是本模块的职责。
    #[test]
    fn bundle_internals_are_never_indexed() {
        let dir = tempfile::tempdir().unwrap();
        fake_app(dir.path(), "Foo.app", Some(CHROME_PLIST), true);
        fs::write(dir.path().join("tool"), b"\0".repeat(128)).unwrap();
        let entries = collect(&[dir.path().to_path_buf()], &IgnoreMatcher::default());
        assert_eq!(entries.len(), 1, "只应有 bundle 本身：{entries:?}");
        for e in &entries {
            assert!(
                !e.target.path.contains("Contents/MacOS"),
                "helper 被当成应用了：{}",
                e.target.path
            );
        }
    }

    /// 反例（硬性约定 2）：忽略规则要吃掉被忽略目录里的整个 bundle。
    #[test]
    fn ignore_rules_drop_whole_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let buried = dir.path().join("node_modules");
        fs::create_dir_all(&buried).unwrap();
        fake_app(&buried, "Foo.app", Some(CHROME_PLIST), false);
        let m = IgnoreMatcher::new(&["**/node_modules/**".to_string()]);
        let entries = collect(&[dir.path().to_path_buf()], &m);
        assert!(entries.is_empty(), "{entries:?}");
    }

    /// Utilities 子目录里的系统工具要能被扫到（深度内）。
    #[test]
    fn nested_utilities_bundle_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let util = dir.path().join("Utilities");
        fs::create_dir_all(&util).unwrap();
        fake_app(&util, "Terminal.app", None, false);
        let entries = collect(&[dir.path().to_path_buf()], &IgnoreMatcher::default());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Terminal");
    }
}
