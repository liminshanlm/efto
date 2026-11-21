//! 窗口管理：搜索主窗口 + 插件 UI 窗口。
//!
//! Windows 上 wry 要求窗口操作在主线程执行，因此这里所有变更都通过
//! `run_on_main_thread` 投递；调用方不等待结果（避免插件线程与主线程互相阻塞）。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, EventTarget, Manager, WebviewUrl, WebviewWindowBuilder};

use efto_core::config::ThemeMode;

use crate::preview::PreviewPayload;
use crate::state::Shared;

pub const MAIN_LABEL: &str = "main";
/// 设置窗口（tauri.conf.json 里常驻隐藏，打开时 show）。
pub const SETTINGS_LABEL: &str = "settings";
/// 剪贴板历史窗口（同样常驻隐藏）。
pub const CLIPBOARD_LABEL: &str = "clipboard";
/// 帮助文档窗口（常驻隐藏）。
pub const HELP_LABEL: &str = "help";
/// 快捷键指南窗口（常驻隐藏）。
pub const SHORTCUTS_LABEL: &str = "shortcuts";
/// 通用动作菜单窗口。
pub const ACTIONS_LABEL: &str = "actions";
/// 文件缓冲窗口。
pub const FILES_LABEL: &str = "files";
/// 截图选区覆盖层（运行时创建，用完即关）。
pub const SCREENSHOT_LABEL: &str = "screenshot";

/// 钉图窗口的 label 前缀，后接 pin id。
pub const PIN_PREFIX: &str = "pin-";

/// 插件 UI 窗口的 label 前缀，后接插件 id。
pub const UI_PREFIX: &str = "plugin-ui-";

/// 预览窗口（文本 / 图片 / 视频，加载 preview.html）。
pub const PREVIEW_LABEL: &str = "preview";
/// 网页预览窗口（WebviewUrl::External，每次按目标 URL 重新导航）。
pub const PREVIEW_WEB_LABEL: &str = "preview-web";

/// 预览窗口的尺寸（逻辑像素）。
const PREVIEW_W: f64 = 640.0;
const PREVIEW_H: f64 = 520.0;

/// 网页预览窗口的注入脚本：在页面里挂一个隔离的浮层，提供「打开」「关闭」。
///
/// 远程页面没法用 iframe（大量站点有 X-Frame-Options），也没有我们的工具栏；
/// 页面又不能调 efto 的 IPC（Tauri 对非本地来源默认拒绝），所以按钮改用
/// "导航到一个保留地址"来通知宿主，由 [`preview_nav`] 在 `on_navigation` 里截住。
/// 用 closed shadow root 承载，避免被站点自身的 CSS 影响。
const WEB_CHROME_SCRIPT: &str = r#"
(function () {
  var OPEN_URL = 'http://efto.localhost/__preview/open';
  var CLOSE_URL = 'http://efto.localhost/__preview/close';
  function mount() {
    if (document.getElementById('__efto_preview_bar')) return;
    var host = document.createElement('div');
    host.id = '__efto_preview_bar';
    host.setAttribute('style', 'position:fixed!important;top:10px!important;right:10px!important;z-index:2147483647!important;');
    var root = host.attachShadow({ mode: 'closed' });
    var box = document.createElement('div');
    box.setAttribute('style', 'display:flex;gap:8px;');
    function make(label, url, primary) {
      var b = document.createElement('button');
      b.textContent = label;
      b.setAttribute('style',
        'padding:6px 14px;border-radius:9px;cursor:pointer;font:12px/1.4 "Segoe UI",system-ui,sans-serif;' +
        'box-shadow:0 6px 20px rgba(0,0,0,.35);backdrop-filter:blur(14px);-webkit-backdrop-filter:blur(14px);' +
        (primary
          ? 'border:1px solid rgba(122,162,255,.9);background:rgba(122,162,255,.22);color:#eaf0ff;font-weight:600;'
          : 'border:1px solid rgba(255,255,255,.16);background:rgba(17,19,26,.74);color:#eef1f7;'));
      b.addEventListener('click', function (e) {
        e.preventDefault();
        e.stopPropagation();
        window.location.href = url;
      }, true);
      return b;
    }
    box.appendChild(make('打开', OPEN_URL, true));
    box.appendChild(make('关闭', CLOSE_URL, false));
    root.appendChild(box);
    (document.body || document.documentElement).appendChild(host);
  }
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Shift' && !e.repeat) window.location.href = CLOSE_URL;
  }, true);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount);
  else mount();
  window.addEventListener('load', mount);
})();
"#;

/// 网页预览窗口里按钮/Shift 使用的保留地址前缀（见 [`WEB_CHROME_SCRIPT`]）。
const PREVIEW_CTL_PREFIX: &str = "http://efto.localhost/__preview/";

