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
    #[serde(default)]
    plugin: Option<PluginSection>,
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
}

impl PluginManifest {
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
