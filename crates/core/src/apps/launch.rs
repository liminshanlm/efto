//! 启动执行。核心原则：
//! - 普通 exe 直接 spawn（保留参数与工作目录）；
//! - .lnk / .url / URI / 文件夹 / msc 交给 Shell，让 Windows 解析提权与关联；
//! - 任何失败都回退一次 Shell 路径，避免"点了没反应"。

use std::process::{Command, Stdio};

use crate::error::{CoreError, Result};
use crate::search::HitPayload;

use super::model::{LaunchTarget, TargetKind};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 拆分命令行参数（Windows 风格：支持双引号与转义）。
pub fn split_args(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut has = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quote = !in_quote;
                has = true;
            }
            '\\' if in_quote && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            c if (c == ' ' || c == '\t') && !in_quote => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has || in_quote {
        out.push(cur);
    }
    out
}

fn needs_shell(kind: TargetKind) -> bool {
    !matches!(kind, TargetKind::Exe)
}

pub fn launch(target: &LaunchTarget) -> Result<()> {
    if target.path.trim().is_empty() {
        return Err(CoreError::Launch("启动目标为空".into()));
    }
    if needs_shell(target.kind) {
        return shell_open(&target.path, target.args.as_deref());
    }
    match spawn_exe(target) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::debug!("直接启动失败({e})，回退 Shell: {}", target.path);
            shell_open(&target.path, target.args.as_deref())
        }
    }
}

fn spawn_exe(target: &LaunchTarget) -> Result<()> {
    let mut cmd = Command::new(&target.path);
    if let Some(args) = &target.args {
        cmd.args(split_args(args));
    }
    if let Some(dir) = &target.working_dir {
        if std::path::Path::new(dir).is_dir() {
            cmd.current_dir(dir);
        }
    }
    // 三个标准流都不能继承启动器的：否则子进程会持有启动器的管道/控制台句柄，
    // 表现为"启动器退出了但子进程还在写"，CLI 下更会直接卡住父进程读取。
    // 控制台程序由 Windows 自己分配新控制台，交互性不受影响。
    let spawned = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| CoreError::Launch(format!("{}: {e}", target.path)))?;
    tracing::info!("已启动 pid={} {}", spawned.id(), target.path);
    Ok(())
}

/// 交给系统外壳：cmd /c start 能处理 .lnk/.url/URI/.msc/文件夹/关联类型。
pub fn shell_open(path: &str, args: Option<&str>) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/C", "start", "", path]);
        if let Some(a) = args {
            cmd.args(split_args(a));
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| CoreError::Launch(format!("shell open {path}: {e}")))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let _ = args;
        Command::new("open")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| CoreError::Launch(format!("open {path}: {e}")))?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = args;
        Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| CoreError::Launch(format!("xdg-open {path}: {e}")))?;
        Ok(())
    }
}

/// 用系统"打开方式"对话框选择程序打开文件。
#[cfg(windows)]
pub fn open_with(path: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;

    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = "openas\0".encode_utf16().collect();
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    } as isize;
    if rc <= 32 {
        return Err(CoreError::Launch(format!("打开方式失败（{rc}）: {path}")));
    }
    Ok(())
}

/// 在资源管理器中定位（选中）某个路径。
///
/// explorer.exe 成功时也可能返回非 0，所以只 spawn 不等待退出码。
pub fn reveal_in_folder(path: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 整段 `/select,<path>` 作为一个参数交给 explorer，路径含空格时由系统自行处理
        Command::new("explorer.exe")
            .arg(format!("/select,{path}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| CoreError::Launch(format!("explorer /select {path}: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let parent = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        shell_open(&parent, None)
    }
}

/// 执行一个搜索命中的 payload。插件/动作由宿主另行处理，这里只管"打开"。
/// 返回 true 表示本层处理掉了。
pub fn apply_payload(payload: &HitPayload) -> Result<bool> {
    match payload {
        HitPayload::Launch { target } => {
            launch(target)?;
            Ok(true)
        }
        HitPayload::Open { path } => {
            shell_open(path, None)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_splitting() {
        assert_eq!(split_args("a b  c"), vec!["a", "b", "c"]);
        assert_eq!(split_args("\"C:\\My Dir\\f.txt\" --flag"), vec!["C:\\My Dir\\f.txt", "--flag"]);
        assert_eq!(split_args("say \"hi \\\"there\\\"\""), vec!["say", "hi \"there\""]);
        assert_eq!(split_args(""), Vec::<String>::new());
    }

    #[test]
    fn empty_target_errors() {
        let t = LaunchTarget {
            path: " ".into(),
            args: None,
            working_dir: None,
            kind: TargetKind::Exe,
        };
        assert!(launch(&t).is_err());
    }
}
