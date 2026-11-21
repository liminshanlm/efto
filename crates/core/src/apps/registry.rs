//! 注册表 Uninstall 项：拿到正规产品名与安装位置。
//!
//! 只收录"能真正启动"的项：InstallLocation 里能找到主 exe，或 DisplayIcon 指向的 exe 存在。
//! 过滤系统组件、更新包、母产品子项（ParentDisplayName）。

use std::path::{Path, PathBuf};

use super::model::{AppEntry, AppSource, LaunchTarget, TargetKind};

/// 常见噪音：VC 运行库、更新包、驱动组件等。
fn is_noise(display_name: &str) -> bool {
    let n = display_name.to_ascii_lowercase();
    const BAD: &[&str] = &[
        "visual c++",
        "distributable",
        "redistributable",
        "security update",
        "update for",
        "hotfix",
        "kb",
        "runtime for",
        ".net runtime",
        "net runtime",
        "aspnetcore",
        ".net framework",
        "language pack",
        "feature on demand",
        "driver",
        "uninstall",
        "repair tool",
        "运行库",
        "辅助功能",
    ];
    if BAD.iter().any(|b| n.contains(b)) {
        return true;
    }
    // 纯 "Microsoft Application Error Reporting" 之类
    n.contains("error reporting") || n.contains("telemetry")
}

/// `C:\a\b.exe,0` / `C:\a\b.exe` / `C:\a\b.exe,-101` → `C:\a\b.exe`
pub fn strip_icon_index(icon: &str) -> String {
    let t = icon.trim().trim_matches('"');
    if let Some(idx) = t.rfind(',') {
        let tail = t[idx + 1..].trim();
        if !tail.is_empty() && tail.parse::<i32>().is_ok() {
            return t[..idx].trim_matches('"').to_string();
        }
    }
    t.to_string()
}

/// 从 InstallLocation + 名称线索猜测主程序 exe。
fn guess_main_exe(install_location: &Path, display_name: &str) -> Option<PathBuf> {
    if !install_location.is_dir() {
        return None;
    }
    let words: Vec<String> = display_name
        .to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect();

    let mut candidates: Vec<(u8, PathBuf)> = Vec::new();
    let rd = std::fs::read_dir(install_location).ok()?;
    for de in rd.flatten() {
        let p = de.path();
        if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("exe"))
            != Some(true)
        {
            continue;
        }
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if crate::fsutil::looks_like_helper(&stem) {
            continue;
        }
        let mut score = 0u8;
        if words.iter().any(|w| stem.starts_with(w)) {
            score += 3;
        }
        if words.iter().any(|w| stem.contains(w)) {
            score += 2;
        }
        if stem == "app" || stem == "main" {
            score += 1;
        }
        if p.metadata().map(|m| m.len() > 200_000).unwrap_or(false) {
            score += 1;
        }
        if score > 0 {
            candidates.push((score, p));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates.into_iter().next().map(|(_, p)| p)
}

#[cfg(windows)]
pub fn collect() -> Vec<AppEntry> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;

    let mut out = Vec::new();
    let roots: Vec<(RegKey, &str)> = vec![
        (RegKey::predef(HKEY_LOCAL_MACHINE), r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (RegKey::predef(HKEY_CURRENT_USER), r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    for (i, (hive, path)) in roots.iter().enumerate() {
        let flags = KEY_READ
            | if i == 0 {
                KEY_WOW64_64KEY
            } else if i == 1 {
                KEY_WOW64_32KEY
            } else {
                0
            };
        let Ok(key) = hive.open_subkey_with_flags(path, flags) else {
            continue;
        };
        for name in key.enum_keys().flatten() {
            let Ok(sub) = key.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            let get = |k: &str| sub.get_value::<String, _>(k).unwrap_or_default();
            let display_name = get("DisplayName");
            if display_name.trim().is_empty() {
                continue;
            }
            let system_component = sub
                .get_value::<u32, _>("SystemComponent")
                .unwrap_or(0);
            if system_component == 1 || !get("ParentDisplayName").is_empty() {
                continue;
            }
            let release_type = get("ReleaseType").to_ascii_lowercase();
            if matches!(release_type.as_str(), "update" | "hotfix" | "security update") {
                continue;
            }
            if is_noise(&display_name) {
                continue;
            }
            let publisher = get("Publisher");
            let install_location = get("InstallLocation");
            let display_icon = get("DisplayIcon");
            let version = get("DisplayVersion");

            let mut target: Option<LaunchTarget> = None;
            let mut icon: Option<String> = None;

            if !display_icon.is_empty() {
                let p = strip_icon_index(&display_icon);
                let path = PathBuf::from(&p);
                if path.is_file() {
                    target = Some(LaunchTarget {
                        path: p.clone(),
                        args: None,
                        working_dir: None,
                        kind: TargetKind::Exe,
                    });
                    icon = Some(p);
                }
            }
            if target.is_none() && !install_location.trim().is_empty() {
                let loc = PathBuf::from(install_location.trim().trim_matches('"'));
                if let Some(exe) = guess_main_exe(&loc, &display_name) {
                    let s = exe.to_string_lossy().to_string();
                    icon = Some(s.clone());
                    target = Some(LaunchTarget {
                        path: s,
                        args: None,
                        working_dir: Some(loc.to_string_lossy().to_string()),
                        kind: TargetKind::Exe,
                    });
                }
            }
            let Some(target) = target else { continue };

            let mut entry = AppEntry::new(display_name, target, AppSource::Registry);
            entry.icon = icon;
            if !publisher.trim().is_empty() {
                entry.aliases.push(publisher.trim().to_string());
            }
            if !version.trim().is_empty() {
                entry
                    .aliases
                    .push(format!("{} {}", entry.name, version.trim()));
            }
            entry.aliases.push(name.clone());
            entry.finalize();
            out.push(entry);
        }
    }
    tracing::info!("注册表卸载项贡献 {} 条应用", out.len());
    out
}

#[cfg(not(windows))]
pub fn collect() -> Vec<AppEntry> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_filter() {
        assert!(is_noise("Microsoft Visual C++ 2019 Redistributable (x64)"));
        assert!(is_noise("Microsoft .NET Runtime - 8.0.1"));
        assert!(is_noise("Security Update for Windows (KB5001234)"));
        assert!(!is_noise("Google Chrome"));
        assert!(!is_noise("DingTalk"));
    }

    #[test]
    fn icon_index_strip() {
        assert_eq!(strip_icon_index("C:\\a\\b.exe,0"), "C:\\a\\b.exe");
        assert_eq!(strip_icon_index("C:\\a\\b.exe,-101"), "C:\\a\\b.exe");
        assert_eq!(strip_icon_index("\"C:\\a b\\c.exe\""), "C:\\a b\\c.exe");
        assert_eq!(strip_icon_index("C:\\a\\b.exe"), "C:\\a\\b.exe");
    }

    #[test]
    fn guess_main_exe_prefers_named() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("DemoApp.exe"), b"MZ").unwrap();
        std::fs::write(dir.path().join("unins000.exe"), b"MZ").unwrap();
        std::fs::write(dir.path().join("crashpad_handler.exe"), b"MZ").unwrap();
        let got = guess_main_exe(dir.path(), "Demo App Pro");
        assert!(got.is_some());
        assert!(got.unwrap().to_string_lossy().ends_with("DemoApp.exe"));
    }
}
