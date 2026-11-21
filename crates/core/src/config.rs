use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fsutil;

/// 一个 OpenAI 兼容的模型配置（`POST {base_url}/chat/completions`）。
///
/// 只支持这一种协议：OpenAI、DeepSeek、Kimi、通义、Ollama / LM Studio
/// 都兼容它，各家差异只在 `base_url` 与 `model` 上。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// 稳定标识，插件用 `efto.ai.chat({ model: id })` 指定；为空时加载阶段自动补 `model-<n>`。
    pub id: String,
    /// 设置页显示名。
    pub name: String,
    /// 接口根地址，例如 `https://api.openai.com/v1`；也接受已带 `/chat/completions` 的完整地址。
    pub base_url: String,
    /// 密钥。本地 Ollama 这类不需要鉴权的服务可以留空。
    pub api_key: String,
    /// 请求体里的 `model`，例如 `gpt-4o-mini`、`deepseek-chat`。
    pub model: String,
}

impl ModelConfig {
    fn trimmed(self) -> Self {
        Self {
            id: self.id.trim().to_string(),
            name: self.name.trim().to_string(),
            base_url: self.base_url.trim().to_string(),
            api_key: self.api_key.trim().to_string(),
            model: self.model.trim().to_string(),
        }
    }

    fn is_blank(&self) -> bool {
        self.id.is_empty()
            && self.name.is_empty()
            && self.base_url.is_empty()
            && self.api_key.is_empty()
            && self.model.is_empty()
    }
}

/// 界面主题：跟随系统 / 固定深色 / 固定浅色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Dark,
    Light,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

/// 搜索框命令执行使用的 Shell。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CommandShell {
    /// Windows PowerShell 5.1（`powershell.exe`）。
    #[default]
    Powershell,
    /// PowerShell 7+（`pwsh.exe`）。
    Pwsh,
    /// 经典命令提示符（`cmd.exe`）。
    Cmd,
    /// Git Bash（`bash.exe`）。
    GitBash,
    /// WSL 默认发行版的 bash。
    Wsl,
    /// 用户自填的可执行文件 + 参数模板。
    Custom,
}

impl CommandShell {
    /// 设置页显示名。
    pub fn display(self) -> &'static str {
        match self {
            Self::Powershell => "PowerShell",
            Self::Pwsh => "PowerShell 7+",
            Self::Cmd => "cmd",
            Self::GitBash => "Git Bash",
            Self::Wsl => "WSL",
            Self::Custom => "自定义",
        }
    }
}

/// 界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    /// 跟随系统区域；无法识别时回落中文。
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "zh-CN", alias = "zhcn")]
    ZhCn,
    #[serde(rename = "en-US", alias = "enus")]
    EnUs,
}

impl Language {
    /// 解析成实际使用的语言代码。
    pub fn code(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            _ => "zh-CN",
        }
    }
}

/// 命令执行后结果的呈现方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CommandRunMode {
    /// 结果回填到搜索框下拉列表，可复制。
    #[default]
    Inline,
    /// 打开一个终端窗口执行（交互式程序用）。
    Terminal,
}

/// 默认命令前缀。
pub const DEFAULT_COMMAND_PREFIX: &str = ">";
/// 通用动作的默认热键。
pub const DEFAULT_UNIVERSAL_ACTIONS_HOTKEY: &str = "Ctrl+Alt+Space";

/// 单项热键默认值：`Ctrl+Alt+1..7` 依次对应 [`KNOWN_TOOLS`]。
///
/// 只作为**字段缺失时**的默认值（`#[serde(default)]`）；用户在设置页清空后
/// 保存的是显式空表，不会再被补回来，否则"解绑"就永远无效。
pub fn default_tool_hotkeys() -> std::collections::BTreeMap<String, String> {
    KNOWN_TOOLS
        .iter()
        .enumerate()
        .map(|(i, tool)| ((*tool).to_string(), format!("Ctrl+Alt+{}", i + 1)))
        .collect()
}

/// 命令超时上限（秒）。
pub const MAX_COMMAND_TIMEOUT_SECS: u64 = 60;

