//! 应用感知的快捷键指南数据。
//!
//! 每个应用一份 JSON，构建时用 `include_str!` 嵌入二进制，离线可用。
//! 用户可以在 `~/.efto/shortcuts/<exe>.json` 覆盖或补充：同名分组按
//! `keys` 合并（用户优先），新分组直接追加；单个文件损坏只影响它自己。

use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// 应用身份：可执行文件名（小写比较）与显示名。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutApp {
    pub exe: Vec<String>,
    pub name: String,
}

/// 一条快捷键。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutItem {
    pub keys: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 一个分类（文件/编辑/视图…）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutGroup {
    pub name: String,
    pub items: Vec<ShortcutItem>,
}

/// 一个应用的完整快捷键集。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutSet {
    pub app: ShortcutApp,
    /// 适用平台：`win32` / `darwin`（与插件清单 `platform` 同词汇）。
    /// 缺省 = 全平台（用户导入数据的默认）。内置数据按平台各备一份、共用 id，
    /// 查询侧只放行当前平台的那条。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    pub groups: Vec<ShortcutGroup>,
}

/// 数据集是否适用于当前平台。
fn os_matches(os: Option<&str>) -> bool {
    match os {
        None => true,
        Some(v) => {
            let cur = if cfg!(windows) {
                "win32"
            } else if cfg!(target_os = "macos") {
                "darwin"
            } else {
                "linux"
            };
            v.eq_ignore_ascii_case(cur)
        }
    }
}

/// 未识别到应用时的通用数据 id。
fn generic_id() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    }
}

/// 下拉选择器里的一项。
#[derive(Debug, Clone, Serialize)]
pub struct ShortcutEntry {
    pub id: String,
    pub name: String,
    pub exe: Vec<String>,
}

/// 应用识别结果 + 实际展示的数据。
#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    /// 当前数据源 id（`windows` 表示通用数据）。
    pub id: String,
    /// 唤起前的前台进程名；没识别到时为空。
    pub exe: Option<String>,
    /// 识别到的应用名；未收录时为 `None`，前端据此展示提示。
    pub name: Option<String>,
    /// 是否回退到平台通用快捷键。
    pub fallback: bool,
    pub set: ShortcutSet,
}

/// 内置数据：id 与 JSON 源。id 也是下拉选择的键。
/// 同一 id 允许多平台各一份（`os` 字段区分），查询侧只放行当前平台的那条。
const BUILTINS: &[(&str, &str)] = &[
    ("windows", include_str!("data/windows.json")),
    ("macos", include_str!("data/macos.json")),
    ("word", include_str!("data/word.json")),
    ("word", include_str!("data/word.macos.json")),
    ("excel", include_str!("data/excel.json")),
    ("excel", include_str!("data/excel.macos.json")),
    ("powerpoint", include_str!("data/powerpoint.json")),
    ("powerpoint", include_str!("data/powerpoint.macos.json")),
    ("outlook", include_str!("data/outlook.json")),
    ("outlook", include_str!("data/outlook.macos.json")),
    ("chrome", include_str!("data/chrome.json")),
    ("chrome", include_str!("data/chrome.macos.json")),
    ("edge", include_str!("data/edge.json")),
    ("edge", include_str!("data/edge.macos.json")),
    ("firefox", include_str!("data/firefox.json")),
    ("firefox", include_str!("data/firefox.macos.json")),
    ("vscode", include_str!("data/vscode.json")),
    ("vscode", include_str!("data/vscode.macos.json")),
    ("photoshop", include_str!("data/photoshop.json")),
    ("photoshop", include_str!("data/photoshop.macos.json")),
    ("figma", include_str!("data/figma.json")),
    ("figma", include_str!("data/figma.macos.json")),
    ("idea", include_str!("data/idea.json")),
    ("idea", include_str!("data/idea.macos.json")),
    ("trae", include_str!("data/trae.json")),
    ("trae", include_str!("data/trae.macos.json")),
];

/// 解析全部内置数据（只做一次）。解析失败会打日志，测试会单独校验。
static PARSED: Lazy<Vec<(String, ShortcutSet)>> = Lazy::new(|| {
    BUILTINS
        .iter()
        .filter_map(|(id, raw)| match serde_json::from_str::<ShortcutSet>(raw) {
            Ok(set) => Some(((*id).to_string(), set)),
            Err(e) => {
                tracing::error!("内置快捷键数据 {id}.json 解析失败: {e}");
                None
            }
        })
        .collect()
});

