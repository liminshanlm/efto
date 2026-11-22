//! 应用共享状态。所有字段都用锁包住，命令与宿主能力都在别的线程访问它们。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use efto_core::apps::indexer::{AppIndex, Indexer, ScanReport};
use efto_core::clipboard::ClipboardStore;
use efto_core::config::Settings;
use efto_core::plugins::registry::{DynamicItem, PluginRegistry};
use efto_core::recent::RecentStore;
use efto_core::search::{DefaultPanel, SearchHit};
use efto_core::store::StoreManager;
use efto_core::usage::UsageStore;
use efto_jsrt::{Host, PluginRuntime};

use crate::screenshot::Shot;

/// 单个路径的图标缓存：失败时累计尝试次数，到上限才彻底放弃。
struct IconSlot {
    url: Option<String>,
    attempts: u8,
}

/// 取图标失败最多试几次：每次失败都要打一次 Shell，不能无限重试。
const ICON_MAX_ATTEMPTS: u8 = 3;

/// 进程级共享状态。Tauri 里以 `Arc<AppData>` 的形式 manage，宿主能力通过 AppHandle 取回。
pub struct AppData {
    pub settings: Mutex<Settings>,
    pub index: RwLock<AppIndex>,
    pub registry: RwLock<PluginRegistry>,
    pub usage: Mutex<UsageStore>,
    pub stores: Mutex<StoreManager>,
    pub runtime: Mutex<PluginRuntime>,
    pub report: Mutex<ScanReport>,
    /// 默认面板第一行的快照（最近执行 / 打开 / 使用）。
    pub recent: Mutex<RecentStore>,
    /// 剪贴板文本历史。
    pub clipboard: Mutex<ClipboardStore>,
    /// 浏览器收藏夹快照（仅 `enable_bookmarks` 打开时参与搜索）。
    pub bookmarks: Mutex<efto_core::bookmarks::BookmarkStore>,
    /// 最近一次全屏捕获，等选区窗口来裁剪。
    pub shot: Mutex<Option<Shot>>,
    /// 进行中的长截图（后台滚动拼接线程），同一时刻只允许一个。
    pub long_capture: Mutex<Option<crate::screenshot::LongCapture>>,
    /// 文件图标缓存：key 是 exe / .lnk / logo 的路径。
    icon_cache: Mutex<std::collections::HashMap<String, IconSlot>>,
    /// 搜索框命令的取消令牌（Esc）。同一时刻只允许一条命令。
    pub command_cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// 用户片段（Snippets）。
    pub snippets: Mutex<efto_core::snippets::SnippetStore>,
    /// 文件缓冲：待批量处理的文件路径。
    pub file_buffer: Mutex<Vec<String>>,
    /// 便签纸（`~/.efto/notes.json`）。
    pub notes: Mutex<crate::notes::NoteStore>,
    /// 插件 UI 的文件读写授权（只放行用户选择 / 拖入的路径）。
    pub file_grants: crate::fileapi::FileGrants,
    /// 通用动作抓取到的选中文本（最近一次）。
    pub selection: Mutex<Option<String>>,
    /// 剪贴板图片缩略图缓存：图片路径 → data URL。
    clip_thumbs: Mutex<std::collections::HashMap<String, String>>,
}

pub type Shared = Arc<AppData>;

/// 未配置运行时（空 / 空白）时，从系统 PATH 探测一次 node / python。
///
/// 找到就写进内存设置并返回 `true`（调用方负责落盘）；已显式配置的路径永远不覆盖，
/// 用户之后仍可在 设置 → 运行时 改成别的解释器。
pub(crate) fn autofill_runtimes(settings: &mut Settings) -> bool {
    autofill_with(settings, efto_core::exec::detect_on_path)
}

