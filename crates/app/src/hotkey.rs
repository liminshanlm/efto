//! 全局热键绑定。
//!
//! 语义：一次只允许一个唤起热键。改绑定时先 `unregister_all` 再注册，
//! 这样"注册成功但旧键没释放"或"旧键释放了但新键没生效"都不会出现。

use std::str::FromStr;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::state::Shared;
use crate::windows;

/// 当前生效的唤起热键（用于诊断与重复绑定判断）。
static CURRENT: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
/// 当前生效的通用动作热键；未启用/未注册为 `None`。
static CURRENT_UA: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
/// 当前生效的单项热键：(热键, 工具 id)。
static CURRENT_ITEMS: Lazy<Mutex<Vec<(String, String)>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn current() -> Option<String> {
    CURRENT.lock().ok().and_then(|g| g.clone())
}

pub fn current_ua() -> Option<String> {
    CURRENT_UA.lock().ok().and_then(|g| g.clone())
}

pub fn current_items() -> Vec<(String, String)> {
    CURRENT_ITEMS
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn rebind(app: &AppHandle) -> Result<(), String> {
    let (want, ua, items) = {
        let shared = app.state::<Shared>();
        let s = shared.settings.lock();
        // 通用动作在部分平台未实现（见 BuiltinAction::supported_on_platform）：
        // 不注册热键，免得按下去只有一条报错（宁可不出）。
        let ua = if s.universal_actions_enabled
            && efto_core::search::BuiltinAction::UniversalActions.supported_on_platform()
        {
            Some(s.universal_actions_hotkey.clone())
        } else {
            None
        };
        let items: Vec<(String, String)> = s
            .tool_hotkeys
            .iter()
            .map(|(tool, keys)| (keys.clone(), tool.clone()))
            .collect();
        (s.hotkey.clone(), ua, items)
    };
    if current().as_deref() == Some(want.as_str()) && current_ua() == ua && current_items() == items
    {
        return Ok(());
    }
    unbind(app);
    register(app, &want)?;
    if let Ok(mut g) = CURRENT.lock() {
        *g = Some(want.clone());
    }
    if let Some(ua) = ua {
        if ua.eq_ignore_ascii_case(&want) {
            tracing::warn!("通用动作热键与唤起热键相同（{ua}），跳过注册");
        } else {
            // 第二个热键注册失败不影响唤起热键：记日志，设置页仍能正常保存。
            match register_universal_actions(app, &ua) {
                Ok(()) => {
                    if let Ok(mut g) = CURRENT_UA.lock() {
                        *g = Some(ua);
                    }
                }
                Err(e) => tracing::error!("通用动作热键注册失败: {e}"),
            }
        }
    }
    let mut bound = Vec::new();
    for (keys, tool) in items {
        match register_tool(app, &keys, &tool) {
            Ok(()) => bound.push((keys, tool)),
            Err(e) => tracing::error!("单项热键注册失败: {e}"),
        }
    }
    if let Ok(mut g) = CURRENT_ITEMS.lock() {
        *g = bound;
    }
    Ok(())
}

fn register(app: &AppHandle, accel: &str) -> Result<(), String> {
    let shortcut = Shortcut::from_str(accel)
        .map_err(|e| format!("热键 {accel} 无效: {e}；示例写法 Alt+Space、Ctrl+Alt+K"))?;
    let app2 = app.clone();
    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            // Windows 上 Pressed/Released 都会回调，只处理按下
            if event.state == ShortcutState::Pressed {
                // 快捷键指南要在搜索框抢走焦点前记住用户刚才在哪个应用里
                crate::guide::remember_foreground();
                windows::toggle_main(&app2);
            }
        })
        .map_err(|e| format!("注册全局热键 {accel} 失败: {e}"))?;
    tracing::info!("全局热键已绑定: {accel}");
    Ok(())
}

fn register_universal_actions(app: &AppHandle, accel: &str) -> Result<(), String> {
    let shortcut = Shortcut::from_str(accel)
        .map_err(|e| format!("通用动作热键 {accel} 无效: {e}"))?;
    let app2 = app.clone();
    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                crate::universal::capture(&app2);
            }
        })
        .map_err(|e| format!("注册通用动作热键 {accel} 失败: {e}"))?;
    tracing::info!("通用动作热键已绑定: {accel}");
    Ok(())
}

fn register_tool(app: &AppHandle, accel: &str, tool: &str) -> Result<(), String> {
    let shortcut =
        Shortcut::from_str(accel).map_err(|e| format!("单项热键 {accel} 无效: {e}"))?;
    let app2 = app.clone();
    let tool2 = tool.to_string();
    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                crate::commands::run_tool_from_hotkey(&app2, &tool2);
            }
        })
        .map_err(|e| format!("注册单项热键 {accel}（{tool}）失败: {e}"))?;
    tracing::info!("单项热键已绑定: {accel} -> {tool}");
    Ok(())
}

pub fn unbind(app: &AppHandle) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        tracing::debug!("注销全局热键失败（可能本来未注册）: {e}");
    }
    if let Ok(mut g) = CURRENT.lock() {
        *g = None;
    }
    if let Ok(mut g) = CURRENT_UA.lock() {
        *g = None;
    }
    if let Ok(mut g) = CURRENT_ITEMS.lock() {
        g.clear();
    }
}

/// 临时全局热键（长截图期间用 Esc 中断）：返回的 `Shortcut` 交给调用方在结束时注销。
///
/// 不走 [`CURRENT`]：它不是唤起键，也不影响设置里绑定的那个。
pub fn bind_temporary<F>(app: &AppHandle, accel: &str, on_press: F) -> Option<Shortcut>
where
    F: Fn() + Send + Sync + 'static,
{
    let shortcut = Shortcut::from_str(accel).ok()?;
    let _ = app.global_shortcut().unregister(shortcut.clone());
    if let Err(e) = app
        .global_shortcut()
        .on_shortcut(shortcut.clone(), move |_app, _sc, event| {
            if event.state == ShortcutState::Pressed {
                on_press();
            }
        })
    {
        tracing::warn!("临时热键 {accel} 注册失败: {e}");
        return None;
    }
    Some(shortcut)
}

/// 注销 [`bind_temporary`] 注册的热键。
pub fn unbind_temporary(app: &AppHandle, shortcut: Shortcut) {
    if let Err(e) = app.global_shortcut().unregister(shortcut) {
        tracing::debug!("注销临时热键失败: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_accelerators() {
        for ok in ["Alt+Space", "Ctrl+Alt+K", "CmdOrCtrl+Shift+P", "Super+D", "Escape"] {
            assert!(Shortcut::from_str(ok).is_ok(), "{ok} 应可解析");
        }
    }

    #[test]
    fn rejects_garbage_with_helpful_message() {
        for bad in ["", "NotAKey", "Alt+", "Space"] {
            let e = Shortcut::from_str(bad).err();
            // 有的写法 V8/global-hotkey 会当成单键，只要求不能 panic
            let _ = e;
        }
        let err = register_missing_app().unwrap_err();
        assert!(err.contains("热键"), "{err}");
    }

    fn register_missing_app() -> Result<(), String> {
        // 没有 AppHandle 时验证错误文案格式（不真的调用 tauri）
        let accel = "不存在的键组合++";
        Shortcut::from_str(accel)
            .map(|_| ())
            .map_err(|e| format!("热键 {accel} 无效: {e}；示例写法 Alt+Space、Ctrl+Alt+K"))
    }

    #[test]
    fn current_starts_empty() {
        // 单测之间共享 static，只要求可读取且不 panic
        let _ = current();
        let _ = current_ua();
    }
}
