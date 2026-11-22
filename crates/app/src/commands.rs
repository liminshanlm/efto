//! 前端与插件 UI 页面调用的命令。
//!
//! 约定（与 tauri-macros 的默认参数命名一致）：所有参数都用**单个单词**，
//! 这样 JS 侧的 key 与 Rust 侧完全相同，不依赖 camelCase 转换规则。

use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use efto_core::apps::indexer::ScanReport;
use efto_core::config::Settings;
use efto_core::search::SearchHit;

use crate::state::{self, Shared};
use crate::windows;

/// 前端搜索。
#[tauri::command]
pub fn search(query: String, data: State<'_, Shared>) -> Vec<SearchHit> {
    data.search(&query, None)
}

/// 文件检索（Everything）。单独一条异步命令：es 是子进程，必须离开 UI 线程；
/// 而且晚到的文件行只补在应用结果后面，不能拖慢应用结果先上屏。
/// 不可用时返回空列表（状态在设置页看）。
#[tauri::command]
pub async fn search_files(
    query: String,
    data: State<'_, Shared>,
) -> Result<Vec<SearchHit>, String> {
    let settings = data.settings.lock().clone();
    let hits = tauri::async_runtime::spawn_blocking(move || {
        // 命令前缀命中时文件源必须让路，否则 `> ipconfig` 后面会补一串文件；
        // 网页搜索触发词与系统命令同理，保持一屏只有用户要的那类结果。
        // Windows 分支里要补 win_settings 条件；mac 上不会重新赋值
        #[allow(unused_mut)]
        let mut blocked = efto_core::search::command::parse(&query, &settings).is_some()
            || efto_core::websearch::parse(&query, &settings.web_searches).is_some()
            || efto_core::system::parse(&query).is_some();
        #[cfg(windows)]
        {
            blocked = blocked || efto_core::search::win_settings::is_query(&query);
        }
        if blocked {
            return Vec::new();
        }
        // 文件检索源：Windows 走 Everything/es.exe，macOS 走 Spotlight/mdfind
        //（均在 core::search 内，分数压应用下）；其它平台无文件源，返回空。
        #[cfg(windows)]
        {
            efto_core::search::everything::hits(&query)
        }
        #[cfg(target_os = "macos")]
        {
            efto_core::search::spotlight::hits(&query)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            Vec::new()
        }
    })
    .await
    .map_err(|e| format!("文件检索失败: {e}"))?;
    Ok(hits)
}

/// 清空"最近使用"；前端随后重新 search 即可拿到去掉 recent 分组的面板。
#[tauri::command]
pub fn clear_recent(data: State<'_, Shared>) {
    data.clear_recent();
}

/// 执行一条搜索结果。返回 Err 时前端展示失败原因，避免"点了没反应"。
#[tauri::command]
pub fn invoke_hit(
    hit: SearchHit,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<InvokeOutcome, String> {
    use efto_core::search::HitPayload;
    match &hit.payload {
        HitPayload::Launch { target } => {
            efto_core::apps::launch::launch(target).map_err(|e| e.to_string())?;
            data.record_usage(&hit.id);
            data.record_recent(&hit);
            windows::hide_main(&app);
            Ok(InvokeOutcome::now(format!("已启动 {}", hit.title)))
        }
        HitPayload::Open { path } => {
            efto_core::apps::launch::shell_open(path, None).map_err(|e| e.to_string())?;
            windows::hide_main(&app);
            Ok(InvokeOutcome::now(format!("已打开 {path}")))
        }
        HitPayload::Plugin {
            plugin_id,
            cmd,
            feature,
        } => {
            let out = open_plugin(&app, &data, plugin_id, cmd, feature)?;
            data.record_recent(&hit);
            Ok(out)
        }
        HitPayload::Action { name } => {
            let msg = run_action(&app, &data, name)?;
            data.record_recent(&hit);
            Ok(InvokeOutcome::now(msg))
        }
        HitPayload::CopyText { text, .. } => {
            // 计算器结果：复制并收起窗口
            crate::host::copy_text(text).map_err(|e| e.to_string())?;
            windows::hide_main(&app);
            Ok(InvokeOutcome::now(format!("已复制 {text} 到剪贴板")))
        }
        HitPayload::Snippet { id: _, text } => {
            // 片段：复制后粘贴回唤起前的窗口；没有前台记录时只复制。
            crate::host::copy_text(text).map_err(|e| e.to_string())?;
            windows::hide_main(&app);
            match crate::guide::last().map(|w| w.hwnd) {
                Some(hwnd) => {
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(160));
                        let _ = crate::winclip::paste_to(hwnd);
                    });
                    Ok(InvokeOutcome::now("已粘贴片段"))
                }
                None => Ok(InvokeOutcome::now("已复制片段")),
            }
        }
        HitPayload::WebSearch { url } => {
            efto_core::apps::launch::shell_open(url, None).map_err(|e| e.to_string())?;
            data.record_usage(&hit.id);
            data.record_recent(&hit);
            windows::hide_main(&app);
            Ok(InvokeOutcome::now("已在浏览器中打开搜索"))
        }
        HitPayload::SystemAction { action } => {
            let action = efto_core::system::SystemAction::from_id(action)
                .ok_or_else(|| format!("未知系统命令: {action}"))?;
            // 先收起搜索框，确认框才不会被主窗口盖住
            windows::hide_main(&app);
            let msg = crate::system::run(action)?;
            Ok(InvokeOutcome::now(msg))
        }
        HitPayload::RunCommand { command } => {
            let settings = data.settings.lock().clone();
            if command.trim().is_empty() {
                return Err("命令不能为空".into());
            }
            if settings.command_run_mode == efto_core::config::CommandRunMode::Terminal {
                efto_core::exec::run_command_terminal(&settings, command)
                    .map_err(|e| e.to_string())?;
                windows::hide_main(&app);
                // 终端模式没有输出回传，但也发一条结果，让前端清掉"执行中"状态
                let _ = app.emit_to(
                    tauri::EventTarget::window(windows::MAIN_LABEL),
                    "command:result",
                    json!({ "ok": true, "terminal": true, "command": command }),
                );
                return Ok(InvokeOutcome::now("已在终端窗口中执行"));
            }
            let cancel = Arc::new(AtomicBool::new(false));
            *data.command_cancel.lock() = Some(cancel.clone());
            let shared = data.inner().clone();
            let handle = app.clone();
            let command = command.clone();
            // 执行可能持续到超时：必须离开 UI 线程，结果通过事件回填下拉列表
            std::thread::spawn(move || {
                let res = efto_core::exec::run_command(
                    &settings,
                    &command,
                    settings.command_timeout(),
                    Some(&cancel),
                );
                {
                    let mut slot = shared.command_cancel.lock();
                    let same = slot
                        .as_ref()
                        .map(|cur| Arc::ptr_eq(cur, &cancel))
                        .unwrap_or(false);
                    if same {
                        *slot = None;
                    }
                }
                let payload = match res {
                    Ok(ok) => json!({
                        "ok": true,
                        "command": command,
                        "shell": settings.command_shell.display(),
                        "code": ok.code,
                        "stdout": ok.stdout,
                        "stderr": ok.stderr,
                        "truncated": ok.truncated,
                    }),
                    Err(e) => json!({
                        "ok": false,
                        "command": command,
                        "shell": settings.command_shell.display(),
                        "error": e.to_string(),
                    }),
                };
                let _ = handle.emit_to(
                    tauri::EventTarget::window(windows::MAIN_LABEL),
                    "command:result",
                    payload,
                );
            });
            Ok(InvokeOutcome::now("执行中…"))
        }
    }
}

/// 取消正在执行的搜索框命令（Esc）。返回是否有命令被取消。
#[tauri::command]
pub fn cancel_command(data: State<'_, Shared>) -> bool {
    if let Some(c) = data.command_cancel.lock().as_ref() {
        c.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// 预览当前选中的结果（按住 Shift）。返回 false 表示该条目不支持预览，前端静默。
#[tauri::command]
pub async fn preview_show(hit: SearchHit, app: AppHandle) -> Result<bool, String> {
    let payload = tauri::async_runtime::spawn_blocking(move || crate::preview::build(&hit))
        .await
        .map_err(|e| format!("预览失败: {e}"))?;
    match payload {
        Some(p) => {
            // 图片/视频由 asset 协议直接伺服，scope 未配通配，必须逐个放行
            if p.kind == "image" || p.kind == "video" || p.kind == "pdf" {
                if let Some(path) = p.path.as_deref() {
                    let _ = app.asset_protocol_scope().allow_file(path);
                }
            }
            // 本地 html 在 iframe 里加载，相对引用的样式/图片也要放行：
            // 只放行文件本身会让页面缺图缺样式（与浏览器直接打开的效果不一致）
            if p.kind == "html" {
                if let Some(path) = p.path.as_deref() {
                    let _ = app.asset_protocol_scope().allow_file(path);
                    if let Some(dir) = std::path::Path::new(path).parent() {
                        let _ = app.asset_protocol_scope().allow_directory(dir, true);
                    }
                }
            }
            windows::show_preview(&app, p);
            Ok(true)
        }
        None => {
            windows::hide_preview(&app);
            Ok(false)
        }
    }
}

/// 预览页加载完成后主动来取最新内容：窗口是懒创建的，事件可能比监听先到。
#[tauri::command]
pub fn preview_ready() -> Option<crate::preview::PreviewPayload> {
    crate::preview::current()
}

/// 松开 Shift：预览窗口留着不关，只退出"按住预览"状态
/// （主窗口的失焦隐藏要恢复正常，否则预览一开就再也收不起搜索框）。
#[tauri::command]
pub fn preview_release() {
    windows::release_preview();
}

/// 预览窗口的关闭按钮：收起预览，并把焦点还给搜索框。
#[tauri::command]
pub fn preview_hide(app: AppHandle) {
    windows::hide_preview(&app);
    // 主窗口不知道用户已经在预览页点过关，通知它清掉 Shift/选中项的缓存
    let _ = app.emit_to(
        tauri::EventTarget::window(windows::MAIN_LABEL),
        "preview:closed",
        (),
    );
}

/// 预览窗口的打开按钮：交给系统默认程序打开文件 / 浏览器打开网址。
#[tauri::command]
pub fn preview_open() -> Result<(), String> {
    let payload = crate::preview::current().ok_or("没有可打开的预览内容")?;
    // 本地文件优先用 path（本地 html 同时有 path 与 file:// url）
    let target = payload
        .path
        .as_deref()
        .or(payload.url.as_deref())
        .ok_or("预览内容没有可打开的地址")?;
    efto_core::apps::launch::shell_open(target, None).map_err(|e| e.to_string())
}

/// 唤起结果。`wait_output` 只对"没有 UI 的后端插件"为 true：
/// 前端先等 `plugin:output`，超时没等到才收起搜索框。
#[derive(Debug, Clone, serde::Serialize)]
pub struct InvokeOutcome {
    pub message: String,
    pub wait_output: bool,
}

impl InvokeOutcome {
    fn now(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            wait_output: false,
        }
    }
}

/// 唤起插件：必要时启动后端 + 打开 UI 窗口。
fn open_plugin(
    app: &AppHandle,
    data: &Shared,
    plugin_id: &str,
    cmd: &str,
    feature: &str,
) -> Result<InvokeOutcome, String> {
    // 一次性把需要的信息取出来，避免在后续步骤里长时间持有注册表读锁
    let snapshot = {
        let registry = data.registry.read();
        let p = registry
            .get(plugin_id)
            .ok_or_else(|| format!("插件未安装或清单无效: {plugin_id}"))?;
        PluginSnapshot {
            root: p.root.clone(),
            version: p.manifest.version.clone(),
            has_backend: p.has_backend(),
            has_ui: p.has_ui(),
            main_entry: p.main_entry.clone(),
            ui_rel: p
                .ui_entry
                .as_ref()
                .and_then(|ui| ui.strip_prefix(&p.root).ok())
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| "index.html".into()),
            problems: p.problems.clone(),
        }
    };

    if !snapshot.has_backend && !snapshot.has_ui {
        return Err(format!(
            "插件 {plugin_id} 无可执行入口：{}",
            snapshot.problems.join("；")
        ));
    }

    if snapshot.has_backend {
        let entry = snapshot
            .main_entry
            .clone()
            .ok_or_else(|| format!("插件 {plugin_id} 缺少 main 入口"))?;
        let ctx = efto_jsrt::PluginContext {
            plugin_id: plugin_id.to_string(),
            root: snapshot.root.to_string_lossy().to_string(),
            version: snapshot.version.clone(),
            locale: crate::i18n::resolve(&data.settings.lock()).to_string(),
            theme: theme_of(),
            host_version: app.package_info().version.to_string(),
        };
        let mut runtime = data.runtime.lock();
        runtime
            .start(efto_jsrt::StartRequest { ctx, entry })
            .map_err(|e| format!("插件启动失败: {e}"))?;
        runtime
            .enter(plugin_id, cmd, serde_json::json!(feature))
            .map_err(|e| format!("插件唤起失败: {e}"))?;
    }

    if snapshot.has_ui {
        let title = data.plugin_name(plugin_id);
        windows::open_plugin_ui(
            app,
            plugin_id,
            &snapshot.ui_rel,
            &title,
            780.0,
            520.0,
        )?;
    }
    // 无 UI：不在这里收起。前端会等插件输出；确认没有输出（或没等）时才 hide。
    Ok(InvokeOutcome {
        message: format!("已唤起 {plugin_id}"),
        wait_output: snapshot.has_backend && !snapshot.has_ui,
    })
}

