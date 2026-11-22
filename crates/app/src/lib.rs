//! efto 主程序装配。
//!
//! 线程模型（第一阶段）：
//! - 主线程：Tauri/WebView2 消息循环，所有窗口创建与显示
//! - 命令线程池：tauri 的 sync command（搜索、启动应用）
//! - 插件线程：每个运行中的插件一个（内含 V8 isolate）
//! - `efto-pump`：消费插件事件
//! - `efto-refresher`：按间隔重建索引
//! - 能力线程池（jsrt::pool）：HTTP/AI 等异步能力
//!
//! 锁顺序约定：允许 `settings → registry → index`，禁止反向；
//! 任何宿主能力都不得同步等待插件线程，插件线程也不得等待主线程（UI 类能力一律异步投递）。

pub mod cli;
mod ai;
mod clipwatch;
mod commands;
pub mod hosts;
mod hotkey;
mod guide;
mod host;
mod http;
mod i18n;
mod icons;
mod protocol;
mod pump;
mod permissions;
mod preview;
mod screenshot;
mod share;
mod state;
mod system;
mod tile;
mod tray;
mod ui;
mod universal;
mod windows;
mod winclip;
mod winproc;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Manager, WindowEvent};
use tauri_plugin_global_shortcut::Builder as ShortcutBuilder;

use crate::host::TauriHost;
use crate::state::{AppData, Shared};

/// 让前端与插件页面在控制台看到可读日志（RUST_LOG 可覆盖级别）。
///
/// stderr 不能省（CLI 的 `--json` 走 stdout，日志混进去会让调用方解析失败），
/// 但 GUI 方式启动时 stderr 无人接收——没有文件日志就等于没有日志
/// （file-share 排障时踩过：用户报"日志里没信号"，实际是日志根本没被收集）。
/// 因此同时追加到 `~/.efto/efto.log`，两个 sink 各写一份。
fn init_tracing() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,hyper=warn,ureq=warn,wry=warn"));
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(efto_core::fsutil::data_root().join("efto.log"))
        .ok()
        .map(std::sync::Arc::new);
    let registry = tracing_subscriber::registry().with(stderr_layer);
    match file {
        Some(f) => {
            registry
                .with(tracing_subscriber::fmt::layer().with_target(false).with_writer(f))
                .with(filter)
                .init();
        }
        // 文件打不开（权限/磁盘满）不能挡启动：退回只写 stderr
        None => registry.with(filter).init(),
    }
}

/// macOS：不进 Dock / 菜单栏（等价 Windows 的 skipTaskbar）。
///
/// 打包后 Info.plist 的 `LSUIElement` 已经生效；这里是裸 `cargo run` 开发态的兜底——
/// 没有 bundle 就没有 Info.plist，Dock 图标会冒出来。启动时在主线程切一次即可。
#[cfg(target_os = "macos")]
fn adopt_accessory_mode() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

#[cfg(not(target_os = "macos"))]
fn adopt_accessory_mode() {}

