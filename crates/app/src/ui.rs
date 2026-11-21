//! 插件 UI 页面调用的命令。
//!
//! 安全要点：`plugin_id` 一律从**窗口 label** 反查，不接受前端传入，
//! 因此某个插件页面无法读写另一个插件的数据。

use serde_json::Value;
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::host::TauriHost;
use crate::state::Shared;
use crate::windows;

/// label 形如 `plugin-ui-<sanitize(id)>`，还原成注册表里的真实 id。
fn plugin_of(app: &AppHandle, window: &WebviewWindow) -> Result<String, String> {
    let label_id = window
        .label()
        .strip_prefix("plugin-ui-")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "该窗口不是插件 UI 窗口".to_string())?
        .to_string();
    // 先把 id 拷出来再释放注册表读锁：guard 的生命周期不能长于 State 借用
    let ids: Vec<String> = {
        let data = app.state::<Shared>();
        let registry = data.registry.read();
        registry.plugins().iter().map(|p| p.id()).collect()
    };
    ids.into_iter()
        .find(|id| efto_core::fsutil::sanitize_key(id) == label_id)
        .ok_or_else(|| format!("未知插件: {label_id}"))
}

fn call(app: &AppHandle, window: &WebviewWindow, module: &str, method: &str, args: Vec<Value>) -> Result<Value, String> {
    // 这两条日志是排查"插件页面点了没反应"的关键：能看到 UI 的调用到底有没有到 Rust，
    // 以及反查 plugin id 是否成功（失败时 SDK 的 promise 会静默 reject）
    let plugin = plugin_of(app, window).map_err(|e| {
        tracing::warn!("[ui→main] {module}.{method} 反查插件失败: {e}");
        e
    })?;
    let host = TauriHost::new(app.clone());
    let r = <TauriHost as efto_jsrt::Host>::call(&host, &plugin, module, method, &args);
    match &r {
        Ok(_) => tracing::info!("[ui→main] {plugin} {module}.{method} ok"),
        Err(e) => tracing::warn!("[ui→main] {plugin} {module}.{method} 失败: {e}"),
    }
    r
}

#[tauri::command]
pub fn plugin_db_get(key: String, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "db", "get", vec![Value::String(key)])
}

#[tauri::command]
pub fn plugin_db_set(key: String, value: Value, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "db", "set", vec![Value::String(key), value])
}

#[tauri::command]
pub fn plugin_db_remove(key: String, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "db", "remove", vec![Value::String(key)])
}

#[tauri::command]
pub fn plugin_db_all(app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "db", "all", vec![])
}

#[tauri::command]
pub fn plugin_db_keys(app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "db", "keys", vec![])
}

#[tauri::command]
pub fn plugin_db_size(app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "db", "size", vec![])
}

#[tauri::command]
pub fn plugin_copy_text(text: String, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "system", "copyText", vec![Value::String(text)])
}

#[tauri::command]
pub fn plugin_paste_text(app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "system", "pasteText", vec![])
}

#[tauri::command]
pub fn plugin_notify(text: String, sub: String, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "notify", "show", vec![Value::String(text), Value::String(sub)])
}

#[tauri::command]
pub fn plugin_shell_open(path: String, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "shell", "openPath", vec![Value::String(path)])
}

#[tauri::command]
pub fn plugin_shell_show_in_folder(
    path: String,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Value, String> {
    call(&app, &window, "shell", "showItemInFolder", vec![Value::String(path)])
}

#[tauri::command]
pub fn plugin_open_external(url: String, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "shell", "openExternal", vec![Value::String(url)])
}

/// 插件页面调用原生文件选择框，返回用户选中的绝对路径（取消时为空数组）。
///
/// rfd 是阻塞式对话框，放到阻塞线程池里跑，避免卡住 Tauri 的异步运行时。
#[tauri::command]
pub async fn plugin_pick_files(app: AppHandle, window: WebviewWindow) -> Result<Vec<String>, String> {
    plugin_of(&app, &window)?;
    tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_title("选择要分享的文件")
            .set_parent(&window)
            .pick_files()
            .map(|paths| {
                paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .map_err(|e| format!("文件选择框失败：{e}"))
}

/// 插件 UI 页面的日志出口。
///
/// 插件页面里的 `console.warn/error` 只会进 webview 的开发者工具，宿主日志里看不到 ——
/// 排查"点了没反应"时等于瞎。SDK 把关键事件（订阅失败、命令报错）转到这里。
#[tauri::command]
pub fn plugin_ui_log(level: String, message: String, window: WebviewWindow) {
    let label = window.label();
    match level.as_str() {
        "error" => tracing::error!("[ui:{label}] {message}"),
        "warn" => tracing::warn!("[ui:{label}] {message}"),
        _ => tracing::info!("[ui:{label}] {message}"),
    }
}

/// UI 页面注册好事件监听后调用：此时才把主进程排队的消息发给它。
#[tauri::command]
pub fn plugin_ui_ready(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    let plugin = plugin_of(&app, &window)?;
    windows::flush_pending(&app, &plugin);
    Ok(())
}

/// UI → 本插件 main 的消息转发（`efto.ipcMessage`）。
#[tauri::command]
pub fn plugin_to_main(name: String, data: Value, app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    call(&app, &window, "ui", "ipcMessage", vec![Value::String(name), data])?;
    Ok(())
}

/// 跨插件广播。
#[tauri::command]
pub fn plugin_broadcast(name: String, data: Value, app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    call(&app, &window, "broadcast", "send", vec![Value::String(name), data])?;
    Ok(())
}

#[tauri::command]
pub fn plugin_set_subtitle(text: String, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    call(&app, &window, "ui", "setSubTitle", vec![Value::String(text)])
}

#[tauri::command]
pub fn plugin_redirect(label: String, options: Value, app: AppHandle, window: WebviewWindow) -> Result<Value, String> {
    let _ = window;
    crate::host::redirect_to(&app, &label, &options)
}

#[tauri::command]
pub fn plugin_hide_self(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    let plugin = plugin_of(&app, &window)?;
    windows::hide_plugin_ui(&app, &plugin);
    // 同时告诉后端 UI 已隐藏（`onWindowHide` 属于 main）
    let data = app.state::<Shared>();
    let _ = data.runtime.lock().main_message(&plugin, "window:hide", Value::Null);
    Ok(())
}

#[tauri::command]
pub fn plugin_show_main(app: AppHandle) {
    windows::show_main(&app);
}

#[tauri::command]
pub fn plugin_quit(app: AppHandle) {
    {
        let data = app.state::<Shared>();
        data.flush_stores();
        data.runtime.lock().stop_all();
    }
    app.exit(0);
}

#[cfg(test)]
mod tests {
    #[test]
    fn label_roundtrip_matches_sanitize() {
        // plugin_of 的纯逻辑部分：label 前缀 + sanitize 一致性
        let id = "中文 插件/v2";
        let label = format!("plugin-ui-{}", efto_core::fsutil::sanitize_key(id));
        let stripped = label.strip_prefix("plugin-ui-").unwrap();
        assert_eq!(stripped, efto_core::fsutil::sanitize_key(id));
        assert!(!stripped.contains('/'));
    }
}
