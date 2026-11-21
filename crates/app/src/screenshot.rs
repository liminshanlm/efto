//! 截图：捕获屏幕、选区裁剪、PNG 编码、滚动拼接、标注层解码。
//!
//! 只负责"像素"这一层；窗口编排在 [`crate::windows`]，命令在 [`crate::commands`]，
//! 标注的绘制与合成在前端 `screenshot.html`（见 docs/architecture.md）。

use base64::Engine as _;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// 一次进行中的长截图会话。后台线程持有 `Arc`，停止标志与进度从这里读写。
pub struct LongCapture {
    pub stop: Arc<AtomicBool>,
    /// 已追加的总行数（物理像素），供覆盖层画"框在变长"。
    pub rows: Arc<AtomicU32>,
}

impl LongCapture {
    pub fn new(stop: Arc<AtomicBool>, rows: Arc<AtomicU32>) -> Self {
        Self { stop, rows }
    }

    /// 请求停止（Esc / 新截图 / 退出）。
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn extra_rows(&self) -> u32 {
        self.rows.load(Ordering::SeqCst)
    }
}

/// 一次全屏捕获的结果（RGBA，自顶向下）。
#[derive(Debug, Clone)]
pub struct Shot {
    pub width: u32,
    pub height: u32,
    /// 该屏幕区域在虚拟桌面中的物理坐标原点（钉图窗口定位用）。
    pub origin_x: i32,
    pub origin_y: i32,
    pub rgba: Vec<u8>,
}

impl Shot {
    /// 整屏 RGBA 图像（选区窗口的背景图）。
    pub fn to_image(&self) -> Result<image::RgbaImage, String> {
        image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())
            .ok_or_else(|| "截图为空".into())
    }

    /// 裁剪一块区域。坐标是相对虚拟桌面左上角的物理像素，越界部分会被夹到图像内。
    pub fn crop(&self, x: i32, y: i32, w: u32, h: u32) -> Result<image::RgbaImage, String> {
        if self.width == 0 || self.height == 0 {
            return Err("截图为空".into());
        }
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        let w = w.clamp(1, self.width - x);
        let h = h.clamp(1, self.height - y);
        let mut buf = Vec::with_capacity((w * h * 4) as usize);
        for row in y..(y + h) {
            let start = ((row * self.width + x) * 4) as usize;
            buf.extend_from_slice(&self.rgba[start..start + (w * 4) as usize]);
        }
        image::RgbaImage::from_raw(w, h, buf).ok_or_else(|| "裁剪尺寸无效".into())
    }
}

/// PNG 字节。
pub fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;
    Ok(buf)
}

/// `data:image/png;base64,...`，直接给 `<img src>` 用。
pub fn data_url(img: &image::RgbaImage) -> Result<String, String> {
    let png = png_bytes(img)?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

/// 解析前端标注层回传的 PNG data URL（`data_url` 的逆操作）。
///
/// 截图覆盖层把"原始画面 + 标注"合成到 canvas 后以 data URL 提交，
/// 宿主用它替代对整屏图的裁剪；复制 / 保存 / 钉图共用这一个入口。
pub fn overlay_png(data_url: &str) -> Result<image::RgbaImage, String> {
    let b64 = data_url
        .trim()
        .strip_prefix("data:image/png;base64,")
        .ok_or("标注层必须是 PNG data URL")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("解码标注层失败: {e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("解析标注层失败: {e}"))?;
    Ok(img.to_rgba8())
}

/// 捕获虚拟桌面上一块区域（`x/y` 为虚拟桌面物理像素，`width/height` 为像素尺寸）。
///
/// 长截图按固定选区反复调用它；覆盖层窗口在抓帧前已经关掉，所以抓到的是底层真实画面。
/// 会连同桌面上的分层窗口一起抓（CAPTUREBLT）。
#[cfg(windows)]
pub fn capture_region(x: i32, y: i32, width: u32, height: u32) -> Result<Shot, String> {
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT,
        DIB_RGB_COLORS, SRCCOPY,
    };

    let width = width.max(1);
    let height = height.max(1);
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err("无法获取屏幕设备上下文".into());
        }
        let mem = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width as i32, height as i32);
        if mem.is_null() || bitmap.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap as _);
            }
            if !mem.is_null() {
                DeleteDC(mem);
            }
            ReleaseDC(std::ptr::null_mut(), screen);
            return Err("创建位图失败".into());
        }

        let old = SelectObject(mem, bitmap as _);
        let copied = BitBlt(
            mem,
            0,
            0,
            width as i32,
            height as i32,
            screen,
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        );
        // GetDIBits 要求位图不在 DC 里，先还原
        SelectObject(mem, old);

        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = width as i32;
        bi.bmiHeader.biHeight = -(height as i32); // 负数 = 自顶向下
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let lines = if copied != 0 {
            GetDIBits(
                mem,
                bitmap,
                0,
                height,
                rgba.as_mut_ptr() as *mut _,
                &mut bi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        DeleteObject(bitmap as _);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);

        if lines == 0 {
            return Err("截取屏幕失败".into());
        }
        // 32bpp BI_RGB 给的是 BGRA（alpha 不可信），换成 RGBA
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        Ok(Shot {
            width,
            height,
            origin_x: x,
            origin_y: y,
            rgba,
        })
    }
}