struct PluginSnapshot {
    root: std::path::PathBuf,
    version: String,
    has_backend: bool,
    has_ui: bool,
    main_entry: Option<std::path::PathBuf>,
    ui_rel: String,
    problems: Vec<String>,
}

fn theme_of() -> String {
    // 深色模式由 WebView2 的 prefers-color-scheme 决定；后端侧只提供提示值
    "light".into()
}

/// 单项热键的执行入口：热键回调可能在主线程，`run_action` 里有截图/平铺这类
/// 阻塞动作，统一挪到独立线程，失败只记日志（热键没有地方展示错误）。
pub(crate) fn run_tool_from_hotkey(app: &AppHandle, tool: &str) {
    let app = app.clone();
    let tool = tool.to_string();
    std::thread::spawn(move || {
        let data = app.state::<Shared>().inner().clone();
        if let Err(e) = run_action(&app, &data, &tool) {
            tracing::warn!("单项热键执行 {tool} 失败: {e}");
        }
    });
}

pub(crate) fn run_action(app: &AppHandle, data: &Shared, name: &str) -> Result<String, String> {
    match name {
        "settings" => {
            windows::show_settings(app);
            Ok("已打开设置".into())
        }
        "reindex" => {
            let report = data.rebuild();
            Ok(format!("索引已重建：{} 条应用", report.merged))
        }
        "open_plugin_dir" => {
            let dir = {
                let s = data.settings.lock();
                s.effective_plugin_dirs()
                    .first()
                    .cloned()
                    .unwrap_or_else(efto_core::fsutil::data_root)
            };
            if !dir.exists() {
                std::fs::create_dir_all(&dir).map_err(|e| format!("创建插件目录失败: {e}"))?;
            }
            efto_core::apps::launch::shell_open(&dir.to_string_lossy(), None)
                .map_err(|e| e.to_string())?;
            Ok(format!("已打开插件目录 {}", dir.display()))
        }
        "open_data_dir" => {
            let dir = efto_core::fsutil::data_root();
            efto_core::apps::launch::shell_open(&dir.to_string_lossy(), None)
                .map_err(|e| e.to_string())?;
            Ok(format!("已打开数据目录 {}", dir.display()))
        }
        "quit" => {
            let app = app.clone();
            std::thread::spawn(move || {
                app.state::<Shared>().flush_stores();
                app.state::<Shared>().runtime.lock().stop_all();
                app.exit(0);
            });
            Ok("正在退出".into())
        }
        "lock_screen" => {
            windows::hide_main(app);
            lock_workstation()?;
            Ok("已锁屏".into())
        }
        "clipboard" => {
            windows::show_clipboard(app);
            windows::hide_main(app);
            Ok("已打开剪贴板".into())
        }
        "tile_windows" => run_window_op(app, crate::tile::Op::Grid),
        "window_min_others" => run_window_op(app, crate::tile::Op::MinimizeOthers),
        "window_min_all" => run_window_op(app, crate::tile::Op::MinimizeAll),
        "window_half_left" => run_window_op(app, crate::tile::Op::HalfLeft),
        "window_half_right" => run_window_op(app, crate::tile::Op::HalfRight),
        "window_undo_tile" => run_window_op(app, crate::tile::Op::Undo),
        "shortcut_guide" => {
            windows::show_shortcuts(app);
            windows::hide_main(app);
            Ok("已打开快捷键指南".into())
        }
        "snippets" => {
            windows::show_settings(app);
            let _ = app.emit_to(
                tauri::EventTarget::window(windows::SETTINGS_LABEL),
                "settings:page",
                "snippets",
            );
            windows::hide_main(app);
            Ok("已打开片段设置".into())
        }
        "universal_actions" => {
            crate::universal::from_clipboard(app);
            windows::hide_main(app);
            Ok("已打开通用动作".into())
        }
        "file_buffer" => {
            windows::show_files(app);
            windows::hide_main(app);
            Ok("已打开文件缓冲".into())
        }
        "color_picker" => {
            windows::close_screenshot_overlays(app);
            let shot = capture_after_hiding_main(app)?;
            *data.shot.lock() = Some(shot.clone());
            if let Err(e) = windows::open_color_picker_overlay(app, &shot) {
                *data.shot.lock() = None;
                return Err(e);
            }
            Ok("移动光标取色，单击复制色值".into())
        }
        "sticky_note" => {
            let note = {
                let mut store = data.notes.lock();
                let note = store.create();
                store.save();
                note
            };
            windows::open_note(app, &note, true)?;
            windows::hide_main(app);
            Ok("已新建便签".into())
        }
        "command_hint" => Ok("输入要执行的命令".into()),
        "websearch_hint" => Ok("在触发词后加空格和关键词，例如 g rust".into()),
        "screenshot" => {
            windows::close_color_picker_overlay(app);
            // 上一次长截图还没收尾时先叫停，避免它把新的覆盖层截进结果
            if let Some(lc) = data.long_capture.lock().as_ref() {
                lc.request_stop();
            }
            let shot = capture_after_hiding_main(app)?;
            // 覆盖层页面会立刻调 screenshot_begin 取图，所以先存再开窗
            *data.shot.lock() = Some(shot.clone());
            windows::open_screenshot_overlay(app, &shot)?;
            // 截图期间主窗口必须一直收着：会话中万一又被显示出来就重新收起
            // （插件输出、单实例转发等都可能在会话里请求显示主窗口）。
            let app2 = app.clone();
            let _ = std::thread::Builder::new()
                .name("shot-guard".into())
                .spawn(move || {
                    for tick in 0..300 {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        if !windows::screenshot_session_active(&app2) {
                            break;
                        }
                        let Some(w) = app2.get_webview_window(windows::MAIN_LABEL) else {
                            break;
                        };
                        if w.is_visible().unwrap_or(false) {
                            tracing::warn!(
                                "截图会话中主窗口又显示了（{}ms 后），重新收起",
                                (tick + 1) * 200
                            );
                            windows::hide_main(&app2);
                        }
                    }
                });
            Ok("拖拽选择截图区域".into())
        }
        other => Err(format!("未知指令: {other}")),
    }
}

/// 收起搜索框并抓一张整屏图（截图与颜色提取器共用）。
fn capture_after_hiding_main(app: &AppHandle) -> Result<crate::screenshot::Shot, String> {
    // 先把搜索框收起来，否则它会被截进图里（拍进去就再也去不掉了）。
    //
    // 判据必须是**窗口服务器**，不是 `is_visible()`：后者在 `orderOut`
    // 之后立刻为假，但合成器还要再画一两帧——用户看到的正是"截图里
    // 带着启动器"。最多等 600ms，之后无条件继续（宁可拍到也不能不响应）。
    windows::hide_main(app);
    // `hide_main` 在主线程上同步把窗口挪出屏 + orderOut，随后等 AppKit 确认
    // 不可见（通常第一次就过）。**不能**按窗口服务器的在屏列表来等：
    // 实测有环境里 orderOut 之后服务器仍把窗口列在旧位置好几分钟，
    // 那条记录只是簿记滞后，画面早就不画了——照它等会一直等到超时。
    for _ in 0..60 {
        if !windows::main_visible(app) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // orderOut 到合成器真正重绘这块屏之间还有一帧间隙
    std::thread::sleep(std::time::Duration::from_millis(150));
    let shot = crate::screenshot::capture_virtual_screen()?;
    // 窗口服务器仍把主窗口画在画面上时（AppKit 已 orderOut、服务器表面状态
    // 卡住），普通显示器捕获会把启动器拍进底图。改从"主窗口之下"合成：
    // 这样拍不到它，代价是同一层之上的系统菜单栏也不在结果里。
    #[cfg(target_os = "macos")]
    let shot = if windows::main_on_screen(app) {
        tracing::warn!("主窗口已收起但窗口服务器仍画着：{}", windows::main_state(app));
        match windows::main_window_number(app)
            .map(crate::screenshot::capture_virtual_screen_below)
        {
            Some(Ok(alt)) => alt,
            Some(Err(e)) => {
                tracing::warn!("按下层合成重抓失败，退回普通捕获: {e}");
                shot
            }
            None => shot,
        }
    } else {
        shot
    };
    Ok(shot)
}

/// 窗口操作统一入口：先执行再隐藏搜索框。
///
/// 顺序不能反：hide_main 会把主窗口挪到屏幕外，之后
/// MonitorFromWindow 就找不到搜索框所在的那块屏了。
fn run_window_op(app: &AppHandle, op: crate::tile::Op) -> Result<String, String> {
    #[cfg(windows)]
    let main_hwnd = app
        .get_webview_window(windows::MAIN_LABEL)
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize);
    #[cfg(not(windows))]
    let main_hwnd = None;
    let report = crate::tile::run_op(main_hwnd, op, false)?;
    windows::hide_main(app);
    Ok(report.message)
}

