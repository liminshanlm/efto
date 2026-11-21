//! 插件导入：把目录或 `.zip` 包复制到受管插件目录。
//!
//! 导入只做三件事：找到清单根、校验清单、整目录复制。运行期行为由 `efto-plugin.json` 决定。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::manifest::{has_manifest, load_descriptor};
use crate::error::{CoreError, Result};
use crate::fsutil;

/// 单个插件包解压后的体积上限，避免 zip 炸弹。
const MAX_EXTRACT_BYTES: u64 = 64 << 20;
const MAX_ENTRIES: usize = 5000;
const MARKER: &str = ".efto-import.json";

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub name: String,
    pub version: String,
    pub target: PathBuf,
    /// 清单校验发现的问题（例如没有 UI 入口）；导入本身仍然成功
    pub problems: Vec<String>,
}

/// 一次导入的结果。一个包里可以装多个插件，也允许部分失败。
#[derive(Debug, Clone, Serialize)]
pub struct ImportBatch {
    pub imported: Vec<ImportReport>,
    /// 包里没能导入的插件（重名、清单坏了…）。不影响其它插件，
    /// 全部失败时才整体报错。
    pub failed: Vec<ImportFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportFailure {
    /// 包内目录名（导入失败时可能拿不到清单里的 name）
    pub plugin: String,
    pub reason: String,
}

/// 导入插件包：目录或 `.zip`，包内可以有一个或多个插件。
///
/// 兼容两种打包方式：插件目录直接在包根（`ai-chat/efto-plugin.json`），
/// 或外面多套一层容器目录（`plugins/ai-chat/efto-plugin.json`）。
pub fn import_path(src: &Path, dest_root: &Path) -> Result<ImportBatch> {
    if !src.exists() {
        return Err(CoreError::InvalidManifest(format!(
            "导入路径不存在: {}",
            src.display()
        )));
    }
    fs::create_dir_all(dest_root).map_err(|e| CoreError::Io {
        path: dest_root.to_path_buf(),
        source: e,
    })?;

    let staging = dest_root.join(format!(
        ".import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));

    let result = (|| {
        let root = if src.is_dir() {
            src.to_path_buf()
        } else {
            extract_zip(src, &staging)?;
            staging.clone()
        };

        let roots = find_manifest_roots(&root)?;
        let mut imported = Vec::new();
        let mut failed = Vec::new();
        for dir in roots {
            match import_one(&dir, src, dest_root) {
                Ok(report) => imported.push(report),
                Err(e) => failed.push(ImportFailure {
                    plugin: dir
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| dir.display().to_string()),
                    reason: e.to_string(),
                }),
            }
        }
        if imported.is_empty() {
            let reasons: Vec<String> = failed
                .iter()
                .map(|f| format!("{}：{}", f.plugin, f.reason))
                .collect();
            return Err(CoreError::InvalidManifest(format!(
                "没有插件导入成功（{}）",
                reasons.join("；")
            )));
        }
        Ok(ImportBatch { imported, failed })
    })();

    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// 导入单个插件根目录到 `dest_root/<name>`。
fn import_one(root: &PathBuf, src: &Path, dest_root: &Path) -> Result<ImportReport> {
    let descriptor = load_descriptor(root)?;
    let name = fsutil::sanitize_key(descriptor.manifest.name.trim());
    if name.is_empty() || name == "." || name == ".." {
        return Err(CoreError::InvalidManifest("插件 name 不能作为目录名".into()));
    }

    let target = dest_root.join(&name);
    if target.exists() {
        return Err(CoreError::InvalidManifest(format!(
            "已存在同名插件 {}，请先在设置里删除后再导入",
            target.display()
        )));
    }

    copy_tree(root, &target)?;
    let marker = serde_json::json!({
        "source": src.display().to_string(),
    });
    fsutil::write_atomic(&target.join(MARKER), marker.to_string().as_bytes())?;

    Ok(ImportReport {
        name: descriptor.manifest.name.clone(),
        version: descriptor.manifest.version.clone(),
        target,
        problems: descriptor.problems.clone(),
    })
}

/// 找到包内所有插件根（含 `efto-plugin.json` 的目录）。
fn find_manifest_roots(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    collect_manifest_roots(dir, 0, &mut roots)?;
    if roots.is_empty() {
        return Err(CoreError::InvalidManifest(format!(
            "{} 里没有 efto-plugin.json",
            dir.display()
        )));
    }
    Ok(roots)
}

/// 向下找清单的最大层数：包根 → 容器目录 → 插件目录。
const MAX_SCAN_DEPTH: usize = 3;

/// 深度优先收集插件根。某个目录本身是插件就不再往里找——
/// 插件自己的子目录（assets 等）不该被当成插件。
fn collect_manifest_roots(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if has_manifest(dir) {
        out.push(dir.to_path_buf());
        return Ok(());
    }
    if depth >= MAX_SCAN_DEPTH {
        return Ok(());
    }
    let mut children: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| CoreError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    children.sort();
    for child in children {
        collect_manifest_roots(&child, depth + 1, out)?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(src).follow_links(false) {
        let entry = entry.map_err(|e| CoreError::Io {
            path: src.to_path_buf(),
            source: e.into(),
        })?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .map_err(|_| CoreError::InvalidManifest("插件路径越界".into()))?;
        let out = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&out).map_err(|e| CoreError::Io {
                path: out.clone(),
                source: e,
            })?;
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), &out).map_err(|e| CoreError::Io {
                path: out.clone(),
                source: e,
            })?;
        }
    }
    Ok(())
}