#[cfg(not(windows))]
pub fn capture_region(_x: i32, _y: i32, width: u32, height: u32) -> Result<Shot, String> {
    let _ = (width, height);
    Err("截图仅支持 Windows".into())
}

/// 捕获整个虚拟桌面（所有显示器拼成的矩形）。
///
/// 只抓鼠标所在显示器的话，用户在搜索框里点“截图”时鼠标一定在启动器那块屏上，
/// 其它屏幕永远选不到。所以这里一次性抓全部显示器，覆盖层也铺满虚拟桌面。
#[cfg(windows)]
pub fn capture_virtual_screen() -> Result<Shot, String> {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };

    /// 收集每个显示器的物理像素矩形；GetMonitorInfo 对 DPI-aware 进程给的就是物理坐标。
    unsafe extern "system" fn collect_monitor(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let rects = unsafe { &mut *(data as *mut Vec<(i32, i32, i32, i32)>) };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            let rc = info.rcMonitor;
            rects.push((rc.left, rc.top, rc.right, rc.bottom));
        }
        TRUE
    }

    let mut rects: Vec<(i32, i32, i32, i32)> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut rects as *mut _ as LPARAM,
        )
    };
    if ok == 0 || rects.is_empty() {
        return Err("无法读取显示器信息".into());
    }
    let x = rects.iter().map(|r| r.0).min().unwrap();
    let y = rects.iter().map(|r| r.1).min().unwrap();
    let right = rects.iter().map(|r| r.2).max().unwrap();
    let bottom = rects.iter().map(|r| r.3).max().unwrap();
    capture_region(x, y, (right - x).max(1) as u32, (bottom - y).max(1) as u32)
}

#[cfg(not(windows))]
pub fn capture_virtual_screen() -> Result<Shot, String> {
    Err("截图仅支持 Windows".into())
}

