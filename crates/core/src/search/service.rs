//! 搜索编排：应用 + 插件 cmd + 内置动作 + 外部输入，统一打分排序。

use serde::{Deserialize, Serialize};

use super::fuzzy::MatchReason;
use super::{external_hit, HitPayload, SearchHit, SearchText};
use crate::apps::indexer::AppIndex;
use crate::plugins::registry::PluginRegistry;
use crate::ItemKind;

/// 内置指令。第一阶段先覆盖索引/设置/插件目录这几件事，外加三个自带工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinAction {
    OpenSettings,
    RebuildIndex,
    OpenPluginDir,
    OpenDataDir,
    Quit,
    Screenshot,
    Clipboard,
    LockScreen,
    TileWindows,
    ShortcutGuide,
    Snippets,
    UniversalActions,
    FileBuffer,
    ColorPicker,
    StickyNote,
}

impl BuiltinAction {
    pub const ALL: &'static [BuiltinAction] = &[
        BuiltinAction::OpenSettings,
        BuiltinAction::RebuildIndex,
        BuiltinAction::OpenPluginDir,
        BuiltinAction::OpenDataDir,
        BuiltinAction::Quit,
        BuiltinAction::Screenshot,
        BuiltinAction::Clipboard,
        BuiltinAction::LockScreen,
        BuiltinAction::TileWindows,
        BuiltinAction::ShortcutGuide,
        BuiltinAction::Snippets,
        BuiltinAction::UniversalActions,
        BuiltinAction::FileBuffer,
        BuiltinAction::ColorPicker,
        BuiltinAction::StickyNote,
    ];

    /// 按 id 反查（默认面板只存 id）。
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|a| a.name() == name)
    }

    /// 平台支持判定：不支持的动作**不进搜索**（宁可不出，不可乱出——硬性约定 2）。
    /// 执行层仍有明确报错兜底（硬性约定 3），平台清单见 docs/macos-support-plan.md。
    pub fn supported_on_platform(self) -> bool {
        // mac 的捕获层（CGWindowListCreateImage）已落地并真机验证；不再有平台限定动作
        true
    }

    pub fn name(&self) -> &'static str {
        match self {
            BuiltinAction::OpenSettings => "settings",
            BuiltinAction::RebuildIndex => "reindex",
            BuiltinAction::OpenPluginDir => "open_plugin_dir",
            BuiltinAction::OpenDataDir => "open_data_dir",
            BuiltinAction::Quit => "quit",
            BuiltinAction::Screenshot => "screenshot",
            BuiltinAction::Clipboard => "clipboard",
            BuiltinAction::LockScreen => "lock_screen",
            BuiltinAction::TileWindows => "tile_windows",
            BuiltinAction::ShortcutGuide => "shortcut_guide",
            BuiltinAction::Snippets => "snippets",
            BuiltinAction::UniversalActions => "universal_actions",
            BuiltinAction::FileBuffer => "file_buffer",
            BuiltinAction::ColorPicker => "color_picker",
            BuiltinAction::StickyNote => "sticky_note",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            BuiltinAction::OpenSettings => "偏好设置",
            BuiltinAction::RebuildIndex => "重建应用索引",
            BuiltinAction::OpenPluginDir => "打开插件目录",
            BuiltinAction::OpenDataDir => "打开数据目录",
            BuiltinAction::Quit => "退出 efto",
            BuiltinAction::Screenshot => "截图",
            BuiltinAction::Clipboard => "剪贴板",
            BuiltinAction::LockScreen => "锁屏",
            BuiltinAction::TileWindows => "操作窗口",
            BuiltinAction::ShortcutGuide => "快捷键指南",
            BuiltinAction::Snippets => "片段",
            BuiltinAction::UniversalActions => "通用动作",
            BuiltinAction::FileBuffer => "文件缓冲",
            BuiltinAction::ColorPicker => "颜色提取器",
            BuiltinAction::StickyNote => "便签纸",
        }
    }

    fn keywords(&self) -> &'static [&'static str] {
        match self {
            BuiltinAction::OpenSettings => &["设置", "偏好设置", "settings", "config", "preference"],
            BuiltinAction::RebuildIndex => &["重建索引", "刷新索引", "reindex", "refresh index", "索引"],
            BuiltinAction::OpenPluginDir => &["插件目录", "plugins", "plugin dir"],
            BuiltinAction::OpenDataDir => &["数据目录", "data dir", "配置目录"],
            BuiltinAction::Quit => &["退出", "quit", "exit", "关闭程序"],
            BuiltinAction::Screenshot => &["截图", "截屏", "screenshot", "snip", "capture"],
            BuiltinAction::Clipboard => &["剪贴板", "粘贴板", "clipboard", "paste"],
            BuiltinAction::LockScreen => &["锁屏", "锁定", "lock", "lock screen"],
            BuiltinAction::TileWindows => &[
                "操作窗口",
                "窗口操作",
                "平铺窗口",
                "平铺",
                "排列窗口",
                "tile",
                "tile windows",
                "window layout",
            ],
            BuiltinAction::ShortcutGuide => &[
                "快捷键指南",
                "快捷键",
                "键位",
                "shortcut",
                "shortcuts",
                "hotkey",
                "keymap",
            ],
            BuiltinAction::Snippets => &[
                "片段",
                "片段管理",
                "文本模板",
                "snippet",
                "snippets",
                "template",
            ],
            BuiltinAction::UniversalActions => &[
                "通用动作",
                "动作",
                "选中文本",
                "universal action",
                "universal actions",
                "selection",
            ],
            BuiltinAction::FileBuffer => &[
                "文件缓冲",
                "文件篮",
                "批量文件",
                "file buffer",
                "file basket",
                "batch files",
            ],
            BuiltinAction::ColorPicker => &[
                "颜色提取器",
                "颜色提取",
                "取色器",
                "取色",
                "拾色",
                "吸管",
                "色值",
                "color picker",
                "colorpicker",
                "eyedropper",
                "color",
            ],
            BuiltinAction::StickyNote => &[
                "便签纸",
                "便签",
                "便条",
                "贴纸",
                "记事",
                "sticky note",
                "sticky notes",
                "note",
                "memo",
            ],
        }
    }

    fn search_text(&self) -> SearchText {
        SearchText::build(self.keywords())
    }

    pub fn hit(&self, score: f32, reason: MatchReason) -> SearchHit {
        SearchHit::new(
            ItemKind::Action,
            format!("action:{}", self.name()),
            self.title(),
            score,
            HitPayload::Action {
                name: self.name().to_string(),
            },
        )
        .subtitle("内置指令")
        .reason(reason)
        .source("action")
    }
}