#[cfg(windows)]
pub(crate) fn lock_workstation() -> Result<(), String> {
    use windows_sys::Win32::System::Shutdown::LockWorkStation;
    if unsafe { LockWorkStation() } == 0 {
        return Err("锁屏失败：系统拒绝了 LockWorkStation（需要交互式登录会话）".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn lock_workstation() -> Result<(), String> {
    // 显示器睡眠即锁定（"唤醒后立即需要密码"由系统设置保证）。pmset 是系统自带工具，
    // 固定参数无用户输入，直接 spawn（同 shell_open 的先例），不走 exec 白名单。
    std::process::Command::new("pmset")
        .arg("displaysleepnow")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("锁屏失败：pmset displaysleepnow：{e}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn lock_workstation() -> Result<(), String> {
    Err("锁屏仅支持 Windows / macOS".into())
}

#[tauri::command]
pub fn refresh_index(data: State<'_, Shared>) -> ScanReport {
    data.rebuild()
}

#[tauri::command]
pub fn index_status(data: State<'_, Shared>) -> serde_json::Value {
    let report = data.report.lock().clone();
    serde_json::json!({
        "apps": data.index.read().len(),
        "plugins": data.registry.read().len(),
        "report": report,
    })
}

#[tauri::command]
pub fn list_plugins(data: State<'_, Shared>) -> Vec<state::PluginView> {
    state::plugin_views(&data)
}

/// 设置某个插件命令的自定义触发词（空字符串 = 删除，恢复清单原词）。
///
/// 与原词并存：两个词都能搜到；命中后仍然按原始 `cmd` 唤醒插件，插件无感。
#[tauri::command]
pub fn set_plugin_cmd_alias(
    plugin: String,
    cmd: String,
    alias: String,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let alias = alias.trim().to_string();
    if alias.chars().any(char::is_whitespace) {
        return Err("触发词不能包含空格".into());
    }
    if alias.chars().count() > 24 {
        return Err("触发词最多 24 个字符".into());
    }
    let plugin = plugin.trim().to_string();
    // 先确认命令真实存在，避免手改 settings.json 留下永远匹配不上的死键
    let exists = data
        .registry
        .read()
        .get(&plugin)
        .map(|p| p.manifest.cmds.iter().any(|c| c.cmd == cmd))
        .unwrap_or(false);
    if !exists {
        return Err(format!("插件 {plugin} 里没有命令 {cmd}"));
    }
    let aliases = {
        let mut settings = data.settings.lock();
        let drop_plugin = {
            let entry = settings.cmd_aliases.entry(plugin.clone()).or_default();
            if alias.is_empty() {
                entry.remove(&cmd);
            } else {
                entry.insert(cmd.clone(), alias.clone());
            }
            entry.is_empty()
        };
        if drop_plugin {
            settings.cmd_aliases.remove(&plugin);
        }
        settings.save().map_err(|e| e.to_string())?;
        settings.cmd_aliases.clone()
    };
    data.registry.write().set_cmd_aliases(aliases);
    Ok(alias)
}

/// 启用 / 停用一个插件。
///
/// 停用写进设置里的停用名单：立即释放已加载的 isolate，并从搜索与唤起入口消失，
/// 但目录与清单保留（设置页要能重新启用）。启用后重新发现，下次唤起时按需加载。
#[tauri::command]
pub fn set_plugin_enabled(
    plugin: String,
    enabled: bool,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<Vec<state::PluginView>, String> {
    let plugin = plugin.trim().to_string();
    // 只接受已安装的插件，避免手改 settings.json 留下永远匹配不上的死条目
    let known = data
        .registry
        .read()
        .plugins()
        .iter()
        .any(|p| p.id().eq_ignore_ascii_case(&plugin));
    if !known {
        return Err(format!("插件未安装: {plugin}"));
    }
    {
        let mut settings = data.settings.lock();
        settings
            .disabled_plugins
            .retain(|id| !id.eq_ignore_ascii_case(&plugin));
        if !enabled {
            settings.disabled_plugins.push(plugin.clone());
        }
        settings.save().map_err(|e| e.to_string())?;
    }
    if !enabled {
        // 先收插件自己的窗口（此时 isolate 还活着，能收到 ui_closed 收尾）
        windows::close_plugin_ui(&app, &plugin, "");
    }
    data.reload_plugins();
    Ok(state::plugin_views(&data))
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    windows::show_settings(&app);
}

/// 导入插件包（目录或 `.zip`，包内可有多个插件），导入后立即重建注册表。
#[tauri::command]
pub fn import_plugin(
    path: String,
    data: State<'_, Shared>,
) -> Result<efto_core::plugins::ImportBatch, String> {
    let target_root = efto_core::fsutil::data_root().join("plugins");
    let report = efto_core::plugins::import_path(std::path::Path::new(path.trim()), &target_root)
        .map_err(|e| e.to_string())?;
    data.rebuild();
    Ok(report)
}

/// 打开系统选择框，返回插件来源路径（`.zip` 文件或插件目录）；取消返回 `None`。
///
/// rfd 是阻塞式对话框，放到阻塞线程池里跑，避免卡住 Tauri 的异步运行时。
#[tauri::command]
pub async fn pick_plugin_source(
    kind: String,
    window: tauri::WebviewWindow,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        let dialog = rfd::FileDialog::new().set_parent(&window);
        let picked = match kind.as_str() {
            "file" => dialog
                .set_title("选择插件 .zip 包")
                .add_filter("插件压缩包", &["zip"])
                .pick_file(),
            "dir" => dialog.set_title("选择插件目录").pick_folder(),
            _ => return Err(format!("未知的选择类型：{kind}")),
        };
        Ok(picked.map(|p| p.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| format!("文件选择框失败：{e}"))?
}

#[tauri::command]
pub async fn pick_index_dir(
    window: tauri::WebviewWindow,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("选择要加入索引的目录")
            .pick_folder()
            .map(|p| p.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("文件选择框失败：{e}"))
}

#[tauri::command]
pub fn open_plugin_dir(app: AppHandle, data: State<'_, Shared>) -> Result<String, String> {
    run_action(&app, &data, "open_plugin_dir")
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle, data: State<'_, Shared>) -> Result<String, String> {
    run_action(&app, &data, "open_data_dir")
}

// ---------- 截图 ----------

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShotInfo {
    /// 整屏 PNG 的 data URL，选区窗口用它当背景
    pub data_url: String,
    pub width: u32,
    pub height: u32,
    /// 覆盖层窗口在截图坐标系里的偏移与跨度（物理像素）。
    /// macOS 上窗口会被菜单栏下推、跨屏摆放也未必对齐图原点——
    /// 前端按这四个值换算选区：图内坐标 = 窗口坐标 × (跨度/窗口宽) + 偏移。
    pub off_x: i32,
    pub off_y: i32,
    pub view_w: u32,
    pub view_h: u32,
}

/// 选区窗口启动后取回被截的整屏图。`window` 是发起调用的覆盖层窗口（mac 上
/// 每屏一个），偏移/跨度按它在图内的实际位置换算。
#[tauri::command]
pub fn screenshot_begin(
    window: tauri::WebviewWindow,
    data: State<'_, Shared>,
) -> Result<ShotInfo, String> {
    let shot = data.shot.lock();
    let shot = shot.as_ref().ok_or("没有待裁剪的截图")?;
    let (off_x, off_y, view_w, view_h) = overlay_mapping(&window, shot)?;
    Ok(ShotInfo {
        data_url: crate::screenshot::data_url(&shot.to_image()?)?,
        width: shot.width,
        height: shot.height,
        off_x,
        off_y,
        view_w,
        view_h,
    })
}

/// 覆盖层窗口在截图坐标系里的偏移与跨度（图内物理像素）。
///
/// 图坐标 = 全局点 × 主屏 scale；窗口的物理坐标按**本窗** scale 记，
/// 混合缩放多屏下二者不同源，先换算到同一单位再作差。
fn overlay_mapping(
    window: &tauri::WebviewWindow,
    shot: &crate::screenshot::Shot,
) -> Result<(i32, i32, u32, u32), String> {
    // macOS：覆盖层是"每屏一窗"，窗口就是从 `displays()[idx]` 摆出来的，映射
    // 直接按显示器矩形算。**不读窗口几何**——tao 的 outer_position/inner_size
    // 是 async setter 的快照、scale_factor 在 1x 屏上还会报成主屏的 2.0，
    // "点击瞬间对、一会后偏移"就是读了过期几何（补偿量按旧位置算）。
    #[cfg(target_os = "macos")]
    if let Some(m) = display_mapping(window.label(), shot) {
        return Ok(m);
    }
    let pos = window
        .outer_position()
        .map_err(|e| format!("取截图窗口位置失败: {e}"))?;
    let size = window
        .inner_size()
        .map_err(|e| format!("取截图窗口尺寸失败: {e}"))?;
    #[cfg(target_os = "macos")]
    let k = crate::tile::ax::display_scale() / window.scale_factor().unwrap_or(1.0);
    #[cfg(not(target_os = "macos"))]
    let k = 1.0f64;
    Ok((
        (pos.x as f64 * k).round() as i32 - shot.origin_x,
        (pos.y as f64 * k).round() as i32 - shot.origin_y,
        ((size.width as f64 * k).round() as u32).max(1),
        ((size.height as f64 * k).round() as u32).max(1),
    ))
}

/// macOS 覆盖层窗口 → 图内偏移/跨度，按标签里的显示器下标查 `displays()`。
///
/// 图坐标口径是"点 × 主屏 scale"，与 [`crate::screenshot::capture_virtual_screen`]
/// 的合成槽位一致；窗口几何由 `open_screenshot_overlay` 直接对 NSWindow 设定，
/// 与之同源，因此这里是确定性映射而不是读回。
#[cfg(target_os = "macos")]
fn display_mapping(label: &str, shot: &crate::screenshot::Shot) -> Option<(i32, i32, u32, u32)> {
    let rest = label
        .strip_prefix(crate::windows::SCREENSHOT_LABEL)
        .or_else(|| label.strip_prefix(crate::windows::COLOR_PICKER_LABEL))?;
    let idx: usize = rest.strip_prefix('-')?.parse().ok()?;
    let displays = crate::tile::ax::displays();
    let (_, r) = displays.get(idx)?;
    let s = crate::tile::ax::display_scale();
    Some((
        (r.x as f64 * s).round() as i32 - shot.origin_x,
        (r.y as f64 * s).round() as i32 - shot.origin_y,
        ((r.width as f64 * s).round() as u32).max(1),
        ((r.height as f64 * s).round() as u32).max(1),
    ))
}

/// 覆盖层窗口落位后偏移会变（异步摆放、菜单栏约束重算都会挪窗口）——
/// 前端轮询/提交前刷新用的轻量版 [`screenshot_begin`]（不带整图）。
#[tauri::command]
pub fn screenshot_offset(
    window: tauri::WebviewWindow,
    data: State<'_, Shared>,
) -> Result<serde_json::Value, String> {
    let shot = data.shot.lock();
    let shot = shot.as_ref().ok_or("截图已失效，请重新开始")?;
    let (off_x, off_y, view_w, view_h) = overlay_mapping(&window, shot)?;
    Ok(serde_json::json!({
        "off_x": off_x, "off_y": off_y, "view_w": view_w, "view_h": view_h,
    }))
}

/// 完成截图。`x/y/w/h` 是相对该显示器的**物理像素**。
/// `overlay` 是前端标注层合成的 PNG data URL：有标注时整张图以它为准
/// （钉图的摆放位置仍按选区的 `x/y` 计算）。
#[tauri::command]
pub fn screenshot_finish(
    action: String,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    overlay: Option<String>,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let shot = data.shot.lock().clone().ok_or("截图已失效，请重新开始")?;
    let img = match overlay {
        Some(url) => crate::screenshot::overlay_png(&url)?,
        None => shot.crop(x, y, w, h)?,
    };
    let result = match action.as_str() {
        "copy" => {
            copy_image(&img)?;
            windows::close_screenshot_overlays(&app);
            Ok("已复制到剪贴板".into())
        }
        "save" => {
            let path = screenshot_path();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("创建截图目录失败: {e}"))?;
            }
            std::fs::write(&path, crate::screenshot::png_bytes(&img)?)
                .map_err(|e| format!("保存截图失败: {e}"))?;
            windows::close_screenshot_overlays(&app);
            Ok(format!("已保存到 {}", path.display()))
        }
        "pin" => {
            let id = pin_new_id();
            let dir = pins_dir();
            std::fs::create_dir_all(&dir).map_err(|e| format!("创建钉图目录失败: {e}"))?;
            std::fs::write(
                dir.join(format!("{id}.png")),
                crate::screenshot::png_bytes(&img)?,
            )
            .map_err(|e| format!("保存钉图失败: {e}"))?;
            windows::open_pin(
                &app,
                &id,
                img.width(),
                img.height(),
                shot.origin_x + x,
                shot.origin_y + y,
            )?;
            windows::close_screenshot_overlays(&app);
            Ok("已钉在屏幕上".into())
        }
        other => Err(format!("未知截图动作: {other}")),
    };
    // 成功就把整屏图释放掉（4K 下 33MB）；失败保留，方便用户重试
    if result.is_ok() {
        *data.shot.lock() = None;
    }
    result
}

#[tauri::command]
pub fn screenshot_cancel(app: AppHandle, data: State<'_, Shared>) -> Result<(), String> {
    *data.shot.lock() = None;
    windows::close_screenshot_overlays(&app);
    Ok(())
}

/// 颜色提取器：把色值写进剪贴板并关掉覆盖层。
#[tauri::command]
pub fn color_picker_finish(
    color: String,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let color = color.trim().to_string();
    if color.is_empty() {
        return Err("色值为空".into());
    }
    crate::host::copy_text(&color).map_err(|e| e.to_string())?;
    *data.shot.lock() = None;
    windows::close_color_picker_overlay(&app);
    Ok(format!("已复制 {color}"))
}

#[tauri::command]
pub fn color_picker_cancel(app: AppHandle, data: State<'_, Shared>) {
    *data.shot.lock() = None;
    windows::close_color_picker_overlay(&app);
}

/// 长截图：确认选区后覆盖层继续留在屏幕上（鼠标穿透），后台按固定选区连续抓帧，
/// 用相邻帧的重叠内容估计滚动位移、把底部新露出的行拼到结果上。
///
/// 交互：用户自己在目标窗口里滚动；停止滚动约 2.5 秒后自动结束并保存 + 复制；
/// 期间按 Esc 取消（不保存）。`x/y/w/h` 是相对虚拟桌面的物理像素。
#[tauri::command]
pub fn screenshot_long_start(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    window: tauri::WebviewWindow,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let shot = data.shot.lock().clone().ok_or("截图已失效，请重新开始")?;
    let x = x.clamp(0, shot.width as i32 - 1);
    let y = y.clamp(0, shot.height as i32 - 1);
    let w = w.clamp(1, shot.width - x as u32);
    let h = h.clamp(1, shot.height - y as u32);
    let (px, py) = (shot.origin_x + x, shot.origin_y + y);

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let rows = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    {
        let mut slot = data.long_capture.lock();
        if slot.is_some() {
            return Err("已有一个长截图在进行".into());
        }
        *slot = Some(crate::screenshot::LongCapture::new(stop.clone(), rows.clone()));
    }
    // 只保留发起长图的这一个覆盖层（画边框和提示），其余屏的先关掉；
    // 鼠标事件全部透传给底下的窗口，用户才能拖动滚动条；
    // 同时把自己从屏幕捕获里排除，否则边框和提示会进结果图。
    {
        let prefix = format!("{}-", windows::SCREENSHOT_LABEL);
        for (label, w) in app.webview_windows() {
            if (label == windows::SCREENSHOT_LABEL || label.starts_with(&prefix))
                && w.label() != window.label()
            {
                let _ = w.close();
            }
        }
    }
    let _ = window.set_ignore_cursor_events(true);
    windows::exclude_from_capture(&window);
    // 整屏图到这里就用完了，先释放（4K 下几十 MB）
    *data.shot.lock() = None;

    let shared = data.inner().clone();
    let app2 = app.clone();
    let exclude = windows::window_number(&window);
    if let Err(e) = std::thread::Builder::new()
        .name("efto-longshot".into())
        .spawn(move || run_long_capture(app2, shared, px, py, w, h, exclude, stop, rows))
    {
        *data.long_capture.lock() = None;
        let _ = window.set_ignore_cursor_events(false);
        return Err(format!("无法启动长截图线程: {e}"));
    }
    Ok("长截图已开始：滚动目标窗口，停止滚动 2.5 秒后自动完成".into())
}

/// 长截图实时进度，覆盖层用它把选区下边缘往下画。
#[derive(serde::Serialize)]
pub struct LongStatus {
    pub running: bool,
    pub rows: u32,
}

#[tauri::command]
pub fn screenshot_long_status(data: State<'_, Shared>) -> LongStatus {
    match data.long_capture.lock().as_ref() {
        Some(lc) => LongStatus {
            running: true,
            rows: lc.extra_rows(),
        },
        None => LongStatus {
            running: false,
            rows: 0,
        },
    }
}

/// 长截图后台循环：抓帧 → 估计位移 → 追加新行 → 收尾保存。
fn run_long_capture(
    app: AppHandle,
    data: Shared,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] exclude: Option<u32>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    rows_done: std::sync::Arc<std::sync::atomic::AtomicU32>,
) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    /// 抓帧间隔（毫秒）。太快没有意义，太慢会漏掉快速滚动的内容。
    const INTERVAL_MS: u64 = 180;
    /// 有过追加后，多久没新增内容就收尾。
    const IDLE_END_MS: u64 = 2500;
    /// 用户一直没滚动时的等待上限。
    const NEVER_SCROLL_END_MS: u64 = 30_000;
    /// 硬上限，防止忘记结束。
    const HARD_LIMIT_MS: u64 = 90_000;
    /// 结果总像素上限（约 120MB RGBA），超过就提前收尾。
    const MAX_PIXELS: u64 = 30_000_000;

    // 让覆盖层的鼠标穿透与"排除捕获"先生效，再抓第一帧
    std::thread::sleep(Duration::from_millis(120));
    let esc = crate::hotkey::bind_temporary(&app, "Escape", {
        let stop = stop.clone();
        move || stop.store(true, Ordering::SeqCst)
    });

    let row_bytes = width as usize * 4;
    let start = Instant::now();
    let mut rows: Vec<u8> = Vec::new();
    let mut last_frame: Option<Vec<u8>> = None;
    let mut extra_rows: u32 = 0;
    let mut last_append = Instant::now();

    loop {
        if stop.load(Ordering::SeqCst) || start.elapsed().as_millis() as u64 >= HARD_LIMIT_MS {
            break;
        }
        // mac 上覆盖层必须从帧里排除：`NSWindowSharingNone` 对显示器捕获无效，
        // 只能"在这个覆盖层窗口之下"合成——覆盖层在屏幕保护级，比菜单栏高，
        // 所以菜单栏仍在结果里。
        #[cfg(target_os = "macos")]
        let frame = match exclude {
            Some(id) => crate::screenshot::capture_region_below(x, y, width, height, id),
            None => crate::screenshot::capture_region(x, y, width, height),
        };
        #[cfg(not(target_os = "macos"))]
        let frame = crate::screenshot::capture_region(x, y, width, height);
        match frame {
            Ok(shot) => {
                match &last_frame {
                    Some(prev) => {
                        if let Some(d) =
                            crate::screenshot::scroll_delta(prev, &shot.rgba, width, height)
                        {
                            if d > 0 && d < height {
                                let from = (height - d) as usize * row_bytes;
                                rows.extend_from_slice(&shot.rgba[from..]);
                                extra_rows += d;
                                rows_done.store(extra_rows, Ordering::SeqCst);
                                last_append = Instant::now();
                            }
                        }
                    }
                    None => rows = shot.rgba.clone(),
                }
                last_frame = Some(shot.rgba);
                if (width as u64) * (height + extra_rows) as u64 > MAX_PIXELS {
                    tracing::info!("长截图达到尺寸上限，提前结束");
                    break;
                }
            }
            Err(e) => tracing::warn!("长截图抓帧失败: {e}"),
        }
        if extra_rows > 0 && last_append.elapsed().as_millis() as u64 >= IDLE_END_MS {
            break;
        }
        if extra_rows == 0 && start.elapsed().as_millis() as u64 >= NEVER_SCROLL_END_MS {
            break;
        }
        std::thread::sleep(Duration::from_millis(INTERVAL_MS));
    }

    if let Some(s) = esc {
        crate::hotkey::unbind_temporary(&app, s);
    }
    // 只清自己的会话：用户可能已经开了下一个长截图
    {
        let mut slot = data.long_capture.lock();
        let mine = slot
            .as_ref()
            .map(|cur| std::sync::Arc::ptr_eq(&cur.stop, &stop))
            .unwrap_or(false);
        if mine {
            *slot = None;
        }
    }
    windows::close_screenshot_overlays(&app);

    let cancelled = stop.load(Ordering::SeqCst);
    if cancelled || extra_rows == 0 {
        let why = if cancelled {
            "已按 Esc 取消"
        } else {
            "没有检测到滚动，未生成图片"
        };
        notify(&app, "长截图已取消", why);
        return;
    }
    let total_h = height + extra_rows;
    let Some(img) = image::RgbaImage::from_raw(width, total_h, rows) else {
        notify(&app, "长截图失败", "拼接结果尺寸无效");
        return;
    };
    let png = match crate::screenshot::png_bytes(&img) {
        Ok(v) => v,
        Err(e) => {
            notify(&app, "长截图失败", &e);
            return;
        }
    };
    // 首选图片目录；被拒时退到应用数据目录，并在通知里给出真实路径
    let primary = long_screenshot_path();
    let path = match write_file(&primary, &png) {
        Ok(()) => primary,
        Err(e) => {
            tracing::warn!("保存长截图到 {} 失败: {e}", primary.display());
            let fallback = efto_core::fsutil::data_root()
                .join("Screenshots")
                .join(primary.file_name().map(|n| n.to_os_string()).unwrap_or_default());
            match write_file(&fallback, &png) {
                Ok(()) => fallback,
                Err(e2) => {
                    notify(
                        &app,
                        "长截图失败",
                        &format!("保存失败：{e}；备用目录也失败：{e2}"),
                    );
                    return;
                }
            }
        }
    };
    let body = match copy_image_retry(&img) {
        Ok(()) => format!(
            "{} × {}，已复制到剪贴板 · {}",
            width,
            total_h,
            path.display()
        ),
        Err(e) => format!(
            "{} × {}，已保存（复制失败：{e}）· {}",
            width,
            total_h,
            path.display()
        ),
    };
    notify(&app, "长截图已保存", &body);
    tracing::info!("长截图完成: {}x{} -> {}", width, total_h, path.display());
}

/// 写文件（顺带建目录）；错误里带上路径，便于用户反馈"没权限"时定位。
fn write_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("创建目录 {} 失败：{e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

/// 剪贴板常被输入法/剪贴板管理工具短暂占用，失败时重试几次。
fn copy_image_retry(img: &image::RgbaImage) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match copy_image(img) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e;
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            }
        }
    }
    Err(last)
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
    {
        tracing::warn!("通知发送失败: {e}");
    }
}

