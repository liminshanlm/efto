//! Windows 原生剪贴板（文件列表）与键盘模拟。
//!
//! arboard 只覆盖文本/图片；文件列表要走 CF_HDROP，粘贴到前台窗口与
//! 通用动作抓取选中文本要走 SendInput。这两件事都集中在这里，
//! 命令层只调用，不各写一份 unsafe。

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{GlobalFree, HWND, POINT};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows_sys::Win32::System::Ole::CF_HDROP;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };
    use windows_sys::Win32::UI::Shell::{DragQueryFileW, DROPFILES, SHFileOperationW, SHFILEOPSTRUCTW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

    const CF_HDROP_U32: u32 = CF_HDROP as u32;

    fn open_clipboard() -> bool {
        for _ in 0..10 {
            // SAFETY: 传空 owner 是 GetClipboardData 文档允许的用法。
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        false
    }

    /// 读取剪贴板里的文件列表；没有 CF_HDROP 时返回空。
    pub fn read_files() -> Vec<String> {
        if !open_clipboard() {
            return Vec::new();
        }
        let mut out = Vec::new();
        unsafe {
            let handle = GetClipboardData(CF_HDROP_U32);
            if !handle.is_null() {
                let hdrop = handle as windows_sys::Win32::UI::Shell::HDROP;
                let count = DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0);
                for i in 0..count {
                    let len = DragQueryFileW(hdrop, i, std::ptr::null_mut(), 0);
                    if len == 0 {
                        continue;
                    }
                    let mut buf = vec![0u16; len as usize + 1];
                    let got = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), len + 1);
                    if got > 0 {
                        out.push(String::from_utf16_lossy(&buf[..got as usize]));
                    }
                }
            }
            CloseClipboard();
        }
        out
    }

    /// 纯函数：拼 CF_HDROP 需要的 DROPFILES + 宽字符路径（双 NUL 结尾）。
    fn dropfiles_bytes(paths: &[String]) -> Vec<u8> {
        let header = DROPFILES {
            pFiles: std::mem::size_of::<DROPFILES>() as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: 0,
            fWide: 1,
        };
        let mut data = Vec::new();
        unsafe {
            let raw = std::slice::from_raw_parts(
                &header as *const DROPFILES as *const u8,
                std::mem::size_of::<DROPFILES>(),
            );
            data.extend_from_slice(raw);
        }
        for path in paths {
            for unit in path.encode_utf16().chain(std::iter::once(0)) {
                data.extend_from_slice(&unit.to_le_bytes());
            }
        }
        data.extend_from_slice(&0u16.to_le_bytes());
        data
    }

    /// 把文件列表写进剪贴板（资源管理器里可直接 Ctrl+V）。
    pub fn write_files(paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Err("没有要复制的文件".into());
        }
        let data = dropfiles_bytes(paths);
        unsafe {
            let hmem = GlobalAlloc(GMEM_MOVEABLE, data.len());
            if hmem.is_null() {
                return Err("分配剪贴板内存失败".into());
            }
            let ptr = GlobalLock(hmem);
            if ptr.is_null() {
                GlobalFree(hmem);
                return Err("锁定剪贴板内存失败".into());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
            GlobalUnlock(hmem);

            if !open_clipboard() {
                GlobalFree(hmem);
                return Err("剪贴板被其他程序占用".into());
            }
            EmptyClipboard();
            if SetClipboardData(CF_HDROP_U32, hmem).is_null() {
                CloseClipboard();
                GlobalFree(hmem);
                return Err("写入剪贴板失败".into());
            }
            // 成功后内存所有权归系统，不能再 GlobalFree。
            CloseClipboard();
        }
        Ok(())
    }

    fn key(vk: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// 发送 Ctrl+<vk>。
    pub fn send_chord(vk: u16) {
        const VK_CONTROL: u16 = 0x11;
        let inputs = [
            key(VK_CONTROL, false),
            key(vk, false),
            key(vk, true),
            key(VK_CONTROL, true),
        ];
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    /// 向当前焦点窗口发送 Ctrl+C（通用动作抓取选中文本）。
    pub fn send_copy() {
        send_chord(0x43);
    }

    /// 向当前焦点窗口发送 Ctrl+V。
    pub fn send_paste() {
        send_chord(0x56);
    }

    /// 把前台窗口切回 `hwnd`，稍等激活后粘贴。
    ///
    /// 目标窗口以管理员权限运行、而 efto 不是时，Windows 会拒绝切换前台，
    /// 此时粘贴会落到当前焦点（通常是 efto 自己）。这里不假装成功。
    pub fn paste_to(hwnd: isize) -> Result<(), String> {
        if hwnd != 0 {
            let ok = unsafe { SetForegroundWindow(hwnd as HWND) };
            if ok == 0 {
                return Err("无法切回前台窗口（可能权限不同）".into());
            }
            std::thread::sleep(Duration::from_millis(120));
        }
        std::thread::sleep(Duration::from_millis(40));
        send_paste();
        Ok(())
    }

    /// 删除到回收站（SHFileOperationW + FOF_ALLOWUNDO）。
    pub fn recycle(paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut from: Vec<u16> = Vec::new();
        for path in paths {
            from.extend(path.encode_utf16());
            from.push(0);
        }
        from.push(0);
        let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
        op.wFunc = 3; // FO_DELETE
        op.pFrom = from.as_ptr();
        // FOF_SILENT | FOF_NOCONFIRMATION | FOF_ALLOWUNDO
        op.fFlags = (0x0004 | 0x0010 | 0x0040) as u16;
        let rc = unsafe { SHFileOperationW(&mut op) };
        if rc != 0 {
            return Err(format!("删除失败（错误码 {rc}）"));
        }
        if op.fAnyOperationsAborted != 0 {
            return Err("删除被取消".into());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dropfiles_layout_is_wide_double_null() {
            let data = dropfiles_bytes(&["C:\\a.txt".into(), "D:\\b c.txt".into()]);
            let header = std::mem::size_of::<DROPFILES>();
            assert_eq!(
                data.len(),
                header + ("C:\\a.txt".encode_utf16().count() + 1) * 2
                    + ("D:\\b c.txt".encode_utf16().count() + 1) * 2
                    + 2
            );
            let tail = &data[data.len() - 4..];
            assert_eq!(tail, &[0, 0, 0, 0], "最后一个路径与列表都要 NUL 结尾");
        }
    }
}

#[cfg(windows)]
pub use imp::{paste_to, read_files, recycle, send_copy, write_files};

#[cfg(not(windows))]
mod imp {
    pub fn read_files() -> Vec<String> {
        Vec::new()
    }
    pub fn write_files(_: &[String]) -> Result<(), String> {
        Err("文件剪贴板仅支持 Windows".into())
    }
    pub fn send_copy() {}
    pub fn send_paste() {}
    pub fn send_chord(_: u16) {}
    pub fn paste_to(_: isize) -> Result<(), String> {
        Err("粘贴到前台仅支持 Windows".into())
    }
    pub fn recycle(_: &[String]) -> Result<(), String> {
        Err("回收站删除仅支持 Windows".into())
    }
}

#[cfg(not(windows))]
pub use imp::{paste_to, read_files, recycle, send_copy, write_files};
