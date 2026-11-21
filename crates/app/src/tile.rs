//! 窗口操作：网格平铺、左右半屏、最小化，以及撤销上一次平铺。
//!
//! 布局计算是纯函数（[`grid_layout`] / [`half_layout`]），Win32 枚举/移动单独一层，
//! 这样边界可以在没有窗口的 CI 上单测。
//!
//! 「搜索框下方的窗口」取唤起前的那个前台窗口：launcher 可见时前台是 efto 自己，
//! 此时回落到 [`crate::guide::last`]；通过单项热键直接执行时前台就是要操作的应用。

use serde::Serialize;

/// 屏幕矩形（物理像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// 一个参与窗口操作的窗口。
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub pid: u32,
    pub hwnd: isize,
    pub process: String,
    pub title: String,
}

/// 执行结果；`message` 直接用于前端 toast。
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub work: Rect,
    pub candidates: Vec<Candidate>,
    pub layout: Vec<Rect>,
    /// 平铺成功数；最小化操作里表示实际最小化的窗口数。
    pub tiled: usize,
    pub skipped: usize,
    pub failed: usize,
    pub message: String,
}

/// 支持的窗口操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// 网格铺满当前显示器。
    Grid,
    /// 把搜索框下方的窗口放到左半屏。
    HalfLeft,
    /// 把搜索框下方的窗口放到右半屏。
    HalfRight,
    /// 最小化其他窗口，保留搜索框下方的那个。
    MinimizeOthers,
    /// 最小化全部可见窗口。
    MinimizeAll,
    /// 还原上一次平铺前的窗口位置。
    Undo,
}

impl Op {
    /// CLI `--mode` 的取值。
    pub fn from_name(name: &str) -> Option<Op> {
        match name {
            "grid" => Some(Op::Grid),
            "left" | "half-left" => Some(Op::HalfLeft),
            "right" | "half-right" => Some(Op::HalfRight),
            "min-others" => Some(Op::MinimizeOthers),
            "min-all" => Some(Op::MinimizeAll),
            "undo" => Some(Op::Undo),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn as_name(self) -> &'static str {
        match self {
            Op::Grid => "grid",
            Op::HalfLeft => "left",
            Op::HalfRight => "right",
            Op::MinimizeOthers => "min-others",
            Op::MinimizeAll => "min-all",
            Op::Undo => "undo",
        }
    }
}

/// 网格布局：`n` 个窗口铺满 `work`，窗口间距 `gap`，四周留 `margin`。
///
/// `cols = ceil(sqrt(n))`、`rows = ceil(n / cols)`；最后一行不足时
/// 按实际个数均分整行宽度，避免右侧留空。
pub fn grid_layout(work: Rect, n: usize, gap: i32, margin: i32) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let mut cols = 1usize;
    while cols * cols < n {
        cols += 1;
    }
    let rows = (n + cols - 1) / cols;

    let gap = gap.max(0);
    let inner_w = (work.width - margin * 2).max(1) as i64;
    let inner_h = (work.height - margin * 2).max(1) as i64;
    let cell_w = ((inner_w - gap as i64 * (cols as i64 - 1)) / cols as i64).max(1);
    let cell_h = ((inner_h - gap as i64 * (rows as i64 - 1)) / rows as i64).max(1);

    let mut out = Vec::with_capacity(n);
    for r in 0..rows {
        let start = r * cols;
        let count = (n - start).min(cols);
        if count == 0 {
            break;
        }
        // 满行用统一列宽；最后一行不足时均分整行，右侧不留空
        let w = if count == cols {
            cell_w
        } else {
            ((inner_w - gap as i64 * (count as i64 - 1)) / count as i64).max(1)
        };
        for c in 0..count {
            out.push(Rect {
                x: work.x + margin + (c as i64 * (w + gap as i64)) as i32,
                y: work.y + margin + (r as i64 * (cell_h + gap as i64)) as i32,
                width: w as i32,
                height: cell_h as i32,
            });
        }
    }
    out
}

/// 半屏布局：把 `work` 从中间切开，`right` 为真取右半屏。
///
/// 两半之间留 `gap`，四周留 `margin`，与网格平铺保持同样的边距。
pub fn half_layout(work: Rect, right: bool, gap: i32, margin: i32) -> Rect {
    let gap = gap.max(0);
    let inner_w = (work.width - margin * 2).max(1) as i64;
    let w = ((inner_w - gap as i64) / 2).max(1) as i32;
    Rect {
        x: if right {
            work.x + margin + w + gap
        } else {
            work.x + margin
        },
        y: work.y + margin,
        width: w,
        height: (work.height - margin * 2).max(1),
    }
}