fn copy_image(img: &image::RgbaImage) -> Result<(), String> {
    use std::borrow::Cow;
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用: {e}"))?;
    clipboard
        .set_image(arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: Cow::Borrowed(img.as_raw().as_slice()),
        })
        .map_err(|e| format!("写入图片到剪贴板失败: {e}"))
}

fn screenshot_path() -> std::path::PathBuf {
    let base = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(efto_core::fsutil::data_root);
    base.join("efto")
        .join("Screenshots")
        .join(format!(
            "Screenshot_{}.png",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ))
}

fn pins_dir() -> std::path::PathBuf {
    efto_core::fsutil::data_root().join("pins")
}

/// 长截图和普通截图放在同一个目录，前缀区分。
fn long_screenshot_path() -> std::path::PathBuf {
    let base = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(efto_core::fsutil::data_root);
    base.join("efto")
        .join("Screenshots")
        .join(format!(
            "LongShot_{}.png",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ))
}

fn pin_path(id: &str) -> std::path::PathBuf {
    pins_dir().join(format!("{id}.png"))
}

fn pin_new_id() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}")
}

// ---------- 钉图窗口 ----------

#[tauri::command]
pub fn pin_load(id: String) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = std::fs::read(pin_path(&id)).map_err(|e| format!("读取钉图失败: {e}"))?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub fn pin_drag(id: String, app: AppHandle) -> Result<(), String> {
    let w = app
        .get_webview_window(&windows::pin_label(&id))
        .ok_or("钉图窗口不存在")?;
    w.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pin_close(id: String, app: AppHandle) {
    windows::close_window(&app, &windows::pin_label(&id));
}

#[tauri::command]
pub fn note_load(id: String, data: State<'_, Shared>) -> Result<String, String> {
    data.notes
        .lock()
        .get(&id)
        .map(|n| n.text.clone())
        .ok_or_else(|| "便签不存在".into())
}

#[tauri::command]
pub fn note_save(id: String, text: String, app: AppHandle, data: State<'_, Shared>) {
    let geometry = app.get_webview_window(&windows::note_label(&id)).and_then(|w| {
        let pos = w.outer_position().ok()?;
        let size = w.inner_size().ok()?;
        Some((pos.x, pos.y, size.width, size.height))
    });
    let mut store = data.notes.lock();
    store.set_text(&id, text);
    if let Some((x, y, w, h)) = geometry {
        store.set_geometry(&id, x, y, w, h);
    }
    store.save();
}

#[tauri::command]
pub fn note_close(id: String, app: AppHandle, data: State<'_, Shared>) {
    {
        let mut store = data.notes.lock();
        if store.remove(&id) {
            store.save();
        }
    }
    windows::close_window(&app, &windows::note_label(&id));
}

#[tauri::command]
pub fn note_drag(id: String, app: AppHandle) -> Result<(), String> {
    let w = app
        .get_webview_window(&windows::note_label(&id))
        .ok_or("便签窗口不存在")?;
    w.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pin_copy(id: String) -> Result<(), String> {
    let img = image::open(pin_path(&id))
        .map_err(|e| format!("读取钉图失败: {e}"))?
        .to_rgba8();
    copy_image(&img)
}

// ---------- 搜索结果图标 ----------

/// 取"系统为这个文件分配的图标"，返回 `data:image/png;base64,...`；取不到返回 null。
#[tauri::command]
pub fn icon_for(path: String, data: State<'_, Shared>) -> Option<String> {
    data.icon_for(&path)
}

// ---------- 剪贴板历史 ----------

#[tauri::command]
pub fn clipboard_list(
    query: String,
    data: State<'_, Shared>,
) -> Vec<efto_core::clipboard::ClipItem> {
    data.clipboard.lock().search(&query)
}

/// 把一条历史写回系统剪贴板：文本、图片（解码为 RGBA）或文件列表。
fn write_clip_item(item: &efto_core::clipboard::ClipItem) -> Result<(), String> {
    use efto_core::clipboard::ClipKind;
    match item.kind {
        ClipKind::Text => crate::host::copy_text(&item.text)
            .map_err(|e| e.to_string())
            .map(|_| ()),
        ClipKind::Files => crate::winclip::write_files(&item.files),
        ClipKind::Image => {
            let path = item.image.as_deref().ok_or("图片条目缺少文件路径")?;
            let img = image::open(path)
                .map_err(|e| format!("读取图片失败: {e}"))?
                .to_rgba8();
            let (w, h) = img.dimensions();
            let data = arboard::ImageData {
                width: w as usize,
                height: h as usize,
                bytes: std::borrow::Cow::Owned(img.into_raw()),
            };
            arboard::Clipboard::new()
                .and_then(|mut c| c.set_image(data))
                .map_err(|e| format!("写入图片到剪贴板失败: {e}"))
        }
    }
}

fn clip_item(data: &Shared, id: &str) -> Option<efto_core::clipboard::ClipItem> {
    data.clipboard
        .lock()
        .items()
        .iter()
        .find(|i| i.id == id)
        .cloned()
}

#[tauri::command]
pub fn clipboard_copy(
    id: String,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let item = clip_item(&data, &id).ok_or("剪贴板条目已不存在")?;
    write_clip_item(&item)?;
    windows::hide_clipboard(&app);
    Ok("已复制".into())
}

/// 复制并粘贴回唤起前的窗口；没有记录前台窗口时退化为"只复制"。
#[tauri::command]
pub async fn clipboard_paste(
    id: String,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let item = clip_item(&data, &id).ok_or("剪贴板条目已不存在")?;
    write_clip_item(&item)?;
    windows::hide_clipboard(&app);
    let hwnd = crate::guide::last().map(|w| w.hwnd);
    tauri::async_runtime::spawn_blocking(move || match hwnd {
        Some(hwnd) => crate::winclip::paste_to(hwnd)
            .map(|_| "已粘贴到前台窗口".to_string())
            .map_err(|e| format!("{e}；内容已复制到剪贴板")),
        None => Ok("已复制（没有可粘贴的前台窗口）".to_string()),
    })
    .await
    .map_err(|e| format!("粘贴失败: {e}"))?
}

#[tauri::command]
pub fn clipboard_pin(id: String, data: State<'_, Shared>) -> Option<bool> {
    let mut store = data.clipboard.lock();
    let pinned = store.toggle_pin(&id);
    if pinned.is_some() {
        let _ = store.save();
    }
    pinned
}

/// 图片条目的缩略图（96px PNG data URL）；非图片或读取失败返回 null。
#[tauri::command]
pub fn clipboard_thumb(id: String, data: State<'_, Shared>) -> Option<String> {
    let image = clip_item(&data, &id)?.image?;
    data.clip_thumb(&image)
}

#[tauri::command]
pub fn clipboard_remove(id: String, data: State<'_, Shared>) -> bool {
    let mut store = data.clipboard.lock();
    let Some(removed) = store.remove(&id) else {
        return false;
    };
    crate::clipwatch::remove_image(&removed);
    let _ = store.save();
    true
}

#[tauri::command]
pub fn clipboard_clear(data: State<'_, Shared>) {
    let mut store = data.clipboard.lock();
    for item in store.clear() {
        crate::clipwatch::remove_image(&item);
    }
    let _ = store.save();
}

#[tauri::command]
pub fn hide_clipboard(app: AppHandle) {
    windows::hide_clipboard(&app);
}

// ---------- 片段（Snippets） ----------

#[tauri::command]
pub fn snippets_list(data: State<'_, Shared>) -> Vec<efto_core::snippets::Snippet> {
    data.snippets.lock().items().to_vec()
}

#[tauri::command]
pub fn snippets_save(
    items: Vec<efto_core::snippets::Snippet>,
    data: State<'_, Shared>,
) -> Result<usize, String> {
    let mut store = data.snippets.lock();
    store.replace_all(items)?;
    store.save().map_err(|e| e.to_string())?;
    Ok(store.len())
}

// ---------- 文件动作与文件缓冲 ----------

#[tauri::command]
pub fn file_action(
    path: String,
    action: String,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let target = std::path::Path::new(&path);
    if action == "copy_path" {
        crate::host::copy_text(&path).map_err(|e| e.to_string())?;
        return Ok("已复制路径".into());
    }
    if action == "add_buffer" {
        let added = data.buffer_add(&path);
        let count = data.file_buffer.lock().len();
        return Ok(if added {
            format!("已加入文件缓冲（{count}）")
        } else {
            format!("文件缓冲里已有该文件（{count}）")
        });
    }
    if !target.exists() {
        return Err(format!("文件不存在: {path}"));
    }
    match action.as_str() {
        "open" => {
            efto_core::apps::launch::shell_open(&path, None).map_err(|e| e.to_string())?;
            windows::hide_main(&app);
            Ok(format!("已打开 {path}"))
        }
        "open_with" => {
            efto_core::apps::launch::open_with(&path).map_err(|e| e.to_string())?;
            windows::hide_main(&app);
            Ok("已打开「打开方式」".into())
        }
        "reveal" => {
            efto_core::apps::launch::reveal_in_folder(&path).map_err(|e| e.to_string())?;
            windows::hide_main(&app);
            Ok("已在资源管理器中定位".into())
        }
        "copy_file" => {
            crate::winclip::write_files(&[path.clone()])?;
            windows::hide_main(&app);
            Ok("已复制文件".into())
        }
        "delete" => {
            crate::winclip::recycle(&[path.clone()])?;
            windows::hide_main(&app);
            Ok("已删除到回收站".into())
        }
        "remove_buffer" => {
            let mut buf = data.file_buffer.lock();
            buf.retain(|p| !p.eq_ignore_ascii_case(&path));
            Ok(format!("已从文件缓冲移除（{}）", buf.len()))
        }
        other => Err(format!("未知文件动作: {other}")),
    }
}

#[tauri::command]
pub fn file_buffer_list(data: State<'_, Shared>) -> Vec<String> {
    data.file_buffer.lock().clone()
}

#[tauri::command]
pub fn file_buffer_remove(path: String, data: State<'_, Shared>) -> usize {
    let mut buf = data.file_buffer.lock();
    buf.retain(|p| !p.eq_ignore_ascii_case(&path));
    buf.len()
}

#[tauri::command]
pub fn file_buffer_clear(data: State<'_, Shared>) -> usize {
    data.file_buffer.lock().clear();
    0
}

/// 文件缓冲的批量动作：复制到剪贴板 / 删除到回收站。
#[tauri::command]
pub fn file_buffer_action(action: String, app: AppHandle, data: State<'_, Shared>) -> Result<String, String> {
    let items = data.file_buffer.lock().clone();
    if items.is_empty() {
        return Err("文件缓冲是空的".into());
    }
    match action.as_str() {
        "copy" => {
            crate::winclip::write_files(&items)?;
            windows::hide_files(&app);
            Ok(format!("已复制 {} 个文件", items.len()))
        }
        "delete" => {
            crate::winclip::recycle(&items)?;
            data.file_buffer.lock().clear();
            Ok(format!("已删除 {} 个文件到回收站", items.len()))
        }
        other => Err(format!("未知文件缓冲动作: {other}")),
    }
}

/// 选择目标文件夹后复制/移动；`paths` 为空时处理整个文件缓冲。
#[tauri::command]
pub async fn file_transfer(
    paths: Vec<String>,
    moveit: bool,
    window: tauri::WebviewWindow,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let dest = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_parent(&window)
            .set_title(if moveit { "移动到文件夹" } else { "复制到文件夹" })
            .pick_folder()
    })
    .await
    .map_err(|e| format!("文件夹选择失败: {e}"))?
    .ok_or("已取消")?;
    let mut items = paths;
    if items.is_empty() {
        items = data.file_buffer.lock().clone();
    }
    if items.is_empty() {
        return Err("没有要处理的文件".into());
    }
    let dest = dest.to_string_lossy().to_string();
    tauri::async_runtime::spawn_blocking(move || transfer_all(&items, &dest, moveit))
        .await
        .map_err(|e| format!("操作失败: {e}"))?
}