fn normalize_exe(exe: &str) -> String {
    exe.trim().to_ascii_lowercase()
}

fn user_dir() -> PathBuf {
    crate::fsutil::data_root().join("shortcuts")
}

/// 读取用户覆盖数据；文件不存在或损坏都返回 `None`（后者打日志）。
fn user_override(exe: &str) -> Option<ShortcutSet> {
    let path = user_dir().join(format!("{}.json", normalize_exe(exe)));
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<ShortcutSet>(&raw) {
        Ok(set) => Some(set),
        Err(e) => {
            tracing::warn!("用户快捷键数据 {} 解析失败，忽略: {e}", path.display());
            None
        }
    }
}

/// 用户数据合并进内置数据：同名分组按 `keys` 覆盖，新分组/新条目追加。
fn merge(base: &mut ShortcutSet, user: ShortcutSet) {
    for group in user.groups {
        if let Some(existing) = base.groups.iter_mut().find(|g| g.name == group.name) {
            for item in group.items {
                if let Some(slot) = existing
                    .items
                    .iter_mut()
                    .find(|i| i.keys.eq_ignore_ascii_case(&item.keys))
                {
                    *slot = item;
                } else {
                    existing.items.push(item);
                }
            }
        } else {
            base.groups.push(group);
        }
    }
}

/// 导入结果：CLI / agent 据此确认写了哪个文件、加了多少、改了多少。
#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub path: String,
    pub exe: String,
    pub name: String,
    pub groups: usize,
    pub added: usize,
    pub updated: usize,
    pub dry_run: bool,
    pub message: String,
}

fn validate(set: &ShortcutSet) -> Result<(), String> {
    if set.app.name.trim().is_empty() {
        return Err("app.name 不能为空".into());
    }
    if set.app.exe.iter().all(|e| e.trim().is_empty()) {
        return Err("app.exe 不能为空".into());
    }
    if set.groups.is_empty() {
        return Err("groups 不能为空".into());
    }
    for group in &set.groups {
        if group.name.trim().is_empty() {
            return Err("分组名不能为空".into());
        }
        if group.items.is_empty() {
            return Err(format!("分组「{}」没有条目", group.name));
        }
        let mut seen = std::collections::HashSet::new();
        for item in &group.items {
            if item.keys.trim().is_empty() {
                return Err(format!("分组「{}」有空 keys", group.name));
            }
            if item.action.trim().is_empty() {
                return Err(format!("分组「{}」的 {} 缺少 action", group.name, item.keys));
            }
            if !seen.insert(item.keys.to_ascii_lowercase()) {
                return Err(format!("分组「{}」里 keys {} 重复", group.name, item.keys));
            }
        }
    }
    Ok(())
}

fn validate_exe(exe: &str) -> Result<(), String> {
    if exe.is_empty() || exe == "." || exe == ".." {
        return Err("exe 不能为空".into());
    }
    if exe
        .chars()
        .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return Err(format!("exe 不是合法文件名: {exe}"));
    }
    Ok(())
}

fn count_changes(base: &ShortcutSet, incoming: &ShortcutSet) -> (usize, usize) {
    let mut added = 0;
    let mut updated = 0;
    for group in &incoming.groups {
        let existing = base.groups.iter().find(|g| g.name == group.name);
        for item in &group.items {
            match existing.and_then(|g| {
                g.items
                    .iter()
                    .find(|i| i.keys.eq_ignore_ascii_case(&item.keys))
            }) {
                Some(_) => updated += 1,
                None => added += 1,
            }
        }
    }
    (added, updated)
}

/// 把一份快捷键数据导入 `~/.efto/shortcuts/<exe>.json`。
///
/// 与既有文件按「同名分组 + 相同 keys」合并（导入数据优先），不会清掉其他分组；
/// 同一份数据重复导入是幂等的（`updated` 累加、`added` 为 0）。
pub fn import_set(
    set: ShortcutSet,
    target_exe: Option<&str>,
    dry_run: bool,
) -> Result<ImportReport, String> {
    import_into(&user_dir(), set, target_exe, dry_run)
}