/// 用户设置，落在 `~/.efto/settings.json`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 全局唤起热键，Tauri global-shortcut 语法，例如 `Alt+Space`、`CmdOrCtrl+Space`。
    pub hotkey: String,
    /// 扫描根目录覆盖；为空时使用平台默认（Program Files 等）。
    pub scan_roots: Vec<PathBuf>,
    /// 额外索引目录（自定义文件夹里的绿色软件）。
    pub extra_dirs: Vec<PathBuf>,
    /// 忽略规则，glob 语法，匹配完整路径。
    pub ignore_globs: Vec<String>,
    /// 插件搜索目录。
    pub plugin_dirs: Vec<PathBuf>,
    /// 插件命令的自定义触发词：`plugin_id` → 原始 `cmd` → 用户自定义词。
    /// 自定义词与清单里的原词**同时**生效（两个词都能搜到）；删掉条目即恢复原词。
    pub cmd_aliases: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    /// 被用户停用的插件 id。停用后不进搜索、不能被唤起，运行时也会被释放；
    /// 目录与清单保留，设置页据此显示"已停用"并可重新启用。
    pub disabled_plugins: Vec<String>,
    /// 搜索结果条数上限。
    pub max_results: usize,
    /// 索引自动刷新间隔（秒），0 = 仅手动。
    pub refresh_interval_secs: u64,
    /// 是否索引注册表卸载项（拿到正规产品名与安装位置）。
    pub enable_registry: bool,
    /// 是否扫描安装目录里的裸 exe。
    pub enable_dir_scan: bool,
    /// 是否把 Chrome / Edge 收藏夹纳入搜索。
    pub enable_bookmarks: bool,
    /// Everything 命令行工具 `es.exe` 的位置（文件或所在目录）。
    /// 留空时按"仓库 tools/es → 数据目录 → PATH → Everything 安装目录"顺序自动发现。
    /// 无论来自哪里，**都必须通过内置 sha256 校验才会被执行**。
    pub everything_es_path: Option<String>,
    /// 界面主题，默认跟随系统明暗。
    pub theme: ThemeMode,
    /// 窗口失焦自动隐藏。
    pub hide_on_blur: bool,
    /// 置顶显示。
    pub always_on_top: bool,
    /// 默认面板第二行展示的常用工具，顺序即展示顺序（也是 Alt+1..9 的编号）。
    ///
    /// 条目有三种形态：内置工具 id（`screenshot` 等）、`app:<应用 id>`、
    /// `plugin:<插件 id>`。后两种在面板构建时按当前索引/插件表解析，失效项直接跳过。
    pub tools: Vec<String>,
    /// 插件可用的 Node.js 解释器（node.exe 完整路径，或 PATH 上的名字）。
    /// 为空 = 未配置，插件调用 `efto.node.run` 会被明确拒绝。
    pub node_path: Option<String>,
    /// 插件可用的 Python 解释器（python.exe 完整路径，或 PATH 上的名字）。
    /// 为空 = 未配置，插件调用 `efto.python.run` 会被明确拒绝。
    pub python_path: Option<String>,
    /// 已配置的大模型（OpenAI 兼容 chat/completions），插件通过 `efto.ai.chat` 调用。
    pub models: Vec<ModelConfig>,
    /// 默认模型 id。只要 [`Settings::models`] 非空，它就必定指向其中一个。
    pub default_model: Option<String>,
    /// 是否允许在搜索框里用前缀直接执行命令。
    pub command_enabled: bool,
    /// 命令前缀，必须以它开头才会进入命令源（默认 `>`）。
    pub command_prefix: String,
    /// 命令执行使用的 Shell。
    pub command_shell: CommandShell,
    /// 自定义可执行文件路径；为空时按 [`Settings::command_shell`] 自动发现。
    pub command_shell_path: Option<String>,
    /// WSL 发行版名；为空使用默认发行版。
    pub command_wsl_distro: Option<String>,
    /// 自定义 Shell 的参数模板，`{cmd}` 会被替换成实际命令；不含占位符时命令追加在末尾。
    pub command_custom_args: String,
    /// 命令超时（秒），1..=[`MAX_COMMAND_TIMEOUT_SECS`]。
    pub command_timeout_secs: u64,
    /// 命令结果的呈现方式。
    pub command_run_mode: CommandRunMode,
    /// 界面语言（托盘、设置页、帮助文档）。
    pub language: Language,
    /// 通用动作：按全局热键抓取前台选中文本并弹出动作菜单。
    pub universal_actions_enabled: bool,
    /// 通用动作的全局热键，不能与 [`Settings::hotkey`] 相同。
    pub universal_actions_hotkey: String,
    /// 自定义网页搜索：关键词 + 空格 + 正文时直接打开对应搜索引擎。
    pub web_searches: Vec<crate::websearch::WebSearch>,
    /// 本地没有结果时，追加一条“用默认引擎搜索”的兜底项。
    pub web_search_fallback: bool,
    /// 兜底使用的引擎关键词；失效时回落到列表第一项。
    pub web_search_engine: String,
    /// 给内置工具绑定的独立全局热键：工具 id → 热键。空字符串表示不绑定。
    pub tool_hotkeys: std::collections::BTreeMap<String, String>,
    /// 单项热键是否已经落过默认值。用于区分"老配置文件从未配置过"与
    /// "用户主动全部解绑"，避免解绑后下次启动又被补回来。
    pub tool_hotkeys_seeded: bool,
}