fn transfer_all(paths: &[String], dest: &str, moveit: bool) -> Result<String, String> {
    let dest = std::path::Path::new(dest);
    let mut done = 0;
    let mut errors = Vec::new();
    for path in paths {
        let src = std::path::Path::new(path);
        let Some(name) = src.file_name() else {
            errors.push(format!("{path}: 无法获取文件名"));
            continue;
        };
        let target = unique_target(dest, name);
        let result = if moveit {
            move_one(src, &target)
        } else {
            copy_one(src, &target)
        };
        match result {
            Ok(()) => done += 1,
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    let verb = if moveit { "移动" } else { "复制" };
    if errors.is_empty() {
        Ok(format!("已{verb} {done} 个文件到 {}", dest.display()))
    } else {
        Err(format!(
            "已{verb} {done} 个，失败 {} 个：{}",
            errors.len(),
            errors.join("；")
        ))
    }
}

fn unique_target(dest: &std::path::Path, name: &std::ffi::OsStr) -> std::path::PathBuf {
    let mut target = dest.join(name);
    if !target.exists() {
        return target;
    }
    let stem = name.to_string_lossy();
    for n in 2..1000 {
        target = dest.join(format!("{stem} ({n})"));
        if !target.exists() {
            return target;
        }
    }
    target
}

fn move_one(src: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    if let Err(e) = std::fs::rename(src, target) {
        // 跨卷时 rename 会失败：退化为复制后删除
        copy_one(src, target).map_err(|copy_err| format!("{e}; {copy_err}"))?;
        if src.is_dir() {
            std::fs::remove_dir_all(src).map_err(|e| e.to_string())?;
        } else {
            std::fs::remove_file(src).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn copy_one(src: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    if src.is_dir() {
        std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_one(&entry.path(), &target.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, target).map(|_| ()).map_err(|e| e.to_string())
    }
}

// ---------- 通用动作 ----------

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActionsData {
    pub text: String,
    pub ids: Vec<&'static str>,
}

#[tauri::command]
pub fn actions_data(data: State<'_, Shared>) -> ActionsData {
    let text = data.selection.lock().clone().unwrap_or_default();
    let (translate, ai) = selection_plugins(&data);
    ActionsData {
        ids: crate::universal::action_ids(&text, translate, ai),
        text,
    }
}

/// 通用动作菜单是否给出"翻译 / AI"：模型已配置，且对应插件已安装、启用。
fn selection_plugins(data: &Shared) -> (bool, bool) {
    if data.settings.lock().models.is_empty() {
        return (false, false);
    }
    let registry = data.registry.read();
    (
        registry.get(crate::universal::TRANSLATE_PLUGIN).is_some(),
        registry.get(crate::universal::AI_PLUGIN).is_some(),
    )
}

/// 把选中文本交给插件：翻译走无 UI 后端（结果内联到主窗口），
/// AI 插件自行打开 UI 并预填文本。调用前复核模型与插件，避免菜单过期后静默失败。
fn run_selection_plugin(
    app: &AppHandle,
    data: &Shared,
    plugin_id: &str,
    cmd: &str,
    text: &str,
) -> Result<String, String> {
    if text.is_empty() {
        return Err("没有选中的文本".into());
    }
    if data.settings.lock().models.is_empty() {
        return Err("未配置模型：请在 设置 → 模型 中添加模型".into());
    }
    let out = open_plugin(app, data, plugin_id, cmd, text)?;
    windows::hide_actions(app);
    Ok(out.message)
}

#[tauri::command]
pub async fn actions_run(
    action: String,
    app: AppHandle,
    data: State<'_, Shared>,
) -> Result<String, String> {
    let text = data.selection.lock().clone().unwrap_or_default();
    let text = text.trim().to_string();
    match action.as_str() {
        "copy" => {
            if text.is_empty() {
                return Err("没有选中的文本".into());
            }
            crate::host::copy_text(&text).map_err(|e| e.to_string())?;
            Ok("已复制到剪贴板".into())
        }
        "paste" => {
            if text.is_empty() {
                return Err("没有选中的文本".into());
            }
            crate::host::copy_text(&text).map_err(|e| e.to_string())?;
            windows::hide_actions(&app);
            let hwnd = crate::guide::last().map(|w| w.hwnd);
            tauri::async_runtime::spawn_blocking(move || match hwnd {
                Some(hwnd) => crate::winclip::paste_to(hwnd)
                    .map(|_| "已粘贴到前台窗口".to_string())
                    .map_err(|e| format!("{e}；内容已复制到剪贴板")),
                None => Ok("已复制（没有可粘贴的前台窗口）".to_string()),
            })
            .await
            .map_err(|e| format!("粘贴失败: {e}"))?
        }
        "web" => {
            if text.is_empty() {
                return Err("没有选中的文本".into());
            }
            let encoded =
                percent_encoding::utf8_percent_encode(&text, percent_encoding::NON_ALPHANUMERIC);
            let url = format!("https://www.bing.com/search?q={encoded}");
            efto_core::apps::launch::shell_open(&url, None).map_err(|e| e.to_string())?;
            windows::hide_actions(&app);
            Ok("已用浏览器搜索".into())
        }
        "translate" => run_selection_plugin(
            &app,
            &data,
            crate::universal::TRANSLATE_PLUGIN,
            "翻译",
            &text,
        ),
        "ai" => run_selection_plugin(&app, &data, crate::universal::AI_PLUGIN, "ai", &text),
        "open" => {
            if text.is_empty() {
                return Err("没有可打开的内容".into());
            }
            let looks_url = text.starts_with("http://")
                || text.starts_with("https://")
                || text.starts_with("www.");
            if !looks_url && !std::path::Path::new(&text).exists() {
                return Err("内容既不是链接也不是已存在的路径".into());
            }
            efto_core::apps::launch::shell_open(&text, None).map_err(|e| e.to_string())?;
            windows::hide_actions(&app);
            Ok("已打开".into())
        }
        other => Err(format!("未知通用动作: {other}")),
    }
}

#[tauri::command]
pub fn hide_actions(app: AppHandle) {
    windows::hide_actions(&app);
}

#[tauri::command]
pub fn show_file_buffer(app: AppHandle) {
    windows::show_files(&app);
}

#[tauri::command]
pub fn hide_files(app: AppHandle) {
    windows::hide_files(&app);
}

/// 非 Windows 平台的文件检索体检（macOS：Spotlight / mdfind）。
///
/// 设置页据此把「Everything 文件检索」换成「文件检索（Spotlight）」，
/// 避免 mac 用户看到 es.exe 下载按钮以为功能坏了。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileSearchStatus {
    /// 检索通道：spotlight | none
    pub channel: String,
    /// 是否可用（mdfind 存在且有可检索目录）
    pub ready: bool,
    /// 实际参与检索的目录（默认 + 用户追加，绝对路径）。
    pub dirs: Vec<String>,
    /// 默认常用目录中存在的那些（前端据此把"默认"与"用户添加"分开显示）。
    pub default_dirs: Vec<String>,
    pub message: String,
}

#[tauri::command]
pub fn file_search_status(data: State<'_, Shared>) -> FileSearchStatus {
    let settings = data.settings.lock().clone();
    let dirs: Vec<String> = settings
        .effective_file_search_dirs()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect();
    let default_dirs: Vec<String> = efto_core::search::spotlight::default_search_dirs()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect();
    if cfg!(target_os = "macos") {
        let has_mdfind = efto_core::exec::lookup("mdfind").is_some();
        let (ready, message) = match (has_mdfind, dirs.is_empty()) {
            (false, _) => (false, "未找到 /usr/bin/mdfind，文件检索不可用".to_string()),
            (true, true) => (
                false,
                "没有可检索的目录（默认目录不存在且未添加任何目录）".to_string(),
            ),
            (true, false) => (true, "Spotlight 索引就绪（系统自带）".to_string()),
        };
        return FileSearchStatus {
            channel: "spotlight".into(),
            ready,
            dirs,
            default_dirs,
            message,
        };
    }
    FileSearchStatus {
        channel: "none".into(),
        ready: false,
        dirs,
        default_dirs,
        message: "当前平台的文件检索通道由 Everything 提供，见下方 Everything 区块".into(),
    }
}

/// 追加一个 Spotlight 检索目录（设置页「添加目录」走系统目录选择框后再调这里）。
///
/// 只接受**绝对路径且存在**的目录：mdfind 的 `-onlyin` 必须是绝对路径（相对路径
/// 会按当前工作目录解释），而且不存在的目录加进去只会让设置页显示一条死配置。
#[tauri::command]
pub fn add_file_search_dir(
    path: String,
    data: State<'_, Shared>,
) -> Result<FileSearchStatus, String> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err("只能添加绝对路径目录".into());
    }
    if !p.is_dir() {
        return Err(format!("不是有效目录：{}", p.display()));
    }
    {
        let mut s = data.settings.lock();
        if !s.file_search_dirs.contains(&p) {
            s.file_search_dirs.push(p);
            s.save().map_err(|e| format!("保存设置失败: {e}"))?;
        } else {
            return Err("该目录已在检索范围里".into());
        }
    }
    Ok(file_search_status(data))
}

/// 移除一个用户追加的检索目录（默认三个目录不受影响）。
#[tauri::command]
pub fn remove_file_search_dir(
    path: String,
    data: State<'_, Shared>,
) -> Result<FileSearchStatus, String> {
    let p = PathBuf::from(path.trim());
    {
        let mut s = data.settings.lock();
        let before = s.file_search_dirs.len();
        s.file_search_dirs.retain(|d| d != &p);
        if s.file_search_dirs.len() == before {
            return Err("该目录不在用户添加列表里（默认目录不能移除）".into());
        }
        s.save().map_err(|e| format!("保存设置失败: {e}"))?;
    }
    Ok(file_search_status(data))
}

/// Everything 环境体检：设置页据此显示"就绪 / 只缺 es.exe / 完全没装"。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EverythingStatus {
    /// ready | missing_es | mismatch
    pub state: String,
    pub es_path: Option<String>,
    pub everything_installed: bool,
    pub message: String,
    pub download_es_url: String,
    pub download_everything_url: String,
}

const ES_DOWNLOAD_URL: &str = "https://www.voidtools.com/es.zip";
const EVERYTHING_DOWNLOAD_URL: &str = "https://www.voidtools.com/downloads/";

#[tauri::command]
pub fn everything_status(data: State<'_, Shared>) -> EverythingStatus {
    let hint = data.settings.lock().everything_es_path.clone();
    everything_state(hint.as_deref().map(std::path::Path::new))
}

/// Everything 体检的判定本体。设置页与 CLI 的 `capabilities` 共用，避免两套结论。
pub(crate) fn everything_state(hint: Option<&std::path::Path>) -> EverythingStatus {
    use efto_core::exec;
    let installed = exec::everything_installed();
    let (state, es_path, message) = match exec::lookup("es") {
        None => ("mismatch", None, "未配置 es 白名单，文件检索不可用".to_string()),
        Some(a) => match exec::resolve_verified(a, hint) {
            Ok(p) => (
                "ready",
                Some(p.display().to_string()),
                "Everything 命令行检索就绪".to_string(),
            ),
            Err(exec::ExecError::DigestMismatch { path, .. }) => (
                "mismatch",
                Some(path),
                "找到 es.exe，但版本校验失败（文件可能被替换），已拒绝执行".to_string(),
            ),
            Err(_) if installed => (
                "missing_es",
                None,
                "已安装 Everything，但缺少命令行工具 es.exe（下载后放到数据目录或 PATH 即可）"
                    .to_string(),
            ),
            Err(_) => (
                "missing_es",
                None,
                "未检测到 Everything，文件检索不可用；安装后可在设置里重新检测".to_string(),
            ),
        },
    };
    EverythingStatus {
        state: state.into(),
        es_path,
        everything_installed: installed,
        message,
        download_es_url: ES_DOWNLOAD_URL.into(),
        download_everything_url: EVERYTHING_DOWNLOAD_URL.into(),
    }
}

/// 下载官方 es.zip、校验 sha256 后安装到数据目录。
/// 走阻塞线程池：下载可能持续数秒，不能卡住主线程。
#[tauri::command]
pub async fn install_es() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let a = efto_core::exec::lookup("es").ok_or("未配置 es 白名单")?;
        let p = efto_core::exec::fetch_and_install(a).map_err(|e| e.to_string())?;
        Ok(p.display().to_string())
    })
    .await
    .map_err(|e| format!("安装任务失败: {e}"))?
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    efto_core::apps::launch::shell_open(&url, None).map_err(|e| e.to_string())
}

