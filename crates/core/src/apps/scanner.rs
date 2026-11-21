//! 常见安装目录扫描 + 系统工具白名单。

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::model::{AppEntry, AppSource, LaunchTarget, TargetKind};
use super::startmenu::IgnoreMatcher;
use crate::fsutil;

/// 平台默认安装根目录。
pub fn default_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<PathBuf>, v: Option<std::ffi::OsString>, tail: &str| {
        if let Some(base) = v {
            let p = Path::new(&base).join(tail);
            if p.is_dir() {
                out.push(p);
            }
        }
    };
    push(
        &mut out,
        std::env::var_os("ProgramFiles"),
        "",
    );
    push(&mut out, std::env::var_os("ProgramFiles(x86)"), "");
    push(&mut out, std::env::var_os("programw6432"), "");
    push(&mut out, std::env::var_os("LOCALAPPDATA"), "Programs");
    push(&mut out, std::env::var_os("ProgramData"), "Microsoft\\Windows\\Start Menu");
    out.retain(|p| p.is_dir());
    out.dedup();
    out
}

/// 单文件 exe 扫描的最大深度：Program Files\Foo\bin\app.exe 足够，再深基本是依赖。
pub const MAX_DEPTH: usize = 4;
/// 小于此体积的 exe 多半是 shim/helper。
const MIN_SIZE: u64 = 64 * 1024;

pub fn collect(roots: &[PathBuf], ignore: &IgnoreMatcher) -> Vec<AppEntry> {
    let mut out = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(|e| {
                // 根目录本身不过滤：调用方显式指定的目录（包括临时目录、隐藏目录）应被尊重
                if e.depth() == 0 {
                    return !ignore.is_ignored(e.path());
                }
                let name = e
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                if name.starts_with('$') || name.starts_with('.') {
                    return false;
                }
                if matches!(
                    name.as_str(),
                    "node_modules"
                        | "temp"
                        | "tmp"
                        | "bin64_arm"
                        | "locales"
                        | "resources"
                        | "plugins"
                        | "extensions"
                        | "packages"
                        | "assemblies"
                        | "winsxs"
                        | "serviceprofiles"
                ) {
                    return false;
                }
                !ignore.is_ignored(e.path())
            })
        {
            let Ok(de) = entry else { continue };
            if !de.file_type().is_file() {
                continue;
            }
            let path = de.path();
            let is_exe = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
            if !is_exe {
                continue;
            }
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if fsutil::looks_like_helper(&stem) {
                continue;
            }
            let size = de.metadata().map(|m| m.len()).unwrap_or(0);
            if size < MIN_SIZE {
                continue;
            }
            // 已经在开始菜单里的重复项由 Indexer 去重，这里不查注册表以免慢
            // 显示名保留原始大小写（RealApp.exe → RealApp），只做分隔符可读化
            let raw_stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = prettify_stem(&raw_stem);
            let mut e = AppEntry::new(
                name,
                LaunchTarget {
                    path: path.to_string_lossy().to_string(),
                    args: None,
                    working_dir: path.parent().map(|p| p.to_string_lossy().to_string()),
                    kind: TargetKind::Exe,
                },
                AppSource::DirScan,
            );
            e.icon = Some(e.target.path.clone());
            e.aliases = vec![stem];
            e.finalize();
            out.push(e);
        }
    }
    out
}