/// efto 自带工具的 id，顺序即设置页的基础顺序。
pub const KNOWN_TOOLS: &[&str] = &[
    "screenshot",
    "clipboard",
    "lock_screen",
    "tile_windows",
    "shortcut_guide",
    "snippets",
    "file_buffer",
];

/// 常用工具里用户自选应用的条目前缀（后接 [`crate::apps::model::AppEntry::id`]）。
pub const TOOL_APP_PREFIX: &str = "app:";
/// 常用工具里用户自选插件的条目前缀（后接插件清单里的 name）。
pub const TOOL_PLUGIN_PREFIX: &str = "plugin:";
/// 最多 9 个，对应 Alt+1..9。
pub const MAX_TOOLS: usize = 9;

/// 条目是否是合法形态（内置 id，或带非空 id 的 app:/plugin: 前缀）。
pub fn is_known_tool(entry: &str) -> bool {
    if KNOWN_TOOLS.contains(&entry) {
        return true;
    }
    if let Some(id) = entry.strip_prefix(TOOL_APP_PREFIX) {
        return !id.trim().is_empty();
    }
    if let Some(id) = entry.strip_prefix(TOOL_PLUGIN_PREFIX) {
        return !id.trim().is_empty();
    }
    false
}

/// 去重、丢未知项，**保留用户给定顺序**（顺序即卡片顺序与 Alt+编号），最多 [`MAX_TOOLS`] 个。
///
/// 以前这里按 [`KNOWN_TOOLS`] 重排，那是因为只有内置工具；现在顺序本身是
/// 用户可感知的（Alt+1 必须是第一张卡片），不能再被规整掉。
pub fn normalize_tools(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in ids {
        let v = raw.trim().to_string();
        if !is_known_tool(&v) || out.iter().any(|x| *x == v) {
            continue;
        }
        out.push(v);
        if out.len() == MAX_TOOLS {
            break;
        }
    }
    out
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "Alt+Space".into(),
            scan_roots: vec![],
            extra_dirs: vec![],
            ignore_globs: vec![
                "**/WindowsApps/**".into(),
                "**/$Recycle.Bin/**".into(),
                "**/WinSxS/**".into(),
                "**/Temp/**".into(),
                "**/node_modules/**".into(),
                "**/.git/**".into(),
                // .NET/Mono 的依赖目录，与 scanner.rs 的目录名黑名单保持一致。
                // 这里以前写成 "**/ assemblies/**"（多一个前导空格），
                // 只能匹配名字真以空格开头的目录 —— 等于一条死规则，且无人发现，
                // 因为忽略规则当时一条测试都没有。
                "**/assemblies/**".into(),
            ],
            plugin_dirs: vec![],
            cmd_aliases: Default::default(),
            disabled_plugins: vec![],
            everything_es_path: None,
            max_results: 12,
            refresh_interval_secs: 600,
            enable_registry: true,
            enable_dir_scan: true,
            enable_bookmarks: false,
            theme: ThemeMode::System,
            hide_on_blur: true,
            always_on_top: true,
            tools: KNOWN_TOOLS.iter().map(|s| (*s).to_string()).collect(),
            node_path: None,
            python_path: None,
            models: vec![],
            default_model: None,
            command_enabled: true,
            command_prefix: DEFAULT_COMMAND_PREFIX.into(),
            command_shell: CommandShell::Powershell,
            command_shell_path: None,
            command_wsl_distro: None,
            command_custom_args: String::new(),
            command_timeout_secs: 10,
            command_run_mode: CommandRunMode::Inline,
            language: Language::Auto,
            universal_actions_enabled: true,
            universal_actions_hotkey: DEFAULT_UNIVERSAL_ACTIONS_HOTKEY.into(),
            web_searches: crate::websearch::default_engines(),
            web_search_fallback: true,
            web_search_engine: "g".into(),
            tool_hotkeys: default_tool_hotkeys(),
            tool_hotkeys_seeded: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        fsutil::data_root().join("settings.json")
    }

    /// 读设置；文件不存在返回默认值，解析失败也回退默认值但会打日志。
    pub fn load() -> Self {
        let path = Self::path();
        match fsutil::read_to_string_opt(&path) {
            None => Self::default(),
            Some(s) => match serde_json::from_str::<Settings>(&s) {
                Ok(mut v) => {
                    // 手改 settings.json 可能漏 id / 让默认模型悬空，加载时统一收敛
                    v.seed_tool_hotkeys();
                    v.normalize_models();
                    v.normalize_command();
                    v.normalize_universal_actions();
                    v.normalize_web_searches();
                    v.normalize_tool_hotkeys();
                    v
                }
                Err(e) => {
                    tracing::warn!("settings.json 解析失败({e})，使用默认配置");
                    Self::default()
                }
            },
        }
    }

    pub fn save(&self) -> crate::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fsutil::write_atomic(&Self::path(), json.as_bytes())
    }

    /// 命令前缀校验：非空、最多 4 个字符、不含空白或字母数字。
    ///
    /// 允许字母数字会让 `>` 这类前缀退化成普通搜索劫持：例如把前缀设为 `c`，
    /// 用户输入 `chrome` 就会直接变成命令。宁可拒绝，也不让搜索源互相抢入口。
    pub fn validate_command_prefix(raw: &str) -> Result<String, String> {
        let p = raw.trim();
        if p.is_empty() {
            return Err("命令前缀不能为空".into());
        }
        if p.chars().count() > 4 {
            return Err("命令前缀最多 4 个字符".into());
        }
        if p.chars().any(|c| c.is_whitespace() || c.is_alphanumeric()) {
            return Err("命令前缀不能包含空白或字母数字（否则会抢走普通搜索）".into());
        }
        Ok(p.to_string())
    }

    /// 加载与保存时统一收敛命令相关字段，避免手改 settings.json 后状态悬空。
    pub fn normalize_command(&mut self) {
        if let Ok(p) = Self::validate_command_prefix(&self.command_prefix) {
            self.command_prefix = p;
        } else {
            self.command_prefix = DEFAULT_COMMAND_PREFIX.into();
        }
        self.command_timeout_secs = self.command_timeout_secs.clamp(1, MAX_COMMAND_TIMEOUT_SECS);
        self.command_shell_path = blank_to_none(self.command_shell_path.take());
        self.command_wsl_distro = blank_to_none(self.command_wsl_distro.take());
        self.command_custom_args = self.command_custom_args.trim().to_string();
    }

    /// 已收敛的命令超时。
    pub fn command_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.command_timeout_secs.clamp(1, MAX_COMMAND_TIMEOUT_SECS))
    }

    /// 通用动作热键不能为空；与唤起热键冲突时回退默认值，
    /// 否则注册第二个全局热键会失败，用户只会看到"热键没反应"。
    pub fn normalize_universal_actions(&mut self) {
        let key = self.universal_actions_hotkey.trim();
        let key = if key.is_empty() {
            DEFAULT_UNIVERSAL_ACTIONS_HOTKEY
        } else {
            key
        };
        self.universal_actions_hotkey = if key.eq_ignore_ascii_case(self.hotkey.trim()) {
            "Ctrl+Alt+Shift+Space".to_string()
        } else {
            key.to_string()
        };
    }

    /// 收敛网页搜索引擎列表，并保证兜底引擎指向现存项。
    pub fn normalize_web_searches(&mut self) {
        self.web_searches =
            crate::websearch::normalize_engines(std::mem::take(&mut self.web_searches));
        let want = self.web_search_engine.trim();
        self.web_search_engine = self
            .web_searches
            .iter()
            .find(|e| e.keyword.eq_ignore_ascii_case(want))
            .or_else(|| self.web_searches.first())
            .map(|e| e.keyword.clone())
            .unwrap_or_default();
    }

    /// 老配置文件（没有 `tool_hotkeys_seeded`）在整份表为空时补上默认单项热键。
    ///
    /// 只在**表为空**时补：用户逐个解绑后表非空，显式全部解绑则由
    /// `tool_hotkeys_seeded` 标记守护，不会"解绑了下次启动又被装上"。
    fn seed_tool_hotkeys(&mut self) {
        if self.tool_hotkeys_seeded {
            return;
        }
        self.tool_hotkeys_seeded = true;
        if self.tool_hotkeys.is_empty() {
            self.tool_hotkeys = default_tool_hotkeys();
        }
    }

    /// 单项热键：只接受内置工具；与唤起热键/通用动作热键冲突的直接丢弃。
    /// 手改 settings.json 时重复绑定很常见，静默丢弃比留着"注册失败但看不出来"好。
    pub fn normalize_tool_hotkeys(&mut self) {
        let mut seen = vec![
            self.hotkey.trim().to_ascii_lowercase(),
            self.universal_actions_hotkey.trim().to_ascii_lowercase(),
        ];
        let mut out = std::collections::BTreeMap::new();
        for (tool, keys) in std::mem::take(&mut self.tool_hotkeys) {
            if !KNOWN_TOOLS.contains(&tool.as_str()) {
                continue;
            }
            let keys = keys.trim().to_string();
            if keys.is_empty() {
                continue;
            }
            let low = keys.to_ascii_lowercase();
            if seen.iter().any(|s| *s == low) {
                continue;
            }
            seen.push(low);
            out.insert(tool, keys);
        }
        self.tool_hotkeys = out;
    }

    /// 未显式配置时，补上默认插件目录（数据目录下的 plugins/）。
    ///
    /// 顺序：环境变量 EFTO_PLUGIN_DIRS → settings.plugin_dirs → 内置数据目录。
    /// 环境变量必须在 core 这里统一读取：之前只有 CLI 路径读它，导致
    /// `EFTO_PLUGIN_DIRS=... efto.exe`（GUI）里"发现 0 个插件"，两条路径行为不一致。
    pub fn effective_plugin_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Self::env_plugin_dirs();
        dirs.extend(self.plugin_dirs.iter().cloned());
        let builtin = fsutil::data_root().join("plugins");
        if !dirs.contains(&builtin) {
            dirs.push(builtin);
        }
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut out = Vec::new();
        for d in dirs {
            if !seen.contains(&d) {
                seen.push(d.clone());
                out.push(d);
            }
        }
        out
    }

    /// 解析插件目录环境变量（`EFTO_PLUGIN_DIRS`）。
    /// Windows 用 `;` 分隔，其他平台用 `:`（Windows 路径本身含 `C:`）。
    pub fn env_plugin_dirs() -> Vec<PathBuf> {
        let raw = match std::env::var("EFTO_PLUGIN_DIRS") {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        if raw.trim().is_empty() {
            return Vec::new();
        }
        let sep = if cfg!(windows) { ';' } else { ':' };
        raw.split(sep)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    /// 校验设置页提交的模型列表：缺字段就拒绝，不做"猜你想填什么"。
    ///
    /// 空白行由设置页在提交前过滤；能走到这里的空白项视为用户明确要保存的半成品，
    /// 明确报错比静默丢弃更容易发现问题。
    pub fn validate_models(models: Vec<ModelConfig>) -> Result<Vec<ModelConfig>, String> {
        let mut out = Vec::with_capacity(models.len());
        for (i, m) in models.into_iter().map(ModelConfig::trimmed).enumerate() {
            let n = i + 1;
            if m.name.is_empty() {
                return Err(format!("第 {n} 个模型缺少名称"));
            }
            if m.model.is_empty() {
                return Err(format!("模型「{}」缺少模型名（例如 gpt-4o-mini）", m.name));
            }
            if m.base_url.is_empty() {
                return Err(format!("模型「{}」缺少 Base URL", m.name));
            }
            if !(m.base_url.starts_with("http://") || m.base_url.starts_with("https://")) {
                return Err(format!(
                    "模型「{}」的 Base URL 必须以 http:// 或 https:// 开头",
                    m.name
                ));
            }
            out.push(m);
        }
        Ok(out)
    }

    /// 收敛模型状态：去空白、补 id、id 去重，并保证默认模型指向现存项。
    ///
    /// 只在空列表时清空默认模型 —— 有模型却没有默认值会让翻译这类插件无处可去。
    pub fn normalize_models(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut models: Vec<ModelConfig> = Vec::with_capacity(self.models.len());
        for (i, m) in std::mem::take(&mut self.models).into_iter().enumerate() {
            let mut m = m.trimmed();
            if m.is_blank() {
                continue;
            }
            if m.id.is_empty() {
                m.id = format!("model-{}", i + 1);
            }
            let base = m.id.clone();
            let mut n = 2;
            while !seen.insert(m.id.clone()) {
                m.id = format!("{base}-{n}");
                n += 1;
            }
            models.push(m);
        }
        self.models = models;

        let valid = |id: &str| self.models.iter().any(|m| m.id == id);
        let keep = self.default_model.as_deref().map(valid).unwrap_or(false);
        self.default_model = if keep {
            self.default_model.clone()
        } else {
            self.models.first().map(|m| m.id.clone())
        };
    }

    /// 取要调用的模型：显式 `id`（也接受显示名或底层模型名）优先，否则用默认模型。
    pub fn resolve_model(&self, id: Option<&str>) -> Result<&ModelConfig, String> {
        let want = id.map(str::trim).filter(|s| !s.is_empty());
        if let Some(want) = want {
            return self
                .models
                .iter()
                .find(|m| m.id == want || m.name == want || m.model == want)
                .ok_or_else(|| {
                    format!("未找到模型 {want:?}；可用模型：{}", self.model_names())
                });
        }
        if let Some(d) = self.default_model.as_deref() {
            if let Some(m) = self.models.iter().find(|m| m.id == d) {
                return Ok(m);
            }
        }
        self.models
            .first()
            .ok_or_else(|| "未配置模型：请在 设置 → 模型 中添加模型并设为默认".to_string())
    }

    fn model_names(&self) -> String {
        if self.models.is_empty() {
            return "（无）".into();
        }
        self.models
            .iter()
            .map(|m| format!("{}={}", m.id, m.name))
            .collect::<Vec<_>>()
            .join("、")
    }
}

fn blank_to_none(v: Option<String>) -> Option<String> {
    v.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hotkey, "Alt+Space");
        assert_eq!(back.max_results, 12);
        // 命令执行默认开启，前缀 `>`，走 PowerShell 内联
        assert!(back.command_enabled);
        assert_eq!(back.command_prefix, ">");
        assert_eq!(back.command_shell, CommandShell::Powershell);
        assert_eq!(back.command_run_mode, CommandRunMode::Inline);
        assert_eq!(back.command_timeout_secs, 10);
    }

    #[test]
    fn disabled_plugins_default_empty_and_roundtrip() {
        // 旧 settings.json 没有该字段：默认空列表（全部插件启用）
        let s: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+P"}"#).unwrap();
        assert!(s.disabled_plugins.is_empty());
        let s: Settings = serde_json::from_str(r#"{"disabled_plugins":["timer"]}"#).unwrap();
        assert_eq!(s.disabled_plugins, vec!["timer".to_string()]);
    }

    #[test]
    fn missing_command_fields_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+P"}"#).unwrap();
        assert!(s.command_enabled);
        assert_eq!(s.command_prefix, ">");
        assert_eq!(s.command_shell, CommandShell::Powershell);
        assert_eq!(s.command_timeout().as_secs(), 10);
    }

    #[test]
    fn tool_hotkeys_seed_defaults_only_once() {
        // 老配置（没有标记、表为空）→ 补默认 Ctrl+Alt+1..7
        let mut s = Settings::default();
        s.tool_hotkeys_seeded = false;
        s.tool_hotkeys.clear();
        s.seed_tool_hotkeys();
        assert_eq!(s.tool_hotkeys.len(), KNOWN_TOOLS.len());
        assert_eq!(s.tool_hotkeys["screenshot"], "Ctrl+Alt+1");
        assert_eq!(s.tool_hotkeys["file_buffer"], "Ctrl+Alt+7");

        // 显式全部解绑后不再回填
        s.tool_hotkeys.clear();
        s.seed_tool_hotkeys();
        assert!(s.tool_hotkeys.is_empty());

        // 已有自定义项时不覆盖
        let mut t = Settings::default();
        t.tool_hotkeys_seeded = false;
        t.tool_hotkeys.clear();
        t.tool_hotkeys.insert("screenshot".into(), "Ctrl+Shift+F1".into());
        t.seed_tool_hotkeys();
        assert_eq!(t.tool_hotkeys["screenshot"], "Ctrl+Shift+F1");
    }

    #[test]
    fn command_prefix_rejects_alnum_and_whitespace() {
        assert_eq!(Settings::validate_command_prefix(">").unwrap(), ">");
        assert_eq!(Settings::validate_command_prefix(" / ").unwrap(), "/");
        assert!(Settings::validate_command_prefix("  ").is_err());
        assert!(Settings::validate_command_prefix("c").is_err());
        // 首尾空白会被裁掉，不构成"含空白"的非法前缀
        assert_eq!(Settings::validate_command_prefix("> ").unwrap(), ">");
        assert!(Settings::validate_command_prefix("> >").is_err());
        assert!(Settings::validate_command_prefix(">>>>>").is_err());
    }

    #[test]
    fn universal_actions_hotkey_normalizes() {
        let mut s = Settings::default();
        s.universal_actions_hotkey = "  ".into();
        s.normalize_universal_actions();
        assert_eq!(s.universal_actions_hotkey, DEFAULT_UNIVERSAL_ACTIONS_HOTKEY);

        // 与唤起热键冲突时必须换一个能注册的键位
        s.hotkey = DEFAULT_UNIVERSAL_ACTIONS_HOTKEY.into();
        s.universal_actions_hotkey = DEFAULT_UNIVERSAL_ACTIONS_HOTKEY.into();
        s.normalize_universal_actions();
        assert_ne!(s.universal_actions_hotkey, s.hotkey);
    }

    #[test]
    fn normalize_command_falls_back_and_clamps() {
        let mut s = Settings::default();
        s.command_prefix = "run".into();
        s.command_timeout_secs = 0;
        s.command_shell_path = Some("   ".into());
        s.command_custom_args = "  -x {cmd} ".into();
        s.normalize_command();
        assert_eq!(s.command_prefix, ">");
        assert_eq!(s.command_timeout_secs, 1);
        assert_eq!(s.command_timeout().as_secs(), 1);
        assert_eq!(s.command_shell_path, None);
        assert_eq!(s.command_custom_args, "-x {cmd}");

        s.command_timeout_secs = 9999;
        s.normalize_command();
        assert_eq!(s.command_timeout_secs, MAX_COMMAND_TIMEOUT_SECS);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+P"}"#).unwrap();
        assert_eq!(s.hotkey, "Ctrl+Shift+P");
        assert!(s.enable_registry);
        assert_eq!(
            s.tools,
            vec![
                "screenshot",
                "clipboard",
                "lock_screen",
                "tile_windows",
                "shortcut_guide",
                "snippets",
                "file_buffer"
            ]
        );
        // 旧 settings.json 没有运行时字段：默认未配置，而不是空字符串
        assert_eq!(s.node_path, None);
        assert_eq!(s.python_path, None);
        // 没有 theme 字段的旧配置按"跟随系统"处理
        assert_eq!(s.theme, ThemeMode::System);
    }

    #[test]
    fn tools_are_normalized_and_unknown_dropped() {
        // 顺序必须原样保留：卡片顺序和 Alt+1..9 的编号都依赖它
        let got = normalize_tools(vec![
            "lock_screen".into(),
            "bogus".into(),
            "screenshot".into(),
            "screenshot".into(),
        ]);
        assert_eq!(got, vec!["lock_screen", "screenshot"]);
        assert!(normalize_tools(Vec::new()).is_empty());
    }

    #[test]
    fn tools_accept_app_and_plugin_entries() {
        let got = normalize_tools(vec![
            "app:C:\\p\\code.exe".into(),
            "plugin:timestamp-ts".into(),
            "app:".into(),
            "plugin:  ".into(),
            "screenshot".into(),
        ]);
        assert_eq!(
            got,
            vec!["app:C:\\p\\code.exe", "plugin:timestamp-ts", "screenshot"]
        );
    }

    #[test]
    fn tools_are_capped_for_alt_digits() {
        let raw: Vec<String> = (0..12).map(|i| format!("app:C:\\p\\a{i}.exe")).collect();
        let got = normalize_tools(raw);
        assert_eq!(got.len(), MAX_TOOLS);
        assert_eq!(got[0], "app:C:\\p\\a0.exe");
        assert_eq!(got[MAX_TOOLS - 1], format!("app:C:\\p\\a{}.exe", MAX_TOOLS - 1));
    }

    fn model(id: &str, name: &str, model: &str) -> ModelConfig {
        ModelConfig {
            id: id.into(),
            name: name.into(),
            base_url: "https://api.example.com/v1".into(),
            api_key: "sk-test".into(),
            model: model.into(),
        }
    }

    /// 旧 settings.json 没有模型字段：默认空列表，不能凭空造一个默认模型。
    #[test]
    fn missing_model_fields_default_to_empty() {
        let s: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+P"}"#).unwrap();
        assert!(s.models.is_empty());
        assert_eq!(s.default_model, None);
        assert!(s.resolve_model(None).unwrap_err().contains("未配置模型"));
    }

    #[test]
    fn normalize_assigns_ids_and_keeps_one_default() {
        let mut s = Settings::default();
        s.models = vec![
            ModelConfig {
                name: "DeepSeek".into(),
                base_url: " https://api.deepseek.com/v1 ".into(),
                api_key: " sk-x ".into(),
                model: " deepseek-chat ".into(),
                ..Default::default()
            },
            model("b", "备用", "gpt-4o-mini"),
        ];
        // 悬空的默认模型必须纠正为第一个
        s.default_model = Some("gone".into());
        s.normalize_models();
        assert_eq!(s.models[0].id, "model-1");
        assert_eq!(s.models[0].base_url, "https://api.deepseek.com/v1");
        assert_eq!(s.models[0].api_key, "sk-x");
        assert_eq!(s.default_model.as_deref(), Some("model-1"));

        // 空列表必须清空默认值
        s.models.clear();
        s.normalize_models();
        assert_eq!(s.default_model, None);
    }

    #[test]
    fn duplicate_model_ids_are_disambiguated() {
        let mut s = Settings::default();
        s.models = vec![model("dup", "A", "a"), model("dup", "B", "b")];
        s.normalize_models();
        assert_eq!(s.models[0].id, "dup");
        assert_eq!(s.models[1].id, "dup-2");
    }

    #[test]
    fn resolve_model_prefers_explicit_then_default() {
        let mut s = Settings::default();
        s.models = vec![model("a", "DeepSeek", "deepseek-chat"), model("b", "OpenAI", "gpt-4o-mini")];
        s.normalize_models();
        assert_eq!(s.resolve_model(None).unwrap().id, "a");
        assert_eq!(s.resolve_model(Some("b")).unwrap().id, "b");
        assert_eq!(s.resolve_model(Some("OpenAI")).unwrap().id, "b");
        assert_eq!(s.resolve_model(Some("gpt-4o-mini")).unwrap().id, "b");
        s.default_model = Some("b".into());
        assert_eq!(s.resolve_model(None).unwrap().id, "b");
        let err = s.resolve_model(Some("nope")).unwrap_err();
        assert!(err.contains("未找到模型"), "{err}");
    }

    #[test]
    fn validate_models_rejects_incomplete_entries() {
        let ok = Settings::validate_models(vec![model("a", "A", "m")]).unwrap();
        assert_eq!(ok.len(), 1);

        let mut bad = model("a", "A", "m");
        bad.base_url = "ftp://x".into();
        let err = Settings::validate_models(vec![bad]).unwrap_err();
        assert!(err.contains("http://"), "{err}");

        let mut no_name = model("a", "", "m");
        assert!(Settings::validate_models(vec![no_name.clone()]).unwrap_err().contains("缺少名称"));
        no_name.name = "A".into();
        assert!(Settings::validate_models(vec![no_name]).is_ok());
    }

    /// 回归：GUI 路径以前不读 EFTO_PLUGIN_DIRS，导致同一个环境变量在 CLI 能找到插件、
    /// 在 GUI 里"发现 0 个插件"。现在两条路径共用 effective_plugin_dirs。
    /// 合成一个测试：环境变量是进程级状态，拆成两个会被并行执行而互相干扰。
    #[test]
    fn env_plugin_dirs_roundtrip() {
        std::env::remove_var("EFTO_PLUGIN_DIRS");
        assert!(Settings::env_plugin_dirs().is_empty());
        let base = Settings::default().effective_plugin_dirs();
        assert_eq!(base.len(), 1, "无 env 时只应有内置目录：{base:?}");
        assert!(base[0].to_string_lossy().ends_with("plugins"));

        std::env::set_var("EFTO_PLUGIN_DIRS", "C:\\plugins;D:\\tools");
        let dirs = Settings::env_plugin_dirs();
        if cfg!(windows) {
            assert!(dirs.iter().any(|d| d.to_string_lossy() == "C:\\plugins"), "{dirs:?}");
            assert!(dirs.iter().any(|d| d.to_string_lossy() == "D:\\tools"), "{dirs:?}");
            assert!(!dirs.iter().any(|d| d.as_os_str() == "C"), "盘符被切断：{dirs:?}");
            let all = Settings::default().effective_plugin_dirs();
            assert_eq!(all[0].to_string_lossy(), "C:\\plugins", "env 目录应排在最前");
            // 内置目录直接和 data_root() 比，不做字符串嗅探
            let builtin = crate::fsutil::data_root().join("plugins");
            assert!(all.contains(&builtin), "内置目录不能丢：{all:?} (builtin={builtin:?})");
        }

        // 3) 与 settings.plugin_dirs 重复时不应出现两次
        let mut s = Settings::default();
        s.plugin_dirs = vec![PathBuf::from("C:\\plugins")];
        let all = s.effective_plugin_dirs();
        let dup = all.iter().filter(|d| d.to_string_lossy() == "C:\\plugins").count();
        assert_eq!(dup, 1, "同一目录应去重：{all:?}");

        std::env::remove_var("EFTO_PLUGIN_DIRS");
    }

    /// 数据目录固定在用户主目录下的 `.efto`，不再跟随 %APPDATA%。
    #[test]
    fn data_root_is_home_dot_efto() {
        let root = crate::fsutil::data_root();
        assert_eq!(
            root.file_name().and_then(|s| s.to_str()),
            Some(".efto"),
            "{root:?}"
        );
        if let Some(home) = dirs::home_dir() {
            assert_eq!(root, home.join(".efto"), "{root:?}");
        }
    }
}
