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

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 没有原生确认框；返回 true 让流程继续，不支持的动作会在执行步
    /// 返回明确错误（硬性约定 3）。不能返回 false——那会把"不支持"吞成"已取消"。
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