pub fn run() {
    init_tracing();
    adopt_accessory_mode();
    tracing::info!("efto {} 启动", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        // 单实例：二次启动只唤起已有窗口
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            windows::show_main(app);
        }))
        // 空 Builder，运行期按设置注册热键（便于改绑定）
        .plugin(ShortcutBuilder::new().build())
        .plugin(tauri_plugin_notification::init())
        .register_asynchronous_uri_scheme_protocol(protocol::EFTO_SCHEME, move |ctx, req, responder| {
            let data = ctx.app_handle().state::<Shared>().inner().clone();
            protocol::handle(data, req, responder);
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let host = Arc::new(TauriHost::new(handle.clone()));
            let data: Shared = Arc::new(AppData::new(host));
            app.manage(data.clone());

            // 必须在 manage 之后建窗口：tauri 的内建 setup 会在用户 setup **之前**
            // 创建 tauri.conf.json 里的窗口（tauri/src/app.rs 的 `fn setup`），
            // 而 `AppData::new` 要载入/重建索引（几百毫秒）。页面脚本一加载就发
            // `search`，此时 `Shared` 还没 manage，命令返回 "state not managed" ——
            // 表现就是刚启动立刻按 Alt+Space，搜索框带着「搜索失败」的 toast 弹出来。
            create_config_windows(app)?;
            // 配置里的窗口默认跟随系统；用户在设置里固定过明暗时在这里纠回来
            windows::apply_theme(&handle, data.settings.lock().theme);
            // 原生标题栏按当前语言设置一次（内容区由前端 i18n 自己替换）
            windows::apply_language(&handle, i18n::resolve(&data.settings.lock()));

            hotkey::rebind(&handle).map_err(|e| {
                tracing::error!("热键绑定失败: {e}");
                e
            })?;
            tray::build(app)?;
            pump::start(&handle);
            clipwatch::start(handle.clone());
            spawn_refresher(handle.clone(), data.clone());
            // Windows 上预热预览窗口很便宜（WebView2 初始化快）；macOS 上建一个
            // WKWebView 要占用主线程 1~3 秒，正好会和"唤起启动器后立刻点截图"撞上
            // ——hide_main 的投递排队、启动器被拍进底图、甚至截图界面起不来。
            // 所以 mac 上不预热，Shift 首次预览慢一点可以接受。
            #[cfg(not(target_os = "macos"))]
            {
                let h = handle.clone();
                let _ = std::thread::Builder::new()
                    .name("preview-warmup".into())
                    .spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(2000));
                        windows::warm_up_preview(&h);
                    });
            }

            // 首次启动不抢焦点，但告知用户热键在哪
            if let Some(w) = handle.get_webview_window(windows::MAIN_LABEL) {
                let _ = w.hide();
            }
            tracing::info!(
                "就绪：应用 {} 个，插件 {} 个，热键 {}，通用动作 {}",
                data.index.read().len(),
                data.registry.read().len(),
                hotkey::current().unwrap_or_default(),
                hotkey::current_ua().unwrap_or_else(|| "未启用".into())
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::platform_info,
            commands::permission_status,
            commands::request_permission,
            commands::open_permission_settings,
            commands::search_files,
            commands::clear_recent,
            commands::invoke_hit,
            commands::cancel_command,
            commands::preview_show,
            commands::preview_ready,
            commands::preview_hide,
            commands::preview_release,
            commands::preview_open,
            commands::refresh_index,
            commands::index_status,
            commands::list_plugins,
            commands::set_plugin_cmd_alias,
            commands::set_plugin_enabled,
            commands::open_settings,
            commands::import_plugin,
            commands::pick_plugin_source,
            commands::pick_index_dir,
            commands::open_plugin_dir,
            commands::open_data_dir,
            commands::screenshot_begin,
            commands::screenshot_offset,
            commands::screenshot_finish,
            commands::screenshot_cancel,
            commands::screenshot_long_start,
            commands::screenshot_long_status,
            commands::pin_load,
            commands::pin_drag,
            commands::pin_close,
            commands::pin_copy,
            commands::clipboard_list,
            commands::clipboard_copy,
            commands::clipboard_paste,
            commands::clipboard_pin,
            commands::clipboard_thumb,
            commands::clipboard_remove,
            commands::clipboard_clear,
            commands::hide_clipboard,
            commands::snippets_list,
            commands::snippets_save,
            commands::file_action,
            commands::file_transfer,
            commands::file_buffer_list,
            commands::file_buffer_remove,
            commands::file_buffer_clear,
            commands::file_buffer_action,
            commands::show_file_buffer,
            commands::hide_files,
            commands::actions_data,
            commands::actions_run,
            commands::hide_actions,
            commands::icon_for,
            commands::everything_status,
            commands::install_es,
            commands::open_url,
            commands::get_settings,
            commands::set_settings,
            commands::theme_mode,
            commands::ui_language,
            commands::runtime_status,
            commands::runtime_check,
            commands::command_check,
            commands::help_document,
            commands::hide_help,
            commands::shortcut_guide_data,
            commands::shortcut_guide_lookup,
            commands::hide_shortcuts,
            commands::model_check,
            commands::show_main,
            commands::hide_main,
            commands::quit,
            ui::plugin_db_get,
            ui::plugin_db_set,
            ui::plugin_db_remove,
            ui::plugin_db_all,
            ui::plugin_db_keys,
            ui::plugin_db_size,
            ui::plugin_copy_text,
            ui::plugin_paste_text,
            ui::plugin_notify,
            ui::plugin_shell_open,
            ui::plugin_shell_show_in_folder,
            ui::plugin_open_external,
            ui::plugin_pick_files,
            ui::plugin_to_main,
            ui::plugin_broadcast,
            ui::plugin_ui_log,
            ui::plugin_ui_ready,
            ui::plugin_set_subtitle,
            ui::plugin_redirect,
            ui::plugin_hide_self,
            ui::plugin_show_main,
            ui::plugin_quit
        ])
        .on_menu_event(|app, event| tray::handle(app, &event))
        .on_window_event(|window, event| {
            if window.label() == windows::SETTINGS_LABEL {
                // 设置窗口常驻：关闭只隐藏，下次从设置入口复用
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if window.label() == windows::CLIPBOARD_LABEL {
                // 剪贴板窗口同样常驻隐藏
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if window.label() == windows::HELP_LABEL {
                // 帮助窗口常驻：关闭只隐藏，避免每次重开都要重新加载 WebView2
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if window.label() == windows::SHORTCUTS_LABEL {
                // 快捷键指南同样常驻，关闭只隐藏
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if window.label() == windows::ACTIONS_LABEL || window.label() == windows::FILES_LABEL {
                // 通用动作与文件缓冲常驻，关闭只隐藏
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if let Some(id) = window.label().strip_prefix(windows::PIN_PREFIX) {
                // 钉图是临时窗口：关掉就把落盘的 png 删掉
                if let WindowEvent::Destroyed = event {
                    let path = efto_core::fsutil::data_root()
                        .join("pins")
                        .join(format!("{id}.png"));
                    let _ = std::fs::remove_file(path);
                }
                return;
            }
            if window.label() != windows::MAIN_LABEL {
                return;
            }
            match event {
                WindowEvent::Focused(false) => {
                    // Shift 预览会短暂抢走焦点，此时不能按"失焦即隐藏"处理
                    if windows::preview_active() {
                        return;
                    }
                    let data = window.app_handle().state::<Shared>();
                    if data.settings.lock().hide_on_blur {
                        windows::hide_main(window.app_handle());
                    }
                }
                WindowEvent::CloseRequested { api, .. } => {
                    // 搜索框没有"关闭"，只有隐藏
                    api.prevent_close();
                    windows::hide_main(window.app_handle());
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| panic!("构建 efto 失败: {e}"))
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { .. } => {
                let data = app.state::<Shared>();
                data.flush_stores();
                data.runtime.lock().stop_all();
                hotkey::unbind(app);
            }
            _ => {}
        });
}

/// 创建 `tauri.conf.json` 里声明的窗口。
///
/// 配置里三个窗口都写了 `"create": false`，交给这里在 `app.manage(Shared)` 之后创建，
/// 保证页面脚本开始执行时宿主状态一定已经就绪（原因见 setup 里的注释）。
fn create_config_windows(app: &tauri::App) -> tauri::Result<()> {
    for cfg in app.config().app.windows.iter() {
        tauri::WebviewWindowBuilder::from_config(app.handle(), cfg)?.build()?;
    }
    Ok(())
}

/// 按设置间隔重建索引；间隔为 0 时只依赖手动触发。
fn spawn_refresher(_app: tauri::AppHandle, data: Shared) {
    let _ = std::thread::Builder::new()
        .name("efto-refresher".into())
        .spawn(move || {
            let mut consecutive_failures = 0u32;
            loop {
                let interval = data.settings.lock().refresh_interval_secs;
                if interval == 0 {
                    std::thread::sleep(Duration::from_secs(30));
                    continue;
                }
                std::thread::sleep(Duration::from_secs(interval));
                let before = data.index.read().len();
                let report = data.rebuild();
                if report.merged == 0 && before > 0 {
                    consecutive_failures += 1;
                    tracing::warn!(
                        "索引重建得到 0 条（上次 {before} 条），连续失败 {consecutive_failures} 次"
                    );
                    if consecutive_failures >= 3 {
                        tracing::error!("连续 3 次重建为空，停止自动重建，请手动触发");
                        return;
                    }
                } else {
                    consecutive_failures = 0;
                }
            }
        });
}

/// 给集成测试用的纯逻辑入口：不启动 GUI 也能验证协议与路径规则。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_name_is_valid_for_urls() {
        assert_eq!(protocol::EFTO_SCHEME, "efto");
        let u = url::Url::parse(&format!(
            "{}://localhost/demo/index.html",
            protocol::EFTO_SCHEME
        ))
        .unwrap();
        assert_eq!(u.host_str(), Some("localhost"));
        assert_eq!(u.path(), "/demo/index.html");
    }

    #[test]
    fn main_label_is_stable() {
        assert_eq!(windows::MAIN_LABEL, "main");
        assert_eq!(windows::ui_label("demo"), "plugin-ui-demo");
    }

    /// 回归守卫：配置里的窗口必须留给 [`create_config_windows`] 在 `app.manage(Shared)` 之后建，
    /// 所以 tauri.conf.json 里每个窗口都得写 `"create": false`。
    /// 漏写会让页面脚本赶在状态就绪前发 `search`，开屏报 "state not managed"。
    #[test]
    fn config_windows_are_deferred_until_state_ready() {
        let cfg: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json 不是合法 JSON");
        let windows = cfg["app"]["windows"].as_array().expect("缺少 app.windows");
        assert!(!windows.is_empty());
        for w in windows {
            assert_eq!(
                w["create"].as_bool(),
                Some(false),
                "窗口 {} 必须写 create:false",
                w["label"]
            );
        }
    }
}