/// 探测函数可注入，单测不依赖本机是否装了 node / python。
fn autofill_with(
    settings: &mut Settings,
    detect: impl Fn(efto_core::exec::ScriptRuntime) -> Option<String>,
) -> bool {
    use efto_core::exec::ScriptRuntime;

    fn blank(v: Option<&String>) -> bool {
        v.map(|s| s.trim().is_empty()).unwrap_or(true)
    }

    let mut changed = false;
    if blank(settings.node_path.as_ref()) {
        if let Some(path) = detect(ScriptRuntime::Node) {
            tracing::info!("未配置 Node 运行时，已从 PATH 自动检测: {path}");
            settings.node_path = Some(path);
            changed = true;
        }
    }
    if blank(settings.python_path.as_ref()) {
        if let Some(path) = detect(ScriptRuntime::Python) {
            tracing::info!("未配置 Python 运行时，已从 PATH 自动检测: {path}");
            settings.python_path = Some(path);
            changed = true;
        }
    }
    changed
}

impl AppData {
    pub fn new(host: Arc<dyn Host>) -> Self {
        let settings = Settings::load();
        let usage = UsageStore::load();
        let (index, report) = match Indexer::load_cache() {
            Some((index, report)) => {
                tracing::info!(
                    "载入索引缓存 {} 条（{}ms 前构建）",
                    index.len(),
                    report.elapsed_ms
                );
                (index, report)
            }
            None => {
                let (index, report) = Indexer::build(&settings, &usage);
                let _ = Indexer::save_cache(&index, &report);
                (index, report)
            }
        };
        let mut registry = PluginRegistry::discover(&settings.effective_plugin_dirs());
        registry.set_cmd_aliases(settings.cmd_aliases.clone());
        registry.set_disabled(&settings.disabled_plugins);
        load_dynamic_items(&mut registry);
        Self {
            settings: Mutex::new(settings),
            index: RwLock::new(index),
            registry: RwLock::new(registry),
            usage: Mutex::new(usage),
            stores: Mutex::new(StoreManager::default()),
            runtime: Mutex::new(PluginRuntime::new(host)),
            report: Mutex::new(report),
            recent: Mutex::new(RecentStore::load()),
            clipboard: Mutex::new(ClipboardStore::load()),
            bookmarks: Mutex::new(efto_core::bookmarks::BookmarkStore::default()),
            shot: Mutex::new(None),
            long_capture: Mutex::new(None),
            icon_cache: Mutex::new(std::collections::HashMap::new()),
            command_cancel: Mutex::new(None),
            snippets: Mutex::new(efto_core::snippets::SnippetStore::load()),
            file_buffer: Mutex::new(Vec::new()),
            notes: Mutex::new(crate::notes::NoteStore::load()),
            file_grants: crate::fileapi::FileGrants::new(),
            selection: Mutex::new(None),
            clip_thumbs: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 重新扫描索引并刷新插件目录。
    pub fn rebuild(&self) -> ScanReport {
        let settings = self.settings.lock().clone();
        let usage = self.usage.lock().clone();
        let (index, report) = Indexer::build(&settings, &usage);
        let _ = Indexer::save_cache(&index, &report);
        self.reload_plugins();
        *self.index.write() = index;
        *self.report.lock() = report.clone();
        report
    }

    /// 只重新发现插件，不动应用索引。插件启停 / 导入后调用。
    ///
    /// 停用与"目录里消失"对运行时的效果相同：已经跑着的 isolate 立即释放；
    /// 启用后不预启动，等用户下次唤起时按需加载。
    pub fn reload_plugins(&self) {
        let settings = self.settings.lock().clone();
        let mut registry = PluginRegistry::discover(&settings.effective_plugin_dirs());
        registry.set_cmd_aliases(settings.cmd_aliases.clone());
        registry.set_disabled(&settings.disabled_plugins);
        load_dynamic_items(&mut registry);

        let old_active: Vec<String> = self
            .registry
            .read()
            .plugins()
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.id())
            .collect();
        let new_active: Vec<String> = registry
            .plugins()
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.id())
            .collect();
        for id in old_active.iter().filter(|id| !new_active.contains(id)) {
            self.runtime.lock().stop(id);
        }
        *self.registry.write() = registry;
    }

    /// 插件运行时登记自己的动态搜索项（`efto.setSearchItems`），并落盘。
    /// 空列表等价于注销；插件未装载时明确报错，不静默丢弃。
    pub fn set_plugin_search_items(
        &self,
        plugin_id: &str,
        items: Vec<DynamicItem>,
    ) -> Result<(), String> {
        if self.registry.read().get(plugin_id).is_none() {
            return Err(format!("插件未安装或清单无效: {plugin_id}"));
        }
        self.registry.write().set_dynamic_items(plugin_id, items);
        let snapshot = self
            .registry
            .read()
            .dynamic_items(plugin_id)
            .map(|s| s.to_vec())
            .unwrap_or_default();
        save_plugin_search_items_file(plugin_id, &snapshot)
    }

