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

/// macOS 捕获层（CGWindowListCreateImage，C 接口）。
///
/// 坐标口径与 Windows 版一致：**虚拟桌面的物理像素**（全局坐标，原点在主屏左上、
/// Y 向下）。CG 的全局显示坐标是"点"，按主屏 backing scale 换算成物理像素——
/// 单屏/同缩放多屏精确；混合缩放多屏按主屏缩放统一采样（v1 取舍）。
/// 10.15+ 捕获需要屏幕录制权限；缺权限时明确报错并触发一次系统授权引导，
/// 不返回空白图（硬性约定 3）。
#[cfg(target_os = "macos")]
mod ffi {
    // C 命名对齐 Apple 头文件（kCG* 常量 / CGRect 结构），不按 Rust 风格改写
    #![allow(non_camel_case_types, non_upper_case_globals)]
    use std::ffi::c_void;

    pub type CGImageRef = *mut c_void;
    pub type CGContextRef = *mut c_void;
    pub type CGColorSpaceRef = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGPoint {
        pub x: f64,
        pub y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGSize {
        pub width: f64,
        pub height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGRect {
        pub origin: CGPoint,
        pub size: CGSize,
    }

    /// 位图上下文的内存序：R、G、B、A（premultipliedLast + 32Big）
    pub const BITMAP_RGBA: u32 = 1 | (4 << 12);

    // 显示器枚举 / scale 在 crate::tile::ax（与平铺共用一套 CG 声明，避免符号重复声明）
    /// kCGWindowListOptionOnScreenBelowWindow：只合成"这个窗口之下"的内容
    pub const WINDOW_LIST_ON_SCREEN_BELOW: u32 = 1 << 2;
    /// kCGWindowImageBestResolution
    pub const WINDOW_IMAGE_BEST_RESOLUTION: u32 = 1 << 3;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGDisplayCreateImage(display: u32) -> CGImageRef;
        pub fn CGWindowListCreateImage(
            screen_bounds: CGRect,
            list_option: u32,
            window_id: u32,
            image_option: u32,
        ) -> CGImageRef;
        pub fn CGImageRelease(img: CGImageRef);
        pub fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
        pub fn CGColorSpaceRelease(space: CGColorSpaceRef);
        pub fn CGBitmapContextCreate(
            data: *mut u8,
            w: usize,
            h: usize,
            bpc: usize,
            bytes_per_row: usize,
            space: CGColorSpaceRef,
            bitmap_info: u32,
        ) -> CGContextRef;
        pub fn CGContextRelease(ctx: CGContextRef);
        pub fn CGContextDrawImage(ctx: CGContextRef, rect: CGRect, img: CGImageRef);
    }
}

#[cfg(target_os = "macos")]
pub fn capture_region(x: i32, y: i32, width: u32, height: u32) -> Result<Shot, String> {
    capture_region_impl(x, y, width, height, None)
}

/// macOS：区域捕获，但**排除 `exclude` 这个窗口**（从它下面合成）。
///
/// 长截图时覆盖层要留在屏幕上给用户看边框和提示，又不能被抓进结果帧。
/// `NSWindowSharingNone` 对显示器捕获无效（实测 CGDisplayCreateImage 照样拍进去），
/// 所以走"在这个窗口之下合成"。
#[cfg(target_os = "macos")]
pub fn capture_region_below(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    exclude: u32,
) -> Result<Shot, String> {
    capture_region_impl(x, y, width, height, Some(exclude))
}

#[cfg(target_os = "macos")]
fn capture_region_impl(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    exclude: Option<u32>,
) -> Result<Shot, String> {
    let width = width.max(1);
    let height = height.max(1);
    if !crate::permissions::screen_recording_allowed() {
        // 系统弹窗每个进程只触发一次；反复弹只会变成骚扰
        crate::permissions::request_screen_recording_once();
        return Err(format!(
            "缺少屏幕录制（Screen Recording）权限：请在 系统设置 → 隐私与安全性 → 屏幕录制 中勾选 efto{}",
            crate::permissions::REGRANT_HINT
        ));
    }
    let s = crate::tile::ax::display_scale();
    unsafe {
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let space = ffi::CGColorSpaceCreateDeviceRGB();
        let ctx = ffi::CGBitmapContextCreate(
            rgba.as_mut_ptr(),
            width as usize,
            height as usize,
            8,
            width as usize * 4,
            space,
            ffi::BITMAP_RGBA,
        );
        if ctx.is_null() {
            ffi::CGColorSpaceRelease(space);
            return Err("创建位图上下文失败".into());
        }
        // 逐屏原生捕获再合成：每块屏的原生像素画进"点矩形 × 主屏 scale"的槽位，
        // 混合缩放多屏下映射精确（单次 CGWindowListCreateImage 的光栅化不保证
        // 与统一坐标系一致，实测会让选区映射漂到别的屏）。
        // 位图上下文的缓冲行序已是自顶向下：直接画，**不要**再翻转（实测翻了会上下颠倒）。
        let mut captured = 0u32;
        for (id, r) in crate::tile::ax::displays() {
            // SAFETY: 两种来源的都是 CGImage，非空时由本函数 Release；
            // 目标槽位矩形可超出画布——上下文自动裁剪（区域捕获即靠这一点）。
            let img = match exclude {
                // 参数是全局"点"矩形（与 CGDisplayBounds 同口径）
                Some(excl) => ffi::CGWindowListCreateImage(
                    ffi::CGRect {
                        origin: ffi::CGPoint {
                            x: r.x as f64,
                            y: r.y as f64,
                        },
                        size: ffi::CGSize {
                            width: r.width as f64,
                            height: r.height as f64,
                        },
                    },
                    ffi::WINDOW_LIST_ON_SCREEN_BELOW,
                    excl,
                    ffi::WINDOW_IMAGE_BEST_RESOLUTION,
                ),
                None => ffi::CGDisplayCreateImage(id),
            };
            if img.is_null() {
                continue;
            }
            captured += 1;
            // CGBitmapContext 的绘制矩形是**左下原点**（缓冲行序读起来自顶向下是
            // 因为图像本身被上下翻转画进去的）。显示器矩形是全局左上原点，y 必须
            // 换成"距画布底边的高度"：较矮的那块屏（如 900 点 + 1080 点双屏）直接
            // 用左上 y 画会整块下沉 180 点，顶部留一条空槽。
            let top = r.y as f64 * s - y as f64;
            let bottom = height as f64 - top - r.height as f64 * s;
            ffi::CGContextDrawImage(
                ctx,
                ffi::CGRect {
                    origin: ffi::CGPoint {
                        x: r.x as f64 * s - x as f64,
                        y: bottom,
                    },
                    size: ffi::CGSize {
                        width: r.width as f64 * s,
                        height: r.height as f64 * s,
                    },
                },
                img,
            );
            ffi::CGImageRelease(img);
        }
        ffi::CGContextRelease(ctx);
        ffi::CGColorSpaceRelease(space);
        if captured == 0 {
            return Err("屏幕捕获失败（CGDisplayCreateImage 无输出）".into());
        }
        // 屏幕内容不透明；premultiplied 的 alpha 恒 255
        for px in rgba.chunks_exact_mut(4) {
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

#[cfg(not(any(windows, target_os = "macos")))]
pub fn capture_region(_x: i32, _y: i32, width: u32, height: u32) -> Result<Shot, String> {
    let _ = (width, height);
    Err("截图仅支持 Windows / macOS".into())
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

/// macOS：逐屏合成，但**排除 `exclude_window` 及其之上的窗口**。
///
/// 用 `CGWindowListCreateImage(.optionOnScreenBelowWindow)`：窗口服务器把某个
/// 窗口的表面状态卡住时（AppKit 已 `orderOut`、服务器仍按旧位置画），普通显示器
/// 捕获会把启动器一起拍进底图；从"它下面"合成就能保证拿到干净的桌面。
/// 代价是同一层之上的系统菜单栏也不在结果里，所以只在检测到残留时兜底。
#[cfg(target_os = "macos")]
pub fn capture_virtual_screen_below(exclude_window: u32) -> Result<Shot, String> {
    let displays = crate::tile::ax::displays();
    if displays.is_empty() {
        return Err("无法读取显示器信息".into());
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (_, r) in &displays {
        min_x = min_x.min(r.x);
        min_y = min_y.min(r.y);
        max_x = max_x.max(r.x + r.width);
        max_y = max_y.max(r.y + r.height);
    }
    let s = crate::tile::ax::display_scale();
    let x = (min_x as f64 * s).round() as i32;
    let y = (min_y as f64 * s).round() as i32;
    let width = ((max_x - min_x) as f64 * s).round().max(1.0) as u32;
    let height = ((max_y - min_y) as f64 * s).round().max(1.0) as u32;
    if !crate::permissions::screen_recording_allowed() {
        crate::permissions::request_screen_recording_once();
        return Err(format!(
            "缺少屏幕录制（Screen Recording）权限：请在 系统设置 → 隐私与安全性 → 屏幕录制 中勾选 efto{}",
            crate::permissions::REGRANT_HINT
        ));
    }
    unsafe {
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let space = ffi::CGColorSpaceCreateDeviceRGB();
        let ctx = ffi::CGBitmapContextCreate(
            rgba.as_mut_ptr(),
            width as usize,
            height as usize,
            8,
            width as usize * 4,
            space,
            ffi::BITMAP_RGBA,
        );
        if ctx.is_null() {
            ffi::CGColorSpaceRelease(space);
            return Err("创建位图上下文失败".into());
        }
        let mut captured = 0u32;
        for (_, r) in displays {
            // 参数是全局"点"矩形（与 CGDisplayBounds 同口径）
            let rect = ffi::CGRect {
                origin: ffi::CGPoint {
                    x: r.x as f64,
                    y: r.y as f64,
                },
                size: ffi::CGSize {
                    width: r.width as f64,
                    height: r.height as f64,
                },
            };
            let img = ffi::CGWindowListCreateImage(
                rect,
                ffi::WINDOW_LIST_ON_SCREEN_BELOW,
                exclude_window,
                ffi::WINDOW_IMAGE_BEST_RESOLUTION,
            );
            if img.is_null() {
                continue;
            }
            captured += 1;
            let top = r.y as f64 * s - y as f64;
            let bottom = height as f64 - top - r.height as f64 * s;
            ffi::CGContextDrawImage(
                ctx,
                ffi::CGRect {
                    origin: ffi::CGPoint {
                        x: r.x as f64 * s - x as f64,
                        y: bottom,
                    },
                    size: ffi::CGSize {
                        width: r.width as f64 * s,
                        height: r.height as f64 * s,
                    },
                },
                img,
            );
            ffi::CGImageRelease(img);
        }
        ffi::CGContextRelease(ctx);
        ffi::CGColorSpaceRelease(space);
        if captured == 0 {
            return Err("屏幕捕获失败（CGWindowListCreateImage 无输出）".into());
        }
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let mut shot = Shot {
            width,
            height,
            origin_x: x,
            origin_y: y,
            rgba,
        };
        // 系统菜单栏在本窗口层级之上，"从它下面合成"拿不到——用一张普通捕获把
        // 每块屏顶部那条菜单栏拼回来（普通捕获里那一块不含启动器，启动器在屏幕中部）。
        if let Ok(normal) = capture_virtual_screen() {
            patch_top_strips(&mut shot, &normal);
        }
        Ok(shot)
    }
}

/// 把 `normal` 里每块屏顶部"菜单栏那一条"复制进 `shot`（两者几何一致）。
#[cfg(target_os = "macos")]
fn patch_top_strips(shot: &mut Shot, normal: &Shot) {
    if normal.width != shot.width || normal.height != shot.height {
        return;
    }
    let s = crate::tile::ax::display_scale();
    for (id, r) in crate::tile::ax::displays() {
        // 工作区的上边界就是菜单栏下沿（visibleFrame 已扣掉菜单栏与 Dock）
        let Ok(work) = crate::tile::ax::work_area(id) else {
            continue;
        };
        let inset_pt = (work.y - r.y).max(0) as f64;
        if inset_pt <= 0.0 {
            continue;
        }
        let x0 = (r.x as f64 * s).round() as i64 - shot.origin_x as i64;
        let x1 = ((r.x + r.width) as f64 * s).round() as i64 - shot.origin_x as i64;
        let y0 = (r.y as f64 * s).round() as i64 - shot.origin_y as i64;
        let y1 = y0 + (inset_pt * s).round() as i64;
        let (x0, x1) = (x0.max(0), x1.min(shot.width as i64));
        let (y0, y1) = (y0.max(0), y1.min(shot.height as i64));
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        for row in y0..y1 {
            let start = ((row * shot.width as i64 + x0) * 4) as usize;
            let end = ((row * shot.width as i64 + x1) * 4) as usize;
            if end <= shot.rgba.len() && end <= normal.rgba.len() {
                shot.rgba[start..end].copy_from_slice(&normal.rgba[start..end]);
            }
        }
    }
}

/// macOS：所有活动显示器的包围盒（点）× 主屏缩放 = 虚拟桌面物理像素。
/// 显示器枚举与 scale 复用 `crate::tile::ax`（与平铺共用一套 CG 声明）。
#[cfg(target_os = "macos")]
pub fn capture_virtual_screen() -> Result<Shot, String> {
    let displays = crate::tile::ax::displays();
    if displays.is_empty() {
        return Err("无法读取显示器信息".into());
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (_, r) in &displays {
        min_x = min_x.min(r.x);
        min_y = min_y.min(r.y);
        max_x = max_x.max(r.x + r.width);
        max_y = max_y.max(r.y + r.height);
    }
    let s = crate::tile::ax::display_scale();
    capture_region(
        (min_x as f64 * s).round() as i32,
        (min_y as f64 * s).round() as i32,
        ((max_x - min_x) as f64 * s).round().max(1.0) as u32,
        ((max_y - min_y) as f64 * s).round().max(1.0) as u32,
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn capture_virtual_screen() -> Result<Shot, String> {
    Err("截图仅支持 Windows / macOS".into())
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

    /// macOS 真机捕获：无屏幕录制权限时优雅 skip；有权限时断言尺寸/非全黑。
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_capture_real_screen_or_skip() {
        if !crate::permissions::screen_recording_allowed() {
            println!("skip: 无屏幕录制权限，真机捕获未验证");
            return;
        }
        let shot = capture_virtual_screen().expect("应能抓到屏幕");
        assert!(shot.width > 0 && shot.height > 0);
        assert_eq!(shot.rgba.len(), (shot.width * shot.height * 4) as usize);
        assert!(
            shot.rgba.chunks_exact(4).any(|px| px[..3] != [0, 0, 0]),
            "全黑说明捕获没有内容"
        );
        assert!(shot.rgba.chunks_exact(4).all(|px| px[3] == 255), "屏幕内容应不透明");
        // 区域捕获与整屏同口径：抓中间一小块，尺寸精确、origin 原样
        let r = capture_region(shot.origin_x + 8, shot.origin_y + 8, 64, 48).expect("区域捕获");
        assert_eq!((r.width, r.height), (64, 48));
        assert_eq!(r.rgba.len(), 64 * 48 * 4);
        let c = r.crop(0, 0, 32, 24).expect("裁剪");
        assert_eq!((c.width(), c.height()), (32, 24));
    }

    /// 缺权限必须明确报错，绝不静默给空白图（硬性约定 3）。
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_capture_errors_without_permission() {
        if crate::permissions::screen_recording_allowed() {
            println!("skip: 有权限，缺权限报错路径未验证");
            return;
        }
        let e = capture_virtual_screen().expect_err("无权限必须报错");
        assert!(e.contains("屏幕录制"), "{e}");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod debug_dump {
    use super::*;

    #[test]
    #[ignore]
    fn dump_virtual_screen() {
        let disp = crate::tile::ax::displays();
        eprintln!("displays = {:?}, scale = {}", disp, crate::tile::ax::display_scale());
        let shot = capture_virtual_screen().expect("capture");
        eprintln!("shot = {}x{} origin=({}, {})", shot.width, shot.height, shot.origin_x, shot.origin_y);
        let img = shot.to_image().unwrap();
        img.save("/tmp/efto-shot.png").unwrap();
    }
}