/// 执行一次窗口操作。
#[cfg(windows)]
pub fn run_op(main_hwnd: Option<isize>, op: Op, dry_run: bool) -> Result<Report, String> {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};

    let monitor = monitor_of(main_hwnd);
    if monitor.is_null() {
        return Err("找不到目标显示器".into());
    }
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return Err("读取显示器工作区失败".into());
    }
    let work = Rect {
        x: info.rcWork.left,
        y: info.rcWork.top,
        width: info.rcWork.right - info.rcWork.left,
        height: info.rcWork.bottom - info.rcWork.top,
    };

    match op {
        Op::Grid => {
            let (candidates, skipped) = enumerate(Some(monitor as isize), None);
            let layout = grid_layout(work, candidates.len(), 8, 8);
            let (tiled, failed) = if dry_run {
                (0, 0)
            } else {
                apply_layout(&candidates, &layout, true)
            };
            let message = if candidates.is_empty() {
                "当前显示器没有可平铺的窗口".to_string()
            } else if dry_run {
                format!("将平铺 {} 个窗口（跳过 {} 个）", candidates.len(), skipped)
            } else if failed > 0 {
                format!(
                    "已平铺 {} 个窗口，{} 个失败（跳过 {} 个）",
                    tiled, failed, skipped
                )
            } else {
                format!("已平铺 {} 个窗口（跳过 {} 个）", tiled, skipped)
            };
            Ok(Report { work, candidates, layout, tiled, skipped, failed, message })
        }
        Op::HalfLeft | Op::HalfRight => {
            let right = op == Op::HalfRight;
            let target = target_window()?;
            let rect = half_layout(work, right, 8, 8);
            let candidates = vec![target.clone()];
            let layout = vec![rect];
            let (tiled, failed) = if dry_run {
                (0, 0)
            } else {
                apply_layout(&candidates, &layout, true)
            };
            let side = if right { "右半屏" } else { "左半屏" };
            let message = if dry_run {
                format!("将把「{}」放到{}", target.title, side)
            } else if failed > 0 {
                format!("无法移动「{}」到{}", target.title, side)
            } else {
                format!("已把「{}」放到{}", target.title, side)
            };
            Ok(Report { work, candidates, layout, tiled, skipped: 0, failed, message })
        }
        Op::MinimizeOthers => {
            let keep = target_window()?;
            let (candidates, skipped) = enumerate(None, Some(keep.hwnd));
            let (done, failed) = if dry_run {
                (0, 0)
            } else {
                minimize_all(&candidates)
            };
            let message = if candidates.is_empty() {
                format!("除「{}」外没有可最小化的窗口", keep.title)
            } else if dry_run {
                format!(
                    "将最小化 {} 个窗口，保留「{}」（跳过 {} 个）",
                    candidates.len(),
                    keep.title,
                    skipped
                )
            } else if failed > 0 {
                format!(
                    "已最小化 {} 个窗口，{} 个失败，保留「{}」（跳过 {} 个）",
                    done, failed, keep.title, skipped
                )
            } else {
                format!(
                    "已最小化 {} 个窗口，保留「{}」（跳过 {} 个）",
                    done, keep.title, skipped
                )
            };
            Ok(Report {
                work,
                candidates,
                layout: Vec::new(),
                tiled: done,
                skipped,
                failed,
                message,
            })
        }
        Op::MinimizeAll => {
            let (candidates, skipped) = enumerate(None, None);
            let (done, failed) = if dry_run {
                (0, 0)
            } else {
                minimize_all(&candidates)
            };
            let message = if candidates.is_empty() {
                "当前没有可最小化的窗口".to_string()
            } else if dry_run {
                format!("将最小化 {} 个窗口（跳过 {} 个）", candidates.len(), skipped)
            } else if failed > 0 {
                format!(
                    "已最小化 {} 个窗口，{} 个失败（跳过 {} 个）",
                    done, failed, skipped
                )
            } else {
                format!("已最小化 {} 个窗口（跳过 {} 个）", done, skipped)
            };
            Ok(Report {
                work,
                candidates,
                layout: Vec::new(),
                tiled: done,
                skipped,
                failed,
                message,
            })
        }
        Op::Undo => undo_last(work, dry_run),
    }
}

#[cfg(windows)]
fn monitor_of(main_hwnd: Option<isize>) -> *mut core::ffi::c_void {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        MonitorFromPoint, MonitorFromWindow, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    };
    if let Some(h) = main_hwnd {
        let m = unsafe { MonitorFromWindow(h as _, MONITOR_DEFAULTTONEAREST) };
        if !m.is_null() {
            return m;
        }
    }
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

