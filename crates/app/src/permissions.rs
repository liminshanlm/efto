//! 平台权限探测与引导（macOS：辅助功能 / 屏幕录制）。
//!
//! 需要权限的能力（窗口平铺、键盘模拟、截图）在执行前查这里，缺权限时给出
//! 明确报错 + 设置页"权限"区块引导授权（对应系统设置的
//! `x-apple.systempreferences:com.apple.preference.security?Privacy_*`）。
//! Windows 没有这套授权模型，全部视为已授权。

/// 辅助功能（Accessibility）：CGEvent 键盘模拟、AX 窗口移动/缩放等需要。
pub fn accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: 无参无副作用的纯查询；HIServices 的 Boolean 按 u8 返回。
        unsafe { ffi::AXIsProcessTrusted() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// 屏幕录制（Screen Recording）：截图/取窗口标题（CGWindowList）需要。
pub fn screen_recording_allowed() -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: 无参纯查询（10.15+）。
        unsafe { ffi::CGPreflightScreenCaptureAccess() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// 请求屏幕录制权限（触发系统引导/弹窗；已授权时无副作用）。
/// 是否立刻可用以 [`screen_recording_allowed`] 复查为准——用户可能跳去设置页再回来。
pub fn request_screen_recording() {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: 触发系统权限引导；可能弹窗/跳设置，幂等。
        unsafe {
            ffi::CGRequestScreenCaptureAccess();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {}
}

/// 权限状态（给设置页与 CLI 的机器可读形状）。
pub fn status() -> serde_json::Value {
    serde_json::json!({
        "accessibility": accessibility_trusted(),
        "screen_recording": screen_recording_allowed(),
    })
}

#[cfg(target_os = "macos")]
mod ffi {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub fn AXIsProcessTrusted() -> u8;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGPreflightScreenCaptureAccess() -> u8;
        pub fn CGRequestScreenCaptureAccess() -> u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_shape_is_stable() {
        let v = status();
        assert!(v["accessibility"].is_boolean());
        assert!(v["screen_recording"].is_boolean());
    }
}