/// 网页预览窗口里的工具栏动作：这两个地址不进协议处理器，
/// 由 `on_navigation` 截住后直接执行，再把导航取消掉（页面保持原样）。
fn preview_nav(app: &AppHandle, url: &str) {
    if !url.starts_with(PREVIEW_CTL_PREFIX) {
        return;
    }
    if url.ends_with("/open") {
        // 拉起系统浏览器可能耗时，别堵 webview 的消息循环
        std::thread::spawn(|| {
            if let Err(e) = crate::commands::preview_open() {
                tracing::warn!("预览页打开失败: {e}");
            }
        });
    } else {
        crate::commands::preview_hide(app.clone());
    }
}

/// 网页预览的"缩放到窗口宽度"脚本。
///
/// 桌面站点在 640 宽的窗口里会按约 1000px 排版，右侧被裁掉且要横向滚动。
/// 这里在页面加载后量一次固有宽度，再用根节点的 CSS `zoom` 整体缩小到刚好装下。
/// 每次都先清掉 zoom 再测量，避免在已缩放坐标系里反复累乘。
///
/// 通过 `initialization_script` 注入，会作用到每个文档（含 iframe）：
/// 本地 html 预览就是靠它在 iframe 内部完成缩放。
const FIT_WEB_SCRIPT: &str = r#"
(function () {
  // PDF 查看器是内部扩展页，缩放会打乱它自己的排版，直接跳过
  var proto = location.protocol;
  if (proto !== 'http:' && proto !== 'https:' && proto !== 'file:') return;
  function fit() {
    var d = document.documentElement;
    if (!d) return;
    d.style.zoom = '';
    var w = d.clientWidth;
    var sw = d.scrollWidth;
    if (!w || !sw || sw <= w) return;
    var z = w / sw;
    if (z < 0.25) z = 0.25;
    d.style.zoom = String(Math.round(z * 1000) / 1000);
  }
  function later() {
    fit();
    setTimeout(fit, 300);
    setTimeout(fit, 1500);
  }
  if (document.readyState === 'complete') later();
  else window.addEventListener('load', later);
})();
"#;

/// 预览窗口此刻是否应当显示。松开 Shift 后仍为 true，直到用户点关闭或收起主窗口。
static PREVIEW_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Shift 是否还按着。建窗/显示可能让主窗口短暂失焦，只在这段窗口期豁免 `hide_on_blur`。
static PREVIEW_HELD: AtomicBool = AtomicBool::new(false);
/// 两个预览窗口各自只允许一个建窗线程在跑，避免同 label 并发 build 报错。
static PREVIEW_CREATING: AtomicBool = AtomicBool::new(false);
static PREVIEW_WEB_CREATING: AtomicBool = AtomicBool::new(false);

pub fn pin_label(id: &str) -> String {
    format!("{PIN_PREFIX}{id}")
}

/// 插件 UI 窗口的 label（同时作为协议与事件的寻址键）。
pub fn ui_label(plugin_id: &str) -> String {
    format!("{UI_PREFIX}{}", efto_core::fsutil::sanitize_key(plugin_id))
}

/// 主窗口"藏"到屏幕外的坐标（虚拟屏幕外，任何显示器都看不到）。
fn park_position() -> tauri::PhysicalPosition<i32> {
    tauri::PhysicalPosition::new(-32_000, -32_000)
}

/// 隐藏前保存的位置，显示时还原。
static PARKED_AT: once_cell::sync::Lazy<std::sync::Mutex<Option<tauri::PhysicalPosition<i32>>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

/// 隐藏前先把窗口挪到屏幕外。
///
/// Windows 上「透明 + 无边框 + 置顶」的 WebView2 窗口调用 `hide()` 后，DWM 偶尔会把
/// 最后一帧留在屏幕上：肉眼能看到一块半透明的搜索框残影，但 UI 自动化查不到窗口
/// （`WS_VISIBLE` 已经是 false）。先挪走再隐藏，残留帧就落在屏幕外，用户看不到。
fn park_before_hide(w: &tauri::WebviewWindow) {
    if let Ok(pos) = w.outer_position() {
        // 失焦和 Esc 可能连续触发 hide：已经停在屏幕外时不要用停车位坐标覆盖原位置
        if pos.x > park_position().x / 2 {
            *PARKED_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(pos);
        }
    }
    let _ = w.set_position(park_position());
}

/// 显示前把窗口挪回原位置（没停过车就保持配置里的居中位置）。
fn restore_after_park(w: &tauri::WebviewWindow) {
    let saved = PARKED_AT.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(pos) = saved {
        let _ = w.set_position(pos);
    }
}

pub fn show_main(app: &AppHandle) {
    // run_on_main_thread 借用 self，闭包又要拥有句柄：必须用另一个克隆体
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(MAIN_LABEL) {
            restore_after_park(&w);
            let _ = w.show();
            let _ = w.set_focus();
            let _ = w.emit_to(EventTarget::window(MAIN_LABEL), "main:show", ());
            // 顺手把预览窗口建好：Shift 按下时不用再等 WebView2 初始化
            warm_up_preview(&inner);
        } else {
            tracing::warn!("主窗口不存在，无法显示");
        }
    });
}