/// 枚举参与操作的可见顶层窗口。
///
/// `monitor` 为 `Some` 时只要该显示器上的窗口（平铺需要）；`keep` 用于
/// 「最小化其他窗口」，既不进入结果也不计入 skipped。
#[cfg(windows)]
fn enumerate(monitor: Option<isize>, keep: Option<isize>) -> (Vec<Candidate>, usize) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows_sys::Win32::Graphics::Gdi::{
        MonitorFromWindow, MONITOR_DEFAULTTONULL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, GW_OWNER,
        WS_EX_TOOLWINDOW,
    };

    struct Collect {
        monitor: Option<isize>,
        keep: Option<isize>,
        self_pid: u32,
        out: Vec<Candidate>,
        skipped: usize,
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam as *mut Collect);
        if IsWindowVisible(hwnd) == 0 {
            return TRUE;
        }
        if GetWindowTextLengthW(hwnd) <= 0 {
            return TRUE;
        }
        // 最小化窗口不弹出、不还原
        if IsIconic(hwnd) != 0 {
            state.skipped += 1;
            return TRUE;
        }
        // 工具窗口、owned 窗口不是"用户感知的应用窗口"
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
            return TRUE;
        }
        if !GetWindow(hwnd, GW_OWNER).is_null() {
            return TRUE;
        }
        let mut class_buf = [0u16; 256];
        let cn = GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32);
        let class = String::from_utf16_lossy(&class_buf[..cn.max(0) as usize]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 || pid == state.self_pid {
            return TRUE;
        }
        if let Some(keep) = state.keep {
            if hwnd as isize == keep {
                return TRUE;
            }
        }
        // 只处理指定显示器；其他屏窗口保持不动
        if let Some(m) = state.monitor {
            if MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) as isize != m {
                return TRUE;
            }
        }
        // 其他虚拟桌面/系统隐藏的窗口
        let mut cloaked: u32 = 0;
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if hr >= 0 && cloaked != 0 {
            return TRUE;
        }

        let mut buf = vec![0u16; GetWindowTextLengthW(hwnd) as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        state.out.push(Candidate {
            pid,
            hwnd: hwnd as isize,
            process: unsafe { crate::winproc::process_name(pid) },
            title: String::from_utf16_lossy(&buf[..n.max(0) as usize]),
        });
        TRUE
    }

    let mut state = Collect {
        monitor,
        keep,
        self_pid: std::process::id(),
        out: Vec::new(),
        skipped: 0,
    };
    unsafe {
        EnumWindows(Some(collect), &mut state as *mut Collect as LPARAM);
    }
    (state.out, state.skipped)
}

/// 唤起前的窗口；launcher 自己在前台时回落到 [`crate::guide::last`]。
#[cfg(windows)]
fn target_window() -> Result<Candidate, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindow, IsWindowVisible,
    };

    let mut hwnd = unsafe { GetForegroundWindow() };
    if !hwnd.is_null() {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 || pid == std::process::id() || unsafe { IsWindowVisible(hwnd) } == 0 {
            hwnd = std::ptr::null_mut();
        }
    }
    let info = if !hwnd.is_null() {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let mut buf = vec![0u16; unsafe { GetWindowTextLengthW(hwnd) } as usize + 1];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        Candidate {
            pid,
            hwnd: hwnd as isize,
            process: unsafe { crate::winproc::process_name(pid) },
            title: String::from_utf16_lossy(&buf[..n.max(0) as usize]),
        }
    } else {
        let saved = crate::guide::last().ok_or_else(|| "没有找到搜索框下方的窗口".to_string())?;
        Candidate {
            pid: saved.pid,
            hwnd: saved.hwnd,
            process: saved.process,
            title: saved.title,
        }
    };
    if info.title.trim().is_empty() {
        return Err("没有找到搜索框下方的窗口".into());
    }
    if unsafe { IsWindow(info.hwnd as _) } == 0 {
        return Err("搜索框下方的窗口已经关闭".into());
    }
    Ok(info)
}

/// 上一次平铺前的窗口状态，供「还原上次平铺」使用。
#[cfg(windows)]
#[derive(Clone)]
struct SavedWindow {
    candidate: Candidate,
    rect: Rect,
    zoomed: bool,
}

#[cfg(windows)]
static LAST_TILE: std::sync::Mutex<Vec<SavedWindow>> = std::sync::Mutex::new(Vec::new());

