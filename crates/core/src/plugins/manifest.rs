//! 插件清单。
//!
//! 插件用 `efto-plugin.json` 描述；解析器对未知字段宽容。
//! ```json
//! {
//!   "name": "hello",
//!   "version": "1.0.0",
//!   "main": "main.js",          // 或 main.ts，宿主内转译
//!   "preload": "preload.js",    // UI 页面前置脚本
//!   "logo": "logo.png",
//!   "index": "index.html",      // UI 页面
//!   "fileTypes": ["md", "markdown"],   // 声明可打开的扩展名（用于"打开方式"）
//!   "engines": { "efto": ">=0.1.0" },  // 宿主版本要求，缺省 = 不限制
//!   "platform": [{ "os": "win32" }],   // 插件级平台限定，空/缺省 = 全平台
//!   "cmds": [
//!     { "type": "input", "label": "你好", "cmd": "你好", "icon": "icon.png" },
//!     { "type": "over",  "label": "识别文件", "platform": [{ "os": "win32" }] }
//!   ]
//! }
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::fsutil;

/// 插件清单文件名。
pub const NATIVE_MANIFEST: &str = "efto-plugin.json";

/// 唤起方式，对应 `cmds[].type`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CmdType {
    /// 关键字触发，后续输入作为 feature 传给插件
    Input,
    /// 内容覆盖：拖入文件/选中文本后触发
    Over,
    /// 正则匹配输入触发
    Regex,
    /// 分词触发（选中词）
    Word,
    /// 无 cmd，仅从插件面板进入；未知 type 也归到这里
    #[default]
    #[serde(other)]
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformSpec {
    /// `win32` / `darwin` / `linux`，空 = 全平台
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
}

