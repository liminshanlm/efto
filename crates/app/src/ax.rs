//! macOS 窗口几何后端（[`crate::tile`] 的私有模块）：AX（HIServices）读写窗口
//! 位置/尺寸/最小化，CoreGraphics 取显示器与窗口边界快照，NSScreen 取工作区
//! （扣除菜单栏与 Dock）。全部经裸 extern FFI，不新增依赖。
//!
//! 坐标系与 Win32 一致：全局原点在主屏左上、Y 向下，多屏允许负坐标。
//! NSScreen 的 Cocoa 坐标（主屏左下、Y 向上）经 [`cocoa_to_quartz`] 换算。

use core::ffi::c_void;
use std::collections::HashMap;

use super::Rect;

// ---------- 纯函数（可单测） ----------

/// Cocoa 坐标（原点主屏左下、Y 向上）→ 全局坐标（原点主屏左上、Y 向下）。
/// `primary_height` 是主屏（`NSScreen.screens[0]`）的 Cocoa 高度。
pub(crate) fn cocoa_to_quartz(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    primary_height: f64,
) -> Rect {
    Rect {
        x: x.round() as i32,
        y: (primary_height - y - height).round() as i32,
        width: width.round() as i32,
        height: height.round() as i32,
    }
}

/// 与 `MonitorFromWindow(MONITOR_DEFAULTTONULL)` 对齐：返回与 `bounds` 相交面积
/// 最大的显示器下标；完全不相交时返回 `None`。
pub(crate) fn best_display(bounds: Rect, displays: &[(u32, Rect)]) -> Option<usize> {
    let mut best: Option<(i64, usize)> = None;
    for (i, (_, d)) in displays.iter().enumerate() {
        let w = span_overlap(bounds.x as i64, bounds.width as i64, d.x as i64, d.width as i64);
        let h = span_overlap(bounds.y as i64, bounds.height as i64, d.y as i64, d.height as i64);
        let area = w * h;
        if area > 0 && best.map_or(true, |(a, _)| area > a) {
            best = Some((area, i));
        }
    }
    best.map(|(_, i)| i)
}

