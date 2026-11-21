//! 原生剪贴板（文件列表）与键盘模拟（Windows / macOS）。
//!
//! arboard 只覆盖文本/图片；文件列表 Windows 走 CF_HDROP、macOS 走 NSPasteboard
//! file URL，粘贴到前台窗口与通用动作抓取选中文本分别走 SendInput / CGEvent。
//! 这些 unsafe 都集中在这里，命令层只调用，不各写一份。

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

#[cfg(target_os = "macos")]
mod imp {
    //! macOS 实现：NSPasteboard（file URL）读写文件列表、NSFileManager 回收站、
    //! CGEvent 模拟 Cmd 组合。与 Windows 版的差异：
    //! - `paste_to(hwnd)` 的 hwnd 无意义（mac 没有窗口句柄），语义 = 向前台窗口发 Cmd+V；
    //! - `send_chord(vk)` 的 vk 按 win32 VK 码解释，仅支持 ASCII 字母（映射 kVK_ANSI_*），
    //!   其余明确报错；
    //! - `send_copy`/`send_paste` 签名与 Windows 版一致（无返回值），无辅助功能权限时
    //!   不空跑，记 `tracing::error!`；`paste_to` 返回 `Result`，权限缺失直接报给调用方。

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::ClassType;
    use objc2_app_kit::{NSPasteboard, NSPasteboardWriting};
    use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
    use objc2_foundation::{NSArray, NSFileManager, NSString, NSURL};