    pub fn record_usage(&self, hit_id: &str) {
        let now = efto_core::apps::indexer::unix_now();
        let count = {
            let mut u = self.usage.lock();
            let rec = u.record(hit_id, now).clone();
            if let Err(e) = u.save() {
                tracing::warn!("使用记录保存失败: {e}");
            }
            rec.count
        };
        self.index.write().apply_usage(hit_id, count, now);
    }

    pub fn plugin_root(&self, plugin_id: &str) -> Option<PathBuf> {
        self
            .registry
            .read()
            .get(plugin_id)
            .map(|p| p.root.clone())
    }

    /// 插件的可见名字（宿主日志与错误提示用）。
    pub fn plugin_name(&self, plugin_id: &str) -> String {
        self
            .registry
            .read()
            .get(plugin_id)
            .map(|p| p.manifest.name.clone())
            .unwrap_or_else(|| plugin_id.to_string())
    }

    /// 记录一次"执行 / 打开 / 使用"，供默认面板第一行展示。
    pub fn record_recent(&self, hit: &SearchHit) {
        let mut recent = self.recent.lock();
        recent.record(hit);
        if let Err(e) = recent.save() {
            tracing::warn!("最近使用保存失败: {e}");
        }
    }

    /// 清空"最近使用"（默认面板上的清除按钮）。
    pub fn clear_recent(&self) {
        let mut recent = self.recent.lock();
        recent.clear();
        if let Err(e) = recent.save() {
            tracing::warn!("最近使用清空失败: {e}");
        }
    }

    /// 取文件/快捷方式/插件 logo 的系统图标（data URL）。结果按路径缓存。
    pub fn icon_for(&self, path: &str) -> Option<String> {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        {
            let cache = self.icon_cache.lock();
            if let Some(slot) = cache.get(path) {
                if slot.url.is_some() || slot.attempts >= ICON_MAX_ATTEMPTS {
                    return slot.url.clone();
                }
            }
        }
        let icon = crate::icons::data_url(path).ok();
        let mut cache = self.icon_cache.lock();
        // 图标很小，但条目上限还是要有，避免长跑后无限增长
        if cache.len() >= 512 {
            cache.clear();
        }
        let attempts = cache.get(path).map_or(1, |s| s.attempts.saturating_add(1));
        cache.insert(
            path.to_string(),
            IconSlot {
                url: icon.clone(),
                attempts,
            },
        );
        icon
    }

    /// 最近使用快照。插件已卸载的条目不展示（否则点了只会报"插件未安装"）。
    fn recent_hits(&self, n: usize) -> Vec<SearchHit> {
        let recent = self.recent.lock();
        let mut hits = recent.recent(n, |hit| match &hit.payload {
            efto_core::search::HitPayload::Plugin { plugin_id, .. } => {
                self.registry.read().get(plugin_id).is_some()
            }
            _ => true,
        });
        // 旧版快照可能没存 icon，用启动目标路径补一次；否则历史条目永远停在首字头像
        for hit in &mut hits {
            if hit.icon.is_none() {
                if let efto_core::search::HitPayload::Launch { target } = &hit.payload {
                    hit.icon = Some(target.path.clone());
                }
            }
        }
        hits
    }

    pub fn search(&self, query: &str, limit: Option<usize>) -> Vec<efto_core::search::SearchHit> {
        let (limit, tools, bookmarks_enabled, settings) = {
            let s = self.settings.lock();
            (
                limit.unwrap_or(s.max_results),
                s.tools.clone(),
                s.enable_bookmarks,
                s.clone(),
            )
        };
        // 空查询：默认面板 = 最近使用 + 用户配置的常用工具
        let panel = if query.trim().is_empty() {
            Some(DefaultPanel {
                recent: self.recent_hits(5),
                tools,
            })
        } else {
            None
        };
        let index = self.index.read();
        let registry = self.registry.read();
        let snippets = self.snippets.lock().clone();
        let snippet_ref = if snippets.is_empty() {
            None
        } else {
            Some(&snippets)
        };
        if bookmarks_enabled && !query.trim().is_empty() {
            let mut store = self.bookmarks.lock();
            store.refresh();
            efto_core::search::SearchService::query_with_sources(
                query,
                &index,
                &registry,
                limit,
                panel.as_ref(),
                Some(&store),
                Some(&settings),
                snippet_ref,
            )
        } else {
            efto_core::search::SearchService::query_with_sources(
                query,
                &index,
                &registry,
                limit,
                panel.as_ref(),
                None,
                Some(&settings),
                snippet_ref,
            )
        }
    }