/// 按布局移动窗口；`remember` 为真时先记下原始矩形，成功后写入撤销状态。
#[cfg(windows)]
fn apply_layout(candidates: &[Candidate], layout: &[Rect], remember: bool) -> (usize, usize) {
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, GetWindowRect, IsZoomed,
        SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    };

    if candidates.is_empty() {
        return (0, 0);
    }
    let saved: Vec<SavedWindow> = if remember {
        candidates
            .iter()
            .filter_map(|c| {
                let hwnd = c.hwnd as HWND;
                let mut got: RECT = unsafe { std::mem::zeroed() };
                if unsafe { GetWindowRect(hwnd, &mut got) } == 0 {
                    return None;
                }
                Some(SavedWindow {
                    candidate: c.clone(),
                    rect: Rect {
                        x: got.left,
                        y: got.top,
                        width: got.right - got.left,
                        height: got.bottom - got.top,
                    },
                    zoomed: unsafe { IsZoomed(hwnd) } != 0,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    // 最大化窗口先还原，否则 SetWindowPos 不会改变它的尺寸
    for c in candidates {
        let hwnd = c.hwnd as HWND;
        if unsafe { IsZoomed(hwnd) } != 0 {
            unsafe { ShowWindow(hwnd, SW_RESTORE) };
        }
    }
    let pairs: Vec<(HWND, Rect)> = candidates
        .iter()
        .map(|c| c.hwnd as HWND)
        .zip(layout.iter().copied())
        .collect();
    let count = pairs.len() as i32;
    let mut moved = false;
    unsafe {
        let mut hdwp = BeginDeferWindowPos(count);
        if !hdwp.is_null() {
            for (hwnd, r) in &pairs {
                hdwp = DeferWindowPos(
                    hdwp,
                    *hwnd,
                    std::ptr::null_mut(),
                    r.x,
                    r.y,
                    r.width,
                    r.height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                if hdwp.is_null() {
                    break;
                }
            }
            if !hdwp.is_null() {
                moved = EndDeferWindowPos(hdwp) != 0;
            }
        }
    }

    let mut tiled = 0usize;
    let mut failed = 0usize;
    if moved {
        // 批量调用成功不代表窗口真的动了（UWP/受限窗口会忽略尺寸变更），
        // 逐个复核实际矩形，偏差过大计入失败而不是静默成功
        for (hwnd, r) in &pairs {
            let mut got: RECT = unsafe { std::mem::zeroed() };
            let read = unsafe { GetWindowRect(*hwnd, &mut got) } != 0;
            let placed = read
                && (got.left - r.x).abs() <= 2
                && (got.top - r.y).abs() <= 2
                && (got.right - got.left - r.width).abs() <= 2
                && (got.bottom - got.top - r.height).abs() <= 2;
            if placed {
                tiled += 1;
            } else {
                failed += 1;
            }
        }
    } else {
        // 批量失败时逐个重试，把失败归到具体窗口而不是整批放弃
        for (hwnd, r) in &pairs {
            let ok = unsafe {
                SetWindowPos(
                    *hwnd,
                    std::ptr::null_mut(),
                    r.x,
                    r.y,
                    r.width,
                    r.height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            if ok != 0 {
                tiled += 1;
            } else {
                failed += 1;
            }
        }
    }

    if remember && tiled > 0 {
        if let Ok(mut slot) = LAST_TILE.lock() {
            *slot = saved;
        }
    }
    (tiled, failed)
}

#[cfg(windows)]
fn minimize_all(candidates: &[Candidate]) -> (usize, usize) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_MINIMIZE};

    let mut done = 0usize;
    let mut failed = 0usize;
    for c in candidates {
        let hwnd = c.hwnd as HWND;
        unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
        if unsafe { IsIconic(hwnd) } != 0 {
            done += 1;
        } else {
            failed += 1;
        }
    }
    (done, failed)
}

#[cfg(windows)]
fn undo_last(work: Rect, dry_run: bool) -> Result<Report, String> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsWindow, IsZoomed, SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE,
    };

    if dry_run {
        let n = LAST_TILE.lock().map(|v| v.len()).unwrap_or(0);
        let message = if n == 0 {
            "没有可还原的平铺记录".to_string()
        } else {
            format!("将还原 {n} 个窗口")
        };
        return Ok(Report {
            work,
            candidates: Vec::new(),
            layout: Vec::new(),
            tiled: 0,
            skipped: 0,
            failed: 0,
            message,
        });
    }

    let saved = LAST_TILE
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default();
    if saved.is_empty() {
        return Ok(Report {
            work,
            candidates: Vec::new(),
            layout: Vec::new(),
            tiled: 0,
            skipped: 0,
            failed: 0,
            message: "没有可还原的平铺记录".into(),
        });
    }

    let mut restored = 0usize;
    let mut failed = 0usize;
    for s in &saved {
        let hwnd = s.candidate.hwnd as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            failed += 1;
            continue;
        }
        if s.zoomed {
            unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
            if unsafe { IsZoomed(hwnd) } != 0 {
                restored += 1;
            } else {
                failed += 1;
            }
        } else {
            let ok = unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    s.rect.x,
                    s.rect.y,
                    s.rect.width,
                    s.rect.height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            if ok != 0 {
                restored += 1;
            } else {
                failed += 1;
            }
        }
    }
    let message = if failed > 0 {
        format!("已还原 {} 个窗口，{} 个失败", restored, failed)
    } else {
        format!("已还原 {restored} 个窗口")
    };
    Ok(Report {
        work,
        candidates: saved.iter().map(|s| s.candidate.clone()).collect(),
        layout: saved.iter().map(|s| s.rect).collect(),
        tiled: restored,
        skipped: 0,
        failed,
        message,
    })
}

#[cfg(not(windows))]
pub fn run_op(_main_hwnd: Option<isize>, _op: Op, _dry_run: bool) -> Result<Report, String> {
    Err("窗口操作仅支持 Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn empty_has_no_rects() {
        assert!(grid_layout(work(), 0, 8, 8).is_empty());
    }

    #[test]
    fn single_window_fills_work_area_minus_margin() {
        let out = grid_layout(work(), 1, 8, 8);
        assert_eq!(
            out,
            vec![Rect {
                x: 8,
                y: 8,
                width: 1904,
                height: 1064
            }]
        );
    }

    #[test]
    fn two_windows_split_horizontally() {
        let out = grid_layout(work(), 2, 8, 8);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].y, out[1].y);
        assert_eq!(out[0].height, 1064);
        assert!(out[0].x + out[0].width < out[1].x, "两窗之间必须留间距");
        assert_eq!(out[0].width, out[1].width);
    }

    #[test]
    fn short_last_row_uses_full_width() {
        // n=3 → 2 列 2 行；最后一行只有 1 个窗口，应占满整行
        let out = grid_layout(work(), 3, 8, 8);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2].width, 1904);
        assert_eq!(out[2].x, 8);
        assert!(out[2].y > out[0].y, "最后一行必须在第一行下方");
        assert_eq!(out[0].y, out[1].y, "同一行 y 必须一致");
    }

    #[test]
    fn rects_never_overlap_and_stay_inside_work() {
        for n in 1..=9 {
            let out = grid_layout(work(), n, 8, 8);
            assert_eq!(out.len(), n);
            for (i, r) in out.iter().enumerate() {
                assert!(r.width > 0 && r.height > 0);
                assert!(r.x >= 8 && r.y >= 8);
                assert!(r.x + r.width <= work().width - 8 + 1);
                assert!(r.y + r.height <= work().height - 8 + 1);
                for other in out.iter().skip(i + 1) {
                    let overlap_x = r.x < other.x + other.width && other.x < r.x + r.width;
                    let overlap_y = r.y < other.y + other.height && other.y < r.y + r.height;
                    assert!(!(overlap_x && overlap_y), "矩形重叠: {r:?} vs {other:?}");
                }
            }
        }
    }

    #[test]
    fn narrow_work_area_stays_positive() {
        let tiny = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        };
        let out = grid_layout(tiny, 4, 8, 8);
        assert_eq!(out.len(), 4);
        for r in out {
            assert!(r.width >= 1 && r.height >= 1);
        }
    }

    #[test]
    fn half_layout_splits_and_keeps_gap() {
        let left = half_layout(work(), false, 8, 8);
        let right = half_layout(work(), true, 8, 8);
        assert_eq!(left.x, 8);
        assert_eq!(right.y, 8);
        assert_eq!(left.width, right.width);
        assert_eq!(left.height, 1064);
        assert!(left.x + left.width < right.x, "两半之间必须留间距");
        assert_eq!(right.x + right.width, 1912);
    }

    #[test]
    fn half_layout_tiny_work_stays_positive() {
        let tiny = Rect { x: 0, y: 0, width: 9, height: 9 };
        let left = half_layout(tiny, false, 8, 8);
        assert!(left.width >= 1 && left.height >= 1);
    }

    #[test]
    fn op_names_round_trip() {
        for op in [
            Op::Grid,
            Op::HalfLeft,
            Op::HalfRight,
            Op::MinimizeOthers,
            Op::MinimizeAll,
            Op::Undo,
        ] {
            assert_eq!(Op::from_name(op.as_name()), Some(op));
        }
        assert_eq!(Op::from_name("nope"), None);
    }
}
