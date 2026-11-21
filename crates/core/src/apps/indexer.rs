//! 索引：三渠道汇总 → 去重 → 派生匹配文本 → 落盘缓存。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::model::{AppEntry, AppSource, LaunchTarget};
#[cfg(target_os = "macos")]
use super::appbundle;
use super::{registry, scanner, startmenu};
use crate::config::Settings;
use crate::error::Result;
use crate::fsutil;
use crate::search::service::usage_bonus_from;
use crate::search::{HitPayload, SearchHit};
use crate::usage::UsageStore;
use crate::ItemKind;

/// 缓存结构版本，改字段就升它，避免加载旧格式产生怪结果。
const CACHE_VERSION: u32 = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub start_menu: usize,
    /// 桌面快捷方式（用户级 + 公共桌面）。旧缓存没有这个字段。
    #[serde(default)]
    pub desktop: usize,
    pub registry: usize,
    pub dir_scan: usize,
    /// macOS 的 .app bundle 数量（Windows 上恒为 0）。旧缓存没有这个字段。
    #[serde(default)]
    pub app_bundle: usize,
    pub system: usize,
    pub merged: usize,
    pub dropped_duplicates: usize,
    pub elapsed_ms: u128,
    pub built_at: i64,
}

impl Default for ScanReport {
    fn default() -> Self {
        Self {
            start_menu: 0,
            desktop: 0,
            registry: 0,
            dir_scan: 0,
            app_bundle: 0,
            system: 0,
            merged: 0,
            dropped_duplicates: 0,
            elapsed_ms: 0,
            built_at: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    report: ScanReport,
    entries: Vec<AppEntry>,
}

/// 内存索引。查询走线性扫描（千级条目 < 1ms），够用且零维护成本。
#[derive(Debug, Clone, Default)]
pub struct AppIndex {
    entries: Vec<AppEntry>,
    by_id: HashMap<String, usize>,
    dropped_duplicates: usize,
}

impl AppIndex {
    pub fn empty() -> Self {
        Self::default()
    }

    /// 去重 + finalize。同名不同路径的都保留。
    pub fn from_entries(raw: Vec<AppEntry>) -> Self {
        // 按"实际 exe"归并：开始菜单 > 注册表 > 手动 > 目录扫描
        let mut best: HashMap<String, AppEntry> = HashMap::new();
        let mut dropped = 0usize;

        for mut e in raw {
            e.finalize();
            let key = e.resolve_key();
            match best.get_mut(&key) {
                None => {
                    best.insert(key, e);
                }
                Some(cur) => {
                    let keep_new = e.source.prior() > cur.source.prior()
                        || (e.source.prior() == cur.source.prior()
                            && e.target.path.len() < cur.target.path.len());
                    if keep_new {
                        std::mem::swap(cur, &mut e);
                    }
                    // 被舍弃那条的别名并进胜者
                    for alias in e.aliases.drain(..) {
                        if !cur.aliases.iter().any(|a| a.eq_ignore_ascii_case(&alias)) {
                            cur.aliases.push(alias);
                        }
                    }
                    if !cur.aliases.iter().any(|a| a.eq_ignore_ascii_case(&e.name)) {
                        cur.aliases.push(e.name.clone());
                    }
                    if cur.icon.is_none() {
                        cur.icon = e.icon.clone();
                    }
                    dropped += 1;
                }
            }
        }

        // 别名并进来后要重建派生文本
        let mut entries: Vec<AppEntry> = best
            .into_values()
            .map(|mut e| {
                e.finalize();
                e
            })
            .collect();
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let mut by_id = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            by_id.insert(e.id.clone(), i);
        }
        Self {
            entries,
            by_id,
            dropped_duplicates: dropped,
        }
    }

    /// 被合并掉的重复条目数（同一 exe 的开始菜单项与目录扫描项算一条）。
    pub fn dropped_duplicates(&self) -> usize {
        self.dropped_duplicates
    }

    pub fn entries(&self) -> &[AppEntry] {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut [AppEntry] {
        &mut self.entries
    }

    pub fn get(&self, id: &str) -> Option<&AppEntry> {
        self.by_id.get(&id.to_ascii_lowercase()).map(|i| &self.entries[*i])
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 启动成功后回写使用数据（同时更新内存索引，立即影响排序）。
    pub fn apply_usage(&mut self, id: &str, use_count: u32, last_used_ts: i64) {
        if let Some(i) = self.by_id.get(&id.to_ascii_lowercase()).copied() {
            self.entries[i].use_count = use_count;
            self.entries[i].last_used_ts = last_used_ts;
        }
    }

    /// 空查询时的默认面板：最近用过 N 个。
    pub fn recent_hits(&self, n: usize) -> Vec<SearchHit> {
        let now = unix_now();
        let mut scored: Vec<(f32, &AppEntry)> = self
            .entries
            .iter()
            .filter(|e| e.last_used_ts > 0)
            .map(|e| {
                (
                    usage_bonus_from(e.use_count, e.last_used_ts, now) + e.source.prior() * 10.0,
                    e,
                )
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored
            .into_iter()
            .take(n)
            .map(|(score, e)| {
                SearchHit::new(
                    ItemKind::App,
                    e.id.clone(),
                    e.name.clone(),
                    score + 500.0,
                    HitPayload::Launch {
                        target: e.target.clone(),
                    },
                )
                .subtitle("最近使用")
                .reason(crate::search::MatchReason::Substring)
                .icon(e.icon.clone())
                .source(e.source.label())
            })
            .collect()
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Indexer;

impl Indexer {
    /// 全量扫描。忽略规则来自 settings，注册表/目录扫描可关。
    pub fn scan(settings: &Settings) -> (Vec<AppEntry>, ScanReport) {
        let t = Instant::now();
        let ignore = startmenu::IgnoreMatcher::new(&settings.ignore_globs);
        let mut report = ScanReport::default();
        let mut all: Vec<AppEntry> = Vec::new();

        let sm_dirs = startmenu::start_menu_dirs();
        let mut sm = startmenu::collect(&sm_dirs, &ignore);
        // 开始菜单里的绿色 exe 快捷方式同样有用，不再额外处理
        report.start_menu = sm.len();
        all.append(&mut sm);

        // 桌面可能挂着很深的文件夹，限制递归深度
        let mut desk = startmenu::collect_limited(&startmenu::desktop_dirs(), &ignore, 4);
        // 桌面 .lnk 同样能解析出真实 exe；单独标注来源，去重时让开始菜单优先
        for e in &mut desk {
            e.source = AppSource::Desktop;
        }
        report.desktop = desk.len();
        all.append(&mut desk);

        // macOS：.app bundle 是主发现渠道（与开始菜单同级的规范来源）。
        // scan_roots 只覆盖"裸 exe 目录扫描"，应用目录恒扫，同开始菜单不受设置影响的语义。
        #[cfg(target_os = "macos")]
        {
            let mut bundles = appbundle::collect(&appbundle::app_dirs(), &ignore);
            report.app_bundle = bundles.len();
            all.append(&mut bundles);
        }

        if settings.enable_registry {
            let mut rg = registry::collect();
            report.registry = rg.len();
            all.append(&mut rg);
        }

        if settings.enable_dir_scan {
            let mut roots = settings.scan_roots.clone();
            if roots.is_empty() {
                roots = scanner::default_roots();
            }
            roots.extend(settings.extra_dirs.iter().cloned());
            let mut sc = scanner::collect(&roots, &ignore);
            // 自定义目录里通常是用户自己放的快捷方式，和开始菜单一样解析 .lnk/.url；
            // 安装目录树里的 .lnk 多为卸载项，不跟着扫。
            let mut custom =
                startmenu::collect_limited(&settings.extra_dirs, &ignore, scanner::MAX_DEPTH);
            for e in &mut custom {
                e.source = AppSource::Custom;
            }
            sc.append(&mut custom);
            // 自定义目录里的 .app bundle 同样收（mac 上用户绿色软件常这么放），标 Custom
            #[cfg(target_os = "macos")]
            {
                let mut bundles =
                    appbundle::collect_limited(&settings.extra_dirs, &ignore, appbundle::MAX_DEPTH);
                for e in &mut bundles {
                    e.source = AppSource::Custom;
                }
                sc.append(&mut bundles);
            }
            report.dir_scan = sc.len();
            all.append(&mut sc);
        }

        let mut sys = scanner::system_tools();
        report.system = sys.len();
        all.append(&mut sys);

        report.elapsed_ms = t.elapsed().as_millis();
        report.built_at = unix_now();
        (all, report)
    }

    /// 扫描 + 去重 + 合并使用数据。
    pub fn build(settings: &Settings, usage: &UsageStore) -> (AppIndex, ScanReport) {
        let (all, mut report) = Self::scan(settings);
        let raw = all.len();
        let mut index = AppIndex::from_entries(all);
        report.merged = index.len();
        report.dropped_duplicates = index.dropped_duplicates();
        tracing::info!(
            "索引完成：{} 条（原始 {} 条，合并去重 {} 条；开始菜单 {}，桌面 {}，注册表 {}，bundle {}，目录 {}，系统 {}），用时 {}ms",
            index.len(),
            raw,
            report.dropped_duplicates,
            report.start_menu,
            report.desktop,
            report.registry,
            report.app_bundle,
            report.dir_scan,
            report.system,
            report.elapsed_ms
        );
        Self::apply_usage(&mut index, usage);
        (index, report)
    }

    fn apply_usage(index: &mut AppIndex, usage: &UsageStore) {
        for e in index.entries_mut() {
            if let Some(rec) = usage.get(&e.id) {
                e.use_count = rec.count;
                e.last_used_ts = rec.last_ts;
            }
        }
    }

    pub fn cache_path() -> PathBuf {
        fsutil::data_root().join("app-index.json")
    }

    pub fn save_cache(index: &AppIndex, report: &ScanReport) -> Result<()> {
        let file = CacheFile {
            version: CACHE_VERSION,
            report: report.clone(),
            entries: index.entries().to_vec(),
        };
        let json = serde_json::to_string(&file)?;
        fsutil::write_atomic(&Self::cache_path(), json.as_bytes())?;
        tracing::debug!("索引缓存写入 {} 条", file.entries.len());
        Ok(())
    }

    /// 读缓存；版本不符或不存在返回 None。
    pub fn load_cache() -> Option<(AppIndex, ScanReport)> {
        let text = fsutil::read_to_string_opt(&Self::cache_path())?;
        let file: CacheFile = serde_json::from_str(&text).ok()?;
        if file.version != CACHE_VERSION {
            tracing::info!(
                "索引缓存版本过期({} -> {})，将重建",
                file.version,
                CACHE_VERSION
            );
            return None;
        }
        let index = AppIndex::from_entries(file.entries.clone());
        if index.len() != file.entries.len() {
            tracing::debug!("缓存加载后去重: {} -> {}", file.entries.len(), index.len());
        }
        Some((index, file.report))
    }
}

/// 便捷：手动添加一个目录/程序到索引（settings 之外的用户级白名单）。
pub fn manual_entry(name: &str, target: LaunchTarget) -> AppEntry {
    let mut e = AppEntry::new(name, target, AppSource::Manual);
    e.finalize();
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::model::TargetKind;

    fn e(name: &str, path: &str, src: AppSource) -> AppEntry {
        AppEntry::new(name, LaunchTarget::exe(path), src)
    }

    #[test]
    fn dedupes_same_exe_preferring_start_menu() {
        let entries = vec![
            e("chrome", "C:\\p\\chrome.exe", AppSource::DirScan),
            e("Google Chrome", "C:\\p\\chrome.exe", AppSource::StartMenu),
        ];
        let index = AppIndex::from_entries(entries);
        assert_eq!(index.len(), 1);
        let only = &index.entries()[0];
        assert_eq!(only.name, "Google Chrome");
        assert!(only.aliases.iter().any(|a| a == "chrome"));
        assert!(index.get(&only.id).is_some());
    }

    #[test]
    fn desktop_shortcut_loses_to_start_menu() {
        let entries = vec![
            e("桌面命名", "C:\\p\\chrome.exe", AppSource::Desktop),
            e("Google Chrome", "C:\\p\\chrome.exe", AppSource::StartMenu),
        ];
        let index = AppIndex::from_entries(entries);
        assert_eq!(index.len(), 1);
        assert_eq!(index.entries()[0].name, "Google Chrome");
        assert!(index.entries()[0].aliases.iter().any(|a| a == "桌面命名"));
    }

    #[test]
    fn shortcut_and_exe_of_same_target_merge() {
        let mut lnk = AppEntry::new(
            "微信",
            LaunchTarget {
                path: "C:\\lnk\\WeChat.lnk".into(),
                args: None,
                working_dir: None,
                kind: TargetKind::Shortcut,
            },
            AppSource::StartMenu,
        );
        lnk.icon = Some("C:\\Program Files\\Tencent\\WeChat\\WeChat.exe".into());
        let exe = e(
            "WeChat",
            "C:\\Program Files\\Tencent\\WeChat\\WeChat.exe",
            AppSource::DirScan,
        );
        let index = AppIndex::from_entries(vec![lnk, exe]);
        assert_eq!(index.len(), 1, "快捷方式与其目标 exe 应合并");
        assert_eq!(index.entries()[0].name, "微信");
    }

    #[test]
    fn custom_shortcut_name_beats_bare_exe() {
        let mut lnk = AppEntry::new(
            "我的工具",
            LaunchTarget {
                path: "D:\\tools\\我的工具.lnk".into(),
                args: None,
                working_dir: None,
                kind: TargetKind::Shortcut,
            },
            AppSource::Custom,
        );
        lnk.icon = Some("D:\\tools\\mytool.exe".into());
        let exe = e("mytool", "D:\\tools\\mytool.exe", AppSource::DirScan);
        // exe 先入，确保胜者由来源先验而非插入顺序决定
        let index = AppIndex::from_entries(vec![exe, lnk]);
        assert_eq!(index.len(), 1);
        assert_eq!(index.entries()[0].name, "我的工具");
    }

    #[test]
    fn recent_hits_sorted() {
        let mut index = AppIndex::from_entries(vec![
            e("A", "C:\\a.exe", AppSource::StartMenu),
            e("B", "C:\\b.exe", AppSource::StartMenu),
        ]);
        let now = unix_now();
        index.apply_usage("c:\\b.exe", 9, now);
        index.apply_usage("c:\\a.exe", 1, now - 400_000);
        let hits = index.recent_hits(5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "B");
    }

    #[test]
    fn cache_roundtrip_rebuilds_search_text() {
        let index = AppIndex::from_entries(vec![e("记事本", "C:\\Windows\\notepad.exe", AppSource::Manual)]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let file = CacheFile {
            version: CACHE_VERSION,
            report: ScanReport::default(),
            entries: index.entries().to_vec(),
        };
        fsutil::write_atomic(&path, serde_json::to_string(&file).unwrap().as_bytes()).unwrap();
        let text = fsutil::read_to_string_opt(&path).unwrap();
        let back: CacheFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.version, CACHE_VERSION);
        let re = AppIndex::from_entries(back.entries);
        assert!(re.entries()[0].search.best("jsb").is_some());
        assert!(re.entries()[0].search.best("jishiben").is_some());
    }
}