    /// 剪贴板图片缩略图（96px PNG data URL）。按路径缓存，失败不缓存。
    pub fn clip_thumb(&self, image: &str) -> Option<String> {
        {
            let cache = self.clip_thumbs.lock();
            if let Some(url) = cache.get(image) {
                return Some(url.clone());
            }
        }
        let url = crate::icons::thumbnail_data_url(image, 96).ok()?;
        let mut cache = self.clip_thumbs.lock();
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(image.to_string(), url.clone());
        Some(url)
    }

    /// 加入文件缓冲（去重、上限 100）。返回是否新增。
    pub fn buffer_add(&self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        let mut buf = self.file_buffer.lock();
        if buf.iter().any(|p| p.eq_ignore_ascii_case(path)) {
            return false;
        }
        if buf.len() >= 100 {
            return false;
        }
        buf.push(path.to_string());
        true
    }

    /// 把脏的插件数据落盘（pump 与退出时都会调用）。
    pub fn flush_stores(&self) {
        if let Err(e) = self.stores.lock().flush_all() {
            tracing::error!("插件数据落盘失败: {e}");
        }
    }
}

/// 设置页展示的一条插件命令：原触发词 + 用户自定义触发词。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginCmdView {
    pub cmd: String,
    pub label: String,
    /// 空字符串 = 没有自定义，用清单里的原词
    pub alias: String,
}

/// 插件动态搜索项的落盘目录（`~/.efto/plugin-search`）。
fn dynamic_items_dir() -> PathBuf {
    efto_core::fsutil::data_root().join("plugin-search")
}

/// 把动态搜索项写入 `~/.efto/plugin-search/<id>.json`；空列表等价于删除文件。
/// GUI 与 CLI 宿主共用这一份落盘逻辑。
pub fn save_plugin_search_items_file(
    plugin_id: &str,
    items: &[DynamicItem],
) -> Result<(), String> {
    let dir = dynamic_items_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 {} 失败: {e}", dir.display()))?;
    let path = dir.join(format!("{}.json", file_stem(plugin_id)));
    if items.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let payload = serde_json::json!({ "plugin": plugin_id, "items": items });
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

fn file_stem(plugin_id: &str) -> String {
    let s: String = plugin_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "plugin".into()
    } else {
        s
    }
}

/// 启动 / 重建索引时把上一次落盘的动态搜索项灌回注册表。
/// 插件已被卸载的文件直接跳过（不删，装回来自动恢复）。
pub(crate) fn load_dynamic_items(registry: &mut PluginRegistry) {
    let Ok(rd) = std::fs::read_dir(dynamic_items_dir()) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            tracing::warn!("动态搜索项文件无法解析: {}", path.display());
            continue;
        };
        let Some(plugin) = value.get("plugin").and_then(|v| v.as_str()) else {
            continue;
        };
        if registry.get(plugin).is_none() {
            continue;
        }
        let items = value
            .get("items")
            .and_then(|v| serde_json::from_value::<Vec<DynamicItem>>(v.clone()).ok())
            .unwrap_or_default();
        registry.set_dynamic_items(plugin, items);
    }
}

/// 前端 `list_plugins` 的精简视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub running: bool,
    pub enabled: bool,
    pub has_ui: bool,
    pub has_backend: bool,
    pub cmds: Vec<PluginCmdView>,
    pub problems: Vec<String>,
    pub root: String,
}