/// 与 `MonitorFromWindow(MONITOR_DEFAULTTONEAREST)` 对齐：取窗口中心点距离
/// 最近的显示器下标；没有显示器时返回 `None`。
pub(crate) fn nearest_display(bounds: Rect, displays: &[(u32, Rect)]) -> Option<usize> {
    let cx = bounds.x as f64 + bounds.width as f64 / 2.0;
    let cy = bounds.y as f64 + bounds.height as f64 / 2.0;
    displays
        .iter()
        .enumerate()
        .min_by(|(_, (_, a)), (_, (_, b))| {
            rect_dist2(*a, cx, cy)
                .partial_cmp(&rect_dist2(*b, cx, cy))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

fn span_overlap(x: i64, w: i64, y: i64, h: i64) -> i64 {
    let lo = x.max(y);
    let hi = (x + w).min(y + h);
    (hi - lo).max(0)
}

/// 点到矩形的距离平方（点在矩形内为 0）。
fn rect_dist2(d: Rect, x: f64, y: f64) -> f64 {
    let dx = if x < d.x as f64 {
        d.x as f64 - x
    } else if x > (d.x + d.width) as f64 {
        x - (d.x + d.width) as f64
    } else {
        0.0
    };
    let dy = if y < d.y as f64 {
        d.y as f64 - y
    } else if y > (d.y + d.height) as f64 {
        y - (d.y + d.height) as f64
    } else {
        0.0
    };
    dx * dx + dy * dy
}

/// AXError → 区分性错误文案：窗口拒绝 AX / 窗口失效 / 系统错误。
/// `action` 是正在进行的动作（如"移动""最小化"）。
pub(crate) fn ax_err_text(code: i32, action: &str) -> String {
    match code {
        // 属性/动作不支持、未实现、API 被禁、值不存在、参数不被接受：都是"这个窗口拒绝"
        -25201 | -25205 | -25206 | -25207 | -25208 | -25211 | -25212 => {
            format!("目标窗口不支持 AX 操作（{action}被拒绝）")
        }
        // AX 元素失效：窗口已关闭
        -25202 => format!("窗口已关闭或不可用（{action}时 AX 元素失效）"),
        // kAXErrorFailure / kAXErrorCannotComplete 及其他：系统或目标应用出错
        other => format!("系统错误：{action}失败（AX 返回 {other}）"),
    }
}

// ---------- C 类型 ----------

/// CGRect（CGPoint origin + CGSize size，4×f64）；与 C 布局一致，用于
/// `CGDisplayBounds` 等按值返回的接口。
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl CGRect {
    fn to_rect(self) -> Rect {
        Rect {
            x: self.x.round() as i32,
            y: self.y.round() as i32,
            width: self.width.round() as i32,
            height: self.height.round() as i32,
        }
    }
}

/// CGPoint / CGSize（2×f64），AXValue 装箱用。
#[repr(C)]
#[derive(Clone, Copy)]
struct Pair {
    a: f64,
    b: f64,
}

// ---------- 裸 extern FFI ----------

mod ffi {
    use core::ffi::c_void;

    use super::CGRect;

    /// kAXValueTypeCGPoint / kAXValueTypeCGSize（AXValue.h）。
    pub(super) const K_VALUE_POINT: u32 = 1;
    pub(super) const K_VALUE_SIZE: u32 = 2;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub(super) fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
        pub(super) fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *mut *const c_void,
        ) -> i32;
        pub(super) fn AXUIElementSetAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *const c_void,
        ) -> i32;
        pub(super) fn AXValueCreate(kind: u32, value: *const c_void) -> *mut c_void;
        pub(super) fn AXValueGetValue(value: *const c_void, kind: u32, out: *mut c_void) -> u8;
        /// HIServices 未公开但长期稳定的导出：AX 窗口元素 → CGWindowID。
        /// yabai / Hammerspoon 等同用；返回值类型历史上有出入，调用方只看出参。
        pub(super) fn _AXUIElementGetWindow(element: *const c_void, window_id: *mut u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub(super) fn CFRelease(cf: *const c_void);
        pub(super) fn CFRetain(cf: *const c_void) -> *const c_void;
        pub(super) fn CFArrayGetCount(arr: *const c_void) -> isize;
        pub(super) fn CFArrayGetValueAtIndex(arr: *const c_void, idx: isize) -> *const c_void;
        pub(super) fn CFBooleanGetValue(b: *const c_void) -> u8;
        pub(super) static kCFBooleanTrue: *const c_void;
        pub(super) static kCFBooleanFalse: *const c_void;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub(super) fn CGMainDisplayID() -> u32;
        pub(super) fn CGDisplayBounds(display: u32) -> CGRect;
        pub(super) fn CGGetActiveDisplayList(
            max: u32,
            ids: *mut u32,
            count: *mut u32,
        ) -> i32;
        /// kCGWindowBounds 的 CFDictionary 表示 → CGRect（CGGeometry.h）。
        pub(super) fn CGRectMakeWithDictionaryRepresentation(
            dict: *const c_void,
            rect: *mut CGRect,
        ) -> u8;
    }
}

// ---------- AX 窗口元素 ----------

/// 一个 AX 窗口元素；持有 +1 的 `AXUIElementRef`，Drop 时 `CFRelease`。
pub(crate) struct AxWindow {
    ptr: *mut c_void,
}

impl Drop for AxWindow {
    fn drop(&mut self) {
        // SAFETY: ptr 来自 AXUIElementCreateApplication / CFRetain，持有 +1 引用，
        // 释放使引用计数归还系统。
        unsafe { ffi::CFRelease(self.ptr) };
    }
}

impl AxWindow {
    /// 读窗口矩形（全局坐标）。
    pub(crate) fn frame(&self) -> Result<Rect, String> {
        let pos = self.copy_pair("AXPosition", ffi::K_VALUE_POINT, "读取位置")?;
        let size = self.copy_pair("AXSize", ffi::K_VALUE_SIZE, "读取尺寸")?;
        Ok(Rect {
            x: pos.a.round() as i32,
            y: pos.b.round() as i32,
            width: size.a.round() as i32,
            height: size.b.round() as i32,
        })
    }

    /// 移动并缩放。先尺寸后位置：部分应用在最小尺寸约束下先设位置会被拉回。
    pub(crate) fn set_frame(&self, r: Rect) -> Result<(), String> {
        self.set_pair("AXSize", ffi::K_VALUE_SIZE, r.width as f64, r.height as f64, "缩放")?;
        self.set_pair("AXPosition", ffi::K_VALUE_POINT, r.x as f64, r.y as f64, "移动")
    }

    /// 设置最小化状态（`kAXMinimizedAttribute`）。
    pub(crate) fn set_minimized(&self, minimized: bool) -> Result<(), String> {
        let name = objc2_core_foundation::CFString::from_str("AXMinimized");
        let value = if minimized {
            // SAFETY: kCFBooleanTrue 是 CoreFoundation 导出的全局常量，读取无副作用。
            unsafe { ffi::kCFBooleanTrue }
        } else {
            // SAFETY: 同上。
            unsafe { ffi::kCFBooleanFalse }
        };
        // SAFETY: self.ptr 是有效 AXUIElementRef，name 是有效 CFString，value 是
        // 全局 CFBoolean 常量；函数按值读取，不接管引用。
        let err = unsafe {
            ffi::AXUIElementSetAttributeValue(
                self.ptr,
                &*name as *const _ as *const c_void,
                value,
            )
        };
        if err != 0 {
            let action = if minimized { "最小化" } else { "还原最小化" };
            return Err(ax_err_text(err, action));
        }
        Ok(())
    }

    /// 读最小化状态。
    pub(crate) fn is_minimized(&self) -> Result<bool, String> {
        let value = self.copy_value("AXMinimized", "读取最小化状态")?;
        // SAFETY: value 是 CopyAttributeValue 返回的 +1 CFTypeRef，AXMinimized 的
        // 值是 CFBooleanRef，CFBooleanGetValue 只读取布尔值。
        let got = unsafe {
            let v = ffi::CFBooleanGetValue(value);
            ffi::CFRelease(value);
            v
        };
        Ok(got != 0)
    }

    fn copy_pair(&self, attr: &str, kind: u32, action: &str) -> Result<Pair, String> {
        let value = self.copy_value(attr, action)?;
        let mut pair = Pair { a: 0.0, b: 0.0 };
        // SAFETY: value 是 +1 AXValueRef，kind 与写入的 2×f64 布局匹配；
        // 随后释放引用。
        let ok = unsafe {
            let got = ffi::AXValueGetValue(value, kind, &mut pair as *mut Pair as *mut c_void);
            ffi::CFRelease(value);
            got
        };
        if ok == 0 {
            return Err(format!("系统错误：{action}失败（AXValue 类型不符）"));
        }
        Ok(pair)
    }

    /// 取属性值（+1 CF 对象），错误映射成区分性文案。
    fn copy_value(&self, attr: &str, action: &str) -> Result<*const c_void, String> {
        let name = objc2_core_foundation::CFString::from_str(attr);
        let mut value: *const c_void = core::ptr::null();
        // SAFETY: self.ptr 是有效 AXUIElementRef，name 是有效 CFString；
        // 失败时 value 保持 null；成功时 value 为 +1，由调用方释放。
        let err = unsafe {
            ffi::AXUIElementCopyAttributeValue(
                self.ptr,
                &*name as *const _ as *const c_void,
                &mut value,
            )
        };
        if err != 0 {
            return Err(ax_err_text(err, action));
        }
        if value.is_null() {
            return Err(format!("系统错误：{action}失败（AX 属性值为空）"));
        }
        Ok(value)
    }

    fn set_pair(&self, attr: &str, kind: u32, a: f64, b: f64, action: &str) -> Result<(), String> {
        let name = objc2_core_foundation::CFString::from_str(attr);
        let pair = Pair { a, b };
        // SAFETY: AXValueCreate 按 kind 把 2×f64 装箱成 +1 AXValueRef。
        let value = unsafe { ffi::AXValueCreate(kind, &pair as *const Pair as *const c_void) };
        if value.is_null() {
            return Err(format!("系统错误：{action}失败（AXValueCreate 返回空）"));
        }
        // SAFETY: 三个指针都有效；函数按值读取 value，不接管引用，随后释放。
        let err = unsafe {
            let e = ffi::AXUIElementSetAttributeValue(
                self.ptr,
                &*name as *const _ as *const c_void,
                value,
            );
            ffi::CFRelease(value);
            e
        };
        if err != 0 {
            return Err(ax_err_text(err, action));
        }
        Ok(())
    }
}

/// 按 (pid, CGWindowID) 找对应 AX 窗口；找不到返回 `None`。
pub(crate) fn ax_window(pid: u32, cg_id: isize) -> Option<AxWindow> {
    if pid == 0 || cg_id <= 0 {
        return None;
    }
    // SAFETY: 为 pid 创建应用元素（+1），由下面的 AxWindow 接管释放。
    let app = unsafe { ffi::AXUIElementCreateApplication(pid as i32) };
    if app.is_null() {
        return None;
    }
    let app = AxWindow { ptr: app };
    let arr = app.copy_value("AXWindows", "读取窗口列表").ok()?;
    // SAFETY: arr 是 +1 CFArrayRef，逐项取出数组持有的 +0 AXUIElementRef。
    let count = unsafe { ffi::CFArrayGetCount(arr) };
    for i in 0..count {
        // SAFETY: 0 <= i < count，返回数组持有的元素。
        let w = unsafe { ffi::CFArrayGetValueAtIndex(arr, i) };
        if w.is_null() {
            continue;
        }
        let mut wid: u32 = 0;
        // SAFETY: w 是有效 AXUIElementRef；私有 SPI 仅用出参判定，窗口号 0
        // （kCGNullWindowID）视为不匹配。
        unsafe { ffi::_AXUIElementGetWindow(w, &mut wid) };
        if wid != 0 && wid as isize == cg_id {
            // SAFETY: 数组元素是 +0，CFRetain 后由 AxWindow 持有；数组本身随后释放。
            unsafe { ffi::CFRetain(w) };
            // SAFETY: 释放 copy 得到的 +1 数组。
            unsafe { ffi::CFRelease(arr) };
            return Some(AxWindow { ptr: w as *mut c_void });
        }
    }
    // SAFETY: 同上。
    unsafe { ffi::CFRelease(arr) };
    None
}

// ---------- 显示器与窗口快照 ----------

/// 全部显示器：`(CGDirectDisplayID, 全局边界)`。
pub(crate) fn displays() -> Vec<(u32, Rect)> {
    const MAX_DISPLAYS: usize = 32;
    let mut ids = [0u32; MAX_DISPLAYS];
    let mut count = 0u32;
    // SAFETY: 纯查询；ids 容纳 32 个 id，实际个数回填 count。
    let err =
        unsafe { ffi::CGGetActiveDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count) };
    if err != 0 {
        return Vec::new();
    }
    let n = (count as usize).min(MAX_DISPLAYS);
    ids[..n]
        .iter()
        .map(|id| {
            // SAFETY: id 来自 CGGetActiveDisplayList；CGDisplayBounds 按 C ABI
            // 按值返回 CGRect（rustc 与 C 的结构体返回约定一致）。
            let b = unsafe { ffi::CGDisplayBounds(*id) };
            (*id, b.to_rect())
        })
        .collect()
}