impl PlatformSpec {
    pub fn matches_current(&self) -> bool {
        if self.os.is_empty() {
            return true;
        }
        let cur = if cfg!(windows) {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        self.os.eq_ignore_ascii_case(cur)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCmd {
    #[serde(default, rename = "type")]
    pub cmd_type: CmdType,
    /// 显示名，也是搜索关键字
    pub label: String,
    /// 触发词；缺省与 label 相同
    #[serde(default)]
    pub cmd: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub platform: Vec<PlatformSpec>,
    /// 正则模式的匹配式（type=regex）
    #[serde(default)]
    pub matcher: Option<serde_json::Value>,
    /// 快捷键唤起（每个 cmd 可独立配置 hotkey）
    #[serde(default)]
    pub hotkey: Option<String>,
}

impl PluginCmd {
    pub fn trigger(&self) -> &str {
        if self.cmd.is_empty() {
            &self.label
        } else {
            &self.cmd
        }
    }

    pub fn available_on_this_os(&self) -> bool {
        self.platform.is_empty() || self.platform.iter().any(|p| p.matches_current())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginAuthor {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// 反序列化用中间结构（author 既可能是对象也可能是字符串）。
#[derive(Debug, Clone, Deserialize)]
struct RawManifest {
    #[serde(default)]
    name: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default)]
    main: Option<String>,
    #[serde(default)]
    preload: Option<String>,
    #[serde(default)]
    logo: Option<String>,
    #[serde(default, alias = "ui")]
    index: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    author: Option<serde_json::Value>,
    #[serde(default)]
    cmds: Vec<PluginCmd>,
    #[serde(default, rename = "fileTypes")]
    file_types: Vec<String>,
    #[serde(default)]
    engines: Option<EnginesSection>,
    #[serde(default)]
    platform: Vec<PlatformSpec>,
    #[serde(default)]
    plugin: Option<PluginSection>,
}

/// 清单里的 `engines`：目前只认 `efto`（宿主版本要求）。
#[derive(Debug, Clone, Default, Deserialize)]
struct EnginesSection {
    #[serde(default)]
    efto: Option<String>,
}

fn default_version() -> String {
    "0.0.0".to_string()
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PluginSection {
    #[serde(default)]
    main: Option<String>,
    #[serde(default)]
    preload: Option<String>,
}

/// 规范化后的清单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub main: Option<String>,
    pub preload: Option<String>,
    pub logo: Option<String>,
    pub index_html: Option<String>,
    pub description: String,
    pub homepage: Option<String>,
    pub author: Option<String>,
    pub cmds: Vec<PluginCmd>,
    /// 可打开的扩展名（小写、不含点）。宿主据此把 `--open-file` 路由到插件，
    /// 设置页也用它决定是否提供"注册打开方式"。
    #[serde(default, rename = "fileTypes")]
    pub file_types: Vec<String>,
    /// 宿主版本要求（清单 `engines.efto`），如 `>=0.1.0`；缺省 = 不限制。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_requirement: Option<String>,
    /// 插件级平台限定（与 cmds[].platform 同一套词汇），空 = 全平台。
    #[serde(default)]
    pub platform: Vec<PlatformSpec>,
}

/// 宿主自身版本，用于校验清单 `engines.efto`。
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 校验 `engines.efto` 是否被当前宿主满足。
///
/// 只支持单条比较：`>=x.y.z` / `>x.y.z` / `<=x.y.z` / `<x.y.z` / `=x.y.z`
/// （省略操作符等价 `=`）。`^`、`~`、`||` 等其它写法明确报错，不会当作通过。
pub fn check_host_requirement(requirement: &str) -> std::result::Result<(), String> {
    let req = requirement.trim();
    let (op, rest) = if let Some(r) = req.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = req.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = req.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = req.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = req.strip_prefix('=') {
        ("=", r)
    } else {
        ("=", req)
    };
    let Some(want) = parse_version(rest.trim()) else {
        return Err(format!(
            "engines.efto 无法解析: {requirement:?}（只支持 >= > <= < = 加 x.y.z）"
        ));
    };
    let have = parse_version(HOST_VERSION).expect("宿主版本号必须可解析");
    let ok = match op {
        ">=" => have >= want,
        ">" => have > want,
        "<=" => have <= want,
        "<" => have < want,
        _ => have == want,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("需要 efto 宿主 {requirement}，当前 {HOST_VERSION}"))
    }
}

/// 解析 `x.y.z`（允许 1~3 段，缺省段按 0）；预发布等额外语法不支持。
fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut nums = [0u64; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        nums[i] = part.parse().ok()?;
    }
    Some((nums[0], nums[1], nums[2]))
}

impl PluginManifest {
    /// 插件整体在当前平台是否可用。不可用的插件不进搜索、不能被唤起；
    /// `cli plugins` 仍列出并标注（见 registry），运行入口报"不支持当前平台"。
    pub fn available_on_this_os(&self) -> bool {
        self.platform.is_empty() || self.platform.iter().any(|p| p.matches_current())
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let raw: RawManifest = serde_json::from_str(text).map_err(|e| {
            CoreError::InvalidManifest(format!("{NATIVE_MANIFEST} 解析失败: {e}"))
        })?;
        let name = if raw.name.trim().is_empty() {
            return Err(CoreError::InvalidManifest(format!(
                "{NATIVE_MANIFEST} 缺少 name"
            )));
        } else {
            raw.name.trim().to_string()
        };
        let author = raw.author.and_then(|v| match v {
            serde_json::Value::String(s) => Some(s),
            serde_json::Value::Object(o) => o
                .get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string()),
            _ => None,
        });
        let mut cmds: Vec<PluginCmd> = raw
            .cmds
            .into_iter()
            .filter(|c| !c.label.trim().is_empty())
            .collect();
        for c in cmds.iter_mut() {
            c.cmd = c.cmd.trim().to_string();
            c.label = c.label.trim().to_string();
        }
        // 扩展名统一成小写、不含前导点，去重保序：清单里写 ".MD" / " md " 都能用
        let mut file_types: Vec<String> = Vec::new();
        for raw in raw.file_types {
            let ext = raw.trim().trim_start_matches('.').to_ascii_lowercase();
            if !ext.is_empty() && !file_types.contains(&ext) {
                file_types.push(ext);
            }
        }
        Ok(Self {
            name,
            version: raw.version,
            main: raw.main.or_else(|| raw.plugin.as_ref().and_then(|p| p.main.clone())),
            preload: raw
                .preload
                .or_else(|| raw.plugin.as_ref().and_then(|p| p.preload.clone())),
            logo: raw.logo,
            index_html: raw.index,
            description: raw.description,
            homepage: raw.homepage,
            author,
            cmds,
            file_types,
            host_requirement: raw
                .engines
                .and_then(|e| e.efto)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            platform: raw.platform,
        })
    }
}

/// 磁盘上的插件：清单 + 解析出的入口文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub manifest: PluginManifest,
    pub root: PathBuf,
    /// 实际后端入口（可能是 .js 或 .ts）
    pub main_entry: Option<PathBuf>,
    pub preload_entry: Option<PathBuf>,
    pub ui_entry: Option<PathBuf>,
    /// 解析/校验期间遇到的问题，UI 要能显示
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 清单 `engines.efto` 与当前宿主不匹配。仍会列出（设置页能看到原因），
    /// 但不进搜索、不能被唤起。
    #[serde(default)]
    pub host_incompatible: bool,
}

