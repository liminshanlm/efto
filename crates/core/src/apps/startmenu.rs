//! 开始菜单：用户级 + 公共级 .lnk/.url 收集。

use std::fs;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use super::lnk;
use super::model::{AppEntry, AppSource, LaunchTarget, TargetKind};

/// 开始菜单目录（Programs）。
pub fn start_menu_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(roaming) = std::env::var_os("APPDATA") {
        out.push(
            Path::new(&roaming)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    if let Some(programdata) = std::env::var_os("PROGRAMDATA") {
        out.push(
            Path::new(&programdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    // 公共用户（部分镜像/企业环境把快捷方式放这里）
    if let Some(public) = std::env::var_os("PUBLIC") {
        let p = Path::new(&public)
            .join("AppData")
            .join("Roaming")
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        if p.is_dir() {
            out.push(p);
        }
    }
    out.retain(|p| p.is_dir());
    out
}

/// 桌面目录（用户桌面 + 公共桌面）。
///
/// 用户桌面走 `dirs::desktop_dir()`，能跟随 OneDrive/重定向等系统设置；
/// 公共桌面固定为 `%PUBLIC%\Desktop`，只在目录存在时收录。
pub fn desktop_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = dirs::desktop_dir() {
        out.push(d);
    }
    if let Some(public) = std::env::var_os("PUBLIC") {
        let p = Path::new(&public).join("Desktop");
        if p.is_dir() {
            out.push(p);
        }
    }
    out.retain(|p| p.is_dir());
    out.dedup();
    out
}

pub struct IgnoreMatcher {
    set: GlobSet,
}

impl Default for IgnoreMatcher {
    fn default() -> Self {
        Self::new(&[])
    }
}

/// 编译忽略规则。**必须**显式打开 case_insensitive。
///
/// `is_ignored` 先把路径 `to_lowercase()` 再匹配，而 globset 默认区分大小写，
/// 于是过去所有含大写字母的规则都静默不生效：默认列表里的 `Temp` / `WinSxS` /
/// `WindowsApps` / `$Recycle.Bin`（固定列表里同样形式的三条也是），
/// 以及用户在 settings.json 里最自然写出的 `**/Steam/**`、`**/Adobe/**`。
/// Windows 路径本来就不区分大小写，规则跟着不区分才是用户预期的语义。
fn compile_glob(p: &str) -> Result<Glob, globset::Error> {
    GlobBuilder::new(p).case_insensitive(true).build()
}

impl IgnoreMatcher {
    pub fn new(patterns: &[String]) -> Self {
        let mut b = GlobSetBuilder::new();
        for p in patterns {
            match compile_glob(p) {
                Ok(g) => {
                    b.add(g);
                }
                Err(e) => tracing::warn!("忽略规则 {p} 无效: {e}"),
            }
        }
        // 固定噪音目录：只放"永远不可能是用户想启动的程序位置"的系统内部目录。
        // Temp 不在这里——它留在可编辑的默认设置里（见 config.rs）。把 Temp 做成
        // 不可移除的硬规则，等于剥夺用户索引临时目录里绿色软件的能力；
        // 而目录扫描那侧本来就有按名字剪枝的黑名单，硬编码这条 glob 只会多咬到
        // 开始菜单遍历这种边缘场景（本项目两个用 tempfile 的测试就是这么被打挂的）。
        for p in [
            "**/$Recycle.Bin/**",
            "**/WindowsApps/**",
            "**/WinSxS/**",
            "**/assembly/**",
            "**/~1/**",
        ] {
            if let Ok(g) = compile_glob(p) {
                b.add(g);
            }
        }
        Self {
            set: b.build().unwrap_or_else(|_| GlobSet::empty()),
        }
    }

    pub fn is_ignored(&self, path: &Path) -> bool {
        let s = path.to_string_lossy().to_lowercase();
        if self.set.is_match(&s) {
            return true;
        }
        // Windows 短名 / 隐藏系统目录
        s.contains("\\windowsapps\\") || s.contains("/windowsapps/")
    }
}

/// 收集一个目录下所有快捷方式（开始菜单，层级不做限制）。
pub fn collect(dirs: &[PathBuf], ignore: &IgnoreMatcher) -> Vec<AppEntry> {
    collect_limited(dirs, ignore, usize::MAX)
}

/// 带深度上限的收集。桌面可能挂着很大的文件夹，不能无限递归。
pub fn collect_limited(
    dirs: &[PathBuf],
    ignore: &IgnoreMatcher,
    max_depth: usize,
) -> Vec<AppEntry> {
    let mut out = Vec::new();
    for root in dirs {
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let rd = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for de in rd.flatten() {
                let path = de.path();
                if ignore.is_ignored(&path) {
                    continue;
                }
                let ft = match de.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if ft.is_dir() {
                    // "启动"文件夹是自启动项而非应用，跳过
                    if depth < max_depth && !is_startup_folder(&path) {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                if let Some(entry) = entry_from_link(&path) {
                    out.push(entry);
                }
            }
        }
    }
    out
}

fn is_startup_folder(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(name.as_str(), "startup" | "启动")
}

fn entry_from_link(path: &Path) -> Option<AppEntry> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "lnk" => entry_from_lnk(path),
        "url" => entry_from_url_file(path),
        _ => None,
    }
}

fn entry_from_lnk(path: &Path) -> Option<AppEntry> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 80 {
        return None;
    }
    let link = lnk::parse(&bytes)?;
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let name = link
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| stem.clone());

    let lnk_target = LaunchTarget {
        path: path.to_string_lossy().to_string(),
        args: None,
        working_dir: None,
        kind: TargetKind::Shortcut,
    };

    match &link.target {
        // 解析出了目标：存在才收录（卸载残留的快捷方式点了只会弹"找不到文件"）
        Some(t) => {
            let p = Path::new(t);
            if !(p.is_file() || p.is_dir()) {
                tracing::debug!("跳过失效快捷方式 {} -> {t}", path.display());
                return None;
            }
            // 直接以 .lnk 启动：Windows 负责参数、工作目录与提权
            let mut entry = AppEntry::new(name, lnk_target, AppSource::StartMenu);
            entry.icon = Some(t.clone());
            entry.aliases = vec![stem];
            entry.finalize();
            Some(entry)
        }
        // 只有 IDList：控制面板项、已知文件夹、任务（Magnify/Narrator/Administrative Tools 等）。
        // 文件系统上无路径可解析，但 ShellExecute 认得 .lnk，照样可以启动。
        None => {
            let mut entry = AppEntry::new(name, lnk_target, AppSource::StartMenu);
            // 拿不到目标路径时（AppX/命名空间项）直接用 .lnk 自身取图标，
            // 否则前端只能退回首字头像（如 "Snipping Tool"）
            entry.icon = Some(path.to_string_lossy().to_string());
            entry.aliases = vec![stem];
            entry.finalize();
            Some(entry)
        }
    }
}

