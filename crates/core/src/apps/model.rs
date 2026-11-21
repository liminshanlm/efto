use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::search::SearchText;

/// 条目来自哪个发现渠道，用于排序先验与 UI 标注。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSource {
    /// 开始菜单 .lnk（名称最规范，优先）
    StartMenu,
    /// 桌面快捷方式（用户级 + 公共桌面）
    Desktop,
    /// 注册表 Uninstall 项（正规产品名）
    Registry,
    /// 安装目录裸 exe
    DirScan,
    /// macOS .app bundle（/Applications 等），与开始菜单同级的规范来源
    AppBundle,
    /// 设置里自定义索引目录中的 .lnk/.url
    Custom,
    /// 用户手动添加
    Manual,
}

impl AppSource {
    /// 排序先验：同样分数下，开始菜单/.app > 桌面 > 注册表 > 手动 > 自定义目录 > 目录扫描。
    pub fn prior(&self) -> f32 {
        match self {
            AppSource::StartMenu | AppSource::AppBundle => 1.00,
            AppSource::Desktop => 0.98,
            AppSource::Registry => 0.92,
            AppSource::Manual => 0.90,
            AppSource::Custom => 0.80,
            AppSource::DirScan => 0.78,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AppSource::StartMenu => "start_menu",
            AppSource::Desktop => "desktop",
            AppSource::Registry => "registry",
            AppSource::DirScan => "dir_scan",
            AppSource::AppBundle => "app_bundle",
            AppSource::Custom => "custom",
            AppSource::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// 直接可执行文件
    Exe,
    /// 快捷方式，交给 ShellExecute（Windows 会解析 target/参数/工作目录/提权）
    Shortcut,
    /// .url 网址快捷方式
    Url,
    /// ms-settings: 等 URI 协议
    Uri,
    /// 文件夹
    Folder,
    /// macOS .app bundle，交给 `open`（LaunchServices）启动
    AppBundle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchTarget {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    pub kind: TargetKind,
}

impl LaunchTarget {
    pub fn exe(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            args: None,
            working_dir: None,
            kind: TargetKind::Exe,
        }
    }

    pub fn shortcut(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            args: None,
            working_dir: None,
            kind: TargetKind::Shortcut,
        }
    }

    pub fn pathbuf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

/// 一条可被搜索、可被启动的应用记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    /// 稳定 id：小写化的实际启动目标路径（快捷方式则用 .lnk 路径）。
    pub id: String,
    pub name: String,
    pub source: AppSource,
    pub target: LaunchTarget,
    /// 图标来源（exe/lnk 路径，或带索引的 DisplayIcon 原值）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 额外搜索别名：去扩展名、发布方、中文名等。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub use_count: u32,
    #[serde(default)]
    pub last_used_ts: i64,
    /// 索引期派生的匹配文本，不落盘，重建索引时生成。
    #[serde(skip, default)]
    pub search: SearchText,
}

impl AppEntry {
    pub fn new(
        name: impl Into<String>,
        target: LaunchTarget,
        source: AppSource,
    ) -> Self {
        let name = name.into();
        let id = make_id(&target.path);
        Self {
            id,
            name,
            source,
            target,
            icon: None,
            aliases: vec![],
            use_count: 0,
            last_used_ts: 0,
            search: SearchText::default(),
        }
    }

    /// 快捷方式/裸 exe/.app 背后的真实启动目标路径（用于跨渠道去重与取图标）。
    pub fn exe_hint(&self) -> Option<&str> {
        match self.target.kind {
            TargetKind::Exe | TargetKind::AppBundle => Some(self.target.path.as_str()),
            TargetKind::Shortcut => self.icon.as_deref(),
            _ => None,
        }
    }

    /// 去重键：能解析到真实 exe 就用 exe，否则退回启动目标本身。
    pub fn resolve_key(&self) -> String {
        self.exe_hint()
            .map(make_id)
            .unwrap_or_else(|| self.id.clone())
    }

    /// 补全派生字段，必须在进索引前调用。
    pub fn finalize(&mut self) {
        let mut texts = vec![self.name.clone()];
        texts.extend(self.aliases.iter().cloned());
        // "Google Chrome (x64)" → "Google Chrome"
        if let Some(clean) = cleanup_name(&self.name) {
            if clean != self.name {
                texts.push(clean);
            }
        }
        // 裸 exe 的文件名也作为别名（"chrome" 匹配 "Google Chrome"）
        if let Some(stem) = self.target.pathbuf().file_stem().and_then(|s| s.to_str()) {
            let stem = stem.to_string();
            if !stem.eq_ignore_ascii_case(&self.name) {
                texts.push(stem);
            }
        }
        self.search = SearchText::build(&texts);
    }
}

/// 去掉噪声后缀/前缀，返回更"干净"的产品名。
pub fn cleanup_name(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_string();
    loop {
        let before = s.clone();
        // 尾随括号注解： (x64) (64-bit) (中文版) [beta]
        s = s
            .rsplit_once(['(', '['])
            .map(|(head, _)| head.trim_end().to_string())
            .unwrap_or_else(|| s.clone());
        // 版本尾巴： "App 1.2.3"
        if let Some(idx) = s.find(char::is_whitespace) {
            let tail = s[idx..].trim();
            if tail.len() >= 3
                && tail.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                && tail
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
            {
                s = s[..idx].trim_end().to_string();
            }
        }
        if before == s {
            break;
        }
    }
    if s.is_empty() || s == raw {
        None
    } else {
        Some(s)
    }
}

pub fn make_id(path: &str) -> String {
    path.trim().trim_end_matches('\\').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_cleanup() {
        assert_eq!(cleanup_name("Google Chrome (x64)").as_deref(), Some("Google Chrome"));
        assert_eq!(cleanup_name("Notepad++ [64 bit]").as_deref(), Some("Notepad++"));
        assert_eq!(cleanup_name("ZoomIt 15.21").as_deref(), Some("ZoomIt"));
        assert_eq!(cleanup_name("Code").as_deref(), None);
    }

    #[test]
    fn id_is_normalized() {
        assert_eq!(make_id("C:\\Windows\\System32\\notepad.exe "), "c:\\windows\\system32\\notepad.exe");
    }
}