/// 解压 zip 到 `staging`。只支持 store / deflate 两种压缩方式。
fn extract_zip(src: &Path, staging: &Path) -> Result<()> {
    let bytes = fs::read(src).map_err(|e| CoreError::Io {
        path: src.to_path_buf(),
        source: e,
    })?;
    let eocd = bytes
        .windows(4)
        .rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| CoreError::InvalidManifest("不是有效的 zip 包".into()))?;
    if eocd + 22 > bytes.len() {
        return Err(CoreError::InvalidManifest("zip 结束标记损坏".into()));
    }
    let rd16 = |o: usize| u16::from_le_bytes([bytes[eocd + o], bytes[eocd + o + 1]]) as usize;
    let rd32 = |o: usize| {
        u32::from_le_bytes([
            bytes[eocd + o],
            bytes[eocd + o + 1],
            bytes[eocd + o + 2],
            bytes[eocd + o + 3],
        ]) as usize
    };
    let entries = rd16(10);
    if entries > MAX_ENTRIES {
        return Err(CoreError::InvalidManifest("插件包文件数过多".into()));
    }
    let mut off = rd32(16);
    let mut total: u64 = 0;

    for _ in 0..entries {
        if off + 46 > bytes.len() || bytes[off..off + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err(CoreError::InvalidManifest("zip 中央目录损坏".into()));
        }
        let le16 = |o: usize| u16::from_le_bytes([bytes[off + o], bytes[off + o + 1]]) as usize;
        let le32 = |o: usize| {
            u32::from_le_bytes([
                bytes[off + o],
                bytes[off + o + 1],
                bytes[off + o + 2],
                bytes[off + o + 3],
            ]) as usize
        };
        let method = le16(10);
        let comp_size = le32(20);
        let uncomp_size = le32(24);
        let name_len = le16(28);
        let extra_len = le16(30);
        let comment_len = le16(32);
        let local_off = le32(42);
        let raw_name = bytes
            .get(off + 46..off + 46 + name_len)
            .ok_or_else(|| CoreError::InvalidManifest("zip 文件名越界".into()))?;
        let name = String::from_utf8_lossy(raw_name).replace('\\', "/");
        off += 46 + name_len + extra_len + comment_len;

        if name.ends_with('/') {
            continue;
        }
        total += uncomp_size as u64;
        if total > MAX_EXTRACT_BYTES {
            return Err(CoreError::InvalidManifest("插件包解压后体积过大".into()));
        }
        let rel = safe_rel_path(&name)?;
        if local_off + 30 > bytes.len() || bytes[local_off..local_off + 4] != [0x50, 0x4b, 0x03, 0x04]
        {
            return Err(CoreError::InvalidManifest("zip 本地文件头损坏".into()));
        }
        let l_name = u16::from_le_bytes([bytes[local_off + 26], bytes[local_off + 27]]) as usize;
        let l_extra = u16::from_le_bytes([bytes[local_off + 28], bytes[local_off + 29]]) as usize;
        let data_start = local_off + 30 + l_name + l_extra;
        let raw = bytes
            .get(data_start..data_start + comp_size)
            .ok_or_else(|| CoreError::InvalidManifest("zip 数据越界".into()))?;
        let content = match method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(raw)
                    .read_to_end(&mut out)
                    .map_err(|e| CoreError::InvalidManifest(format!("zip 解压失败: {e}")))?;
                out
            }
            m => {
                return Err(CoreError::InvalidManifest(format!(
                    "不支持的 zip 压缩方式 {m}"
                )))
            }
        };
        let out = staging.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| CoreError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        fs::write(&out, content).map_err(|e| CoreError::Io {
            path: out.clone(),
            source: e,
        })?;
    }
    Ok(())
}

