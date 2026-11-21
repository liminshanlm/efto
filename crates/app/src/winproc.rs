//! 顶层窗口枚举与前台切换。
//!
//! 只服务 `cli ps` / `cli focus`：命令行通道需要"看有哪些窗口、把某个窗口切到前台"，
//! 但没有 GUI 线程与 Tauri 上下文，所以直接走 Win32。不参与 GUI 的窗口模型。

use serde::Serialize;

/// 一个可见的顶层窗口。
#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub pid: u32,
    pub hwnd: isize,
    pub process: String,
    pub title: String,
}

/// 按进程名、标题排序后的窗口列表（给人看的顺序）。
pub fn windows() -> Vec<WindowInfo> {
    let mut list = visible_windows();
    list.sort_by(|a, b| {
        a.process
            .to_lowercase()
            .cmp(&b.process.to_lowercase())
            .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    list
}

/// 按锁定顺序（Z 序，最上层在前）找第一个匹配的窗口。只读，不改窗口状态。
pub fn find(pid: Option<u32>, needle: &str) -> Option<WindowInfo> {
    let needle = needle.trim().to_lowercase();
    visible_windows().into_iter().find(|w| {
        if let Some(want) = pid {
            return w.pid == want;
        }
        !needle.is_empty()
            && (w.title.to_lowercase().contains(&needle)
                || w.process.to_lowercase().contains(&needle))
    })
}

/// 把窗口切到前台；最小化时先还原。
///
/// 前台切换会被 Windows 的"前台锁定"拒绝，这种情况返回错误而不是假装成功。
pub fn activate(w: &WindowInfo) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
        };
        let hwnd = w.hwnd as *mut core::ffi::c_void;
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd) == 0 && GetForegroundWindow() != hwnd {
                return Err(format!(
                    "Windows 拒绝把 {} ({}) 切到前台（可能被前台锁定阻止）",
                    w.title, w.process
                ));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        Err("窗口聚焦仅支持 Windows".into())
    }
}

/// 取指定窗口的信息；窗口已销毁或没有标题时返回 `None`。
#[cfg(windows)]
pub fn info(hwnd: isize) -> Option<WindowInfo> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    };

    let raw = hwnd as *mut core::ffi::c_void;
    unsafe {
        if IsWindow(raw) == 0 {
            return None;
        }
        let len = GetWindowTextLengthW(raw);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(raw, buf.as_mut_ptr(), buf.len() as i32);
        if n <= 0 {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(raw, &mut pid);
        Some(WindowInfo {
            pid,
            hwnd,
            process: process_name(pid),
            title: String::from_utf16_lossy(&buf[..n as usize]),
        })
    }
}

#[cfg(not(windows))]
pub fn info(_hwnd: isize) -> Option<WindowInfo> {
    None
}

/// 当前前台窗口；切到桌面或锁屏时可能没有。
#[cfg(windows)]
pub fn foreground() -> Option<WindowInfo> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return None;
    }
    info(hwnd as isize)
}

#[cfg(not(windows))]
pub fn foreground() -> Option<WindowInfo> {
    None
}

#[cfg(windows)]
fn visible_windows() -> Vec<WindowInfo> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam as *mut Vec<WindowInfo>);
        if IsWindowVisible(hwnd) == 0 {
            return TRUE;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return TRUE; // 无标题的顶层窗口（托盘隐藏窗、输入法等）不算
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n <= 0 {
            return TRUE;
        }
        let title = String::from_utf16_lossy(&buf[..n as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        out.push(WindowInfo {
            pid,
            hwnd: hwnd as isize,
            process: process_name(pid),
            title,
        });
        TRUE
    }

    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe {
        EnumWindows(Some(collect), &mut out as *mut _ as LPARAM);
    }
    out
}

/// 进程主模块文件名（如 `chrome.exe`）；读不到时返回空串，不编造名字。
#[cfg(windows)]
pub(crate) unsafe fn process_name(pid: u32) -> String {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return String::new();
    }
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return String::new(); // 提权进程读不到，不编造名字
    }
    let mut buf = [0u16; 1024];
    let mut size = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
    CloseHandle(handle);
    if ok == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..size as usize]);
    std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(path)
}

#[cfg(not(windows))]
fn visible_windows() -> Vec<WindowInfo> {
    Vec::new()
}