/// 显示主窗口，但不发 `main:show`（用于插件内联输出：
/// 不能让主窗口把搜索列表重置掉，输出行会紧接着推给前端）。
pub fn show_main_quiet(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(MAIN_LABEL) {
            if !w.is_visible().unwrap_or(false) {
                restore_after_park(&w);
                let _ = w.show();
            }
            let _ = w.set_focus();
        }
    });
}

/// 打开设置窗口（常驻窗口，只负责显示与聚焦）。
pub fn show_settings(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(SETTINGS_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
        } else {
            tracing::warn!("设置窗口不存在");
        }
    });
}

/// 打开剪贴板历史窗口（常驻窗口，显示 + 通知页面刷新列表）。
pub fn show_clipboard(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(CLIPBOARD_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
            let _ = w.emit_to(EventTarget::window(CLIPBOARD_LABEL), "clipboard:show", ());
        } else {
            tracing::warn!("剪贴板窗口不存在");
        }
    });
}

/// 打开帮助文档窗口（常驻窗口；每次显示都通知页面按当前语言重载文档）。
pub fn show_help(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(HELP_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
            let _ = w.emit_to(EventTarget::window(HELP_LABEL), "help:show", ());
        } else {
            tracing::warn!("帮助窗口不存在");
        }
    });
}

pub fn hide_help(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(HELP_LABEL) {
            let _ = w.hide();
        }
    });
}

/// 打开快捷键指南窗口；页面收到 `shortcuts:show` 后重新识别前台应用。
pub fn show_shortcuts(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(SHORTCUTS_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
            let _ = w.emit_to(EventTarget::window(SHORTCUTS_LABEL), "shortcuts:show", ());
        } else {
            tracing::warn!("快捷键指南窗口不存在");
        }
    });
}

pub fn hide_shortcuts(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(SHORTCUTS_LABEL) {
            let _ = w.hide();
        }
    });
}

/// 打开通用动作窗口；页面收到 `actions:show` 后拉取选中文本。
pub fn show_actions(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(ACTIONS_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
            let _ = w.emit_to(EventTarget::window(ACTIONS_LABEL), "actions:show", ());
        } else {
            tracing::warn!("通用动作窗口不存在");
        }
    });
}

pub fn hide_actions(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(ACTIONS_LABEL) {
            let _ = w.hide();
        }
    });
}

/// 打开文件缓冲窗口；页面收到 `files:show` 后刷新列表。
pub fn show_files(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(FILES_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
            let _ = w.emit_to(EventTarget::window(FILES_LABEL), "files:show", ());
        } else {
            tracing::warn!("文件缓冲窗口不存在");
        }
    });
}

pub fn hide_files(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(FILES_LABEL) {
            let _ = w.hide();
        }
    });
}

/// 把主题模式应用到常驻窗口的原生标题栏与 WebView2 的 prefers-color-scheme。
///
/// `System` 传 `None`，让窗口重新跟随系统；固定模式则强制对应明暗。
/// 主窗口无边框、没有标题栏，但 WebView2 的 `prefers-color-scheme` 同样跟着变。
/// 插件 UI 窗口也在这里一起对齐，否则插件页面的 `efto.isDarkMode()` 会跟系统而不是跟应用。
pub fn apply_theme(app: &AppHandle, mode: ThemeMode) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        let theme = match mode {
            ThemeMode::System => None,
            ThemeMode::Dark => Some(tauri::Theme::Dark),
            ThemeMode::Light => Some(tauri::Theme::Light),
        };
        for label in [
            MAIN_LABEL,
            SETTINGS_LABEL,
            CLIPBOARD_LABEL,
            HELP_LABEL,
            SHORTCUTS_LABEL,
            ACTIONS_LABEL,
            FILES_LABEL,
        ] {
            if let Some(w) = inner.get_webview_window(label) {
                let _ = w.set_theme(theme);
            }
        }
        for (label, w) in inner.webview_windows() {
            if label.starts_with(UI_PREFIX) {
                let _ = w.set_theme(theme);
            }
        }
    });
}

/// 设置里固定的窗口明暗；`System` 返回 `None`（跟随系统）。
fn theme_setting(app: &AppHandle) -> Option<tauri::Theme> {
    // State 是临时借用，必须先绑定到局部变量，否则 guard 会借用一个已释放的临时值
    let data = app.state::<Shared>();
    let mode = data.settings.lock().theme;
    match mode {
        ThemeMode::System => None,
        ThemeMode::Dark => Some(tauri::Theme::Dark),
        ThemeMode::Light => Some(tauri::Theme::Light),
    }
}