pub fn plugin_views(data: &AppData) -> Vec<PluginView> {
    // 锁顺序 settings → registry（见 AGENTS.md）
    let aliases = data.settings.lock().cmd_aliases.clone();
    let registry = data.registry.read();
    let runtime = data.runtime.lock();
    registry
        .plugins()
        .iter()
        .map(|p| {
            let id = p.id();
            let own = aliases.get(&id);
            PluginView {
                id,
                name: p.manifest.name.clone(),
                version: p.manifest.version.clone(),
                description: p.manifest.description.clone(),
                running: runtime.is_running(&p.id()),
                enabled: p.enabled,
                has_ui: p.has_ui(),
                has_backend: p.has_backend(),
                cmds: p
                    .manifest
                    .cmds
                    .iter()
                    .map(|c| PluginCmdView {
                        cmd: c.cmd.clone(),
                        label: c.label.clone(),
                        alias: own.and_then(|m| m.get(&c.cmd)).cloned().unwrap_or_default(),
                    })
                    .collect(),
                problems: p.problems.clone(),
                root: p.root.to_string_lossy().to_string(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHost;
    impl Host for NullHost {
        fn call(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &[serde_json::Value],
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::Value::Null)
        }
        fn send_to_ui(&self, _: &str, _: &str, _: &serde_json::Value) {}
    }

    fn data() -> AppData {
        AppData::new(Arc::new(NullHost))
    }

    #[test]
    fn bootstraps_without_cache_or_plugins() {
        let d = data();
        // 不校验具体条数（取决于本机），只要求状态可用且不 panic
        let hits = d.search("a", Some(5));
        assert!(hits.len() <= 5);
        let views = plugin_views(&d);
        assert!(views.len() <= 1000);
    }

    #[test]
    fn usage_roundtrip_updates_index() {
        let d = data();
        // 空查询返回的是"最近使用 + 内置动作"，动作项不在应用索引里，这里必须走真实应用命中
        let hits = d.search("a", Some(80));
        let app_hit = hits
            .iter()
            .find(|h| h.kind == efto_core::ItemKind::App)
            .cloned();
        let Some(hit) = app_hit else {
            println!("本机没有能匹配 'a' 的应用，跳过");
            return;
        };
        d.record_usage(&hit.id);
        let idx = d.index.read();
        let e = idx.get(&hit.id).expect("命中项应存在于索引");
        assert!(e.use_count >= 1, "use_count 应随启动增长：{:?}", e.use_count);
        assert!(e.last_used_ts > 0);
        drop(idx);
        // 动作项 id 不在应用索引里，但记录使用不能 panic（前端会无差别调用）
        d.record_usage("action:settings");
        assert!(d.index.read().get("action:settings").is_none());
    }

    #[test]
    fn rebuild_is_idempotent_and_updates_report() {
        let d = data();
        let r1 = d.rebuild();
        let r2 = d.rebuild();
        assert!(r2.merged > 0, "本机应至少索引到一些应用：{r2:?}");
        assert_eq!(r1.merged, r2.merged, "两次重建结果应一致");
    }

    #[test]
    fn autofill_runtimes_only_fills_blank_fields() {
        use efto_core::exec::ScriptRuntime;

        let mut s = Settings::default();
        s.node_path = Some("   ".into());
        s.python_path = Some("D:/custom/python.exe".into());
        let changed = autofill_with(&mut s, |rt| match rt {
            ScriptRuntime::Node => Some("C:/Program Files/nodejs/node.exe".into()),
            ScriptRuntime::Python => Some("should-not-be-used".into()),
        });
        assert!(changed);
        assert_eq!(
            s.node_path.as_deref(),
            Some("C:/Program Files/nodejs/node.exe")
        );
        // 已配置的路径永不覆盖
        assert_eq!(s.python_path.as_deref(), Some("D:/custom/python.exe"));

        // 第二次调用无事可做
        assert!(!autofill_with(&mut s, |_| Some("C:/other.exe".into())));
        assert_eq!(
            s.node_path.as_deref(),
            Some("C:/Program Files/nodejs/node.exe")
        );
    }

    #[test]
    fn autofill_runtimes_keeps_fields_empty_when_nothing_detected() {
        let mut s = Settings::default();
        assert!(!autofill_with(&mut s, |_| None));
        assert!(s.node_path.is_none());
        assert!(s.python_path.is_none());
    }
}