    // AXIsProcessTrusted（HIServices）：进程是否已获辅助功能（Accessibility）授权。
    // CGEvent 发布键盘事件需要它，无授权时事件会被系统丢弃。
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }

    fn accessibility_trusted() -> bool {
        // SAFETY: 无参无副作用的纯查询；HIServices 的 Boolean 按 u8 返回。
        unsafe { AXIsProcessTrusted() != 0 }
    }

    /// 把文件列表写进指定粘贴板（Finder 里可直接 Cmd+V）。
    fn write_files_to(pb: &NSPasteboard, paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Err("没有要复制的文件".into());
        }
        let urls: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths
            .iter()
            .map(|p| ProtocolObject::from_retained(NSURL::fileURLWithPath(&NSString::from_str(p))))
            .collect();
        let objects = NSArray::from_retained_slice(&urls);
        pb.clearContents();
        if pb.writeObjects(&objects) {
            Ok(())
        } else {
            Err("写入剪贴板失败（NSPasteboard writeObjects 返回 false）".into())
        }
    }

    /// 读取指定粘贴板里的文件列表（public.file-url）；没有文件时返回空。
    fn read_files_from(pb: &NSPasteboard) -> Vec<String> {
        let classes = NSArray::from_slice(&[NSURL::class()]);
        // SAFETY: class_array 声明的类是 NSURL，与读取选项（无）匹配。
        let Some(items) = (unsafe { pb.readObjectsForClasses_options(&classes, None) }) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in items.iter() {
            let Some(url) = item.downcast_ref::<NSURL>() else {
                continue;
            };
            if !url.isFileURL() {
                continue;
            }
            // filePathURL 把文件引用 URL（file:///.file/id=...）解析成路径 URL。
            if let Some(path) = url.filePathURL().and_then(|u| u.path()) {
                out.push(path.to_string());
            }
        }
        out
    }

    pub fn read_files() -> Vec<String> {
        read_files_from(&NSPasteboard::generalPasteboard())
    }

    pub fn write_files(paths: &[String]) -> Result<(), String> {
        write_files_to(&NSPasteboard::generalPasteboard(), paths)
    }

    /// win32 VK 码（仅 ASCII 大写字母）→ macOS 虚拟键码 kVK_ANSI_*（Carbon Events.h）。
    fn mac_keycode_for_vk(vk: u16) -> Result<CGKeyCode, String> {
        // A..Z 对应的 kVK_ANSI_* 键码
        const KVK_ANSI_LETTERS: [u16; 26] = [
            0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D,
            0x1F, 0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06,
        ];
        match vk {
            0x41..=0x5A => Ok(KVK_ANSI_LETTERS[(vk - 0x41) as usize]),
            _ => Err(format!(
                "当前平台不支持：send_chord（macOS 仅支持 ASCII 字母键的 Cmd 组合，收到 VK 0x{vk:02X}）"
            )),
        }
    }

    /// 向系统事件流发一次 Cmd+<key>（按下+抬起各一个事件）。
    fn post_cmd_key(key: CGKeyCode) -> Result<(), String> {
        for key_down in [true, false] {
            let Some(ev) = CGEvent::new_keyboard_event(None, key, key_down) else {
                return Err("创建键盘事件失败".into());
            };
            CGEvent::set_flags(Some(&ev), CGEventFlags::MaskCommand);
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&ev));
        }
        Ok(())
    }

    /// 发送 Cmd+<vk>（vk 按 win32 VK 码，仅 ASCII 字母）。
    pub fn send_chord(vk: u16) -> Result<(), String> {
        if !accessibility_trusted() {
            return Err(
                "缺少辅助功能（Accessibility）权限：无法模拟键盘按键。请在 系统设置 → 隐私与安全性 → 辅助功能 中勾选 efto"
                    .into(),
            );
        }
        post_cmd_key(mac_keycode_for_vk(vk)?)
    }

    /// 向当前焦点窗口发送 Cmd+C（通用动作抓取选中文本）。
    ///
    /// 签名与 Windows 版一致（无返回值），失败只记 error 日志，绝不空跑假装成功。
    pub fn send_copy() {
        if let Err(e) = send_chord(0x43) {
            tracing::error!("发送 Cmd+C 失败: {e}");
        }
    }

    /// 向当前焦点窗口发送 Cmd+V。
    pub fn send_paste() -> Result<(), String> {
        send_chord(0x56)
    }

    /// 向前台窗口粘贴。macOS 没有 hwnd，忽略参数（等价 [`send_paste`]）。
    pub fn paste_to(_hwnd: isize) -> Result<(), String> {
        std::thread::sleep(std::time::Duration::from_millis(40));
        send_paste()
    }

    /// 删除到回收站（NSFileManager trashItemAtURL，无需额外权限）。
    pub fn recycle(paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let fm = NSFileManager::defaultManager();
        for p in paths {
            let url = NSURL::fileURLWithPath(&NSString::from_str(p));
            if let Err(e) = fm.trashItemAtURL_resultingItemURL_error(&url, None) {
                return Err(format!(
                    "删除到回收站失败（{p}）: {}",
                    e.localizedDescription()
                ));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn vk_letters_map_to_ansi_keycodes() {
            // win32 VK 码 → kVK_ANSI_*（Carbon HIToolbox/Events.h）
            assert_eq!(mac_keycode_for_vk(0x41).unwrap(), 0x00); // A
            assert_eq!(mac_keycode_for_vk(0x43).unwrap(), 0x08); // C
            assert_eq!(mac_keycode_for_vk(0x56).unwrap(), 0x09); // V
            assert_eq!(mac_keycode_for_vk(0x5A).unwrap(), 0x06); // Z
        }

        #[test]
        fn vk_non_letters_are_explicitly_rejected() {
            let e = mac_keycode_for_vk(0x11).unwrap_err(); // VK_CONTROL
            assert!(e.contains("当前平台不支持：send_chord"), "{e}");
        }

        #[test]
        fn write_and_read_files_roundtrip_on_private_pasteboard() {
            let dir = tempfile::tempdir().unwrap();
            let a = dir.path().join("a.txt");
            let b = dir.path().join("带空格 b.txt");
            std::fs::write(&a, "x").unwrap();
            std::fs::write(&b, "y").unwrap();
            let paths = vec![
                a.to_string_lossy().into_owned(),
                b.to_string_lossy().into_owned(),
            ];
            // 独立粘贴板，不污染用户剪贴板
            let pb = NSPasteboard::pasteboardWithUniqueName();
            write_files_to(&pb, &paths).unwrap();
            assert_eq!(read_files_from(&pb), paths);
        }

        #[test]
        fn write_files_rejects_empty_list() {
            let pb = NSPasteboard::pasteboardWithUniqueName();
            assert_eq!(write_files_to(&pb, &[]).unwrap_err(), "没有要复制的文件");
        }

        #[test]
        fn read_files_from_empty_pasteboard_is_empty() {
            let pb = NSPasteboard::pasteboardWithUniqueName();
            assert!(read_files_from(&pb).is_empty());
        }

        #[test]
        fn recycle_empty_list_is_noop() {
            assert!(recycle(&[]).is_ok());
        }

        #[test]
        fn recycle_missing_path_fails_with_reason() {
            let e = recycle(&["/nonexistent/efto-recycle-test".into()]).unwrap_err();
            assert!(e.contains("删除到回收站失败"), "{e}");
        }

        #[test]
        fn recycle_moves_file_to_trash() {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("efto-recycle-live.txt");
            std::fs::write(&p, "x").unwrap();
            recycle(&[p.to_string_lossy().into_owned()]).unwrap();
            assert!(!p.exists(), "回收站删除后原路径应不存在");
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{paste_to, read_files, recycle, send_copy, write_files};

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    //! 未支持的平台：能力缺失必须明确报错，不留空操作桩。
    //! `read_files` 例外：剪贴板轮询每 500ms 调一次，返回空列表表示"没有文件"，
    //! 不刷错误日志；写入/粘贴/回收站走 Result 通道明确报错。
    pub fn read_files() -> Vec<String> {
        Vec::new()
    }
    pub fn write_files(_: &[String]) -> Result<(), String> {
        Err("当前平台不支持：写入文件剪贴板".into())
    }
    pub fn send_copy() {
        tracing::error!("当前平台不支持：send_copy");
    }
    pub fn send_paste() {
        tracing::error!("当前平台不支持：send_paste");
    }
    pub fn send_chord(_: u16) {
        tracing::error!("当前平台不支持：send_chord");
    }
    pub fn paste_to(_: isize) -> Result<(), String> {
        Err("当前平台不支持：粘贴到前台".into())
    }
    pub fn recycle(_: &[String]) -> Result<(), String> {
        Err("当前平台不支持：删除到回收站".into())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub use imp::{paste_to, read_files, recycle, send_copy, write_files};
