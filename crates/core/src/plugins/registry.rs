//! 插件发现与搜索：把每个插件的 `cmds` 变成可搜索项。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{has_manifest, load_descriptor, CmdType, PluginCmd, PluginDescriptor};
use crate::search::{HitPayload, MatchReason, SearchHit, SearchText};
use crate::ItemKind;

/// 插件运行时登记的动态搜索项（对应 uTools 的 setFeature）。
/// 清单里的 cmds 是静态的；这项能力让插件把自定义数据也挂进搜索。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicItem {
    pub title: String,
    pub cmd: String,
}

pub const MAX_DYNAMIC_ITEMS: usize = 200;
const MAX_DYNAMIC_TITLE: usize = 64;
const MAX_DYNAMIC_CMD: usize = 128;

/// 清洗动态项：去空白、丢弃空标题/空 cmd、同一 cmd 只留第一条、超长截断、总数封顶。
pub fn normalize_dynamic_items(
    items: impl IntoIterator<Item = DynamicItem>,
) -> Vec<DynamicItem> {
    let mut out: Vec<DynamicItem> = Vec::new();
    for raw in items {
        let title = raw.title.trim();
        let cmd = raw.cmd.trim();
        if title.is_empty() || cmd.is_empty() {
            continue;
        }
        if out.iter().any(|x| x.cmd.eq_ignore_ascii_case(cmd)) {
            continue;
        }
        out.push(DynamicItem {
            title: title.chars().take(MAX_DYNAMIC_TITLE).collect(),
            cmd: cmd.chars().take(MAX_DYNAMIC_CMD).collect(),
        });
        if out.len() == MAX_DYNAMIC_ITEMS {
            break;
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    plugins: Vec<PluginDescriptor>,
    /// 扫描过程中失败的目录，供 UI 提示
    load_errors: Vec<(PathBuf, String)>,
    /// 用户自定义触发词：`plugin_id` → 原始 `cmd` → 自定义词（与清单原词并存）
    cmd_aliases: BTreeMap<String, BTreeMap<String, String>>,
    /// 插件运行时登记的动态搜索项：`plugin_id` → 条目（顺序即插件给的顺序）
    dynamic: BTreeMap<String, Vec<DynamicItem>>,
}

impl PluginRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    /// 在给定目录里找插件：目录本身是插件，或其一层子目录是插件。
    pub fn discover(dirs: &[PathBuf]) -> Self {
        let mut plugins = Vec::new();
        let mut errors = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();

        for base in dirs {
            if !base.is_dir() {
                continue;
            }
            // base 自身就是一个插件目录（开发中的工作区）
            if has_manifest(base) {
                push_plugin(base, &mut plugins, &mut errors, &mut seen);
            }
            let rd = match std::fs::read_dir(base) {
                Ok(rd) => rd,
                Err(e) => {
                    tracing::debug!("插件目录不可读 {}: {e}", base.display());
                    continue;
                }
            };
            let mut subdirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            // 稳定顺序：按目录名排序，保证结果可复现
            subdirs.sort();
            for dir in subdirs {
                if has_manifest(&dir) {
                    push_plugin(&dir, &mut plugins, &mut errors, &mut seen);
                } else {
                    // 再下一层：monorepo / packages/xxx 布局
                    if let Ok(rd2) = std::fs::read_dir(&dir) {
                        let mut deeper: Vec<PathBuf> = rd2
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| p.is_dir() && has_manifest(p))
                            .collect();
                        deeper.sort();
                        for d in deeper {
                            push_plugin(&d, &mut plugins, &mut errors, &mut seen);
                        }
                    }
                }
            }
        }

        plugins.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        if !errors.is_empty() {
            tracing::warn!("{} 个插件目录加载失败", errors.len());
        }
        tracing::info!("发现 {} 个插件", plugins.len());
        Self {
            plugins,
            load_errors: errors,
            cmd_aliases: BTreeMap::new(),
            dynamic: BTreeMap::new(),
        }
    }

    /// 应用设置里的自定义触发词。新建 / 重建注册表后由宿主调用。
    pub fn set_cmd_aliases(&mut self, aliases: BTreeMap<String, BTreeMap<String, String>>) {
        self.cmd_aliases = aliases;
    }

    /// 按设置里的停用名单标记插件：仍留在 [`Self::plugins`] 里（设置页要能重新启用），
    /// 但 [`Self::get`] / [`Self::search`] 会跳过，等于"不加载"。
    pub fn set_disabled(&mut self, disabled: &[String]) {
        for plugin in &mut self.plugins {
            let id = plugin.id();
            plugin.enabled = !disabled.iter().any(|d| d.eq_ignore_ascii_case(&id));
        }
    }

    /// 替换某个插件的动态搜索项。空列表等价于注销。
    /// 条目会清洗：去空白、丢弃空标题/空 cmd、同一 cmd 只留第一条、超长截断、总数封顶。
    pub fn set_dynamic_items(&mut self, plugin_id: &str, items: Vec<DynamicItem>) {
        let out = normalize_dynamic_items(items);
        if out.is_empty() {
            self.dynamic.remove(plugin_id);
        } else {
            self.dynamic.insert(plugin_id.to_string(), out);
        }
    }

    pub fn dynamic_items(&self, plugin_id: &str) -> Option<&[DynamicItem]> {
        self.dynamic.get(plugin_id).map(Vec::as_slice)
    }

    /// 某条命令的自定义触发词（空串视为未设置）。
    fn alias_for<'a>(
        aliases: Option<&'a BTreeMap<String, String>>,
        cmd: &PluginCmd,
    ) -> Option<&'a str> {
        aliases
            .and_then(|m| m.get(&cmd.cmd))
            .map(String::as_str)
            .filter(|a| !a.is_empty())
    }

    pub fn plugins(&self) -> &[PluginDescriptor] {
        &self.plugins
    }

    pub fn load_errors(&self) -> &[(PathBuf, String)] {
        &self.load_errors
    }

    pub fn get(&self, id: &str) -> Option<&PluginDescriptor> {
        self.plugins
            .iter()
            // 平台不支持的插件和已停用一样"不能被唤起"；要区分报错时用 plugins() 自查
            .find(|p| p.id().eq_ignore_ascii_case(id) && p.enabled && p.supported_on_this_os())
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// 常用工具里的插件项：按 id 解析成"打开插件"条目。
    ///
    /// 插件已卸载 / 被禁用 / 既无后端也无 UI 时返回 `None`，由调用方直接跳过
    /// （settings.json 里的条目保留，插件装回来即恢复）。
    pub fn pinned_hit(&self, id: &str, score: f32) -> Option<SearchHit> {
        let plugin = self.get(id)?;
        if !cmd_like_open(plugin) {
            return None;
        }
        Some(open_plugin_hit(plugin, score))
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// 搜索所有插件的 cmd。命中规则：label/cmd/插件名任一匹配。
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let q = query.trim();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = Vec::new();
        for plugin in self
            .plugins
            .iter()
            .filter(|p| p.enabled && p.supported_on_this_os())
        {
            if !plugin.has_backend() && !plugin.has_ui() {
                continue;
            }
            let plugin_id = plugin.id();
            let aliases = self.cmd_aliases.get(&plugin_id);
            let mut best: Option<(f32, MatchReason, &PluginCmd)> = None;
            for cmd in plugin.triggers() {
                let alias = Self::alias_for(aliases, cmd);
                // a) 关键字前缀命中："翻译 hello" → 触发词 "翻译"，剩余部分作为 feature
                if let Some(score) = trigger_prefix_score(q, cmd, alias) {
                    if best.as_ref().map(|(s, _, _)| score > *s).unwrap_or(true) {
                        best = Some((score, MatchReason::Prefix, cmd));
                    }
                }
                // b) 关键字本身被输入命中（输入是触发词的一部分或反之）
                let mut keys = vec![cmd.label.clone(), cmd.trigger().to_string()];
                if let Some(a) = alias {
                    keys.push(a.to_string());
                }
                let st = SearchText::build(&keys);
                if let Some((score, reason)) = st.best(q) {
                    let boosted = score + 60.0;
                    if best.as_ref().map(|(s, _, _)| boosted > *s).unwrap_or(true) {
                        best = Some((boosted, reason, cmd));
                    }
                }
            }
            // 运行时登记的动态项（插件自己的数据）与清单 cmd 一起比，取分高的那条。
            let dyn_best = self
                .dynamic
                .get(&plugin_id)
                .and_then(|items| best_dynamic_item(q, items));
            if let Some((score, item)) = dyn_best {
                if best.as_ref().map(|(s, _, _)| score >= *s).unwrap_or(true) {
                    hits.push(dynamic_hit(plugin, item, score));
                    continue;
                }
            }
            if let Some((score, reason, cmd)) = best {
                let alias = Self::alias_for(aliases, cmd);
                let mut hit = cmd_hit(plugin, cmd, extract_feature(q, cmd, alias), score);
                hit.reason = format!("{reason:?}").to_lowercase();
                hits.push(hit);
                continue;
            }
            // 只有插件名匹配时，作为"打开插件"项，分数压低
            let name_st = SearchText::build(&[
                plugin.manifest.name.clone(),
                plugin
                    .manifest
                    .description
                    .split_whitespace()
                    .take(6)
                    .collect::<Vec<_>>()
                    .join(" "),
            ]);
            if let Some((score, _)) = name_st.best(q) {
                if cmd_like_open(plugin) {
                    hits.push(open_plugin_hit(plugin, score * 0.75));
                }
            }
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        hits
    }
}

fn cmd_like_open(plugin: &PluginDescriptor) -> bool {
    // 无 cmd 的插件也能通过插件名打开；有 cmd 的插件同时给一个总入口
    !plugin.manifest.cmds.is_empty() || plugin.has_ui()
}

/// 输入以触发词开头时给一个高分：例如 "翻译 hello" 应该命中"翻译"这个 cmd。
/// 触发词后紧跟空格（或直接相接）视为最标准的形式。
/// `alias` 是用户在设置里给这条命令加的自定义词，和清单里的原词一起参与匹配。
fn trigger_prefix_score(query: &str, cmd: &PluginCmd, alias: Option<&str>) -> Option<f32> {
    let q = query.trim_start();
    let mut triggers = vec![cmd.trigger(), cmd.label.as_str()];
    if let Some(a) = alias {
        triggers.push(a);
    }
    let mut best: Option<f32> = None;
    for trigger in triggers {
        let t = trigger.trim();
        if t.is_empty() {
            continue;
        }
        // 必须用 get(..t.len())：中文按字节切会落在字符中间而 panic
        //
        // 比输入长就跳过这一条，不能 `?` 直接 return None：label 常常比 cmd 长得多
        // （quickweb 的 label 是「网页快开 · 网页 · 百度搜索 <关键词>」），
        // 而 cmd 已经命中；直接返回会把前面算出的 best 一起丢掉，表现为「百度搜索 java」搜不到。
        let Some(head) = q.get(..t.len()) else {
            continue;
        };
        if !head.eq_ignore_ascii_case(t) {
            continue;
        }
        let rest = q[t.len()..].trim_start();
        let score = if rest.is_empty() { 950.0 } else { 900.0 };
        if best.map(|s| score > s).unwrap_or(true) {
            best = Some(score);
        }
    }
    best
}

fn push_plugin(
    dir: &PathBuf,
    plugins: &mut Vec<PluginDescriptor>,
    errors: &mut Vec<(PathBuf, String)>,
    seen: &mut Vec<PathBuf>,
) {
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    if seen.iter().any(|p| p == &canon) {
        return;
    }
    seen.push(canon.clone());
    match load_descriptor(dir) {
        Ok(mut d) => {
            d.root = canon;
            if !d.enabled {
                return;
            }
            for p in &d.problems {
                tracing::debug!("插件 {} 提示: {p}", d.manifest.name);
            }
            plugins.push(d);
        }
        Err(e) => {
            tracing::warn!("插件目录 {:?} 加载失败: {e}", dir);
            errors.push((dir.clone(), e.to_string()));
        }
    }
}

fn base_hit(plugin: &PluginDescriptor, score: f32, title: String, subtitle: String) -> SearchHit {
    SearchHit::new(
        ItemKind::Plugin,
        format!("plugin://{}#{}", plugin.id(), slug(&title)),
        title,
        score,
        HitPayload::Plugin {
            plugin_id: plugin.id(),
            cmd: String::new(),
            feature: String::new(),
        },
        )
        .subtitle(subtitle)
        .icon(plugin.manifest.logo.clone().map(|l| {
            Path::new(&plugin.root_display())
                .join(l)
                .to_string_lossy()
                .to_string()
        }))
        .source(format!("plugin:{}", plugin.id()))
}

fn cmd_hit(plugin: &PluginDescriptor, cmd: &PluginCmd, feature: String, score: f32) -> SearchHit {
    SearchHit::new(
        ItemKind::Plugin,
        format!("plugin://{}#{}", plugin.id(), slug(&cmd.label)),
        cmd.label.clone(),
        score,
        HitPayload::Plugin {
            plugin_id: plugin.id(),
            cmd: cmd.trigger().to_string(),
            feature,
        },
    )
    .subtitle(format!("插件 · {}", plugin.manifest.name))
    .icon(
        cmd.icon
            .clone()
            .or_else(|| plugin.manifest.logo.clone())
            .map(|l| Path::new(&plugin.root_display()).join(l).to_string_lossy().to_string()),
    )
    .source(format!("plugin:{}", plugin.id()))
}

fn open_plugin_hit(plugin: &PluginDescriptor, score: f32) -> SearchHit {
    base_hit(
        plugin,
        score,
        plugin.manifest.name.clone(),
        format!("插件 · {}", short_desc(plugin)),
    )
}

/// 动态项匹配：整串相等 > 前缀 > 包含（双向），中文按原样比较。
fn best_dynamic_item<'a>(query: &str, items: &'a [DynamicItem]) -> Option<(f32, &'a DynamicItem)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    let mut best: Option<(f32, &DynamicItem)> = None;
    for item in items {
        let title = item.title.to_lowercase();
        let score = if title == q {
            980.0
        } else if title.starts_with(&q) {
            900.0
        } else if title.contains(&q) || q.contains(&title) {
            700.0
        } else {
            continue;
        };
        if best.map(|(s, _)| score > s).unwrap_or(true) {
            best = Some((score, item));
        }
    }
    best
}