fn import_into(
    dir: &Path,
    set: ShortcutSet,
    target_exe: Option<&str>,
    dry_run: bool,
) -> Result<ImportReport, String> {
    validate(&set)?;
    let raw_exe = target_exe
        .map(str::to_string)
        .or_else(|| {
            set.app
                .exe
                .iter()
                .map(|e| e.trim().to_string())
                .find(|e| !e.is_empty())
        })
        .ok_or_else(|| "app.exe 不能为空".to_string())?;
    let exe = normalize_exe(&raw_exe);
    validate_exe(&exe)?;
    let path = dir.join(format!("{exe}.json"));

    let mut target = match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<ShortcutSet>(&raw)
            .map_err(|e| format!("{} 解析失败，拒绝覆盖: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ShortcutSet {
            app: ShortcutApp {
                exe: vec![exe.clone()],
                name: set.app.name.trim().to_string(),
            },
            os: None,
            groups: Vec::new(),
        },
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };

    let (added, updated) = count_changes(&target, &set);
    let name = set.app.name.trim().to_string();
    merge(&mut target, set);
    target.app.exe = vec![exe.clone()];
    target.app.name = name;

    let groups = target.groups.len();
    if !dry_run {
        let json =
            serde_json::to_string_pretty(&target).map_err(|e| format!("序列化失败: {e}"))?;
        crate::fsutil::write_atomic(&path, json.as_bytes())
            .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    }
    let message = format!(
        "{}导入 {} 个分组（新增 {} 条，更新 {} 条）→ {}",
        if dry_run { "[dry-run] 将" } else { "已" },
        groups,
        added,
        updated,
        path.display()
    );
    Ok(ImportReport {
        path: path.display().to_string(),
        exe,
        name: target.app.name,
        groups,
        added,
        updated,
        dry_run,
        message,
    })
}

/// 所有内置应用（供下拉选择；不含用户新增的应用）。
pub fn apps() -> Vec<ShortcutEntry> {
    PARSED
        .iter()
        .filter(|(_, set)| os_matches(set.os.as_deref()))
        .map(|(id, set)| ShortcutEntry {
            id: id.clone(),
            name: set.app.name.clone(),
            exe: set.app.exe.clone(),
        })
        .collect()
}

/// 按 id 取数据（合并该应用所有 exe 的用户覆盖）。同 id 的多平台数据里
/// 只取当前平台的那条。
pub fn by_id(id: &str) -> Option<ShortcutSet> {
    let (_, base) = PARSED
        .iter()
        .find(|(key, set)| key == id && os_matches(set.os.as_deref()))?;
    let mut set = base.clone();
    for exe in set.app.exe.clone() {
        if let Some(user) = user_override(&exe) {
            merge(&mut set, user);
        }
    }
    Some(set)
}

/// 按 exe 取数据（大小写不敏感）。内置没有时也接受用户自建数据。
pub fn by_exe(exe: &str) -> Option<ShortcutSet> {
    let needle = normalize_exe(exe);
    if needle.is_empty() {
        return None;
    }
    let builtin = PARSED.iter().find(|(_, set)| {
        os_matches(set.os.as_deref())
            && set
                .app
                .exe
                .iter()
                .any(|e| normalize_exe(e) == needle)
    });
    if let Some((_, base)) = builtin {
        let mut set = base.clone();
        for app_exe in set.app.exe.clone() {
            if let Some(user) = user_override(&app_exe) {
                merge(&mut set, user);
            }
        }
        return Some(set);
    }
    user_override(&needle)
}