/// 平台权限状态（macOS：辅助功能 / 屏幕录制）。设置页"权限"区块与缺权限报错都读它。
#[tauri::command]
pub fn permission_status() -> serde_json::Value {
    crate::permissions::status()
}

/// 请求屏幕录制权限（触发系统引导）；辅助功能只能去系统设置手动开。
#[tauri::command]
pub fn request_permission(kind: String) -> Result<String, String> {
    match kind.as_str() {
        "screen_recording" => {
            crate::permissions::request_screen_recording();
            Ok(format!(
                "已请求屏幕录制权限：当前状态 {}",
                if crate::permissions::screen_recording_allowed() {
                    "已授权"
                } else {
                    "未授权（请在系统设置中勾选 efto）"
                }
            ))
        }
        "accessibility" => Err(
            "辅助功能权限需在 系统设置 → 隐私与安全性 → 辅助功能 中手动勾选 efto".into(),
        ),
        other => Err(format!("未知权限类型: {other}")),
    }
}

/// 打开系统设置的权限页（macOS）。kind: accessibility / screen_recording。
#[tauri::command]
pub fn open_permission_settings(kind: String) -> Result<String, String> {
    let pane = match kind.as_str() {
        "accessibility" => "com.apple.preference.security?Privacy_Accessibility",
        "screen_recording" => "com.apple.preference.security?Privacy_ScreenCapture",
        other => return Err(format!("未知权限类型: {other}")),
    };
    let uri = efto_core::search::sys_settings::pane_uri(pane)
        .ok_or_else(|| format!("设置页未收录: {pane}"))?;
    efto_core::apps::launch::shell_open(&uri, None).map_err(|e| e.to_string())?;
    Ok("已打开系统设置".into())
}