/// 目标显示器：`main_hwnd` 给定时取其所在屏（相交面积最大，不相交时取最近），
/// 否则取主显示器——与 Windows 版 `monitor_of` 同语义。
pub(crate) fn target_display(main_hwnd: Option<isize>) -> Result<u32, String> {
    let list = displays();
    if list.is_empty() {
        return Err("找不到目标显示器".into());
    }
    if let Some(h) = main_hwnd {
        if let Some(bounds) = window_bounds().get(&(h as u32)) {
            let rects: Vec<(u32, Rect)> = list.clone();
            if let Some(i) = best_display(*bounds, &rects).or_else(|| nearest_display(*bounds, &rects))
            {
                return Ok(list[i].0);
            }
        }
    }
    // SAFETY: 无参纯查询，返回主显示器 id。
    let main = unsafe { ffi::CGMainDisplayID() };
    if list.iter().any(|(id, _)| *id == main) {
        Ok(main)
    } else {
        Err("找不到目标显示器".into())
    }
}

/// 目标显示器的工作区（扣除菜单栏与 Dock），全局坐标。
///
/// 工作区取 `NSScreen.visibleFrame`；屏幕按边界与 `CGDisplayBounds` 对齐
/// （同一块屏的 Cocoa / Quartz 两种坐标表示）。
pub(crate) fn work_area(display_id: u32) -> Result<Rect, String> {
    use objc2::encode::{Encode, Encoding, RefEncode};
    use objc2::rc::autoreleasepool;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::msg_send;

    /// NSRect（4×f64），msg_send 结构体返回用。
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NsRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    // SAFETY: NsRect 是 4×f64 的 repr(C) 结构，与 NSRect（CGPoint+CGSize）布局一致。
    unsafe impl Encode for NsRect {
        const ENCODING: Encoding = Encoding::Struct(
            "NSRect",
            &[
                f64::ENCODING,
                f64::ENCODING,
                f64::ENCODING,
                f64::ENCODING,
            ],
        );
    }
    // SAFETY: 同上。
    unsafe impl RefEncode for NsRect {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }

    let target = displays()
        .into_iter()
        .find(|(id, _)| *id == display_id)
        .map(|(_, r)| r)
        .ok_or_else(|| "找不到目标显示器".to_string())?;

    // NSScreen 是 AppKit 对象，消息返回的自动释放对象挂在当前线程的池上；
    // CLI 与测试跑在普通线程，用显式池兜住。
    autoreleasepool(|_pool| {
        let Some(cls) = AnyClass::get(c"NSScreen") else {
            return Err("找不到目标显示器".into());
        };
        // SAFETY: screens 是无参类方法，返回 NSArray（+0）；指针只在池作用域内使用。
        let screens: *const AnyObject = unsafe { msg_send![cls, screens] };
        if screens.is_null() {
            return Err("找不到目标显示器".into());
        }
        // SAFETY: screens 是有效 NSArray。
        let count: usize = unsafe { msg_send![screens, count] };
        if count == 0 {
            return Err("找不到目标显示器".into());
        }
        // SAFETY: 0 < count，取主屏（screens[0]，Cocoa 原点所在屏）。
        let primary: *const AnyObject = unsafe { msg_send![screens, objectAtIndex: 0usize] };
        // SAFETY: 主屏对象的 frame 返回 NSRect（结构体返回由 objc2 按 ABI 处理）。
        let primary_frame: NsRect = unsafe { msg_send![primary, frame] };
        let primary_height = primary_frame.height;

        for i in 0..count {
            // SAFETY: i < count。
            let s: *const AnyObject = unsafe { msg_send![screens, objectAtIndex: i] };
            if s.is_null() {
                continue;
            }
            // SAFETY: 同 primary frame。
            let frame: NsRect = unsafe { msg_send![s, frame] };
            let quartz = cocoa_to_quartz(
                frame.x,
                frame.y,
                frame.width,
                frame.height,
                primary_height,
            );
            if (quartz.x - target.x).abs() <= 1
                && (quartz.y - target.y).abs() <= 1
                && (quartz.width - target.width).abs() <= 1
                && (quartz.height - target.height).abs() <= 1
            {
                // SAFETY: 同 primary frame；visibleFrame 是扣除菜单栏与 Dock 的工作区。
                let visible: NsRect = unsafe { msg_send![s, visibleFrame] };
                return Ok(cocoa_to_quartz(
                    visible.x,
                    visible.y,
                    visible.width,
                    visible.height,
                    primary_height,
                ));
            }
        }
        Err("找不到目标显示器".into())
    })
}