/// 解析"唤起前的应用"：未收录时回退到平台通用数据。
pub fn resolve(exe: Option<&str>) -> Resolved {
    let generic = by_id(generic_id()).unwrap_or_else(|| ShortcutSet {
        app: ShortcutApp {
            exe: vec![],
            name: if cfg!(target_os = "macos") {
                "macOS".into()
            } else {
                "Windows".into()
            },
        },
        os: None,
        groups: vec![],
    });
    let Some(raw) = exe.filter(|e| !e.trim().is_empty()) else {
        return Resolved {
            id: generic_id().into(),
            exe: None,
            name: None,
            fallback: true,
            set: generic,
        };
    };
    match by_exe(raw) {
        Some(set) => Resolved {
            id: PARSED
                .iter()
                .find(|(_, s)| {
                    os_matches(s.os.as_deref())
                        && s.app
                            .exe
                            .iter()
                            .any(|e| normalize_exe(e) == normalize_exe(raw))
                })
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| "custom".into()),
            exe: Some(raw.to_string()),
            name: Some(set.app.name.clone()),
            fallback: false,
            set,
        },
        None => Resolved {
            id: generic_id().into(),
            exe: Some(raw.to_string()),
            name: None,
            fallback: true,
            set: generic,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_json_parses() {
        for (id, raw) in BUILTINS {
            let set: ShortcutSet =
                serde_json::from_str(raw).unwrap_or_else(|e| panic!("{id}.json 解析失败: {e}"));
            assert!(!set.app.exe.is_empty(), "{id} 缺少 exe");
            assert!(!set.app.name.trim().is_empty(), "{id} 缺少应用名");
            assert!(!set.groups.is_empty(), "{id} 没有分组");
            for group in &set.groups {
                assert!(!group.name.trim().is_empty(), "{id} 有空分组名");
                assert!(!group.items.is_empty(), "{id} 分组 {} 为空", group.name);
                let mut seen = std::collections::HashSet::new();
                for item in &group.items {
                    assert!(!item.keys.trim().is_empty(), "{id} 有空的 keys");
                    assert!(
                        !item.action.trim().is_empty(),
                        "{id} 的 {} 缺少 action",
                        item.keys
                    );
                    assert!(
                        seen.insert(item.keys.to_ascii_lowercase()),
                        "{id} 分组 {} 里 keys {} 重复",
                        group.name,
                        item.keys
                    );
                }
            }
        }
    }

    #[test]
    fn exe_mapping_is_unique_across_apps() {
        let mut seen = std::collections::HashSet::new();
        for (id, raw) in BUILTINS {
            let set: ShortcutSet = serde_json::from_str(raw).unwrap();
            for exe in &set.app.exe {
                assert!(
                    seen.insert(normalize_exe(exe)),
                    "{exe} 同时出现在多个应用里（最后一个: {id}）"
                );
            }
        }
    }

    /// 同一 id 允许多平台各一份数据；同一平台上不允许重复。
    #[test]
    fn ids_are_unique_per_platform() {
        let mut seen = std::collections::HashSet::new();
        for (id, raw) in BUILTINS {
            let set: ShortcutSet = serde_json::from_str(raw).unwrap();
            if !os_matches(set.os.as_deref()) {
                continue;
            }
            assert!(seen.insert(*id), "同一平台上重复的应用 id: {id}");
        }
    }

    #[test]
    fn os_matches_known_values() {
        assert!(os_matches(None));
        assert!(os_matches(Some("win32")) != os_matches(Some("darwin")) || cfg!(linux));
        let cur = if cfg!(windows) {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        assert!(os_matches(Some(cur)));
        assert!(!os_matches(Some("plan9")));
    }

    #[test]
    fn lookup_is_case_insensitive() {
        #[cfg(windows)]
        {
            assert!(by_exe("WINWORD.EXE").is_some());
            assert!(by_exe("chrome.exe").is_some());
        }
        #[cfg(target_os = "macos")]
        {
            assert!(by_exe("MICROSOFT WORD").is_some());
            assert!(by_exe("google chrome").is_some());
        }
        assert!(by_exe("").is_none());
        assert!(by_exe("not-a-real-app").is_none());
    }

    #[test]
    fn unknown_app_falls_back_to_generic() {
        let r = resolve(Some("not-a-real-app"));
        assert!(r.fallback);
        assert_eq!(r.id, generic_id());
        assert!(r.name.is_none());
        assert!(!r.set.groups.is_empty());
    }

    #[test]
    fn known_app_reports_its_id_and_name() {
        // 同 id 多平台各一份：输入取本平台的进程名
        #[cfg(windows)]
        let r = resolve(Some("Code.exe"));
        #[cfg(target_os = "macos")]
        let r = resolve(Some("Code"));
        #[cfg(not(any(windows, target_os = "macos")))]
        let r = resolve(Some("Code.exe"));
        assert!(!r.fallback);
        assert_eq!(r.id, "vscode");
        assert!(r.name.is_some());
    }

    #[test]
    fn no_foreground_uses_generic_without_exe() {
        let r = resolve(None);
        assert!(r.fallback);
        assert!(r.exe.is_none());
        assert_eq!(r.id, generic_id());
    }

    #[test]
    fn user_data_wins_on_same_keys_and_appends_new() {
        let mut base = ShortcutSet {
            app: ShortcutApp {
                exe: vec!["demo.exe".into()],
                name: "Demo".into(),
            },
            os: None,
            groups: vec![ShortcutGroup {
                name: "文件".into(),
                items: vec![
                    ShortcutItem {
                        keys: "Ctrl+S".into(),
                        action: "保存".into(),
                        note: None,
                    },
                    ShortcutItem {
                        keys: "Ctrl+O".into(),
                        action: "打开".into(),
                        note: None,
                    },
                ],
            }],
        };
        merge(
            &mut base,
            ShortcutSet {
                app: ShortcutApp {
                    exe: vec!["demo.exe".into()],
                    name: "Demo".into(),
                },
                os: None,
                groups: vec![
                    ShortcutGroup {
                        name: "文件".into(),
                        items: vec![ShortcutItem {
                            keys: "ctrl+s".into(),
                            action: "保存全部".into(),
                            note: None,
                        }],
                    },
                    ShortcutGroup {
                        name: "自定义".into(),
                        items: vec![ShortcutItem {
                            keys: "Ctrl+Shift+X".into(),
                            action: "扩展命令".into(),
                            note: None,
                        }],
                    },
                ],
            },
        );
        let file = base.groups.iter().find(|g| g.name == "文件").unwrap();
        assert_eq!(file.items.len(), 2);
        assert_eq!(
            file.items
                .iter()
                .find(|i| i.keys.eq_ignore_ascii_case("ctrl+s"))
                .unwrap()
                .action,
            "保存全部"
        );
        assert!(base.groups.iter().any(|g| g.name == "自定义"));
    }

    fn sample_set() -> ShortcutSet {
        ShortcutSet {
            app: ShortcutApp {
                exe: vec!["demo.exe".into()],
                name: "Demo".into(),
            },
            os: None,
            groups: vec![ShortcutGroup {
                name: "文件".into(),
                items: vec![ShortcutItem {
                    keys: "Ctrl+S".into(),
                    action: "保存".into(),
                    note: None,
                }],
            }],
        }
    }

    #[test]
    fn import_writes_and_merges_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let first = import_into(dir.path(), sample_set(), None, false).unwrap();
        assert_eq!((first.groups, first.added, first.updated), (1, 1, 0));
        assert!(dir.path().join("demo.exe.json").exists());

        let mut second = sample_set();
        second.groups[0].items[0].action = "保存全部".into();
        second.groups.push(ShortcutGroup {
            name: "编辑".into(),
            items: vec![ShortcutItem {
                keys: "Ctrl+Z".into(),
                action: "撤销".into(),
                note: None,
            }],
        });
        let report = import_into(dir.path(), second, None, false).unwrap();
        assert_eq!((report.groups, report.added, report.updated), (2, 1, 1));

        let saved: ShortcutSet =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("demo.exe.json")).unwrap())
                .unwrap();
        assert_eq!(saved.groups.len(), 2);
        assert_eq!(
            saved
                .groups
                .iter()
                .find(|g| g.name == "文件")
                .unwrap()
                .items[0]
                .action,
            "保存全部"
        );
    }

    #[test]
    fn import_rejects_bad_data_and_dry_run_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut dup = sample_set();
        dup.groups[0].items.push(ShortcutItem {
            keys: "ctrl+s".into(),
            action: "重复".into(),
            note: None,
        });
        assert!(import_into(dir.path(), dup, None, false).is_err());

        assert!(import_into(dir.path(), sample_set(), Some("../evil.exe"), false).is_err());
        assert!(!dir.path().join("evil.exe.json").exists());

        let dry = import_into(dir.path(), sample_set(), None, true).unwrap();
        assert!(dry.dry_run);
        assert!(!dir.path().join("demo.exe.json").exists());
    }
}
