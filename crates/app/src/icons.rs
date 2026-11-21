//! 取"系统为这个文件分配的图标"（exe / .lnk / .ico / 插件 logo），转成 data URL。
//!
//! 每次搜索只对可见结果按需调用，结果由宿主缓存在 `AppData::icon_cache`。

/// 读取 `path` 的图标并编码为 `data:image/png;base64,...`。
///
/// 插件 logo 本身就是 png 时直接透传，否则走系统图标（exe / .lnk / .ico 都是这条）。
pub fn data_url(path: &str) -> Result<String, String> {
    let p = std::path::Path::new(path);
    if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png")) {
        if let Ok(bytes) = std::fs::read(p) {
            if bytes.len() <= 2 * 1024 * 1024 {
                use base64::Engine as _;
                return Ok(format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ));
            }
        }
    }
    shell_icon_data_url(path)
}

/// 把本地图片缩放到 `max` 像素见方，编码为 data URL（剪贴板图片缩略图用）。
pub fn thumbnail_data_url(path: &str, max: u32) -> Result<String, String> {
    let img = image::open(path).map_err(|e| format!("读取图片失败: {e}"))?;
    let thumb = img.thumbnail(max, max).to_rgba8();
    crate::screenshot::data_url(&thumb)
}

/// 读取 `path` 的系统图标并编码为 `data:image/png;base64,...`。
#[cfg(windows)]
fn shell_icon_data_url(path: &str) -> Result<String, String> {
    use windows_sys::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut info: SHFILEINFOW = std::mem::zeroed();
        let rc = SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if rc == 0 || info.hIcon.is_null() {
            return Err("系统没有为该文件提供图标".into());
        }
        let rgba = icon_rgba(info.hIcon);
        DestroyIcon(info.hIcon);
        let (width, height, bytes) = rgba?;
        let img = image::RgbaImage::from_raw(width, height, bytes)
            .ok_or_else(|| "图标像素无效".to_string())?;
        crate::screenshot::data_url(&img)
    }
}

/// 读取 `path` 的系统图标并编码为 `data:image/png;base64,...`（macOS：NSWorkspace）。
///
/// NSWorkspace 对不存在的路径只会给一个通用图标，和 Windows 侧
/// "SHGetFileInfoW 失败即报错"不一致——这里先查存在性，让前端回退首字块，
/// 而不是拿通用图标充数。AppKit 绑定层不要求主线程（iconForFile / NSBitmapImageRep
/// 均非 MainThreadOnly），且 icon_for 是同步 command，调用线程安全。
#[cfg(target_os = "macos")]
fn shell_icon_data_url(path: &str) -> Result<String, String> {
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSString};

    if !std::path::Path::new(path).exists() {
        return Err("文件不存在".into());
    }
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        let image = ws.iconForFile(&NSString::from_str(path));
        let tiff = image.TIFFRepresentation().ok_or("图标没有 TIFF 表示")?;
        let rep = NSBitmapImageRep::imageRepWithData(&tiff).ok_or("图标位图解析失败")?;
        let empty = NSDictionary::<NSString, AnyObject>::new();
        let png = rep
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &empty)
            .ok_or("图标 PNG 编码失败")?;
        use base64::Engine as _;
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png.to_vec())
        ))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn shell_icon_data_url(_path: &str) -> Result<String, String> {
    Err("图标读取仅支持 Windows / macOS".into())
}

/// 把 HICON 拆成 RGBA 像素。
///
/// 彩色位图里带 alpha 的图标（Vista 之后的绝大多数）直接用；
/// 老式图标 alpha 全 0，就退回 AND mask 反推透明区域。
#[cfg(windows)]
unsafe fn icon_rgba(
    hicon: windows_sys::Win32::UI::WindowsAndMessaging::HICON,
) -> Result<(u32, u32, Vec<u8>), String> {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut info: ICONINFO = std::mem::zeroed();
    if GetIconInfo(hicon, &mut info) == 0 {
        return Err("GetIconInfo 失败".into());
    }
    let result = (|| -> Result<(u32, u32, Vec<u8>), String> {
        if info.hbmColor.is_null() {
            return Err("图标没有彩色位图".into());
        }
        let mut bm: BITMAP = std::mem::zeroed();
        if GetObjectW(
            info.hbmColor as _,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bm as *mut _ as *mut _,
        ) == 0
        {
            return Err("读取图标尺寸失败".into());
        }
        let width = bm.bmWidth.max(0) as u32;
        let height = bm.bmHeight.max(0) as u32;
        if width == 0 || height == 0 {
            return Err("图标尺寸为 0".into());
        }

        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return Err("创建图标设备上下文失败".into());
        }
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = width as i32;
        bi.bmiHeader.biHeight = -(height as i32); // 自顶向下
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let lines = GetDIBits(
            dc,
            info.hbmColor,
            0,
            height,
            rgba.as_mut_ptr() as *mut _,
            &mut bi,
            DIB_RGB_COLORS,
        );
        if lines == 0 {
            DeleteDC(dc);
            return Err("读取图标像素失败".into());
        }
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }

        if !rgba.chunks_exact(4).any(|px| px[3] != 0) {
            // 没有 alpha 的老图标：mask 位为 1 的地方是透明
            let stride = ((width as usize + 31) / 32) * 4;
            let mut mask = vec![0u8; stride * height as usize];
            let mut bi1: BITMAPINFO = std::mem::zeroed();
            bi1.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi1.bmiHeader.biWidth = width as i32;
            bi1.bmiHeader.biHeight = -(height as i32);
            bi1.bmiHeader.biPlanes = 1;
            bi1.bmiHeader.biBitCount = 1;
            bi1.bmiHeader.biCompression = BI_RGB;
            let ok = !info.hbmMask.is_null()
                && GetDIBits(
                    dc,
                    info.hbmMask,
                    0,
                    height,
                    mask.as_mut_ptr() as *mut _,
                    &mut bi1,
                    DIB_RGB_COLORS,
                ) != 0;
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let alpha = if ok {
                        let bit = (mask[y * stride + x / 8] >> (7 - (x % 8))) & 1;
                        if bit == 1 { 0 } else { 255 }
                    } else {
                        255
                    };
                    rgba[(y * width as usize + x) * 4 + 3] = alpha;
                }
            }
        }
        DeleteDC(dc);
        Ok((width, height, rgba))
    })();

    if !info.hbmColor.is_null() {
        DeleteObject(info.hbmColor as _);
    }
    if !info.hbmMask.is_null() {
        DeleteObject(info.hbmMask as _);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn reads_system_icon_for_explorer() {
        let path = r"C:\Windows\explorer.exe";
        if !std::path::Path::new(path).exists() {
            return;
        }
        let url = data_url(path).expect("应该能取到 explorer.exe 的图标");
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.len() > 200, "图标数据太小: {}", url.len());
    }

    #[cfg(windows)]
    #[test]
    fn missing_file_is_an_error() {
        assert!(data_url(r"C:\Windows\definitely-not-here-xyz.exe").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_system_icon_for_finder() {
        let path = "/System/Library/CoreServices/Finder.app";
        let url = data_url(path).expect("应该能取到 Finder 的图标");
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.len() > 200, "图标数据太小: {}", url.len());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn missing_file_is_an_error() {
        assert!(data_url("/definitely/not/here.app").is_err());
    }
}