/// 语言变更后同步常驻窗口的原生标题栏。
pub fn apply_language(app: &AppHandle, lang: &str) {
    let inner = app.clone();
    let lang = lang.to_string();
    let _ = app.run_on_main_thread(move || {
        for (label, key) in [
            (SETTINGS_LABEL, "window.settings"),
            (CLIPBOARD_LABEL, "window.clipboard"),
            (HELP_LABEL, "window.help"),
            (SHORTCUTS_LABEL, "window.shortcuts"),
            (ACTIONS_LABEL, "window.actions"),
            (FILES_LABEL, "window.files"),
        ] {
            if let Some(w) = inner.get_webview_window(label) {
                let _ = w.set_title(&crate::i18n::tr(&lang, key));
            }
        }
    });
}

pub fn hide_clipboard(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(CLIPBOARD_LABEL) {
            let _ = w.hide();
        }
    });
}

/// 关闭任意 label 的窗口（截图覆盖层、钉图）。
pub fn close_window(app: &AppHandle, label: &str) {
    let inner = app.clone();
    let label = label.to_string();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(&label) {
            let _ = w.close();
        }
    });
}

/// 打开截图选区覆盖层：无边框、置顶、铺满整个虚拟桌面（所有显示器）。
///
/// 与插件 UI 同样的约束：**必须不在主线程 `build()`**（见 [`open_plugin_ui`]）。
pub fn open_screenshot_overlay(app: &AppHandle, shot: &crate::screenshot::Shot) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(SCREENSHOT_LABEL) {
        let _ = w.close();
    }
    let origin = (shot.origin_x, shot.origin_y);
    let size = (shot.width, shot.height);
    let app = app.clone();
    std::thread::Builder::new()
        .name("screenshot-overlay".into())
        .spawn(move || {
            let builder = WebviewWindowBuilder::new(
                &app,
                SCREENSHOT_LABEL,
                WebviewUrl::App("screenshot.html".into()),
            )
            .title("efto 截图")
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .shadow(false)
            // 长截图时要把背景图藏掉，只留下边框与提示，让用户看到并滚动底下的页面
            .transparent(true)
            // 先建成隐藏的，等定位/定尺寸到位再显示，避免左上角闪一下
            .visible(false);
            match builder.build() {
                Ok(w) => {
                    // 用物理像素定位与定尺寸，避免高 DPI 下 inner_size 的逻辑像素换算误差
                    let _ = w.set_position(tauri::PhysicalPosition::new(origin.0, origin.1));
                    let _ = w.set_size(tauri::PhysicalSize::new(size.0, size.1));
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                Err(e) => tracing::error!("截图窗口创建失败: {e}"),
            }
        })
        .map_err(|e| format!("无法启动截图窗口线程: {e}"))?;
    Ok(())
}

/// 把窗口从屏幕捕获里排除（Windows 10 2004+）。
///
/// 长截图时覆盖层必须留在屏幕上给用户看边框和提示，但它自己不能被抓进结果图。
#[cfg(windows)]
pub fn exclude_from_capture(w: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    let hwnd = match w.hwnd() {
        Ok(h) => h.0 as *mut core::ffi::c_void,
        Err(e) => {
            tracing::warn!("取截图覆盖层句柄失败: {e}");
            return;
        }
    };
    if unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } == 0 {
        tracing::warn!("覆盖层无法从屏幕捕获中排除，边框可能被截进长图");
    }
}

#[cfg(not(windows))]
pub fn exclude_from_capture(_w: &tauri::WebviewWindow) {}

/// 打开钉图窗口：无边框、置顶、尺寸等于裁剪尺寸。
pub fn open_pin(
    app: &AppHandle,
    id: &str,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
) -> Result<(), String> {
    let label = pin_label(id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let app = app.clone();
    let id = id.to_string();
    let init = format!(
        "window.__EFTO_PIN = {};",
        serde_json::to_string(&id).map_err(|e| e.to_string())?
    );
    std::thread::Builder::new()
        .name("pin-open".into())
        .spawn(move || {
            let builder =
                WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("pin.html".into()))
                    .title("efto 钉图")
                    .inner_size(width.max(1) as f64, height.max(1) as f64)
                    .decorations(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .resizable(false)
                    .shadow(true)
                    .visible(false)
                    .initialization_script(&init);
            match builder.build() {
                Ok(w) => {
                    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                Err(e) => tracing::error!("钉图窗口创建失败: {e}"),
            }
        })
        .map_err(|e| format!("无法启动钉图窗口线程: {e}"))?;
    Ok(())
}

pub fn hide_main(app: &AppHandle) {
    PREVIEW_ACTIVE.store(false, Ordering::SeqCst);
    PREVIEW_HELD.store(false, Ordering::SeqCst);
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(MAIN_LABEL) {
            park_before_hide(&w);
            let _ = w.hide();
        }
        for label in [PREVIEW_LABEL, PREVIEW_WEB_LABEL] {
            if let Some(w) = inner.get_webview_window(label) {
                let _ = w.hide();
            }
        }
    });
}