/// 前端平台适配信息：shell 下拉、工具清单、功能开关都以宿主为单一事实源
/// （`config::KNOWN_TOOLS` / `CommandShell::available_on_platform` / `BuiltinAction`），
/// 前端只按这份清单展示或隐藏，不自己猜平台。
#[tauri::command]
pub fn platform_info() -> serde_json::Value {
    use efto_core::config::CommandShell;
    use efto_core::search::BuiltinAction;
    let shells: Vec<serde_json::Value> = [
        CommandShell::Powershell,
        CommandShell::Pwsh,
        CommandShell::Cmd,
        CommandShell::GitBash,
        CommandShell::Wsl,
        CommandShell::Zsh,
        CommandShell::Bash,
        CommandShell::Fish,
        CommandShell::Custom,
    ]
    .iter()
    .filter(|s| s.available_on_platform())
    .map(|s| serde_json::to_value(s).unwrap_or_default())
    .collect();
    serde_json::json!({
        "platform": if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        },
        "default_shell": serde_json::to_value(CommandShell::default()).unwrap_or_default(),
        "shells": shells,
        "tools": efto_core::config::KNOWN_TOOLS,
        "features": {
            "screenshot": BuiltinAction::Screenshot.supported_on_platform(),
            "tile_windows": BuiltinAction::TileWindows.supported_on_platform(),
            "shortcut_guide": BuiltinAction::ShortcutGuide.supported_on_platform(),
            "universal_actions": BuiltinAction::UniversalActions.supported_on_platform(),
            "file_buffer": BuiltinAction::FileBuffer.supported_on_platform(),
            // 终端模式要接 Terminal.app / 新建控制台窗口，非 Windows 是 v1.5
            "terminal_mode": cfg!(windows),
        }
    })
}

#[tauri::command]
pub fn get_settings(data: State<'_, Shared>) -> Settings {
    data.settings.lock().clone()
}

/// 当前主题模式。前端页面启动时用它对齐磁盘设置（本地缓存只负责避免首帧闪白）。
#[tauri::command]
pub fn theme_mode(data: State<'_, Shared>) -> efto_core::config::ThemeMode {
    data.settings.lock().theme
}

/// 解析后的界面语言；前端 i18n 用它决定加载哪个词典。
#[tauri::command]
pub fn ui_language(data: State<'_, Shared>) -> Value {
    let s = data.settings.lock();
    json!({
        "language": crate::i18n::resolve(&s),
        "setting": s.language,
    })
}

#[derive(Debug, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub hotkey: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default)]
    pub refresh_interval_secs: Option<u64>,
    #[serde(default)]
    pub enable_registry: Option<bool>,
    #[serde(default)]
    pub enable_dir_scan: Option<bool>,
    /// 是否把 Chrome / Edge 收藏夹纳入搜索。
    #[serde(default)]
    pub enable_bookmarks: Option<bool>,
    /// 界面主题：system / dark / light。
    #[serde(default)]
    pub theme: Option<efto_core::config::ThemeMode>,
    #[serde(default)]
    pub hide_on_blur: Option<bool>,
    #[serde(default)]
    pub always_on_top: Option<bool>,
    #[serde(default)]
    pub extra_dirs: Option<Vec<std::path::PathBuf>>,
    #[serde(default)]
    pub plugin_dirs: Option<Vec<std::path::PathBuf>>,
    #[serde(default)]
    pub ignore_globs: Option<Vec<String>>,
    /// 默认面板第二行展示哪些常用工具（未知 id 会被丢弃）。
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Node.js 解释器路径；空字符串表示清除配置。
    #[serde(default)]
    pub node_path: Option<String>,
    /// Python 解释器路径；空字符串表示清除配置。
    #[serde(default)]
    pub python_path: Option<String>,
    /// 模型列表（OpenAI 兼容）。只在这个字段出现时整体替换。
    #[serde(default)]
    pub models: Option<Vec<efto_core::config::ModelConfig>>,
    /// 默认模型 id；空字符串表示交给后端选第一个。
    #[serde(default)]
    pub default_model: Option<String>,
    /// 是否允许在搜索框里用前缀执行命令。
    #[serde(default)]
    pub command_enabled: Option<bool>,
    /// 命令前缀；非法（含字母数字/空白）会直接报错。
    #[serde(default)]
    pub command_prefix: Option<String>,
    #[serde(default)]
    pub command_shell: Option<efto_core::config::CommandShell>,
    /// 自定义 Shell 路径；空字符串表示清除（走自动发现）。
    #[serde(default)]
    pub command_shell_path: Option<String>,
    #[serde(default)]
    pub command_wsl_distro: Option<String>,
    #[serde(default)]
    pub command_custom_args: Option<String>,
    #[serde(default)]
    pub command_timeout_secs: Option<u64>,
    #[serde(default)]
    pub command_run_mode: Option<efto_core::config::CommandRunMode>,
    #[serde(default)]
    pub language: Option<efto_core::config::Language>,
    /// 通用动作总开关。
    #[serde(default)]
    pub universal_actions_enabled: Option<bool>,
    /// 通用动作热键；与唤起热键相同时后端会自动换一个可用键位。
    #[serde(default)]
    pub universal_actions_hotkey: Option<String>,
    /// 自定义网页搜索引擎（整体替换）。
    #[serde(default)]
    pub web_searches: Option<Vec<efto_core::websearch::WebSearch>>,
    /// 本地无结果时是否兜底到网页搜索。
    #[serde(default)]
    pub web_search_fallback: Option<bool>,
    /// 兜底使用的引擎关键词。
    #[serde(default)]
    pub web_search_engine: Option<String>,
    /// 内置工具的独立热键（整体替换）。
    #[serde(default)]
    pub tool_hotkeys: Option<std::collections::BTreeMap<String, String>>,
}