fn default_true() -> bool {
    true
}

impl PluginDescriptor {
    pub fn id(&self) -> String {
        self.manifest.name.clone()
    }

    pub fn display_name(&self) -> &str {
        &self.manifest.name
    }

    /// 搜索用的触发词集合。
    pub fn triggers(&self) -> Vec<&PluginCmd> {
        self.manifest
            .cmds
            .iter()
            .filter(|c| c.available_on_this_os())
            .collect()
    }

    pub fn has_backend(&self) -> bool {
        self.main_entry.is_some()
    }

    pub fn has_ui(&self) -> bool {
        self.ui_entry.is_some()
    }

    /// 插件整体在当前平台是否可用（清单 `platform` 字段）。
    pub fn supported_on_this_os(&self) -> bool {
        self.manifest.available_on_this_os()
    }

    /// 宿主版本是否满足清单 `engines.efto`。
    pub fn host_compatible(&self) -> bool {
        !self.host_incompatible
    }

    /// 是否可以被搜索到、被唤起：启用 + 平台匹配 + 宿主版本匹配。
    pub fn usable(&self) -> bool {
        self.enabled && self.supported_on_this_os() && self.host_compatible()
    }
}

/// 目录里是否有插件清单。
pub fn has_manifest(root: &Path) -> bool {
    root.join(NATIVE_MANIFEST).is_file()
}

/// 从插件根目录加载清单并解析入口。
pub fn load_descriptor(root: &PathBuf) -> Result<PluginDescriptor> {
    let manifest_path = root.join(NATIVE_MANIFEST);
    let text = fsutil::read_to_string_opt(&manifest_path).ok_or_else(|| {
        CoreError::InvalidManifest(format!("{:?} 不存在或不可读", manifest_path))
    })?;
    let manifest = PluginManifest::from_json(&text)?;

    let mut problems = Vec::new();
    // 宿主版本不匹配：明确记录原因并整插件停用，不做"尽力加载"
    let host_incompatible = match manifest.host_requirement.as_deref() {
        Some(req) => match check_host_requirement(req) {
            Ok(()) => false,
            Err(msg) => {
                problems.push(msg);
                true
            }
        },
        None => false,
    };

    let main_entry = resolve_entry(root, manifest.main.as_deref(), &mut problems);
    let preload_entry = manifest
        .preload
        .as_deref()
        .and_then(|p| safe_join(root, p));
    if let Some(p) = &preload_entry {
        if !p.exists() {
            problems.push(format!("preload 不存在: {}", p.display()));
        }
    }

    let ui_entry = if let Some(idx) = manifest.index_html.as_deref() {
        safe_join(root, idx).filter(|p| p.exists())
    } else {
        ["index.html", "dist/index.html", "ui/index.html"]
            .iter()
            .map(|f| root.join(f))
            .find(|p| p.exists())
    };

    if main_entry.is_none() && ui_entry.is_none() {
        problems.push("既没有后端入口(main)也没有 UI 入口(index.html)，插件不可执行".into());
    }
    // 只有 main 没有 UI 是正常的"后端模式"：插件用 efto.output 内联结果，
    // 或只做通知/复制这类副作用，不算清单问题。

    Ok(PluginDescriptor {
        manifest,
        root: root.clone(),
        main_entry,
        preload_entry,
        ui_entry,
        problems,
        enabled: true,
        host_incompatible,
    })
}