pub fn toggle_main(app: &AppHandle) {
    let visible = app
        .get_webview_window(MAIN_LABEL)
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false);
    let focused = app
        .get_webview_window(MAIN_LABEL)
        .map(|w| w.is_focused().unwrap_or(false))
        .unwrap_or(false);
    if visible && focused {
        hide_main(app);
    } else {
        show_main(app);
    }
}

/// 展示一条预览（文本 / 图片 / 视频 / 网页）。
///
/// 在异步命令线程调用，窗口创建再另起线程（理由同 [`open_plugin_ui`]）。
/// 文本类窗口复用 `preview.html`，网页类窗口复用 `preview-web` 并重新导航。
pub fn show_preview(app: &AppHandle, payload: PreviewPayload) {
    PREVIEW_ACTIVE.store(true, Ordering::SeqCst);
    PREVIEW_HELD.store(true, Ordering::SeqCst);
    crate::preview::store(payload.clone());

    if payload.kind == "web" {
        if let Some(w) = app.get_webview_window(PREVIEW_WEB_LABEL) {
            navigate_web(&w, &payload);
            hide_window(app, PREVIEW_LABEL);
            present_preview(app, PREVIEW_WEB_LABEL);
            return;
        }
        hide_window(app, PREVIEW_LABEL);
        if PREVIEW_WEB_CREATING.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(url) = payload.url.as_deref().and_then(|u| url::Url::parse(u).ok()) else {
            PREVIEW_WEB_CREATING.store(false, Ordering::SeqCst);
            return;
        };
        let app = app.clone();
        let nav_app = app.clone();
        let _ = std::thread::Builder::new()
            .name("preview-web-open".into())
            .spawn(move || {
                let builder = WebviewWindowBuilder::new(
                    &app,
                    PREVIEW_WEB_LABEL,
                    WebviewUrl::External(url),
                )
                .title("efto 预览")
                .inner_size(PREVIEW_W, PREVIEW_H)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .shadow(true)
                // 不抢焦点：否则 Shift keyup 落到预览页上，主输入框的预览状态会卡住
                .focusable(false)
                // 桌面布局的网页整体缩放到窗口宽度，避免右侧被裁掉
                .initialization_script(FIT_WEB_SCRIPT)
                // 远程页面没有我们的工具栏：注入浮层按钮 + Shift 关闭
                .initialization_script(WEB_CHROME_SCRIPT)
                .on_navigation(move |url| {
                    let s = url.as_str();
                    if !s.starts_with(PREVIEW_CTL_PREFIX) {
                        return true;
                    }
                    preview_nav(&nav_app, s);
                    // 取消导航：页面保持原样，只在宿主侧收起/打开
                    false
                })
                .visible(false);
                let built = builder.build();
                PREVIEW_WEB_CREATING.store(false, Ordering::SeqCst);
                match built {
                    Ok(w) => apply_web_preview(&app, &w),
                    Err(e) => tracing::error!("网页预览窗口创建失败: {e}"),
                }
            });
        return;
    }

    if app.get_webview_window(PREVIEW_LABEL).is_some() {
        hide_window(app, PREVIEW_WEB_LABEL);
        let _ = app.emit_to(EventTarget::window(PREVIEW_LABEL), "preview:update", &payload);
        present_preview(app, PREVIEW_LABEL);
        return;
    }
    hide_window(app, PREVIEW_WEB_LABEL);
    if PREVIEW_CREATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("preview-open".into())
        .spawn(move || {
            let built = build_text_preview(&app);
            PREVIEW_CREATING.store(false, Ordering::SeqCst);
            match built {
                Ok(w) => apply_text_preview(&app, &w),
                Err(e) => tracing::error!("预览窗口创建失败: {e}"),
            }
        });
}

/// 预热文本预览窗口：主窗口每次唤出时确保隐藏窗口已建好。
///
/// WebView2 首次建窗要几百毫秒；等用户按下 Shift 才建，会表现为预览"一闪而过"
/// （建好时 Shift 已经松开）或者"根本不出来"。
pub fn warm_up_preview(app: &AppHandle) {
    if app.get_webview_window(PREVIEW_LABEL).is_some() {
        return;
    }
    if PREVIEW_CREATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("preview-warmup".into())
        .spawn(move || {
            let built = build_text_preview(&app);
            PREVIEW_CREATING.store(false, Ordering::SeqCst);
            match built {
                // 预热期间用户已经按了 Shift：直接把当前内容显示出来
                Ok(w) if PREVIEW_ACTIVE.load(Ordering::SeqCst) => apply_text_preview(&app, &w),
                Ok(_) => {}
                Err(e) => tracing::warn!("预览窗口预热失败: {e}"),
            }
        });
}

