//! 通用动作（Universal Actions）：全局热键抓取前台选中文本，弹出动作菜单。
//!
//! 抓取方式是模拟 Ctrl+C 后读剪贴板——不注入键盘钩子、不读取控件内容。
//! 动作本身（复制 / 纯文本粘贴 / 浏览器搜索 / 打开链接）都在命令层执行。

use tauri::{AppHandle, Manager};

use crate::state::Shared;

/// 热键触发：记录前台窗口 → 复制选中内容 → 打开动作窗口。
pub fn capture(app: &AppHandle) {
    crate::guide::remember_foreground();
    crate::winclip::send_copy();
    std::thread::sleep(std::time::Duration::from_millis(150));
    let text = read_clipboard_text();
    open(app, text);
}

/// 从当前剪贴板文本直接打开动作窗口（内置动作"通用动作"用）。
pub fn from_clipboard(app: &AppHandle) {
    let text = read_clipboard_text();
    open(app, text);
}

fn open(app: &AppHandle, text: String) {
    if let Some(data) = app.try_state::<Shared>() {
        *data.selection.lock() = Some(text);
    }
    crate::windows::show_actions(app);
}

fn read_clipboard_text() -> String {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_default()
}

/// 动作 id 列表；标题与说明由前端按语言渲染。
///
/// `open` 只在内容像链接或已存在的路径时出现，避免给出必然失败的动作。
pub fn action_ids(text: &str) -> Vec<&'static str> {
    let mut ids = vec!["copy", "paste", "web"];
    let t = text.trim();
    if t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("www.")
        || std::path::Path::new(t).exists()
    {
        ids.push("open");
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_only_for_link_or_existing_path() {
        assert!(action_ids("hello").contains(&"copy"));
        assert!(!action_ids("hello").contains(&"open"));
        assert!(action_ids("https://example.com").contains(&"open"));
        assert!(action_ids("www.example.com").contains(&"open"));
    }
}
