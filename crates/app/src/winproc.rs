//! 顶层窗口枚举与前台切换。
//!
//! 只服务 `cli ps` / `cli focus`：命令行通道需要"看有哪些窗口、把某个窗口切到前台"，
//! 但没有 GUI 线程与 Tauri 上下文，所以直接走 Win32 / CGWindowList+AppKit。
//! 不参与 GUI 的窗口模型。

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
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        // 只用 pid 激活整个应用；hwnd 是 CGWindowID，AppKit 没有单窗口前置 API。
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(w.pid as i32)
        else {
            return Err(format!("没有匹配的窗口: 找不到 pid {} 对应的运行中应用", w.pid));
        };
        // IgnoringOtherApps 在 macOS 14+ 已无效（binding 里标了 deprecated），
        // 旧系统上带上它激活更可靠。
        #[allow(deprecated)]
        let ok = app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        if ok {
            Ok(())
        } else {
            Err(format!(
                "macOS 拒绝把 {} ({}) 切到前台",
                w.title, w.process
            ))
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = w;
        Err("当前平台不支持：窗口聚焦".into())
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

/// 取指定窗口的信息；macOS 上 hwnd 装的是 CGWindowID（kCGWindowNumber），
/// 按它在当前可见窗口列表里查找。
#[cfg(target_os = "macos")]
pub fn info(hwnd: isize) -> Option<WindowInfo> {
    if hwnd <= 0 {
        return None;
    }
    visible_windows().into_iter().find(|w| w.hwnd == hwnd)
}

#[cfg(not(any(windows, target_os = "macos")))]
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

/// 当前前台窗口；macOS 上取窗口列表（前到后排序）的第一个普通窗口。
#[cfg(target_os = "macos")]
pub fn foreground() -> Option<WindowInfo> {
    visible_windows().into_iter().next()
}

#[cfg(not(any(windows, target_os = "macos")))]
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

/// 从 CG 窗口字典取 CFString 值（无权限时标题可能缺失，调用方自行降级）。
#[cfg(target_os = "macos")]
fn cf_str(dict: &objc2_core_foundation::CFDictionary, key: &objc2_core_foundation::CFString) -> Option<String> {
    use core::ffi::c_void;
    use objc2_core_foundation::{CFString, CFType};

    // CFDictionaryGetValue 按指针取值；字典的 K/V 是异质 CFType，用 CFGetTypeID 校验后降级。
    let ptr = unsafe { dict.value(key as *const CFString as *const c_void) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: 非空指针来自 CFDictionary，必是有效 CF 对象。
    let val = unsafe { &*(ptr as *const CFType) };
    val.downcast_ref::<CFString>().map(|s| s.to_string())
}

/// 从 CG 窗口字典取 CFNumber 值（pid、layer、window id）。
#[cfg(target_os = "macos")]
fn cf_num(dict: &objc2_core_foundation::CFDictionary, key: &objc2_core_foundation::CFString) -> Option<i64> {
    use core::ffi::c_void;
    use objc2_core_foundation::{CFNumber, CFString, CFType};

    let ptr = unsafe { dict.value(key as *const CFString as *const c_void) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: 同 cf_str。
    let val = unsafe { &*(ptr as *const CFType) };
    val.downcast_ref::<CFNumber>().and_then(|n| n.as_i64())
}

#[cfg(target_os = "macos")]
fn visible_windows() -> Vec<WindowInfo> {
    use objc2_core_foundation::{CFArray, CFType};
    use objc2_core_graphics::{
        kCGNullWindowID, kCGWindowLayer, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName,
        kCGWindowOwnerPID, CGWindowListCopyWindowInfo, CGWindowListOption,
    };

    // 前到后排序（最前在前）；排除桌面元素（壁纸、Dock 等）。
    // 键静态是 extern，读取需要 unsafe。
    let Some(list) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    let (key_layer, key_pid, key_owner, key_name, key_id) = unsafe {
        (
            kCGWindowLayer,
            kCGWindowOwnerPID,
            kCGWindowOwnerName,
            kCGWindowName,
            kCGWindowNumber,
        )
    };

    // SAFETY: CGWindowListCopyWindowInfo 的元素都是 CFDictionary，此处只把
    // 元素类型参数换成 CFType 以便逐个 downcast（cast_unchecked 不改运行时行为）。
    let items: &CFArray<CFType> = unsafe { list.cast_unchecked() };
    let mut out = Vec::new();
    for item in items.iter() {
        let Some(dict) = item.downcast_ref::<objc2_core_foundation::CFDictionary>() else {
            continue;
        };
        // 只要普通窗口层（kCGNormalWindowLevel = 0），过滤菜单/状态项/屏保层
        if cf_num(dict, key_layer) != Some(0) {
            continue;
        }
        // pid 与进程名（ownerName）是必有信息；两者缺一则跳过
        let (Some(pid), Some(process)) = (cf_num(dict, key_pid), cf_str(dict, key_owner)) else {
            continue;
        };
        // 标题需要屏幕录制权限（macOS 10.15+），无权限时为空——允许的降级
        let title = cf_str(dict, key_name).unwrap_or_default();
        let id = cf_num(dict, key_id).unwrap_or(0);
        out.push(WindowInfo {
            pid: pid as u32,
            hwnd: id as isize,
            process,
            title,
        });
    }
    out
}

#[cfg(not(any(windows, target_os = "macos")))]
fn visible_windows() -> Vec<WindowInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_info_serializes_fields() {
        let w = WindowInfo {
            pid: 7,
            hwnd: 3,
            process: "Safari".into(),
            title: "首页".into(),
        };
        let v = serde_json::to_value(&w).unwrap();
        assert_eq!(v["pid"], 7);
        assert_eq!(v["hwnd"], 3);
        assert_eq!(v["process"], "Safari");
        assert_eq!(v["title"], "首页");
    }

    #[test]
    fn find_without_pid_or_needle_matches_nothing() {
        // 不给 pid 也不给关键词时不许乱匹配（cmd_focus 在上层拒绝这种入参，
        // 这里守住 find 本身的语义）
        assert!(find(None, "  ").is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn info_for_missing_window_is_none() {
        assert!(info(0).is_none());
        assert!(info(999_999_999).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn activate_unknown_pid_reports_no_matching_window() {
        let w = WindowInfo {
            pid: u32::MAX,
            hwnd: 0,
            process: String::new(),
            title: String::new(),
        };
        let e = activate(&w).unwrap_err();
        // 文案对齐 cli focus 的 not_found 语义
        assert!(e.contains("没有匹配的窗口"), "{e}");
    }

    /// 真实窗口枚举：无头 CI 没有可见窗口时优雅跳过，不算失败。
    #[cfg(target_os = "macos")]
    #[test]
    fn visible_windows_enumerates_or_skips_when_headless() {
        let list = windows();
        if list.is_empty() {
            println!("skip: 当前会话没有可见窗口（无头环境），跳过真实窗口枚举断言");
            return;
        }
        for w in &list {
            assert_ne!(w.pid, 0, "CGWindow 列表应带 owner pid: {w:?}");
            assert!(!w.process.is_empty(), "进程名（ownerName）不能为空: {w:?}");
        }
        // windows() 按 进程名 / 标题 排序（不区分大小写）
        for pair in list.windows(2) {
            let a = (pair[0].process.to_lowercase(), pair[0].title.to_lowercase());
            let b = (pair[1].process.to_lowercase(), pair[1].title.to_lowercase());
            assert!(a <= b, "windows() 应按 进程名/标题 排序: {a:?} !<= {b:?}");
        }
    }
}