/// 估计两帧之间内容向上滚动的行数。
///
/// 语义：`next` 是把画面向上滚过 `d` 行后的结果，底部露出 `d` 行新内容，
/// 因此 `next[y] ≈ prev[y + d]`。只比较屏幕上方三分之一处的一条横带，
/// 避开吸顶栏（不随滚动移动）和底部动画；返回 `None` 表示没有可靠重叠。
pub fn scroll_delta(prev: &[u8], next: &[u8], width: u32, height: u32) -> Option<u32> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h < 64 {
        return None;
    }
    let row_bytes = w * 4;
    if prev.len() < row_bytes * h || next.len() < row_bytes * h {
        return None;
    }

    // 取样带：跳过顶部 1/5（吸顶栏），带高 48 行、每 2 行取一行
    let band = 48.min(h / 3).max(8);
    let y0 = (h / 5).min(h.saturating_sub(band + 1));
    let max_d = h.saturating_sub(y0 + band);
    if max_d < 8 {
        return None;
    }
    let col_step = (w / 64).max(1);
    let cols: Vec<usize> = (0..w).step_by(col_step).collect();

    let mut best_err = f32::MAX;
    let mut best_d = 0usize;
    for d in 0..=max_d {
        let mut err = 0f64;
        let mut n = 0f64;
        let mut y = 0;
        while y < band {
            let a = (y0 + y) * row_bytes;
            let b = (y0 + y + d) * row_bytes;
            for &c in &cols {
                let i = c * 4;
                err += (next[a + i] as f64 - prev[b + i] as f64).abs()
                    + (next[a + i + 1] as f64 - prev[b + i + 1] as f64).abs()
                    + (next[a + i + 2] as f64 - prev[b + i + 2] as f64).abs();
                n += 3.0;
            }
            y += 2;
        }
        let score = (err / n) as f32;
        if score < best_err {
            best_err = score;
            best_d = d;
        }
    }
    if best_err > 12.0 {
        None
    } else {
        Some(best_d as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot() -> Shot {
        // 3x2 的假图，每像素 RGBA = (x, y, 0, 255)
        let mut rgba = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                rgba.extend_from_slice(&[x, y, 0, 255]);
            }
        }
        Shot {
            width: 3,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            rgba,
        }
    }

    #[test]
    fn crop_takes_requested_region() {
        let img = shot().crop(1, 1, 2, 1).unwrap();
        assert_eq!((img.width(), img.height()), (2, 1));
        assert_eq!(img.get_pixel(0, 0).0, [1, 1, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [2, 1, 0, 255]);
    }

    #[test]
    fn crop_clamps_to_image_bounds() {
        let img = shot().crop(2, 1, 99, 99).unwrap();
        assert_eq!((img.width(), img.height()), (1, 1));
        // 越界起点被夹回图内
        let img = shot().crop(-5, -5, 1, 1).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn png_roundtrips_through_data_url() {
        let img = shot().crop(0, 0, 3, 2).unwrap();
        let url = data_url(&img).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        let raw = base64::engine::general_purpose::STANDARD
            .decode(url.trim_start_matches("data:image/png;base64,"))
            .unwrap();
        let back = image::load_from_memory(&raw).unwrap().to_rgba8();
        assert_eq!((back.width(), back.height()), (3, 2));
    }

    #[test]
    fn overlay_png_decodes_data_url_back_to_image() {
        let img = shot().crop(1, 0, 2, 2).unwrap();
        let back = overlay_png(&data_url(&img).unwrap()).unwrap();
        assert_eq!((back.width(), back.height()), (2, 2));
        assert_eq!(back.get_pixel(0, 0).0, [1, 0, 0, 255]);
    }

    #[test]
    fn overlay_png_rejects_wrong_prefix_and_garbage() {
        assert!(overlay_png("https://example.com/a.png").is_err());
        assert!(overlay_png("data:image/png;base64,!!!not-base64!!!").is_err());
        assert!(overlay_png("data:image/png;base64,YWJjZGVm").is_err()); // 合法 b64，但不是 PNG
    }

    /// 造一帧：每行像素由 `(y + shift)` 的伪随机数决定，便于构造"整体上移"的画面。
    fn frame(width: u32, height: u32, shift: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let seed = (y + shift).wrapping_mul(2_654_435_761);
            for x in 0..width {
                let a = (seed.rotate_left(x % 16) >> 13) as u8;
                let b = (seed.rotate_left((x + 3) % 16) >> 21) as u8;
                out.extend_from_slice(&[a, b, a ^ b, 255]);
            }
        }
        out
    }

    #[test]
    fn scroll_delta_finds_shift() {
        let (w, h) = (80u32, 200u32);
        let prev = frame(w, h, 0);
        let next = frame(w, h, 37);
        assert_eq!(scroll_delta(&prev, &next, w, h), Some(37));
    }

    #[test]
    fn scroll_delta_reports_zero_when_static() {
        let f = frame(80, 200, 0);
        assert_eq!(scroll_delta(&f, &f, 80, 200), Some(0));
    }

    #[test]
    fn scroll_delta_rejects_unrelated_frames() {
        let prev = frame(80, 200, 0);
        let next = frame(80, 200, 7_000);
        assert_eq!(scroll_delta(&prev, &next, 80, 200), None);
    }
}