/// 当前在屏窗口的 `CGWindowID → 边界`（全局坐标）；无窗口时为空表。
pub(crate) fn window_bounds() -> HashMap<u32, Rect> {
    use objc2_core_foundation::{CFArray, CFDictionary, CFString, CFType};
    use objc2_core_graphics::{
        kCGNullWindowID, kCGWindowBounds, kCGWindowNumber, CGWindowListCopyWindowInfo,
        CGWindowListOption,
    };

    let mut out = HashMap::new();
    let Some(list) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return out;
    };
    let (key_id, key_bounds) = unsafe { (kCGWindowNumber, kCGWindowBounds) };
    // SAFETY: CGWindowListCopyWindowInfo 的元素都是 CFDictionary，此处只把
    // 元素类型参数换成 CFType 以便逐个 downcast（cast_unchecked 不改运行时行为）。
    let items: &CFArray<CFType> = unsafe { list.cast_unchecked() };
    for item in items.iter() {
        let Some(dict) = item.downcast_ref::<CFDictionary>() else {
            continue;
        };
        let Some(id) = dict_num(dict, key_id) else {
            continue;
        };
        // SAFETY: key_bounds 是有效 CFString 常量；value 为 null 时跳过。
        let ptr = unsafe { dict.value(key_bounds as *const CFString as *const c_void) };
        if ptr.is_null() {
            continue;
        }
        // SAFETY: 非空指针来自 CFDictionary，必是有效 CF 对象。
        let val = unsafe { &*(ptr as *const CFType) };
        // kCGWindowBounds 的值是矩形的 CFDictionary 表示（CGWindow.h）
        let Some(bounds) = val.downcast_ref::<CFDictionary>() else {
            continue;
        };
        let mut rect = CGRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        // SAFETY: bounds 是 CGRectMakeWithDictionaryRepresentation 认识的字典，
        // 成功时写出 4×f64。
        let ok = unsafe {
            ffi::CGRectMakeWithDictionaryRepresentation(
                bounds as *const CFDictionary as *const c_void,
                &mut rect,
            )
        };
        if ok == 0 {
            continue;
        }
        out.insert(id as u32, rect.to_rect());
    }
    out
}