fn build_text_preview(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(
        app,
        PREVIEW_LABEL,
        WebviewUrl::App("preview.html".into()),
    )
    .title("efto 预览")
    .inner_size(PREVIEW_W, PREVIEW_H)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .shadow(true)
    .focusable(false)
    // 关键：wry 建完 webview 会对 focused 的窗口调 MoveFocus，把隐藏的
    // 预览窗口激活成前台（focusable(false) 拦不住这种程序化激活）。
    // 预热发生在主窗口首次唤出时，这一下抢走前台会让主窗口立刻
    // Focused(false)，hide_on_blur 便把搜索框收起来——"一闪而过"。
    .focused(false)
    .transparent(true)
    // 本地 html 在 iframe 里，桌面布局同样要缩到窗口宽度
    .initialization_script(FIT_WEB_SCRIPT)
    .visible(false)
    .build()
}

/// 预览窗口是否处于"用户按着 Shift"的状态。
///
/// 主窗口的 `Focused(false)` 处理会用它做闸门：显示预览窗口会让主窗口短暂失焦，
/// 不能因此把搜索框收起来（否则按 Shift 就只剩一个空桌面）。
/// 松手后预览仍会留在屏幕上，但此时就不能再豁免失焦隐藏了。
pub fn preview_active() -> bool {
    PREVIEW_HELD.load(Ordering::SeqCst)
}

/// 松开 Shift：预览窗口保持显示，只退出"按住预览"状态。
pub fn release_preview() {
    PREVIEW_HELD.store(false, Ordering::SeqCst);
}

/// 隐藏两个预览窗口（关闭按钮、主窗口收起）。
pub fn hide_preview(app: &AppHandle) {
    PREVIEW_ACTIVE.store(false, Ordering::SeqCst);
    PREVIEW_HELD.store(false, Ordering::SeqCst);
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        for label in [PREVIEW_LABEL, PREVIEW_WEB_LABEL] {
            if let Some(w) = inner.get_webview_window(label) {
                let _ = w.hide();
            }
        }
        // 预览期间主窗口可能失过焦，松手后把焦点还回去，否则接着打字没反应
        if let Some(main) = inner.get_webview_window(MAIN_LABEL) {
            if main.is_visible().unwrap_or(false) {
                let _ = main.set_focus();
            }
        }
    });
}

fn hide_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
    }
}

/// 把预览窗口提到最上层但不激活。
///
/// 主窗口和预览窗口都是 topmost，`show()` 只把窗口放进 topmost 组、不保证压过主窗口，
/// 表现就是预览被居中的搜索框挡住（"一闪而过/出不来"）。因此显示后再显式提一次。
#[cfg(windows)]
fn raise_preview(w: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    if let Ok(hwnd) = w.hwnd() {
        unsafe {
            SetWindowPos(
                hwnd.0 as *mut core::ffi::c_void,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(windows))]
fn raise_preview(_w: &tauri::WebviewWindow) {}

/// 定位 + 显示 + 提层，三者必须在主线程按序执行。
///
/// `show()` 只是把"显示"消息投递到主线程，若调用方（命令线程 / 建窗线程）
/// 紧接着自己调 [`raise_preview`]，提层往往先于真正的 `ShowWindow` 执行而被吞掉，
/// 预览就会被同样置顶的主窗口盖住（表现为"一闪而过 / 出不来"）。
fn present_preview(app: &AppHandle, label: &'static str) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = inner.get_webview_window(label) {
            place_preview(&inner, &w);
            let _ = w.show();
            raise_preview(&w);
        }
    });
}

/// 建窗线程收尾：只要预览仍然是"应当显示"的状态，就把最新内容呈现出来。
///
/// 注意这里看的是 `PREVIEW_ACTIVE` 而不是"Shift 是否还按着"：松手不关窗，
/// 用户快速点一下 Shift 也应该留下预览。不满足条件时只隐藏、不关闭：
/// 窗口是预热出来的，关掉会让下一次按 Shift 重新走几百毫秒的 WebView2 初始化。
fn apply_text_preview(app: &AppHandle, w: &tauri::WebviewWindow) {
    let Some(payload) = crate::preview::current() else {
        let _ = w.hide();
        return;
    };
    if !PREVIEW_ACTIVE.load(Ordering::SeqCst) || payload.kind == "web" {
        let _ = w.hide();
        return;
    }
    let _ = app.emit_to(EventTarget::window(PREVIEW_LABEL), "preview:update", &payload);
    present_preview(app, PREVIEW_LABEL);
}

fn apply_web_preview(app: &AppHandle, w: &tauri::WebviewWindow) {
    let Some(payload) = crate::preview::current() else {
        let _ = w.hide();
        return;
    };
    if !PREVIEW_ACTIVE.load(Ordering::SeqCst) || payload.kind != "web" {
        let _ = w.hide();
        return;
    }
    navigate_web(w, &payload);
    present_preview(app, PREVIEW_WEB_LABEL);
}

fn navigate_web(w: &tauri::WebviewWindow, payload: &PreviewPayload) {
    if let Some(url) = payload.url.as_deref().and_then(|u| url::Url::parse(u).ok()) {
        if let Err(e) = w.navigate(url) {
            tracing::warn!("网页预览导航失败: {e}");
        }
    }
}