/// .url 是 INI 文本：`[InternetShortcut]` + `URL=...`。
fn entry_from_url_file(path: &Path) -> Option<AppEntry> {
    let text = fs::read_to_string(path).ok()?;
    let url = text
        .lines()
        .find_map(|l| l.strip_prefix("URL="))
        .map(|s| s.trim().to_string())?;
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let name = text
        .lines()
        .find_map(|l| l.strip_prefix("FriendlyName="))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(stem.clone());
    let mut entry = AppEntry::new(
        name,
        LaunchTarget {
            path: url,
            args: None,
            working_dir: None,
            kind: TargetKind::Url,
        },
        AppSource::StartMenu,
    );
    entry.aliases = vec![stem];
    entry.finalize();
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_matcher() {
        let m = IgnoreMatcher::new(&["**/node_modules/**".to_string()]);
        assert!(m.is_ignored(Path::new("C:\\a\\node_modules\\b.exe")));
        assert!(m.is_ignored(Path::new("C:\\x\\WindowsApps\\y")));
        assert!(!m.is_ignored(Path::new("C:\\Program Files\\a.exe")));
    }

    /// 大小写无关回归。
    ///
    /// 由来：`is_ignored` 把路径 `to_lowercase()` 后交给 globset，而 globset 默认
    /// 区分大小写 —— 结果**所有含大写字母的规则都是死的**。当时默认列表里
    /// Temp / WinSxS / $Recycle.Bin 三条完全没作用（WindowsApps 只靠 is_ignored
    /// 末尾那条硬编码字符串兜底才没漏），用户在 settings.json 里写
    /// `**/Steam/**` 也不会生效，而且一声不响。
    #[test]
    fn ignore_rules_are_case_insensitive() {
        let m = IgnoreMatcher::new(&["**/Steam/**".to_string()]);
        assert!(
            m.is_ignored(Path::new(r"C:\Program Files\Steam\steamapps\a.exe")),
            "用户写的混合大小写规则必须生效"
        );
        assert!(
            m.is_ignored(Path::new(r"c:\program files\steam\a.exe")),
            "路径本身小写时同样要命中"
        );
        // 固定列表里的混合大小写项要真的靠 glob 生效，而不是靠兜底字符串
        let fixed = IgnoreMatcher::new(&[]);
        for p in [
            r"C:\Windows\WinSxS\amd64_microsoft-windows-a\app.exe",
            r"C:\$Recycle.Bin\S-1-5-21-x\dead.exe",
            r"C:\Windows\Microsoft.NET\assembly\GAC_64\Foo\app.exe",
        ] {
            assert!(fixed.is_ignored(Path::new(p)), "{p} 应被固定噪音目录过滤");
        }
        // Temp 是"可编辑的默认规则"，不是不可移除的硬规则（见 IgnoreMatcher::new 的注释）
        let with_defaults = IgnoreMatcher::new(&crate::config::Settings::default().ignore_globs);
        assert!(
            with_defaults.is_ignored(Path::new(r"C:\Users\x\AppData\Local\Temp\wix\a.exe")),
            "默认设置里的 **/Temp/** 必须真的生效"
        );
    }

    /// 每条默认规则都必须真的命中一个真实形状的路径。
    ///
    /// 这条是针对"死规则"这个类别本身写的：之前列表里是
    /// `"**/ assemblies/**"`（多一个前导空格），只能匹配名字真以空格开头的目录，
    /// 躺了很久没人发现——因为忽略规则一条测试都没有。逐条断言才能把这类
    /// 拼写错误变成编译期/测试期失败，而不是"看起来配了、其实没配"。
    #[test]
    fn every_default_ignore_rule_matches_something() {
        let samples = [
            ("**/WindowsApps/**", r"C:\Program Files\WindowsApps\Vendor.App_1.0\x\app.exe"),
            ("**/$Recycle.Bin/**", r"C:\$Recycle.Bin\S-1-5-21-x\dead.exe"),
            ("**/WinSxS/**", r"C:\Windows\WinSxS\amd64_microsoft-windows-a\app.exe"),
            ("**/Temp/**", r"C:\Users\x\AppData\Local\Temp\wix\a.exe"),
            ("**/node_modules/**", r"C:\x\node_modules\y\cli.exe"),
            ("**/.git/**", r"D:\repo\.git\hooks\pre-push.exe"),
            ("**/assemblies/**", r"C:\Program Files\App\assemblies\lib.exe"),
        ];
        let defaults = crate::config::Settings::default().ignore_globs;
        for (pat, path) in samples {
            assert!(defaults.iter().any(|d| d == pat), "默认列表里找不到 {pat}");
            // 单独编译这一条：IgnoreMatcher 会附加固定列表，混在一起就测不出这条自己是否有效
            let mut b = GlobSetBuilder::new();
            b.add(compile_glob(pat).unwrap());
            let set = b.build().unwrap();
            assert!(
                set.is_match(path.to_lowercase()),
                "{pat} 命中不了 {path}（多半是拼写错误造成的死规则）"
            );
        }
    }

    /// 反向：规则改成大小写无关之后，正常程序路径不能被误杀。
    /// 特别是 `**/Temp/**` 必须只匹配**整段目录名**，不能退化成子串匹配
    /// （Tempero / SteamWorks 这类名字挨着就无辜被过滤，是更难排查的方向）。
    #[test]
    fn defaults_do_not_kill_normal_app_paths() {
        let m = IgnoreMatcher::new(&crate::config::Settings::default().ignore_globs);
        for p in [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Tempero\bin\tempero.exe",
            r"C:\Program Files\Assembly Studio\studio.exe",
            r"D:\Games\SteamWorks\game.exe",
            r"C:\Program Files\Git\cmd\git.exe",
        ] {
            assert!(!m.is_ignored(Path::new(p)), "不该误杀 {p}");
        }
        let steam = IgnoreMatcher::new(&["**/Steam/**".to_string()]);
        assert!(
            !steam.is_ignored(Path::new(r"D:\Games\SteamWorks\game.exe")),
            "**/Steam/** 不该吃掉 SteamWorks（整段匹配，不是子串）"
        );
    }

    #[test]
    fn collects_url_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("My Site.url");
        fs::write(
            &f,
            "[InternetShortcut]\r\nURL=https://qwenwork.cn\r\nFriendlyName=千问办公\r\n",
        )
        .unwrap();
        let entries = collect(&[dir.path().to_path_buf()], &IgnoreMatcher::default());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "千问办公");
        assert_eq!(entries[0].target.kind, TargetKind::Url);
        assert_eq!(entries[0].target.path, "https://qwenwork.cn");
    }

    #[test]
    fn depth_limit_skips_deep_folders() {
        let dir = tempfile::tempdir().unwrap();
        let mut deep = dir.path().to_path_buf();
        for i in 0..5 {
            deep = deep.join(format!("d{i}"));
        }
        fs::create_dir_all(&deep).unwrap();
        let url = "[InternetShortcut]\r\nURL=https://example.com\r\n";
        fs::write(deep.join("deep.url"), url).unwrap();
        fs::write(dir.path().join("top.url"), url).unwrap();
        let got = collect_limited(&[dir.path().to_path_buf()], &IgnoreMatcher::default(), 2);
        let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"top"), "{names:?}");
        assert!(!names.contains(&"deep"), "{names:?}");
    }

    #[test]
    fn startup_folder_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let startup = dir.path().join("Startup");
        fs::create_dir_all(&startup).unwrap();
        fs::write(startup.join("x.lnk"), b"junk").unwrap();
        let entries = collect(&[dir.path().to_path_buf()], &IgnoreMatcher::default());
        assert!(entries.is_empty());
    }
}