/// 拒绝绝对路径、盘符和 `..`，保证解压不逃出 staging。
fn safe_rel_path(name: &str) -> Result<PathBuf> {
    let p = Path::new(name);
    let bad = p.is_absolute()
        || name.contains(':')
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    if bad {
        return Err(CoreError::InvalidManifest(format!("zip 内路径越界: {name}")));
    }
    Ok(p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_root_inside_wrapper_dir() {
        let src = tempfile::tempdir().unwrap();
        let inner = src.path().join("wrapper");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("efto-plugin.json"), r#"{"name":"native","main":"main.js"}"#)
            .unwrap();
        std::fs::write(inner.join("main.js"), "").unwrap();
        let dest = tempfile::tempdir().unwrap();
        let report = import_path(src.path(), dest.path()).unwrap();
        assert_eq!(report.imported.len(), 1);
        assert_eq!(report.imported[0].name, "native");
        assert!(dest.path().join("native/efto-plugin.json").is_file());
    }

    #[test]
    fn refuses_existing_target() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("efto-plugin.json"), r#"{"name":"dup"}"#).unwrap();
        std::fs::write(src.path().join("main.js"), "").unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dest.path().join("dup")).unwrap();
        assert!(import_path(src.path(), dest.path()).is_err());
    }

    #[test]
    fn rejects_missing_manifest() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("readme.txt"), "x").unwrap();
        let dest = tempfile::tempdir().unwrap();
        assert!(import_path(src.path(), dest.path()).is_err());
    }

    #[test]
    fn imports_store_only_zip() {
        let zip = build_store_zip(&[
            ("efto-plugin.json", br#"{"name":"zipped","main":"main.js"}"#),
            ("main.js", b"exports.onPluginEnter=()=>{}"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("zipped.zip");
        std::fs::write(&pkg, zip).unwrap();
        let dest = tempfile::tempdir().unwrap();
        let report = import_path(&pkg, dest.path()).unwrap();
        assert_eq!(report.imported.len(), 1);
        assert_eq!(report.imported[0].name, "zipped");
        assert!(dest.path().join("zipped/main.js").is_file());
    }

    #[test]
    fn imports_multiple_plugins_at_package_root() {
        // plugins1.zip 的布局：插件目录直接在包根
        let zip = build_store_zip(&[
            ("alpha/efto-plugin.json", br#"{"name":"alpha"}"#),
            ("alpha/main.js", b""),
            ("beta/efto-plugin.json", br#"{"name":"beta"}"#),
            ("beta/main.js", b""),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("plugins1.zip");
        std::fs::write(&pkg, zip).unwrap();
        let dest = tempfile::tempdir().unwrap();
        let report = import_path(&pkg, dest.path()).unwrap();
        let names: Vec<&str> = report.imported.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(report.failed.is_empty());
        assert!(dest.path().join("alpha/main.js").is_file());
        assert!(dest.path().join("beta/main.js").is_file());
    }

    #[test]
    fn imports_multiple_plugins_inside_wrapper_dir() {
        // plugins.zip 的布局：外面多套一层容器目录
        let zip = build_store_zip(&[
            ("plugins/alpha/efto-plugin.json", br#"{"name":"alpha"}"#),
            ("plugins/alpha/main.js", b""),
            ("plugins/beta/efto-plugin.json", br#"{"name":"beta"}"#),
            ("plugins/beta/main.js", b""),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("plugins.zip");
        std::fs::write(&pkg, zip).unwrap();
        let dest = tempfile::tempdir().unwrap();
        let report = import_path(&pkg, dest.path()).unwrap();
        assert_eq!(report.imported.len(), 2);
        assert!(dest.path().join("alpha/efto-plugin.json").is_file());
        assert!(dest.path().join("beta/efto-plugin.json").is_file());
    }

    #[test]
    fn partial_failure_keeps_other_plugins() {
        let zip = build_store_zip(&[
            ("alpha/efto-plugin.json", br#"{"name":"alpha"}"#),
            ("dup/efto-plugin.json", br#"{"name":"dup"}"#),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("mixed.zip");
        std::fs::write(&pkg, zip).unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dest.path().join("dup")).unwrap();
        let report = import_path(&pkg, dest.path()).unwrap();
        assert_eq!(report.imported.len(), 1);
        assert_eq!(report.imported[0].name, "alpha");
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].plugin, "dup");
    }

    /// 拼一个只读测试用的 store（method 0）zip：本地头 + 中央目录 + EOCD。
    fn build_store_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut local = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = local.len() as u32;
            local.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
            local.extend_from_slice(&20u16.to_le_bytes());
            local.extend_from_slice(&0u16.to_le_bytes());
            local.extend_from_slice(&0u16.to_le_bytes());
            local.extend_from_slice(&0u16.to_le_bytes());
            local.extend_from_slice(&0u16.to_le_bytes());
            local.extend_from_slice(&0u32.to_le_bytes());
            local.extend_from_slice(&(data.len() as u32).to_le_bytes());
            local.extend_from_slice(&(data.len() as u32).to_le_bytes());
            local.extend_from_slice(&(name.len() as u16).to_le_bytes());
            local.extend_from_slice(&0u16.to_le_bytes());
            local.extend_from_slice(name.as_bytes());
            local.extend_from_slice(data);

            central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = local.len() as u32;
        local.extend_from_slice(&central);
        local.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&(files.len() as u16).to_le_bytes());
        local.extend_from_slice(&(files.len() as u16).to_le_bytes());
        local.extend_from_slice(&(central.len() as u32).to_le_bytes());
        local.extend_from_slice(&cd_offset.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local
    }

    #[test]
    fn zip_paths_cannot_escape() {
        assert!(safe_rel_path("../evil.js").is_err());
        assert!(safe_rel_path("C:/evil.js").is_err());
        assert!(safe_rel_path("assets/a.js").is_ok());
    }
}
