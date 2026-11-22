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

/// 通用动作菜单里直接调用的插件 id。是否出现由命令层判定：
/// 模型已配置（`Settings::models` 非空）且插件已安装、启用。
pub const TRANSLATE_PLUGIN: &str = "translate";
pub const AI_PLUGIN: &str = "ai-chat";

/// 动作 id 列表；标题与说明由前端按语言渲染。
///
/// `translate` / `ai` 由 `translate` / `ai` 参数决定（模型已配置 + 插件存在），
/// 且只在有选中文本时出现——拿空文本去问模型没有意义。
/// `open` 只在内容像链接或已存在的路径时出现，避免给出必然失败的动作。
pub fn action_ids(text: &str, translate: bool, ai: bool) -> Vec<&'static str> {
    let mut ids = vec!["copy", "paste", "web"];
    let t = text.trim();
    if !t.is_empty() {
        if translate {
            ids.push("translate");
        }
        if ai {
            ids.push("ai");
        }
    }
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
        assert!(action_ids("hello", false, false).contains(&"copy"));
        assert!(!action_ids("hello", false, false).contains(&"open"));
        assert!(action_ids("https://example.com", false, false).contains(&"open"));
        assert!(action_ids("www.example.com", false, false).contains(&"open"));
    }

    #[test]
    fn translate_and_ai_are_opt_in_with_selection() {
        let plain = action_ids("hello", false, false);
        assert!(!plain.contains(&"translate") && !plain.contains(&"ai"));

        let both = action_ids("hello", true, true);
        assert!(both.contains(&"translate") && both.contains(&"ai"));

        // 空选区不出现：没有文本可翻译/追问
        let blank = action_ids("   ", true, true);
        assert!(!blank.contains(&"translate") && !blank.contains(&"ai"));
    }
}
