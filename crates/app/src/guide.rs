//! 记住按下唤起热键时的前台应用，供快捷键指南识别。

use std::sync::Mutex;

use crate::winproc::WindowInfo;

static LAST_FOREGROUND: Mutex<Option<WindowInfo>> = Mutex::new(None);

/// 在显示搜索框之前调用：记录当前前台窗口。
///
/// 前台是 efto 自己时（连续按热键隐藏/显示）保留上一次记录，
/// 否则快捷键指南会从"上次用的应用"退化成"没有应用"。
pub fn remember_foreground() {
    let Some(info) = crate::winproc::foreground() else {
        return;
    };
    if info.pid == std::process::id() {
        return;
    }
    if let Ok(mut slot) = LAST_FOREGROUND.lock() {
        *slot = Some(info);
    }
}

/// 最近一次唤起前的应用；从未记录或已失效时返回 `None`。
pub fn last() -> Option<WindowInfo> {
    LAST_FOREGROUND.lock().ok().and_then(|g| g.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_is_readable() {
        // 单测共享 static：只要求读取不 panic
        let _ = last();
    }
}