/// 从 CG 窗口字典取 CFNumber 值（winproc 同款降级读取）。
fn dict_num(
    dict: &objc2_core_foundation::CFDictionary,
    key: &objc2_core_foundation::CFString,
) -> Option<i64> {
    use objc2_core_foundation::{CFNumber, CFType};

    // SAFETY: key 是有效 CFString 常量。
    let ptr = unsafe { dict.value(key as *const objc2_core_foundation::CFString as *const c_void) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: 非空指针来自 CFDictionary，必是有效 CF 对象。
    let val = unsafe { &*(ptr as *const CFType) };
    val.downcast_ref::<CFNumber>().and_then(|n| n.as_i64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(id: u32, x: i32, y: i32, w: i32, h: i32) -> (u32, Rect) {
        (
            id,
            Rect {
                x,
                y,
                width: w,
                height: h,
            },
        )
    }

    #[test]
    fn cocoa_to_quartz_flips_y_against_primary_height() {
        // 主屏 1000 高：Cocoa y=0（主屏顶部）→ Quartz y=1000-0-100=900
        let r = cocoa_to_quartz(0.0, 0.0, 100.0, 100.0, 1000.0);
        assert_eq!(r, Rect { x: 0, y: 900, width: 100, height: 100 });
        // Cocoa y=900（贴近主屏底部）→ Quartz y=0
        let r = cocoa_to_quartz(0.0, 900.0, 100.0, 100.0, 1000.0);
        assert_eq!(r, Rect { x: 0, y: 0, width: 100, height: 100 });
    }

    #[test]
    fn cocoa_to_quartz_keeps_negative_x_for_side_display() {
        // 主屏左侧的副屏：x 为负保持不变，Y 翻转后仍可为负
        let r = cocoa_to_quartz(-800.0, 500.0, 800.0, 1000.0, 1000.0);
        assert_eq!(r, Rect { x: -800, y: -500, width: 800, height: 1000 });
    }

    #[test]
    fn best_display_picks_largest_intersection() {
        let displays = vec![d(1, 0, 0, 1920, 1080), d(2, 1920, 0, 1920, 1080)];
        // 跨屏窗口：左侧 300px 在屏 1、右侧 100px 在屏 2 → 选屏 1
        let w = Rect { x: 1620, y: 0, width: 400, height: 100 };
        assert_eq!(best_display(w, &displays), Some(0));
        // 完全落在屏 2
        let w = Rect { x: 2000, y: 100, width: 300, height: 300 };
        assert_eq!(best_display(w, &displays), Some(1));
    }

    #[test]
    fn best_display_none_when_no_overlap() {
        let displays = vec![d(1, 0, 0, 1920, 1080)];
        let w = Rect { x: 5000, y: 5000, width: 100, height: 100 };
        assert_eq!(best_display(w, &displays), None);
    }

    #[test]
    fn best_display_handles_negative_display_coordinates() {
        let displays = vec![d(1, -1920, -200, 1920, 1080)];
        let w = Rect { x: -1000, y: -100, width: 200, height: 200 };
        assert_eq!(best_display(w, &displays), Some(0));
    }

    #[test]
    fn nearest_display_picks_closest_when_straddling_nothing() {
        let displays = vec![d(1, 0, 0, 1000, 1000), d(2, 2000, 0, 1000, 1000)];
        // 窗口中心在两屏之间的空隙，离屏 2 更近
        let w = Rect { x: 1600, y: 400, width: 100, height: 100 };
        assert_eq!(nearest_display(w, &displays), Some(1));
        // 空显示器列表
        assert_eq!(nearest_display(w, &[]), None);
    }

    #[test]
    fn ax_err_texts_distinguish_causes() {
        let refused = ax_err_text(-25205, "移动");
        let stale = ax_err_text(-25202, "移动");
        let system = ax_err_text(-25204, "移动");
        assert!(refused.contains("目标窗口不支持 AX 操作"), "{refused}");
        assert!(stale.contains("AX 元素失效"), "{stale}");
        assert!(system.contains("系统错误"), "{system}");
        assert_ne!(refused, stale);
        assert_ne!(refused, system);
        assert_ne!(stale, system);
    }

    /// 显示器与工作区的实机冒烟：无头环境拿不到显示器时优雅跳过。
    #[test]
    fn displays_and_work_area_smoke_or_skip() {
        let list = displays();
        if list.is_empty() {
            println!("skip: 无显示器信息（无头环境），跳过工作区实机检查");
            return;
        }
        for (id, bounds) in &list {
            assert!(bounds.width > 0 && bounds.height > 0, "显示器边界应为正: {id}");
        }
        let (id, bounds) = list[0];
        match work_area(id) {
            Ok(work) => {
                assert!(work.width > 0 && work.height > 0, "工作区应为正: {work:?}");
                // 工作区不超过显示器边界（扣除菜单栏/Dock 后只会更小）
                assert!(work.width <= bounds.width + 1, "{work:?} vs {bounds:?}");
                assert!(work.height <= bounds.height + 1, "{work:?} vs {bounds:?}");
            }
            Err(e) => {
                if e == "找不到目标显示器" {
                    println!("skip: NSScreen 与显示器列表对不上（无头环境），跳过");
                } else {
                    panic!("工作区读取应成功或明确报找不到显示器: {e}");
                }
            }
        }
    }
}