/// 频次 + 最近使用加权：高频应用同分靠前，但不越过明显更匹配的项。
pub fn usage_bonus_from(use_count: u32, last_used_ts: i64, now_ts: i64) -> f32 {
    let freq = if use_count == 0 {
        0.0
    } else {
        (use_count as f32 + 1.0).ln() * 22.0
    };
    let age_secs = (now_ts - last_used_ts).max(0);
    let recency = if last_used_ts <= 0 {
        0.0
    } else if age_secs < 3600 {
        60.0
    } else if age_secs < 86_400 {
        35.0
    } else if age_secs < 7 * 86_400 {
        15.0
    } else {
        0.0
    };
    (freq + recency).min(150.0)
}

/// 空查询默认面板的输入：最近使用（宿主过滤好失效项）+ 要展示的工具 id。
#[derive(Debug, Clone, Default)]
pub struct DefaultPanel {
    pub recent: Vec<SearchHit>,
    pub tools: Vec<String>,
}

/// 无状态搜索器。宿主持有 AppIndex / PluginRegistry 的锁并传引用进来。
pub struct SearchService;

impl SearchService {
    pub fn search_apps(index: &AppIndex, query: &str, limit: usize) -> Vec<SearchHit> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let mut out: Vec<SearchHit> = Vec::new();
        for entry in index.entries() {
            let Some((base, reason)) = entry.search.best(query) else {
                continue;
            };
            let score = base * entry.source.prior()
                + usage_bonus_from(entry.use_count, entry.last_used_ts, now);
            out.push(
                SearchHit::new(
                    ItemKind::App,
                    entry.id.clone(),
                    entry.name.clone(),
                    score,
                    HitPayload::Launch {
                        target: entry.target.clone(),
                    },
                )
                .subtitle(hit_subtitle(entry))
                .reason(reason)
                .icon(entry.icon.clone())
                .source(entry.source.label()),
            );
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(limit);
        out
    }

    pub fn search_plugins(registry: &PluginRegistry, query: &str, limit: usize) -> Vec<SearchHit> {
        let mut out = registry.search(query, limit);
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(limit);
        out
    }