/// 把预览窗口放到主窗口所在显示器的正中（扣掉任务栏）。
fn place_preview(app: &AppHandle, w: &tauri::WebviewWindow) {
    let Some(main) = app.get_webview_window(MAIN_LABEL) else {
        return;
    };
    let Ok(own) = w.outer_size() else {
        return;
    };
    let Ok(Some(mon)) = main.current_monitor() else {
        return;
    };
    let area = mon.work_area();
    let x = area.position.x + (area.size.width as i32 - own.width as i32) / 2;
    let y = area.position.y + (area.size.height as i32 - own.height as i32) / 2;
    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
}

/// 唤起/复用插件 UI 窗口。
///
/// **必须不在主线程上建窗口。** 这是排查过的一个真 bug，两处都踩过：
/// 1. 在 `run_on_main_thread` 的闭包里调 `build()` —— 闭包本身就在主线程，
///    Tauri 的 `build()` 内部还要把创建动作投递到主线程并**同步等结果** → 自锁；
/// 2. 现在的 `invoke_hit` 是**同步** Tauri 命令，实测跑在名为 `"main"` 的线程上，
///    直接调 `build()` 同样自锁：WebView2 的初始化是异步的，完成回调要靠主线程的消息循环，
///    而主线程正阻塞在 `build()` 里等它。
/// 症状完全一致：插件窗口的 HWND 已经出现、但整片白（协议请求一次都没到）、
/// 主搜索框卡在"执行中…"、连窗口都关不掉。
///
/// 所以这里固定 spawn 一个专属线程去 build：调用方（可能是主线程）立刻返回，
/// 主线程空出来跑消息循环，`build()` 才能等到 WebView2 初始化完成。
pub fn open_plugin_ui(
    app: &AppHandle,
    plugin_id: &str,
    rel_entry: &str,
    title: &str,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let label = ui_label(plugin_id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let url = format!(
        "{}://localhost/{plugin_id}/{rel_entry}",
        crate::protocol::EFTO_SCHEME
    );
    let parsed = url::Url::parse(&url).map_err(|e| format!("插件 UI 地址无效 {url}: {e}"))?;
    // 与主窗口保持一致：固定浅/深时把窗口主题钉死，插件页面的
    // prefers-color-scheme（以及 efto.isDarkMode()）才会跟应用而不是跟系统。
    let theme = theme_setting(app);
    let app = app.clone();
    let title = title.to_string();
    std::thread::Builder::new()
        .name("plugin-ui-open".into())
        .spawn(move || {
            let mut builder =
                WebviewWindowBuilder::new(&app, &label, WebviewUrl::CustomProtocol(parsed))
                    .title(&title)
                    .inner_size(width, height)
                    .resizable(true)
                    .decorations(true)
                    .always_on_top(false)
                    .skip_taskbar(false)
                    .visible(true)
                    // 页面加载是"窗口创建"和"内容真的渲染出来"之间的关键信号：
                    // 白屏时先看这条有没有出现，能立刻区分"WebView2 没起来"和"页面伺服/JS 出错"
                    .on_page_load(|w, payload| {
                        let label = w.label().to_string();
                        tracing::info!("插件页面已加载: {label} {}", payload.url());
                        // 注意：**不在这里补发**。on_page_load 可能早于页面里
                        // `plugin:event|listen` 注册完成，此时 send 过去没人收（实测过）。
                        // 真正的补发由页面主动握手触发：插件 SDK 注册好监听后会调
                        // `plugin_ui_ready`，见 flush_pending。
                    })
                    .initialization_script(PLUGIN_SDK);
            if let Some(theme) = theme {
                builder = builder.theme(Some(theme));
            }
            let t0 = std::time::Instant::now();
            tracing::info!("正在创建插件 UI 窗口: {label}");
            match builder.build() {
                Ok(_) => {
                    tracing::info!("插件 UI 窗口已打开: {label}（{}ms）", t0.elapsed().as_millis())
                }
                Err(e) => tracing::error!("插件 UI 窗口创建失败 {label}: {e}"),
            }
        })
        .map_err(|e| format!("无法启动插件窗口线程: {e}"))?;
    Ok(())
}

/// 插件 UI 隐藏（保留窗口，下次 showUI 复用）。
pub fn hide_plugin_ui(app: &AppHandle, plugin_id: &str) {
    let label = ui_label(plugin_id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.hide();
    }
}

pub fn close_plugin_ui(app: &AppHandle, plugin_id: &str, _name: &str) {
    let label = ui_label(plugin_id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.close();
    }
    // 通知后端 UI 已关闭（插件可以据此收尾）。
    // 锁插件运行时**不能**放在主线程闭包里：pump 线程也持有它，
    // 主线程等锁、pump 等主线程，又是一个自锁。
    let data = app.state::<Shared>().inner().clone();
    let rt = data.runtime.lock();
    let _ = rt.ui_closed(plugin_id);
}

/// Rust → 插件 UI 页面的一条消息。
pub fn emit_to_plugin_ui(app: &AppHandle, plugin_id: &str, name: &str, data: &serde_json::Value) {
    let label = ui_label(plugin_id);
    let payload = serde_json::json!({ "name": name, "data": data });
    // 窗口不存在时 emit_to 不一定报错，只是没人收 —— 先排队，等页面加载完补发
    if app.get_webview_window(&label).is_none() {
        let mut q = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        let list = q.entry(label.clone()).or_default();
        if list.len() < 50 {
            list.push(payload);
            tracing::info!("[main→ui] {label} {name} 排队等待窗口");
        } else {
            tracing::warn!("[main→ui] {label} {name} 队列已满，丢弃");
        }
        return;
    }
    match app.emit_to(EventTarget::window(&label), "plugin:message", payload) {
        Ok(_) => tracing::info!("[main→ui] {label} {name}"),
        Err(e) => tracing::warn!("[main→ui] {label} {name} 发送失败: {e}"),
    }
}

/// UI 窗口还没建好时，主进程发来的消息先排队，等页面加载完再补发。
///
/// 为什么需要：插件常见的写法是"先 `ipcMessage` 再 `showUI`"，而 `showUI` 是异步的、
/// 窗口要几百毫秒才建好。没有队列时这些消息直接丢掉，表现就是页面一直停在
/// "等待 main 的消息…"、数据区全是 `—`（hello-js 就是这样被发现的）。
static PENDING: once_cell::sync::Lazy<std::sync::Mutex<std::collections::HashMap<String, Vec<serde_json::Value>>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// 页面说"我的监听已注册好"时，把排队的消息补发出去。
/// 用握手而不是"页面加载完就发"，是因为加载完成与 JS 注册监听之间还有窗口期（实测丢消息）。
pub fn flush_pending(app: &AppHandle, plugin_id: &str) {
    let label = ui_label(plugin_id);
    let queued = PENDING
        .lock()
        .map(|mut q| q.remove(&label).unwrap_or_default())
        .unwrap_or_default();
    if queued.is_empty() {
        return;
    }
    let n = queued.len();
    for msg in queued {
        let name = msg
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        match app.emit_to(EventTarget::window(&label), "plugin:message", msg) {
            Ok(_) => tracing::info!("[main→ui] {label} {name} 已补发"),
            Err(e) => tracing::warn!("[main→ui] {label} {name} 补发失败: {e}"),
        }
    }
    tracing::info!("[main→ui] {label} 握手后补发 {n} 条");
}

/// 注入到每个插件 UI 页面的 SDK。
pub const PLUGIN_SDK: &str = include_str!("../assets/plugin-sdk.js");

#[cfg(test)]
mod tests {
    use super::*;

    /// 源码级回归守卫：这两个写法都实测会死锁（见 `open_plugin_ui` 的注释）。
    ///
    /// 不是类型级保证，但足以挡住"顺手把 `build()` 包回 `run_on_main_thread`"
    /// 或"把独立线程去掉"这类回归 —— 它们在测试里看不出来（GUI 线程问题没法单测），
    /// 只在真机上表现为白屏 + 卡死。
    #[test]
    fn window_creation_stays_off_the_main_thread() {
        // 只看 open_plugin_ui 的函数体：show_main/hide_main 里用 run_on_main_thread
        // 是合法的（它们只是投递 show/hide，不等待），不能一起禁掉。
        let src = include_str!("windows.rs");
        let tail = src
            .split("pub fn open_plugin_ui")
            .nth(1)
            .expect("找不到 open_plugin_ui");
        let body = &tail[..tail.find("
}
").unwrap_or(tail.len())];
        assert!(
            !body.contains(".run_on_main_thread("),
            "open_plugin_ui 里不要用 run_on_main_thread 造窗口：闭包本身已在主线程，build() 会自锁"
        );
        assert!(
            body.contains("thread::Builder::new") || body.contains("thread::spawn"),
            "open_plugin_ui 必须在独立线程里 build()，否则主线程被 WebView2 初始化卡死"
        );
        assert!(body.contains("build()"), "open_plugin_ui 里应当真的调用 build()");
    }

    #[test]
    fn ui_label_is_safe() {
        assert_eq!(ui_label("demo"), "plugin-ui-demo");
        assert_eq!(ui_label("a/b:c"), "plugin-ui-a_b_c");
        assert!(ui_label("中文插件").starts_with("plugin-ui-"));
    }

    #[test]
    fn sdk_contains_expected_surface() {
        assert!(PLUGIN_SDK.contains("window.efto"));
        assert!(PLUGIN_SDK.contains("__TAURI_INTERNALS__"));
        assert!(PLUGIN_SDK.contains("plugin:message"));
        // 插件 UI 的原生文件选择与系统拖拽入口
        assert!(PLUGIN_SDK.contains("plugin_pick_files"));
        assert!(PLUGIN_SDK.contains("plugin_shell_show_in_folder"));
        assert!(PLUGIN_SDK.contains("tauri://drag-drop"));
    }
}