/// `chrome_elf` 已由 helper 过滤；这里只做可读化处理：`code` → `Code`。
fn prettify_stem(stem: &str) -> String {
    let mut out = String::with_capacity(stem.len());
    let mut upper = true;
    for c in stem.chars() {
        if c == '_' || c == '-' {
            out.push(' ');
            upper = true;
            continue;
        }
        if upper {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        upper = false;
    }
    out
}

/// Windows 自带工具，给出中文名与规范入口，避免只能搜到 notepad.exe。
pub fn system_tools() -> Vec<AppEntry> {
    const TOOLS: &[(&str, &str, &str, &[&str])] = &[
        ("记事本", "notepad.exe", "C:\\Windows\\System32\\notepad.exe", &["notepad", "文本编辑器"]),
        ("画图", "mspaint.exe", "C:\\Windows\\System32\\mspaint.exe", &["mspaint", "paint"]),
        ("计算器", "calc.exe", "C:\\Windows\\System32\\calc.exe", &["calc", "calculator"]),
        ("命令提示符", "cmd.exe", "C:\\Windows\\System32\\cmd.exe", &["cmd", "shell"]),
        (
            "Windows PowerShell",
            "powershell.exe",
            "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
            &["powershell", "pwsh"],
        ),
        ("任务管理器", "taskmgr.exe", "C:\\Windows\\System32\\taskmgr.exe", &["taskmgr", "任务"]),
        ("注册表编辑器", "regedit.exe", "C:\\Windows\\System32\\regedit.exe", &["regedit"]),
        ("设备管理器", "devmgmt.msc", "C:\\Windows\\System32\\devmgmt.msc", &["device manager"]),
        ("服务", "services.msc", "C:\\Windows\\System32\\services.msc", &["services"]),
        ("控制面板", "control.exe", "C:\\Windows\\System32\\control.exe", &["control panel", "面板"]),
        (
            "运行",
            "shell:AppsFolder\\Microsoft.Windows.Shell.RunCommand",
            "ms-settings:",
            &["run"],
        ),
        ("设置", "ms-settings:", "ms-settings:", &["settings", "windows 设置"]),
        ("资源监视器", "resmon.exe", "C:\\Windows\\System32\\resmon.exe", &["resmon"]),
        ("截图工具", "snippingtool.exe", "C:\\Windows\\System32\\SnippingTool.exe", &["snip", "截图"]),
        (
            "屏幕键盘",
            "osk.exe",
            "C:\\Windows\\System32\\osk.exe",
            &["osk", "keyboard"],
        ),
    ];

    let mut out = Vec::new();
    for (name, _exe, path, extra) in TOOLS {
        let p = Path::new(path);
        let exists = path.starts_with("ms-settings:") || p.exists();
        if !exists {
            continue;
        }
        let kind = if path.starts_with("ms-settings:") || path.contains("shell:") {
            TargetKind::Uri
        } else {
            TargetKind::Exe
        };
        let mut e = AppEntry::new(
            *name,
            LaunchTarget {
                path: path.to_string(),
                args: None,
                working_dir: None,
                kind,
            },
            AppSource::Manual,
        );
        // 系统工具以前不带图标，前端只能显示首字；URI 协议没有可读的 shell 图标
        if matches!(kind, TargetKind::Exe) {
            e.icon = Some(path.to_string());
        }
        e.aliases = extra.iter().map(|s| s.to_string()).collect();
        e.finalize();
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prettify() {
        assert_eq!(prettify_stem("chrome"), "Chrome");
        assert_eq!(prettify_stem("zoom_it"), "Zoom It");
        assert_eq!(prettify_stem("msedge"), "Msedge");
    }

    #[test]
    fn helper_exe_filtered() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("RealApp.exe"), vec![0u8; 100_000]).unwrap();
        std::fs::write(dir.path().join("unins000.exe"), vec![0u8; 100_000]).unwrap();
        std::fs::write(dir.path().join("tiny.exe"), vec![0u8; 100]).unwrap();
        let got = collect(
            &[dir.path().to_path_buf()],
            &IgnoreMatcher::default(),
        );
        let names: Vec<String> = got.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains(&"RealApp".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("unins") || n == "Tiny"));
    }

    #[test]
    fn system_tools_exist_on_this_machine() {
        let tools = system_tools();
        // 至少记事本/设置这类一定存在；不存在也不应报错
        assert!(!tools.is_empty());
        assert!(tools.iter().any(|t| t.name == "设置"));
    }
}