    pub fn search_actions(query: &str, limit: usize) -> Vec<SearchHit> {
        let mut out: Vec<SearchHit> = Vec::new();
        for action in BuiltinAction::ALL {
            if !action.supported_on_platform() {
                continue;
            }
            if let Some((score, reason)) = action.search_text().best(query) {
                // 内置动作本身数量少，给固定加成让它们在同分时可见
                out.push(action.hit(score * 0.85 + 40.0, reason));
            }
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(limit);
        out
    }

    /// 完整查询入口。空查询返回"最近使用 + 常用工具"作为默认面板。
    pub fn query(
        query: &str,
        index: &AppIndex,
        registry: &PluginRegistry,
        limit: usize,
    ) -> Vec<SearchHit> {
        Self::query_with_panel(query, index, registry, limit, None)
    }

    /// 与 [`Self::query`] 相同，但空查询时用宿主提供的默认面板
    /// （最近使用快照 + 用户配置的工具列表）。
    pub fn query_with_panel(
        query: &str,
        index: &AppIndex,
        registry: &PluginRegistry,
        limit: usize,
        panel: Option<&DefaultPanel>,
    ) -> Vec<SearchHit> {
        Self::query_with_panel_and_bookmarks(query, index, registry, limit, panel, None)
    }

    /// 与 [`Self::query_with_panel`] 相同，额外把浏览器收藏夹作为补充搜索源。
    /// 只有设置里打开 `enable_bookmarks` 时宿主才会传入 `bookmarks`。
    pub fn query_with_panel_and_bookmarks(
        query: &str,
        index: &AppIndex,
        registry: &PluginRegistry,
        limit: usize,
        panel: Option<&DefaultPanel>,
        bookmarks: Option<&crate::bookmarks::BookmarkStore>,
    ) -> Vec<SearchHit> {
        Self::query_with_sources(query, index, registry, limit, panel, bookmarks, None, None)
    }

    /// 完整查询入口：在收藏夹之外接入搜索框命令源、网页搜索、系统命令与片段。
    ///
    /// 命令前缀命中时**直接短路**：命令是用户显式指定的意图，不能再混入
    /// 应用/计算器/文件结果（"宁可不出，不可乱出"）。
    /// 网页搜索的触发词 + 空格同理直接短路，避免 `g rust` 后面补一串本地结果；
    /// 只输入触发词本身时不短路，只在结果里补一条用法提示。
    /// 片段只在关键词**完全相等**时出现，不做前缀/子序列匹配，避免劫持应用搜索。
    pub fn query_with_sources(
        query: &str,
        index: &AppIndex,
        registry: &PluginRegistry,
        limit: usize,
        panel: Option<&DefaultPanel>,
        bookmarks: Option<&crate::bookmarks::BookmarkStore>,
        settings: Option<&crate::config::Settings>,
        snippets: Option<&crate::snippets::SnippetStore>,
    ) -> Vec<SearchHit> {
        let q = query.trim();
        if let Some(settings) = settings {
            let hits = super::command::hits(q, settings);
            if !hits.is_empty() {
                return hits;
            }
            if let Some((engine, body)) = crate::websearch::parse(q, &settings.web_searches) {
                return vec![crate::websearch::hit(engine, &body, false)];
            }
        }
        if q.is_empty() {
            return Self::default_panel(panel, index, registry, limit);
        }
        // 冒号后已输入内容：设置页独占结果，避免收藏夹/应用子序列混进来。
        // 只输入 `ms-settings:` 时不短路，仍允许根条目排在列表前面。
        // `ms-settings:` 是 Windows 专有入口（mac 的系统设置搜索见 docs/macos-support-plan.md v1.5）。
        #[cfg(windows)]
        if super::win_settings::is_query(q) && q.len() > super::win_settings::PREFIX.len() {
            return super::win_settings::hits(q, limit);
        }
        // macOS 的 `x-apple.systempreferences:` 同语义（sys_settings 自带平台门）
        if super::sys_settings::is_query(q) && q.len() > super::sys_settings::PREFIX.len() {
            return super::sys_settings::hits(q, limit);
        }
        let mut groups = vec![
            Self::search_apps(index, q, limit),
            Self::search_plugins(registry, q, limit),
            Self::search_actions(q, limit),
        ];
        // 只敲了触发词（`g`）：补一条用法提示，不短路其它结果
        if let Some(settings) = settings {
            if let Some(hint) = crate::websearch::hint(q, &settings.web_searches) {
                groups.push(vec![hint]);
            }
        }
        if let Some(action) = crate::system::parse(q) {
            groups.push(vec![crate::system::hit(action)]);
        }
        if let Some(store) = snippets {
            groups.push(snippet_hits(store, q));
        }
        if let Some(store) = bookmarks {
            groups.push(store.search(q, limit));
        }
        if let Some(calc) = super::calc::calc_hit(q) {
            groups.push(vec![calc]);
        }
        // 一屏一个时刻：系统时间的八种格式与时间戳两行必须指向同一瞬间，
        // 否则跨秒时会出现 `17:48:59` 挨着下一秒的 epoch。
        let now = chrono::Local::now();
        // 输入 date/now/时间 等关键字 -> 常用格式的系统时间（单击即复制）
        groups.push(super::clock::clock_hits(q, now));
        // 输入是时间时给出秒级 + 毫秒级两行
        groups.push(super::timestamp::time_hits_at(q, now));
        // `ms-settings:` 前缀：列出/过滤 Windows 设置页（仅 Windows）
        #[cfg(windows)]
        groups.push(super::win_settings::hits(q, limit));
        // `x-apple.systempreferences:` 前缀：macOS 系统设置（自带平台门）
        groups.push(super::sys_settings::hits(q, limit));
        if let Some(ext) = external_hit(q) {
            groups.push(vec![ext]);
        }
        let hits = merge_hits(groups, limit);
        if hits.is_empty() {
            // 本地什么都没有：给一条兜底网页搜索。启用开关时永远有路可走。
            if let Some(settings) = settings {
                if settings.web_search_fallback {
                    if let Some(engine) = crate::websearch::fallback_engine(
                        &settings.web_searches,
                        &settings.web_search_engine,
                    ) {
                        return vec![crate::websearch::hit(engine, q, true)];
                    }
                }
            }
        }
        hits
    }

    /// 默认面板：第一组最近使用，第二组常用工具。
    ///
    /// 分数只用于保证"最近使用"始终排在工具之前，组内顺序来自输入顺序。
    fn default_panel(
        panel: Option<&DefaultPanel>,
        index: &AppIndex,
        registry: &PluginRegistry,
        limit: usize,
    ) -> Vec<SearchHit> {
        let recent = match panel {
            Some(p) => p.recent.clone(),
            None => index.recent_hits(6),
        };
        let tools = match panel {
            Some(p) => p.tools.clone(),
            None => crate::config::Settings::default().tools,
        };

        let mut recent: Vec<SearchHit> = recent
            .into_iter()
            .take(6)
            .enumerate()
            .map(|(i, mut hit)| {
                hit.group = Some("recent".into());
                hit.score = 600.0 - i as f32;
                hit
            })
            .collect();

        let mut tool_hits: Vec<SearchHit> = Vec::new();
        for (i, entry) in tools.iter().enumerate() {
            let score = 500.0 - i as f32;
            let mut hit = if let Some(action) = BuiltinAction::from_name(entry) {
                action.hit(score, MatchReason::Exact)
            } else if let Some(id) = entry.strip_prefix(crate::config::TOOL_APP_PREFIX) {
                // 应用被卸载 / 索引变了：直接跳过，settings.json 里的条目保留
                match index.get(id) {
                    Some(app) => app_pin_hit(app, score),
                    None => continue,
                }
            } else if let Some(id) = entry.strip_prefix(crate::config::TOOL_PLUGIN_PREFIX) {
                match registry.pinned_hit(id, score) {
                    Some(hit) => hit,
                    None => continue,
                }
            } else {
                continue;
            };
            hit.subtitle = "常用工具".into();
            hit.group = Some("tools".into());
            hit.pin = Some(entry.clone());
            tool_hits.push(hit);
        }

        // 常用工具不能被 max_results 截掉：卡片没了但 Alt+数字还在，
        // 用户按编号只会没反应。位置不够时压"最近使用"。
        let room = limit.max(1).saturating_sub(tool_hits.len()).max(1);
        recent.truncate(room);
        recent.append(&mut tool_hits);
        // 不能按 id 去重：同一个工具常常既在最近使用里，又被用户勾进常用工具；
        // 走 merge_hits 会把常用工具那一排挤空，用户看到的勾选就等于没生效。
        recent.sort_by(|a, b| b.score.total_cmp(&a.score));
        recent
    }
}

/// 片段：只接受关键词完全相等的输入，命中后直接排到最前。
fn snippet_hits(store: &crate::snippets::SnippetStore, query: &str) -> Vec<SearchHit> {
    store
        .matches(query, chrono::Local::now())
        .into_iter()
        .map(|(snippet, text)| {
            let preview: String = text
                .split(['\n', '\r'])
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .chars()
                .take(60)
                .collect();
            SearchHit::new(
                ItemKind::Snippet,
                format!("snippet:{}", snippet.id),
                snippet.display_title(),
                720.0,
                HitPayload::Snippet {
                    id: snippet.id.clone(),
                    text,
                },
            )
            .subtitle(format!("片段 · {preview}"))
            .reason(MatchReason::Exact)
            .source("snippet")
        })
        .collect()
}

/// 常用工具里用户自选的应用：复用搜索结果的应用形态，点击直接启动。
fn app_pin_hit(entry: &crate::apps::model::AppEntry, score: f32) -> SearchHit {
    SearchHit::new(
        ItemKind::App,
        entry.id.clone(),
        entry.name.clone(),
        score,
        HitPayload::Launch {
            target: entry.target.clone(),
        },
    )
    .subtitle(hit_subtitle(entry))
    .icon(entry.icon.clone())
    .source(entry.source.label())
}

fn hit_subtitle(entry: &crate::apps::model::AppEntry) -> String {
    match entry.source {
        crate::apps::model::AppSource::Registry => format!("{} · 已安装程序", entry.name),
        _ => {
            // 快捷方式的 .lnk 文件名对用户没价值，显示它真正指向的程序
            if let Some(exe) = entry.exe_hint() {
                let name = exe.rfind(['\\', '/']).map(|i| &exe[i + 1..]).unwrap_or(exe);
                if !name.is_empty() {
                    return name.to_string();
                }
            }
            let path = &entry.target.path;
            let tail = path
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or(path.as_str());
            tail.to_string()
        }
    }
}

/// 多路结果合并：按 id 去重、按分数降序、截断。
pub fn merge_hits(groups: Vec<Vec<SearchHit>>, limit: usize) -> Vec<SearchHit> {
    let mut all: Vec<SearchHit> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for group in groups {
        for hit in group {
            if seen.iter().any(|s| *s == hit.id) {
                continue;
            }
            seen.push(hit.id.clone());
            all.push(hit);
        }
    }
    all.sort_by(|a, b| b.score.total_cmp(&a.score));
    all.truncate(limit.max(1));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::model::{AppEntry, AppSource, LaunchTarget};
    use crate::apps::indexer::AppIndex;
    use crate::plugins::manifest::PluginManifest;
    use crate::plugins::registry::PluginRegistry;

    fn idx(names: &[(&str, &str, AppSource)]) -> AppIndex {
        let mut entries = Vec::new();
        for (name, path, src) in names {
            let mut e = AppEntry::new(*name, LaunchTarget::exe(*path), *src);
            e.finalize();
            entries.push(e);
        }
        AppIndex::from_entries(entries)
    }

    #[test]
    fn app_query_finds_chrome() {
        let index = idx(&[("Google Chrome", "C:\\p\\chrome.exe", AppSource::StartMenu)]);
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("chro", &index, &reg, 10);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].title, "Google Chrome");
    }

    #[test]
    fn chinese_and_initials() {
        let index = idx(&[("微信", "C:\\p\\WeChat.exe", AppSource::StartMenu)]);
        let reg = PluginRegistry::empty();
        for q in ["微信", "weixin", "wx"] {
            let hits = SearchService::query(q, &index, &reg, 10);
            assert!(
                hits.iter().any(|h| h.title == "微信"),
                "query {q} missed: {hits:?}"
            );
        }
    }

    #[test]
    fn usage_beats_equal_score_source() {
        let mut index = idx(&[
            ("Code", "C:\\a\\code.exe", AppSource::DirScan),
            ("Code Helper", "C:\\a\\helper.exe", AppSource::StartMenu),
        ]);
        index.apply_usage("c:\\a\\code.exe", 30, 1_800_000_000);
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("code", &index, &reg, 10);
        assert_eq!(hits[0].title, "Code");
    }

    #[test]
    fn actions_and_url_mixed_in() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("设置", &index, &reg, 10);
        assert!(hits.iter().any(|h| h.kind == ItemKind::Action));
        let hits = SearchService::query("https://qwenwork.cn", &index, &reg, 10);
        assert_eq!(hits[0].kind, ItemKind::External);
    }

    #[test]
    fn merge_dedupes_by_id() {
        let a = SearchHit::new(ItemKind::App, "x", "A", 10.0, HitPayload::Action { name: "a".into() });
        let b = a.clone();
        let merged = merge_hits(vec![vec![a], vec![b]], 10);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn calc_hit_appears_for_expressions_only() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("6*7", &index, &reg, 10);
        assert!(
            hits.iter().any(|h| h.kind == ItemKind::Calc && h.title == "42"),
            "算式应给出计算结果：{hits:?}"
        );
        // 关键：不能劫持应用搜索
        let hits = SearchService::query("chrome", &index, &reg, 10);
        assert!(
            !hits.iter().any(|h| h.kind == ItemKind::Calc),
            "普通应用名不该出现计算行：{hits:?}"
        );
    }

    #[test]
    fn calc_and_app_results_coexist() {
        // 有同名应用时，两者都在列表里，各自可执行
        let index = idx(&[("e", "C:\\tools\\e.exe", AppSource::DirScan)]);
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("1+1", &index, &reg, 10);
        assert!(hits.iter().any(|h| h.kind == ItemKind::Calc));
    }

    #[test]
    fn clock_keywords_share_one_instant() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();

        // date：八种格式，且它不是时间写法，不该顺手给 epoch
        let hits = SearchService::query("date", &index, &reg, 20);
        let clock: Vec<_> = hits.iter().filter(|h| h.source == "clock").collect();
        assert_eq!(clock.len(), 8, "date 应给出 8 种常用格式：{hits:?}");
        assert!(
            hits.iter().all(|h| h.source != "time"),
            "date 不是时间，不该出时间戳行：{hits:?}"
        );

        // now：格式行 + 秒级/毫秒级，并且必须来自同一个瞬间
        let hits = SearchService::query("now", &index, &reg, 20);
        let clock: Vec<_> = hits.iter().filter(|h| h.source == "clock").collect();
        let ts: Vec<_> = hits.iter().filter(|h| h.source == "time").collect();
        assert_eq!(clock.len(), 8, "now 也要给八种格式：{hits:?}");
        assert_eq!(ts.len(), 2, "now 同时给秒级/毫秒级两行");
        let secs = ts[0].title.parse::<i64>().unwrap();
        for h in &clock {
            match &h.payload {
                HitPayload::CopyText { echo, .. } => {
                    assert_eq!(echo["secs"].as_i64(), Some(secs), "跨秒会让同一屏出现两个时刻")
                }
                other => panic!("clock 行必须是 CopyText：{other:?}"),
            }
        }
        assert_eq!(hits[0].source, "clock", "用户要的是格式，应排在 epoch 之前");

        // 不劫持：包含 date/now 但不是这两个词本身的输入
        for q in ["update", "dated", "datetime", "date 2026-11-16", "runtime", "时间戳"] {
            let h = SearchService::query(q, &index, &reg, 20);
            assert!(
                h.iter().all(|x| x.source != "clock"),
                "{q:?} 不该出系统时间行：{h:?}"
            );
        }
    }

    /// `ms-settings:` 是 Windows 专有搜索源（模块只在 Windows 编译）。
    #[cfg(windows)]
    #[test]
    fn ms_settings_prefix_lists_pages() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("ms-settings:", &index, &reg, 20);
        assert!(hits.iter().any(|h| h.source == "settings"), "{hits:?}");
        assert!(hits.iter().all(|h| h.kind != ItemKind::Calc && h.source != "websearch"));

        let hits = SearchService::query("ms-settings:bluetooth", &index, &reg, 10);
        assert!(hits.iter().all(|h| h.source == "settings"), "{hits:?}");
        assert!(hits.iter().any(|h| {
            h.payload
                == HitPayload::Open {
                    path: "ms-settings:bluetooth".into(),
                }
        }), "{hits:?}");

        // 不劫持：普通输入不出设置页
        let hits = SearchService::query("显示", &index, &reg, 10);
        assert!(hits.iter().all(|h| h.source != "settings"), "{hits:?}");
    }

    /// 反例（硬性约定 2）：mac/Linux 上 `ms-settings:` 不该出任何设置页结果——
    /// 没有实现的搜索源宁可不出，不可乱出。
    #[cfg(not(windows))]
    #[test]
    fn ms_settings_is_not_a_source_on_this_platform() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        for q in ["ms-settings:", "ms-settings:bluetooth", "显示"] {
            let hits = SearchService::query(q, &index, &reg, 10);
            assert!(hits.iter().all(|h| h.source != "settings"), "{q}: {hits:?}");
        }
    }

    /// macOS 的 `x-apple.systempreferences:` 接线 + 不劫持反例（硬性约定 2）。
    #[cfg(target_os = "macos")]
    #[test]
    fn sys_settings_prefix_lists_pages_on_macos() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("x-apple.systempreferences:", &index, &reg, 20);
        assert!(hits.iter().any(|h| h.source == "settings"), "{hits:?}");

        let hits = SearchService::query("显示", &index, &reg, 10);
        assert!(hits.iter().all(|h| h.source != "settings"), "{hits:?}");
    }

    #[test]
    fn empty_query_shows_defaults() {
        let index = idx(&[("Chrome", "C:\\p\\chrome.exe", AppSource::StartMenu)]);
        let reg = PluginRegistry::empty();
        let hits = SearchService::query("", &index, &reg, 10);
        assert!(hits.len() >= 1);
    }

    fn sample_recent(id: &str, title: &str) -> SearchHit {
        SearchHit::new(
            ItemKind::App,
            id,
            title,
            1.0,
            HitPayload::Launch {
                target: LaunchTarget::exe("C:\\a\\a.exe"),
            },
        )
    }

    fn sample_action(name: &str) -> SearchHit {
        BuiltinAction::from_name(name).unwrap().hit(1.0, MatchReason::Exact)
    }

    /// 回归：工具同时出现在最近使用里时，常用工具那一排不能被去重挤空。
    #[test]
    fn default_panel_keeps_all_tools_even_if_recent() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let panel = DefaultPanel {
            recent: vec![sample_action("clipboard")],
            tools: vec!["screenshot".into(), "clipboard".into(), "lock_screen".into()],
        };
        let hits = SearchService::query_with_panel("", &index, &reg, 20, Some(&panel));
        let tools: Vec<&str> = hits
            .iter()
            .filter(|h| h.group.as_deref() == Some("tools"))
            .map(|h| h.title.as_str())
            .collect();
        assert_eq!(tools, vec!["截图", "剪贴板", "锁屏"], "{hits:?}");
    }

    #[test]
    fn default_panel_resolves_pinned_apps() {
        let index = idx(&[("Chrome", "C:\\p\\chrome.exe", AppSource::StartMenu)]);
        let reg = PluginRegistry::empty();
        let app_id = index.entries()[0].id.clone();
        let panel = DefaultPanel {
            recent: vec![],
            tools: vec![
                format!("app:{app_id}"),
                "screenshot".into(),
                "app:c:\\gone\\gone.exe".into(),
            ],
        };
        let hits = SearchService::query_with_panel("", &index, &reg, 20, Some(&panel));
        let titles: Vec<&str> = hits
            .iter()
            .filter(|h| h.group.as_deref() == Some("tools"))
            .map(|h| h.title.as_str())
            .collect();
        // 失效的应用条目被跳过，但 settings 里的原条目由宿主保留
        assert_eq!(titles, vec!["Chrome", "截图"], "{hits:?}");
        assert_eq!(hits[0].pin.as_deref(), Some(format!("app:{app_id}").as_str()));
        assert_eq!(hits[0].subtitle, "常用工具");
    }

    #[test]
    fn default_panel_never_truncates_tools() {
        let names: Vec<String> = (0..9).map(|i| format!("App{i}")).collect();
        let paths: Vec<String> = (0..9).map(|i| format!("C:\\p\\app{i}.exe")).collect();
        let specs: Vec<(&str, &str, AppSource)> = names
            .iter()
            .zip(paths.iter())
            .map(|(n, p)| (n.as_str(), p.as_str(), AppSource::StartMenu))
            .collect();
        let index = idx(&specs);
        let reg = PluginRegistry::empty();
        let tools: Vec<String> = index
            .entries()
            .iter()
            .map(|e| format!("app:{}", e.id))
            .collect();
        assert_eq!(tools.len(), 9);
        let panel = DefaultPanel {
            recent: (0..6)
                .map(|i| sample_recent(&format!("r{i}"), "Recent"))
                .collect(),
            tools,
        };
        let hits = SearchService::query_with_panel("", &index, &reg, 12, Some(&panel));
        let tools_seen = hits
            .iter()
            .filter(|h| h.group.as_deref() == Some("tools"))
            .count();
        // max_results=12 装不下 6 个最近 + 9 个常用工具时，优先压缩最近使用
        assert_eq!(tools_seen, 9, "{hits:?}");
        assert!(hits.len() <= 12, "{hits:?}");
    }

    #[test]
    fn default_panel_is_recent_then_tools() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let panel = DefaultPanel {
            recent: vec![sample_recent("app:1", "One"), sample_recent("app:2", "Two")],
            tools: vec!["screenshot".into(), "lock_screen".into()],
        };
        let hits = SearchService::query_with_panel("", &index, &reg, 20, Some(&panel));
        let groups: Vec<&str> = hits.iter().map(|h| h.group.as_deref().unwrap_or("")).collect();
        assert_eq!(groups, vec!["recent", "recent", "tools", "tools"], "{hits:?}");
        assert_eq!(hits[2].title, "截图");
        assert_eq!(hits[2].payload, HitPayload::Action { name: "screenshot".into() });
        assert_eq!(hits[2].subtitle, "常用工具");
    }

    #[test]
    fn default_panel_respects_disabled_tools() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let panel = DefaultPanel {
            recent: vec![],
            tools: vec!["clipboard".into()],
        };
        let hits = SearchService::query_with_panel("", &index, &reg, 20, Some(&panel));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].payload, HitPayload::Action { name: "clipboard".into() });

        // 工具全关时默认面板为空，前端显示空状态
        let panel = DefaultPanel::default();
        assert!(SearchService::query_with_panel("", &index, &reg, 20, Some(&panel)).is_empty());
    }

    #[test]
    fn tool_keywords_are_searchable() {
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        // 双向断言：支持的动作必须命中，平台不支持的动作必须不出现（宁可不出）
        for (q, name) in [
            ("截图", "screenshot"),
            ("clipboard", "clipboard"),
            ("锁屏", "lock_screen"),
            ("取色", "color_picker"),
            ("便签", "sticky_note"),
        ] {
            let hits = SearchService::query(q, &index, &reg, 10);
            let hit = hits
                .iter()
                .any(|h| h.payload == HitPayload::Action { name: name.into() });
            let supported = BuiltinAction::from_name(name)
                .map(|a| a.supported_on_platform())
                .unwrap_or(false);
            if supported {
                assert!(hit, "{q} 应命中 {name}：{hits:?}");
            } else {
                assert!(!hit, "{q} 不该命中 {name}（平台不支持）：{hits:?}");
            }
        }
    }

    #[test]
    fn snippets_only_trigger_on_exact_keyword() {
        use crate::snippets::{Snippet, SnippetStore};
        let mut store = SnippetStore::default();
        store
            .replace_all(vec![Snippet {
                id: String::new(),
                keyword: "sig".into(),
                title: "签名".into(),
                body: "Best regards".into(),
            }])
            .unwrap();
        let index = idx(&[("Signature App", "C:\\p\\sig.exe", AppSource::StartMenu)]);
        let reg = PluginRegistry::empty();
        let exact = SearchService::query_with_sources(
            "sig",
            &index,
            &reg,
            10,
            None,
            None,
            None,
            Some(&store),
        );
        assert!(
            exact.iter().any(|h| h.kind == ItemKind::Snippet),
            "关键词完全相等时应出片段：{exact:?}"
        );
        assert!(
            exact.iter().any(|h| h.kind == ItemKind::App),
            "片段不能把同名应用挤掉：{exact:?}"
        );
        // 不该触发：前缀、多输入、大小写不同的子串
        for q in ["si", "sig extra", "signature"] {
            let hits = SearchService::query_with_sources(
                q,
                &index,
                &reg,
                10,
                None,
                None,
                None,
                Some(&store),
            );
            assert!(
                hits.iter().all(|h| h.kind != ItemKind::Snippet),
                "{q:?} 不该出片段：{hits:?}"
            );
        }
    }

    #[test]
    fn web_search_prefix_short_circuits() {
        use crate::config::Settings;
        let settings = Settings::default();
        let index = idx(&[("git", "C:\\p\\git.exe", AppSource::StartMenu)]);
        let reg = PluginRegistry::empty();
        let hits = SearchService::query_with_sources(
            "g rust 编程",
            &index,
            &reg,
            10,
            None,
            None,
            Some(&settings),
            None,
        );
        assert_eq!(hits.len(), 1, "触发词命中后应短路：{hits:?}");
        assert_eq!(hits[0].source, "websearch");
        match &hits[0].payload {
            HitPayload::WebSearch { url } => {
                assert!(url.starts_with("https://www.bing.com/search?q="));
                assert!(url.contains("rust%20%E7%BC%96%E7%A8%8B"));
            }
            other => panic!("payload 不对：{other:?}"),
        }
        // 单独输入触发词不短路，仍走普通搜索
        let hits = SearchService::query_with_sources(
            "g",
            &index,
            &reg,
            10,
            None,
            None,
            Some(&settings),
            None,
        );
        // 单独输入触发词不短路：只补一条提示行，不生成可直接搜索的 WebSearch
        assert!(
            hits.iter()
                .all(|h| !matches!(h.payload, HitPayload::WebSearch { .. })),
            "{hits:?}"
        );
        assert!(hits.iter().any(|h| h.id == "websearch:hint:g"), "{hits:?}");
    }

    #[test]
    fn web_search_keyword_alone_hints_without_hiding_apps() {
        use crate::config::Settings;
        let settings = Settings::default();
        let index = idx(&[("git", "C:\\p\\git.exe", AppSource::StartMenu)]);
        let reg = PluginRegistry::empty();
        let hits = SearchService::query_with_sources(
            "g",
            &index,
            &reg,
            10,
            None,
            None,
            Some(&settings),
            None,
        );
        let hint = hits
            .iter()
            .find(|h| h.id == "websearch:hint:g")
            .unwrap_or_else(|| panic!("{hits:?}"));
        assert!(hint.title.contains("Bing"), "{}", hint.title);
        let app = hits.iter().find(|h| h.kind == ItemKind::App).expect("app row");
        assert!(hint.score < app.score, "提示不能压过应用");

        // 带正文时仍短路，只留搜索行
        let hits = SearchService::query_with_sources(
            "g rust",
            &index,
            &reg,
            10,
            None,
            None,
            Some(&settings),
            None,
        );
        assert!(hits.iter().all(|h| h.kind != ItemKind::App), "{hits:?}");
    }

    #[test]
    fn web_search_fallback_only_when_empty_and_enabled() {
        use crate::config::Settings;
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let mut settings = Settings::default();
        let hits = SearchService::query_with_sources(
            "zzzz-no-such-thing",
            &index,
            &reg,
            10,
            None,
            None,
            Some(&settings),
            None,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "websearch");
        assert!(hits[0].score < 500.0);

        settings.web_search_fallback = false;
        let hits = SearchService::query_with_sources(
            "zzzz-no-such-thing",
            &index,
            &reg,
            10,
            None,
            None,
            Some(&settings),
            None,
        );
        assert!(hits.is_empty(), "关掉兜底后不该硬塞结果：{hits:?}");
    }

    #[test]
    fn system_commands_need_exact_keyword() {
        use crate::config::Settings;
        let index = AppIndex::empty();
        let reg = PluginRegistry::empty();
        let settings = Settings::default();
        let exact = SearchService::query_with_sources(
            "重启",
            &index,
            &reg,
            10,
            None,
            None,
            Some(&settings),
            None,
        );
        assert!(
            exact.iter().any(|h| h.source == "system"),
            "精确关键词应出系统命令：{exact:?}"
        );
        for q in ["重启电脑", "restar"] {
            let hits = SearchService::query_with_sources(
                q,
                &index,
                &reg,
                10,
                None,
                None,
                Some(&settings),
                None,
            );
            assert!(
                hits.iter().all(|h| h.source != "system"),
                "{q:?} 不该出系统命令：{hits:?}"
            );
        }
    }

    #[allow(dead_code)]
    fn unused_manifest_helper(m: PluginManifest) -> String {
        m.name
    }
}
