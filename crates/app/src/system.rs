//! 系统命令执行：睡眠、重启、注销、清空回收站、屏保。
//!
//! 破坏性动作（重启/注销/清空回收站）先弹原生确认框。这里只做"执行"，
//! 关键词判定在 `efto_core::system`，避免 UI 与判定各写一份。

use efto_core::system::SystemAction;

pub fn run(action: SystemAction) -> Result<String, String> {
    if action.needs_confirm() && !confirm(action) {
        return Ok("已取消".into());
    }
    match action {
        SystemAction::Sleep => {
            imp::sleep()?;
            Ok("已进入睡眠".into())
        }
        SystemAction::Restart => {
            imp::restart()?;
            Ok("正在重启…".into())
        }
        SystemAction::Logout => {
            imp::logout()?;
            Ok("正在注销…".into())
        }
        SystemAction::EmptyTrash => {
            imp::empty_trash()?;
            Ok("回收站已清空".into())
        }
        SystemAction::ScreenSaver => {
            imp::screensaver()?;
            Ok("已启动屏幕保护".into())
        }
    }
}

fn confirm(action: SystemAction) -> bool {
    imp::confirm(&format!("确定要{}吗？", action.title()))
}

#[cfg(windows)]
mod imp {
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND};
    use windows_sys::Win32::Graphics::Gdi::SC_SCREENSAVE;
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
        SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Power::SetSuspendState;
    use windows_sys::Win32::System::Shutdown::{ExitWindowsEx, EWX_FORCEIFHUNG, EWX_LOGOFF, EWX_REBOOT};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::UI::Shell::{
        SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, SendMessageW, HWND_BROADCAST, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND,
        MB_TOPMOST, MB_YESNO, WM_SYSCOMMAND,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn confirm(message: &str) -> bool {
        let text = wide(message);
        let title = wide("efto");
        unsafe {
            MessageBoxW(
                null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
            ) == IDYES
        }
    }

    /// 开启关机特权。没有它 `ExitWindowsEx(EWX_REBOOT)` 会静默失败。
    unsafe fn enable_shutdown_privilege() {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return;
        }
        let mut luid = std::mem::zeroed();
        if LookupPrivilegeValueW(null(), SE_SHUTDOWN_NAME, &mut luid) != 0 {
            let tp = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            AdjustTokenPrivileges(token, 0, &tp, 0, null_mut(), null_mut());
        }
        CloseHandle(token);
    }

    pub fn sleep() -> Result<(), String> {
        let ok = unsafe { SetSuspendState(0, 0, 0) };
        if ok == 0 {
            Err("进入睡眠失败（可能被电源策略拒绝）".into())
        } else {
            Ok(())
        }
    }

    pub fn restart() -> Result<(), String> {
        unsafe { enable_shutdown_privilege() };
        let ok = unsafe { ExitWindowsEx(EWX_REBOOT | EWX_FORCEIFHUNG, 0) };
        if ok == 0 {
            Err("重启请求被拒绝".into())
        } else {
            Ok(())
        }
    }

    pub fn logout() -> Result<(), String> {
        let ok = unsafe { ExitWindowsEx(EWX_LOGOFF | EWX_FORCEIFHUNG, 0) };
        if ok == 0 {
            Err("注销请求被拒绝".into())
        } else {
            Ok(())
        }
    }

    pub fn empty_trash() -> Result<(), String> {
        let code = unsafe {
            SHEmptyRecycleBinW(
                null_mut::<std::ffi::c_void>() as HWND,
                null(),
                SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
            )
        };
        if code == 0 {
            Ok(())
        } else {
            Err(format!("清空回收站失败（0x{code:08X}）"))
        }
    }

    pub fn screensaver() -> Result<(), String> {
        unsafe {
            SendMessageW(HWND_BROADCAST, WM_SYSCOMMAND, SC_SCREENSAVE as usize, 0);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    //! macOS 实现。系统工具（pmset / osascript / open）都是固定 argv、无用户输入
    //! 拼接，直接 spawn（先例：`commands.rs` 的 `pmset displaysleepnow`）。
    //!
    //! 确认框走 `osascript display dialog`（任意线程可用）——rfd 的 macOS 对话框
    //! 有主线程约束，这里不碰。重启/注销/清空废纸篓走 Apple Events，首次会弹
    //! Automation 授权（提示文案来自 Info.plist 的 NSAppleEventsUsageDescription）。

    /// AppleScript 字符串字面量转义：只处理 `\` 与 `"`，其余原样保留。
    fn apple_string(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// 跑一段 AppleScript，失败时把 stderr 一起带上（含 Automation 被拒的
    /// -1743、用户取消的 -128 等），绝不静默成功（硬性约定 3）。
    fn run_osascript(script: &str) -> Result<(), String> {
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("调用 osascript 失败: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    /// 确认框脚本：取消按钮/关闭 = 用户拒绝；点"确定"才放行。
    fn confirm_script(message: &str) -> String {
        format!(
            "display dialog \"{}\" buttons {{\"取消\", \"确定\"}} default button \"确定\" \
             cancel button \"取消\" with icon caution with title \"efto\"",
            apple_string(message)
        )
    }

    pub fn confirm(message: &str) -> bool {
        // display dialog 点"取消"（或按 Esc）时 osascript 以 -128 退出；
        // 其它失败（osascript 缺失等）保守地当作不放行，同时把错误打到 stderr。
        match run_osascript(&confirm_script(message)) {
            Ok(()) => true,
            Err(e) => {
                if !e.contains("(-128)") {
                    eprintln!("确认框弹出失败，已按取消处理: {e}");
                }
                false
            }
        }
    }

    pub fn sleep() -> Result<(), String> {
        // pmset 是系统自带工具，当前用户可用，无需提权
        let out = std::process::Command::new("pmset")
            .arg("sleepnow")
            .output()
            .map_err(|e| format!("进入睡眠失败：pmset sleepnow：{e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "进入睡眠失败：{}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    pub fn restart() -> Result<(), String> {
        run_osascript("tell application \"System Events\" to restart")
            .map_err(|e| format!("重启请求被拒绝：{e}"))
    }

    pub fn logout() -> Result<(), String> {
        run_osascript("tell application \"System Events\" to log out")
            .map_err(|e| format!("注销请求被拒绝：{e}"))
    }

    pub fn empty_trash() -> Result<(), String> {
        run_osascript("tell application \"Finder\" to empty trash")
            .map_err(|e| format!("清空回收站失败：{e}"))
    }

    pub fn screensaver() -> Result<(), String> {
        let out = std::process::Command::new("open")
            .arg("/System/Library/CoreServices/ScreenSaverEngine.app")
            .output()
            .map_err(|e| format!("启动屏幕保护失败：{e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "启动屏幕保护失败：{}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn apple_string_escapes_backslash_and_quote() {
            assert_eq!(apple_string(r#"a"b\c"#), r#"a\"b\\c"#);
            assert_eq!(apple_string("确定要重启电脑吗？"), "确定要重启电脑吗？");
        }

        #[test]
        fn confirm_script_has_cancel_and_confirm_buttons() {
            let s = confirm_script("确定要重启电脑吗？");
            assert!(s.contains(r#"display dialog "确定要重启电脑吗？""#), "{s}");
            assert!(s.contains(r#"buttons {"取消", "确定"}"#), "{s}");
            assert!(s.contains(r#"cancel button "取消""#), "{s}");
            // 消息里的引号/反斜杠必须被转义进 AppleScript 字面量
            let s = confirm_script(r#"say "hi" \now"#);
            assert!(s.contains(r#"say \"hi\" \\now"#), "{s}");
        }
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
mod imp {
    /// 其它非 Windows 平台没有实现；不支持的动作在执行步返回明确错误
    /// （硬性约定 3）。confirm 返回 true 让流程走到执行步——不能返回 false，
    /// 那会把"不支持"吞成"已取消"。
    pub fn confirm(_message: &str) -> bool {
        true
    }
    pub fn sleep() -> Result<(), String> {
        Err("当前平台不支持：睡眠".into())
    }
    pub fn restart() -> Result<(), String> {
        Err("当前平台不支持：重启电脑".into())
    }
    pub fn logout() -> Result<(), String> {
        Err("当前平台不支持：注销当前用户".into())
    }
    pub fn empty_trash() -> Result<(), String> {
        Err("当前平台不支持：清空回收站".into())
    }
    pub fn screensaver() -> Result<(), String> {
        Err("当前平台不支持：启动屏幕保护".into())
    }
}