/// main 缺省时按 js → ts 顺序探测常见入口。
fn resolve_entry(
    root: &Path,
    declared: Option<&str>,
    problems: &mut Vec<String>,
) -> Option<PathBuf> {
    let candidates: Vec<String> = match declared {
        Some(d) => {
            let trimmed = d.trim();
            let p = root.join(trimmed);
            if p.exists() {
                return Some(p);
            }
            // 声明的是 .js 但目录里只有 .ts（TS 开发常见）
            let mut alt = Vec::new();
            if let Some(stem) = trimmed
                .strip_suffix(".js")
                .or_else(|| trimmed.strip_suffix(".mjs"))
                .or_else(|| trimmed.strip_suffix(".cjs"))
            {
                alt.push(format!("{stem}.ts"));
                alt.push(format!("{stem}.mts"));
            }
            alt.push(trimmed.to_string());
            alt
        }
        None => vec![
            "main.js".into(),
            "main.ts".into(),
            "index.js".into(),
            "index.ts".into(),
            "dist/main.js".into(),
            "src/main.ts".into(),
        ],
    };
    for c in candidates {
        let p = root.join(&c);
        if p.exists() {
            if declared.is_some() && p.to_string_lossy().ends_with(".ts") {
                problems.push(format!(
                    "{NATIVE_MANIFEST} 声明 main={} 不存在，回退到 {}（宿主直接转译 TS）",
                    declared.unwrap(),
                    c
                ));
            }
            return Some(p);
        }
    }
    if let Some(d) = declared {
        problems.push(format!("找不到 main 入口: {d}"));
    }
    None
}

fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let p = root.join(rel);
    let norm = p.to_string_lossy().replace('\\', "/");
    if norm.contains("../") || norm.contains("..\\") {
        tracing::warn!("插件路径越界被拒绝: {rel}");
        return None;
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 插件级 `platform`：缺省/空 = 全平台；不匹配当前 OS 时整插件不可用。
    #[test]
    fn plugin_level_platform_gate() {
        let m = PluginManifest::from_json(r#"{"name":"h"}"#).unwrap();
        assert!(m.available_on_this_os(), "缺省必须是全平台");
        let m = PluginManifest::from_json(r#"{"name":"h","platform":[]}"#).unwrap();
        assert!(m.available_on_this_os(), "空数组必须是全平台");
        let m = PluginManifest::from_json(r#"{"name":"h","platform":[{"os":"plan9"}]}"#).unwrap();
        assert!(!m.available_on_this_os());
        // 与 cmds[].platform 同一套词汇：win32 / darwin / linux
        let m = PluginManifest::from_json(r#"{"name":"h","platform":[{"os":"win32"},{"os":"darwin"},{"os":"linux"}]}"#).unwrap();
        assert!(m.available_on_this_os(), "枚举了所有平台就要在任一平台可用");
    }

    /// `engines.efto`：只支持单条比较；不满足或写法不认识都必须报错，不能当作通过。
    #[test]
    fn host_requirement_comparisons() {
        assert!(check_host_requirement(">=0.1.0").is_ok());
        assert!(check_host_requirement(">=0.0.1").is_ok());
        assert!(check_host_requirement("<=0.1.0").is_ok());
        assert!(check_host_requirement("=0.1.0").is_ok());
        assert!(check_host_requirement("0.1.0").is_ok());
        assert!(check_host_requirement("0.1").is_ok(), "缺省段按 0");
        assert!(check_host_requirement(">0.1.0").is_err());
        assert!(check_host_requirement("<0.1.0").is_err());
        assert!(check_host_requirement(">9.0.0").is_err());
        // 不支持的写法明确报错
        assert!(check_host_requirement("^0.1.0").is_err());
        assert!(check_host_requirement("~0.1").is_err());
        assert!(check_host_requirement(">=0.1.0 || <0.3.0").is_err());
        assert!(check_host_requirement("latest").is_err());
    }

    #[test]
    fn incompatible_host_marks_descriptor_unusable() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(
            dir.path(),
            r#"{"name":"future","version":"1.0.0","main":"main.js","engines":{"efto":">9.0.0"}}"#,
            &[("main.js", "")],
        );
        let d = load_descriptor(&dir.path().to_path_buf()).unwrap();
        assert!(!d.host_compatible());
        assert!(!d.usable());
        assert!(
            d.problems.iter().any(|p| p.contains("需要 efto 宿主")),
            "{:?}",
            d.problems
        );

        // 缺省 engines 与满足要求都不受影响
        let dir2 = tempfile::tempdir().unwrap();
        write_plugin(dir2.path(), r#"{"name":"legacy","main":"main.js"}"#, &[("main.js", "")]);
        let d2 = load_descriptor(&dir2.path().to_path_buf()).unwrap();
        assert!(d2.host_compatible() && d2.usable());
        assert_eq!(d2.manifest.host_requirement, None);
    }

    #[test]
    fn normalizes_file_types() {
        let m = PluginManifest::from_json(
            r#"{"name":"md","fileTypes":[" .MD ","markdown","md",""]}"#,
        )
        .unwrap();
        assert_eq!(m.file_types, vec!["md", "markdown"]);
        // 未声明时为空，不得凭空给出扩展名
        let m = PluginManifest::from_json(r#"{"name":"h"}"#).unwrap();
        assert!(m.file_types.is_empty());
    }

    fn write_plugin(dir: &std::path::Path, manifest: &str, files: &[(&str, &str)]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(NATIVE_MANIFEST), manifest).unwrap();
        for (name, content) in files {
            std::fs::write(dir.join(name), content).unwrap();
        }
    }

    #[test]
    fn parses_manifest_and_ui_alias() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(NATIVE_MANIFEST),
            r#"{"name":"native-demo","version":"2.0.0","main":"main.js","ui":"app.html"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("main.js"), "").unwrap();
        std::fs::write(dir.path().join("app.html"), "").unwrap();
        let d = load_descriptor(&dir.path().to_path_buf()).unwrap();
        assert_eq!(d.manifest.name, "native-demo");
        assert!(d.has_ui());
    }

    #[test]
    fn parses_manifest_fields() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(
            dir.path(),
            r#"{
              "name": "demo",
              "version": "1.2.0",
              "main": "main.js",
              "logo": "logo.png",
              "author": {"name": "老单"},
              "cmds": [
                {"type": "input", "label": "演示", "cmd": "demo"},
                {"type": "over", "label": "文件", "platform": [{"os": "win32"}]},
                {"type": "nope", "label": "未知类型"}
              ]
            }"#,
            &[("main.js", "exports.onMainMessage=()=>{}"), ("index.html", "<p>hi")],
        );
        let d = load_descriptor(&dir.path().to_path_buf()).unwrap();
        assert_eq!(d.manifest.name, "demo");
        assert_eq!(d.manifest.author.as_deref(), Some("老单"));
        assert_eq!(d.manifest.cmds.len(), 3);
        assert_eq!(d.manifest.cmds[0].trigger(), "demo");
        assert_eq!(d.manifest.cmds[0].cmd_type, CmdType::Input);
        assert_eq!(d.manifest.cmds[2].cmd_type, CmdType::None);
        assert!(d.has_backend());
        assert!(d.has_ui());
        assert_eq!(d.id(), "demo");
    }

    #[test]
    fn ts_fallback_when_main_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(
            dir.path(),
            r#"{"name":"tsdemo","main":"main.js"}"#,
            &[("main.ts", "export {}"), ("index.html", "")],
        );
        let d = load_descriptor(&dir.path().to_path_buf()).unwrap();
        assert!(d
            .main_entry
            .as_deref()
            .unwrap()
            .to_string_lossy()
            .ends_with("main.ts"));
        assert!(d.problems.iter().any(|p| p.contains("TS")), "{:?}", d.problems);
    }

    #[test]
    fn defaults_probe_common_entries() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(dir.path(), r#"{"name":"p"}"#, &[("index.ts", "1")]);
        let d = load_descriptor(&dir.path().to_path_buf()).unwrap();
        assert!(d.main_entry.as_deref().unwrap().ends_with("index.ts"));
        // 只有 main 没有 UI 是正常的后端模式，不应记为清单问题
        assert!(!d.has_ui());
        assert!(d.problems.is_empty(), "{:?}", d.problems);
    }

    #[test]
    fn rejects_bad_manifests() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(dir.path().join(NATIVE_MANIFEST), "{ not json").unwrap();
        assert!(load_descriptor(&dir.path().to_path_buf()).is_err());
        std::fs::write(dir.path().join(NATIVE_MANIFEST), r#"{"version":"1"}"#).unwrap();
        assert!(load_descriptor(&dir.path().to_path_buf()).is_err());
    }

    #[test]
    fn blocks_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(
            dir.path(),
            r#"{"name":"esc","main":"main.js","index":"../../etc/passwd"}"#,
            &[("main.js", "")],
        );
        let d = load_descriptor(&dir.path().to_path_buf()).unwrap();
        assert!(d.ui_entry.is_none());
    }
}