fn dynamic_hit(plugin: &PluginDescriptor, item: &DynamicItem, score: f32) -> SearchHit {
    SearchHit::new(
        ItemKind::Plugin,
        format!("plugin://{}#{}", plugin.id(), slug(&item.title)),
        item.title.clone(),
        score,
        HitPayload::Plugin {
            plugin_id: plugin.id(),
            cmd: item.cmd.clone(),
            feature: String::new(),
        },
    )
    .subtitle(format!("插件 · {}", plugin.manifest.name))
    .icon(
        plugin
            .manifest
            .logo
            .clone()
            .map(|l| Path::new(&plugin.root_display()).join(l).to_string_lossy().to_string()),
    )
    .source(format!("plugin:{}", plugin.id()))
}

fn short_desc(plugin: &PluginDescriptor) -> String {
    if plugin.manifest.description.trim().is_empty() {
        return "打开插件".to_string();
    }
    let d = plugin.manifest.description.trim();
    d.chars().take(40).collect()
}

fn root_display_helper(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

trait RootDisplay {
    fn root_display(&self) -> String;
}
impl RootDisplay for PluginDescriptor {
    fn root_display(&self) -> String {
        root_display_helper(&self.root)
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "plugin".to_string()
    } else {
        trimmed
    }
}

/// 关键字后面的内容作为 feature 传给插件（input 模式行为）。
pub fn extract_feature(query: &str, cmd: &PluginCmd, alias: Option<&str>) -> String {
    let q = query.trim();
    if cmd.cmd_type == CmdType::Input || cmd.cmd_type == CmdType::Over {
        // 依次尝试 cmd、label、用户自定义词："翻译 hello world" 里 "翻译" 是 label，
        // "hello world" 是 feature
        let mut triggers = vec![cmd.trigger().trim(), cmd.label.trim()];
        if let Some(a) = alias {
            triggers.push(a.trim());
        }
        for trigger in triggers {
            if trigger.is_empty() {
                continue;
            }
            if let Some(rest) = strip_prefix_ci(q, trigger) {
                return rest.trim_start().to_string();
            }
        }
    }
    q.to_string()
}

fn strip_prefix_ci(hay: &str, needle: &str) -> Option<String> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    // 用 get(..n) 而不是 split_at：字节数对齐但落在中文字符中间时 split_at 会 panic
    let head = hay.get(..needle.len())?;
    if head.eq_ignore_ascii_case(needle) {
        hay.get(needle.len()..).map(|t| t.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mk_plugin(dir: &Path, name: &str, pkg: &str, files: &[(&str, &str)]) -> PathBuf {
        let p = dir.join(name);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("efto-plugin.json"), pkg).unwrap();
        for (f, c) in files {
            fs::write(p.join(f), c).unwrap();
        }
        p
    }

    #[test]
    fn discovers_nested_plugins_and_searches_cmds() {
        let dir = tempfile::tempdir().unwrap();
        mk_plugin(
            dir.path(),
            "translator",
            r#"{"name":"translator","main":"main.js","cmds":[{"type":"input","label":"翻译","cmd":"翻译"}]}"#,
            &[("main.js", ""), ("icon.png", "")],
        );
        mk_plugin(
            dir.path(),
            "timer",
            r#"{"name":"timer","main":"main.js","cmds":[{"type":"input","label":"计时","cmd":"timer"},{"type":"over","label":"文件转码"}]}"#,
            &[("main.js", "")],
        );
        let reg = PluginRegistry::discover(&[dir.path().to_path_buf()]);
        assert_eq!(reg.len(), 2, "{:?}", reg.load_errors());

        let hits = reg.search("翻译 你好世界", 10);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].title, "翻译");
        match &hits[0].payload {
            HitPayload::Plugin {
                plugin_id, feature, ..
            } => {
                assert_eq!(plugin_id, "translator");
                assert_eq!(feature, "你好世界");
            }
            other => panic!("wrong payload {other:?}"),
        }

        let hits = reg.search("timer", 10);
        assert!(hits.iter().any(|h| h.title == "计时"));
    }

    #[test]
    fn disabled_plugins_are_listed_but_not_searchable() {
        let dir = tempfile::tempdir().unwrap();
        mk_plugin(
            dir.path(),
            "timer",
            r#"{"name":"timer","main":"main.js","cmds":[{"type":"input","label":"计时","cmd":"timer"}]}"#,
            &[("main.js", "")],
        );
        mk_plugin(
            dir.path(),
            "translator",
            r#"{"name":"translator","main":"main.js","cmds":[{"type":"input","label":"翻译","cmd":"翻译"}]}"#,
            &[("main.js", "")],
        );
        let mut reg = PluginRegistry::discover(&[dir.path().to_path_buf()]);
        reg.set_disabled(&["timer".to_string()]);

        // 仍然出现在列表里（设置页要能重新启用），但标记为停用
        assert_eq!(reg.len(), 2);
        let timer = reg.plugins().iter().find(|p| p.id() == "timer").unwrap();
        assert!(!timer.enabled);
        assert!(reg.plugins().iter().find(|p| p.id() == "translator").unwrap().enabled);

        // 停用 = 不加载：搜不到、取不到
        assert!(reg.search("timer", 10).is_empty());
        assert!(reg.get("timer").is_none());
        assert!(reg.get("translator").is_some());

        // 重新启用后恢复
        reg.set_disabled(&[]);
        assert!(!reg.search("timer", 10).is_empty());
    }

    /// 平台不支持的插件（清单插件级 `platform` 字段）：列表可见但不能被唤起，
    /// 与"已停用"同语义；报精确错误的路径见 cli plugin-run（unsupported_platform）。
    #[test]
    fn unsupported_platform_plugins_are_listed_but_not_searchable() {
        let dir = tempfile::tempdir().unwrap();
        mk_plugin(
            dir.path(),
            "hosts",
            r#"{"name":"hosts","main":"main.js","platform":[{"os":"plan9"}],"cmds":[{"type":"input","label":"修改 hosts","cmd":"hosts"}]}"#,
            &[("main.js", "")],
        );
        mk_plugin(
            dir.path(),
            "timer",
            r#"{"name":"timer","main":"main.js","cmds":[{"type":"input","label":"计时","cmd":"timer"}]}"#,
            &[("main.js", "")],
        );
        let reg = PluginRegistry::discover(&[dir.path().to_path_buf()]);

        assert_eq!(reg.len(), 2);
        let hosts = reg.plugins().iter().find(|p| p.id() == "hosts").unwrap();
        assert!(!hosts.supported_on_this_os());
        assert!(reg
            .plugins()
            .iter()
            .find(|p| p.id() == "timer")
            .unwrap()
            .supported_on_this_os());

        // 不支持 = 不进搜索、不能被唤起
        assert!(reg.search("hosts", 10).is_empty());
        assert!(reg.get("hosts").is_none());
        assert!(!reg.search("timer", 10).is_empty());
        assert!(reg.get("timer").is_some());
    }

    #[test]
    fn dynamic_items_are_cleaned_and_searchable() {
        let dir = tempfile::tempdir().unwrap();
        mk_plugin(
            dir.path(),
            "quickweb",
            r#"{"name":"quickweb","main":"main.js","cmds":[{"type":"input","label":"网页快开","cmd":"网页快开"}]}"#,
            &[("main.js", "")],
        );
        let mut reg = PluginRegistry::discover(&[dir.path().to_path_buf()]);
        reg.set_dynamic_items(
            "quickweb",
            vec![
                DynamicItem {
                    title: "我的博客".into(),
                    cmd: "我的博客".into(),
                },
                DynamicItem {
                    title: "   ".into(),
                    cmd: "空标题".into(),
                },
                DynamicItem {
                    title: "文档".into(),
                    cmd: "文档".into(),
                },
                DynamicItem {
                    title: "文档重复".into(),
                    cmd: "文档".into(),
                },
            ],
        );
        // 空标题与重复 cmd 被丢弃
        assert_eq!(reg.dynamic_items("quickweb").unwrap().len(), 2);

        let hits = reg.search("我的博客", 10);
        assert_eq!(hits[0].title, "我的博客");
        assert_eq!(hits[0].subtitle, "插件 · quickweb");
        match &hits[0].payload {
            HitPayload::Plugin {
                plugin_id, cmd, ..
            } => {
                assert_eq!(plugin_id, "quickweb");
                assert_eq!(cmd, "我的博客");
            }
            other => panic!("wrong payload {other:?}"),
        }

        // 输入标题的一部分也能命中
        assert_eq!(reg.search("博客", 10)[0].title, "我的博客");

        // 注销后回到只搜清单 cmd
        reg.set_dynamic_items("quickweb", Vec::new());
        assert!(reg.dynamic_items("quickweb").is_none());
        assert!(reg.search("我的博客", 10).is_empty());
    }

    #[test]
    fn skips_broken_dirs_and_reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("broken");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("efto-plugin.json"), "{").unwrap();
        let reg = PluginRegistry::discover(&[dir.path().to_path_buf()]);
        assert_eq!(reg.len(), 0);
        assert_eq!(reg.load_errors().len(), 1);
    }

    #[test]
    fn dedupes_same_dir_seen_twice() {
        let dir = tempfile::tempdir().unwrap();
        let p = mk_plugin(
            dir.path(),
            "solo",
            r#"{"name":"solo","main":"main.js","cmds":[{"label":"solo","cmd":"solo"}]}"#,
            &[("main.js", "")],
        );
        let reg = PluginRegistry::discover(&[dir.path().to_path_buf(), p]);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn feature_extraction_edge_cases() {
        let cmd = PluginCmd {
            cmd_type: CmdType::Input,
            label: "翻译".into(),
            cmd: "翻译".into(),
            icon: None,
            platform: vec![],
            matcher: None,
            hotkey: None,
        };
        assert_eq!(extract_feature("翻译 hello", &cmd, None), "hello");
        assert_eq!(extract_feature("翻译", &cmd, None), "");
        assert_eq!(extract_feature("其他输入", &cmd, None), "其他输入");

        let c2 = PluginCmd {
            cmd_type: CmdType::Input,
            label: "fanyi".into(),
            cmd: "翻译".into(),
            icon: None,
            platform: vec![],
            matcher: None,
            hotkey: None,
        };
        assert_eq!(extract_feature("fanyi abc", &c2, None), "abc");
    }

    #[test]
    fn slug_stabilizes_ids() {
        assert_eq!(slug("文件 转码"), "文件_转码");
        assert_eq!(slug("!!!"), "plugin");
    }

    /// 回归：中文触发词按字节切会落在字符中间，之前直接 panic。
    #[test]
    fn cjk_trigger_never_panics() {
        let cmd = PluginCmd {
            cmd_type: CmdType::Input,
            label: "翻译".into(),
            cmd: "翻译".into(),
            icon: None,
            platform: vec![],
            matcher: None,
            hotkey: None,
        };
        // "译" 的第 5 字节落在字符中间（3..6）
        assert!(trigger_prefix_score("翻译 hello", &cmd, None).is_some());
        assert_eq!(trigger_prefix_score("翻", &cmd, None), None);
        assert_eq!(trigger_prefix_score("译 ", &cmd, None), None);
        assert_eq!(extract_feature("翻译 你好世界", &cmd, None), "你好世界");
        // 混合 ASCII/CJK 也不能崩
        let mixed = PluginCmd {
            cmd_type: CmdType::Input,
            label: "ts时间戳".into(),
            cmd: "ts时间戳".into(),
            icon: None,
            platform: vec![],
            matcher: None,
            hotkey: None,
        };
        assert_eq!(extract_feature("ts时间戳 1700000000", &mixed, None), "1700000000");
    }

    /// 回归：label 比输入长时，不能把已经命中的 cmd 前缀一起吞掉。
    #[test]
    fn long_label_does_not_swallow_prefix_match() {
        let cmd = PluginCmd {
            cmd_type: CmdType::Input,
            label: "网页快开 · 网页 · 百度搜索 <关键词>".into(),
            cmd: "百度搜索".into(),
            icon: None,
            platform: vec![],
            matcher: None,
            hotkey: None,
        };
        assert_eq!(trigger_prefix_score("百度搜索 java", &cmd, None), Some(900.0));
        assert_eq!(extract_feature("百度搜索 java", &cmd, None), "java");
    }

    /// 用户自定义触发词与清单原词并存，后面的内容照样作为 feature。
    #[test]
    fn alias_triggers_in_addition_to_original() {
        let cmd = PluginCmd {
            cmd_type: CmdType::Input,
            label: "网页快开 · 网页 · 百度搜索 <关键词>".into(),
            cmd: "百度搜索".into(),
            icon: None,
            platform: vec![],
            matcher: None,
            hotkey: None,
        };
        assert_eq!(trigger_prefix_score("bd java", &cmd, Some("bd")), Some(900.0));
        assert_eq!(extract_feature("bd java", &cmd, Some("bd")), "java");
        // 原词仍然可用
        assert_eq!(trigger_prefix_score("百度搜索 java", &cmd, Some("bd")), Some(900.0));
        // 空的自定义词视同没设置
        assert_eq!(trigger_prefix_score("bd java", &cmd, Some("")), None);
    }
}
