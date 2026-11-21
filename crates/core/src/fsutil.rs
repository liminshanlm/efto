//! 跨平台小工具：路径判定、原子写、应用数据目录。

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// 应用数据根目录：用户主目录下的 `.efto`（Windows: `C:\Users\<name>\.efto`）。
///
/// 不再使用 `%APPDATA%\efto`。
/// 旧目录里的数据不会自动搬过来；需要保留时请手动复制到 `~/.efto`。
pub fn data_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".efto")
}

/// 原子写：先写 `*.tmp` 再 rename，避免进程被杀时留下半个 JSON。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| CoreError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
    }
    let tmp = path.with_extension(format!("{}.tmp", ext_of(path)));
    fs::write(&tmp, bytes).map_err(|e| CoreError::Io {
        path: tmp.clone(),
        source: e,
    })?;
    if path.exists() {
        fs::remove_file(path).ok();
    }
    fs::rename(&tmp, path).map_err(|e| CoreError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("dat")
        .to_string()
}

pub fn read_to_string_opt(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// 去掉 Windows 文件名非法字符，用于生成安全的目录名。
pub fn sanitize_key(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}

/// 是否像一个可执行体（Windows 优先）。
pub fn is_executable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const EXTS: &[&str] = &[
        ".exe", ".bat", ".cmd", ".com", ".msi", ".lnk", ".url", ".ps1", ".vbs",
    ];
    EXTS.iter().any(|e| lower.ends_with(e))
}

/// 明显不该出现在启动器里的辅助程序名（uninstaller、crashpad、updater 等）。
pub fn looks_like_helper(stem_lower: &str) -> bool {
    const BLACKLIST: &[&str] = &[
        "unins",
        "uninstall",
        "uninst",
        "crashpad",
        "crashreport",
        "bugreport",
        "updater",
        "update_",
        "installer",
        "setup_",
        "redist",
        "vcredist",
        "dotnet",
        "netfx",
        "helper_",
        "_service",
        "service_",
        "background",
        "tray_",
        "notification",
        "elevation",
        "reporter",
        "telemetry",
        "gcoder",
        "sqlite3",
        "libegl",
        "libgles",
        "opengl32",
        "concrt",
        "msvcp",
        "vcruntime",
        "ucrtbase",
        "api-ms",
        "api-ms-win",
        "mfc",
        "node-pty",
        "spawn-helper",
        "chrome_crashpad",
        "crash_renderer",
    ];
    if BLACKLIST.iter().any(|p| stem_lower.contains(p)) {
        return true;
    }
    // 单字符 / 纯数字 / 过长随机名基本是内部模块
    stem_lower.len() <= 2 || stem_lower.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_filter_works() {
        assert!(looks_like_helper("unins000"));
        assert!(looks_like_helper("crashpad_handler"));
        assert!(looks_like_helper("a"));
        assert!(!looks_like_helper("code"));
        assert!(!looks_like_helper("chrome"));
    }

    #[test]
    fn atomic_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        write_atomic(&p, b"{\"a\":1}").unwrap();
        assert_eq!(read_to_string_opt(&p).unwrap(), "{\"a\":1}");
        write_atomic(&p, b"{\"a\":2}").unwrap();
        assert_eq!(read_to_string_opt(&p).unwrap(), "{\"a\":2}");
        assert!(!dir.path().join("x.tmp").exists());
    }
}