#[tauri::command]
pub fn set_settings(patch: SettingsPatch, app: AppHandle, data: State<'_, Shared>) -> Result<String, String> {
    let (hotkey_changed, theme_changed, language_changed) = {
        let mut s = data.settings.lock();
        let before = s.hotkey.clone();
        let before_theme = s.theme;
        let before_language = s.language;
        let before_ua = (
            s.universal_actions_enabled,
            s.universal_actions_hotkey.clone(),
        );
        let before_tool_hk = s.tool_hotkeys.clone();
        if let Some(v) = patch.hotkey {
            s.hotkey = v;
        }
        if let Some(v) = patch.max_results {
            s.max_results = v.clamp(1, 50);
        }
        if let Some(v) = patch.refresh_interval_secs {
            s.refresh_interval_secs = v;
        }
        if let Some(v) = patch.enable_registry {
            s.enable_registry = v;
        }
        if let Some(v) = patch.enable_dir_scan {
            s.enable_dir_scan = v;
        }
        if let Some(v) = patch.enable_bookmarks {
            s.enable_bookmarks = v;
        }
        if let Some(v) = patch.theme {
            s.theme = v;
        }
        if let Some(v) = patch.hide_on_blur {
            s.hide_on_blur = v;
        }
        if let Some(v) = patch.always_on_top {
            s.always_on_top = v;
        }
        if let Some(v) = patch.extra_dirs {
            s.extra_dirs = v;
        }
        if let Some(v) = patch.plugin_dirs {
            s.plugin_dirs = v;
        }
        if let Some(v) = patch.ignore_globs {
            s.ignore_globs = v;
        }
        if let Some(v) = patch.tools {
            s.tools = efto_core::config::normalize_tools(v);
        }
        if let Some(v) = patch.node_path {
            s.node_path = blank_to_none(v);
        }
        if let Some(v) = patch.python_path {
            s.python_path = blank_to_none(v);
        }
        if let Some(models) = patch.models {
            s.models = efto_core::config::Settings::validate_models(models)?;
        }
        if let Some(v) = patch.default_model {
            s.default_model = blank_to_none(v);
        }
        if let Some(v) = patch.command_enabled {
            s.command_enabled = v;
        }
        if let Some(v) = patch.command_prefix {
            s.command_prefix = efto_core::config::Settings::validate_command_prefix(&v)?;
        }
        if let Some(v) = patch.command_shell {
            s.command_shell = v;
        }
        if let Some(v) = patch.command_shell_path {
            s.command_shell_path = blank_to_none(v);
        }
        if let Some(v) = patch.command_wsl_distro {
            s.command_wsl_distro = blank_to_none(v);
        }
        if let Some(v) = patch.command_custom_args {
            s.command_custom_args = v;
        }
        if let Some(v) = patch.command_timeout_secs {
            s.command_timeout_secs = v;
        }
        if let Some(v) = patch.command_run_mode {
            s.command_run_mode = v;
        }
        if let Some(v) = patch.language {
            s.language = v;
        }
        if let Some(v) = patch.universal_actions_enabled {
            s.universal_actions_enabled = v;
        }
        if let Some(v) = patch.universal_actions_hotkey {
            s.universal_actions_hotkey = v;
        }
        if let Some(v) = patch.web_searches {
            s.web_searches = v;
        }
        if let Some(v) = patch.web_search_fallback {
            s.web_search_fallback = v;
        }
        if let Some(v) = patch.web_search_engine {
            s.web_search_engine = v;
        }
        if let Some(v) = patch.tool_hotkeys {
            s.tool_hotkeys = v;
            // 用户显式保存过（哪怕是空表），以后不再补默认值
            s.tool_hotkeys_seeded = true;
        }
        s.normalize_models();
        s.normalize_command();
        s.normalize_universal_actions();
        s.normalize_web_searches();
        s.normalize_tool_hotkeys();
        s.save().map_err(|e| e.to_string())?;
        let ua_changed = before_ua
            != (
                s.universal_actions_enabled,
                s.universal_actions_hotkey.clone(),
            );
        (
            s.hotkey != before || ua_changed || s.tool_hotkeys != before_tool_hk,
            s.theme != before_theme,
            s.language != before_language,
        )
    };
    if hotkey_changed {
        crate::hotkey::rebind(&app).map_err(|e| e.to_string())?;
    }
    if theme_changed {
        let theme = data.settings.lock().theme;
        // 原生标题栏跟着变；各页面收到广播后用同一模式重画内容区
        crate::windows::apply_theme(&app, theme);
        let _ = app.emit("settings:theme", theme);
    }
    if language_changed {
        // 托盘菜单文本无法原地修改，只能重建；窗口标题同步，页面收到广播后重载。
        let lang = crate::i18n::resolve(&data.settings.lock());
        crate::tray::apply_language(&app, lang);
        crate::windows::apply_language(&app, lang);
        let _ = app.emit("settings:language", lang);
    }
    Ok("设置已保存".into())
}

fn blank_to_none(v: String) -> Option<String> {
    let t = v.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// 运行时状态：探测设置里保存的 node / python 解释器是否真的能跑。
#[tauri::command]
pub fn runtime_status(data: State<'_, Shared>) -> Value {
    let mut s = data.settings.lock();
    // 用户在本次会话里把路径清空后重新打开本页，也补一次自动检测
    if crate::state::autofill_runtimes(&mut s) {
        if let Err(e) = s.save() {
            tracing::warn!("自动检测运行时后保存设置失败: {e}");
        }
    }
    json!({
        "node": probe_saved(efto_core::exec::ScriptRuntime::Node, s.node_path.as_deref()),
        "python": probe_saved(efto_core::exec::ScriptRuntime::Python, s.python_path.as_deref()),
    })
}

/// 设置页“检测”按钮：探测一个尚未保存的路径。
#[tauri::command]
pub fn runtime_check(runtime: String, path: String) -> Result<Value, String> {
    let rt = efto_core::exec::script_runtime(&runtime)
        .ok_or_else(|| format!("不支持的运行时 {runtime:?}：只支持 node / python"))?;
    Ok(probe_saved(rt, Some(path.as_str())))
}

/// 帮助文档正文：编译期嵌入，不依赖安装目录里的文件。
#[tauri::command]
pub fn help_document(data: State<'_, Shared>) -> Value {
    let zh = crate::i18n::resolve(&data.settings.lock()) == "zh-CN";
    if zh {
        json!({
            "language": "zh-CN",
            "title": "efto 使用帮助",
            "markdown": HELP_ZH_CN,
        })
    } else {
        json!({
            "language": "en-US",
            "title": "efto Help",
            "markdown": HELP_EN_US,
        })
    }
}

#[tauri::command]
pub fn hide_help(app: AppHandle) {
    windows::hide_help(&app);
}

/// 快捷键指南首屏：识别唤起前的应用，未收录时回退到 Windows 通用数据。
#[tauri::command]
pub fn shortcut_guide_data() -> Value {
    // 执行时复核：记录之后窗口可能已经关掉，不能拿失效的 exe 硬套
    let last = crate::guide::last().filter(|w| crate::winproc::info(w.hwnd).is_some());
    let title = last.as_ref().map(|w| w.title.clone()).unwrap_or_default();
    let exe = last
        .as_ref()
        .map(|w| w.process.clone())
        .filter(|s| !s.trim().is_empty());
    let resolved = efto_core::shortcuts::resolve(exe.as_deref());
    json!({
        "exe": resolved.exe,
        "window_title": title,
        "name": resolved.name,
        "id": resolved.id,
        "fallback": resolved.fallback,
        "set": resolved.set,
        "apps": efto_core::shortcuts::apps(),
    })
}

/// 下拉切换应用：按内置 id 取数据（仍会合并用户覆盖）。
#[tauri::command]
pub fn shortcut_guide_lookup(id: String) -> Result<Value, String> {
    let set = efto_core::shortcuts::by_id(&id)
        .ok_or_else(|| format!("未知的快捷键应用: {id}"))?;
    Ok(json!({
        "id": id,
        "name": set.app.name,
        "exe": set.app.exe.first().cloned().unwrap_or_default(),
        "fallback": false,
        "set": set,
    }))
}

#[tauri::command]
pub fn hide_shortcuts(app: AppHandle) {
    windows::hide_shortcuts(&app);
}

const HELP_ZH_CN: &str = include_str!("../../../docs/help.zh-CN.md");
const HELP_EN_US: &str = include_str!("../../../docs/help.en-US.md");

/// 设置页“检测”按钮（命令分组）：真跑一条 `echo`，确认 Shell 能启动。
#[tauri::command]
pub fn command_check(
    shell: efto_core::config::CommandShell,
    path: String,
    distro: String,
    custom_args: String,
) -> Result<Value, String> {
    let (program, sample) = efto_core::exec::probe_command_shell(
        shell,
        Some(path.as_str()),
        Some(distro.as_str()),
        &custom_args,
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "path": program, "sample": sample }))
}

/// 设置页“测试”按钮：用尚未保存的表单值发一次最小请求。
/// 走阻塞线程池：网络请求可能持续数秒，不能卡住主线程。
#[tauri::command]
pub async fn model_check(model: efto_core::config::ModelConfig) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || crate::ai::probe(&model))
        .await
        .map_err(|e| format!("检测任务失败: {e}"))?
}

fn probe_saved(rt: efto_core::exec::ScriptRuntime, path: Option<&str>) -> Value {
    let path = path.map(str::trim).filter(|p| !p.is_empty());
    let Some(p) = path else {
        return json!({
            "runtime": rt.key(), "configured": false, "ok": false,
            "path": "", "version": "", "error": "未配置",
        });
    };
    match efto_core::exec::probe_interpreter(p) {
        Ok(version) => json!({
            "runtime": rt.key(), "configured": true, "ok": true,
            "path": p, "version": version, "error": "",
        }),
        Err(e) => json!({
            "runtime": rt.key(), "configured": true, "ok": false,
            "path": p, "version": "", "error": e.to_string(),
        }),
    }
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    windows::show_main(&app);
}

#[tauri::command]
pub fn hide_main(app: AppHandle) {
    windows::hide_main(&app);
}

#[tauri::command]
pub fn quit(app: AppHandle, data: State<'_, Shared>) {
    data.flush_stores();
    data.runtime.lock().stop_all();
    app.exit(0);
}

// 插件 UI 页面调用的命令在 ui.rs（plugin_* 系列）。

/// 给测试与文档用的固定值校验：确保 LaunchTarget 的序列化形状稳定。
#[cfg(test)]
mod tests {
    use super::*;
    use efto_core::apps::model::LaunchTarget;

    #[test]
    fn settings_patch_partial_deserialize() {
        let p: SettingsPatch = serde_json::from_str(r#"{"hotkey":"Ctrl+Alt+K"}"#).unwrap();
        assert_eq!(p.hotkey.as_deref(), Some("Ctrl+Alt+K"));
        assert!(p.max_results.is_none());
    }

    /// platform_info 是前端展示的单一事实源：tools 必须等于平台 KNOWN_TOOLS，
    /// shells 必须只含当前平台可用的 shell，default_shell 必须在 shells 里。
    #[test]
    fn platform_info_is_consistent_with_this_platform() {
        let v = platform_info();
        let tools: Vec<String> = v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert_eq!(tools, efto_core::config::KNOWN_TOOLS);
        let shells: Vec<String> = v["shells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        let default_shell = v["default_shell"].as_str().unwrap();
        assert!(shells.iter().any(|s| s == default_shell), "{default_shell} 不在 {shells:?}");
        // 平台名字只能是三种之一
        assert!(["windows", "macos", "linux"].contains(&v["platform"].as_str().unwrap()));
    }

    #[test]
    fn search_hit_roundtrip_from_rust_side() {
        let hit = SearchHit::new(
            efto_core::ItemKind::App,
            "c:\\a.exe",
            "A",
            1.0,
            efto_core::search::HitPayload::Launch {
                target: LaunchTarget::exe("C:\\a.exe"),
            },
        );
        let json = serde_json::to_string(&hit).unwrap();
        let back: SearchHit = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "c:\\a.exe");
        assert_eq!(back.title, "A");
    }

    #[test]
    fn ui_plugin_id_requires_prefix() {
        // 只测纯函数逻辑，不依赖真实窗口
        let strip = |label: &str| {
            label
                .strip_prefix("plugin-ui-")
                .map(|s| efto_core::fsutil::sanitize_key(s))
                .ok_or_else(|| "该窗口不是插件 UI 窗口".to_string())
        };
        assert_eq!(strip("plugin-ui-demo").unwrap(), "demo");
        assert_eq!(strip("main").unwrap_err(), "该窗口不是插件 UI 窗口");
        assert!(strip("plugin-ui-").is_err() || strip("plugin-ui-").unwrap().is_empty());
    }
}
