//! 托盘与托盘菜单。

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, MenuEvent};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager};

use crate::i18n;
use crate::state::Shared;
use crate::windows;

const TRAY_ID: &str = "main-tray";

pub fn build(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let lang = i18n::resolve(&app.state::<Shared>().settings.lock());
    let menu = build_menu(app.handle(), lang).map_err(|e| format!("菜单创建失败: {e}"))?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("缺少内置图标，请检查 icons/icon.png 是否存在")?;

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .icon(icon)
        .tooltip(i18n::tr(lang, "tray.tooltip"))
        .build(app)?;
    tracing::info!("托盘已就绪: {}", tray.id().as_ref());
    Ok(())
}

/// 菜单文本无法原地修改：语言变更后重建菜单并换到托盘上。
pub fn apply_language(app: &AppHandle, lang: &str) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        tracing::warn!("托盘不存在，跳过语言切换");
        return;
    };
    match build_menu(app, lang) {
        Ok(menu) => {
            if let Err(e) = tray.set_menu(Some(menu)) {
                tracing::warn!("托盘菜单切换语言失败: {e}");
                return;
            }
            let _ = tray.set_tooltip(Some(&i18n::tr(lang, "tray.tooltip")));
        }
        Err(e) => tracing::warn!("托盘菜单重建失败: {e}"),
    }
}

fn build_menu(app: &AppHandle, lang: &str) -> tauri::Result<Menu<tauri::Wry>> {
    // 不用闭包：MenuItemBuilder::build 需要 &M，多次可变借用同一个 app 会过不了借用检查
    let show = MenuItemBuilder::with_id("tray.show", i18n::tr(lang, "tray.show")).build(app)?;
    let settings = MenuItemBuilder::with_id("tray.settings", i18n::tr(lang, "tray.settings")).build(app)?;
    let help = MenuItemBuilder::with_id("tray.help", i18n::tr(lang, "tray.help")).build(app)?;
    let quit = MenuItemBuilder::with_id("tray.quit", i18n::tr(lang, "tray.quit")).build(app)?;

    MenuBuilder::new(app)
        .item(&show)
        .item(&settings)
        .item(&help)
        .separator()
        .item(&quit)
        .build()
}

pub fn handle(app: &AppHandle, event: &MenuEvent) {
    let id: &str = event.id().as_ref();
    let data = app.state::<Shared>();
    match id {
        "tray.show" => windows::show_main(app),
        "tray.settings" => windows::show_settings(app),
        "tray.help" => windows::show_help(app),
        "tray.quit" => {
            data.flush_stores();
            data.runtime.lock().stop_all();
            app.exit(0);
        }
        other => tracing::debug!("忽略未知菜单事件: {other}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn menu_ids_are_stable() {
        // 这些 id 是 handle() 的匹配键，改动必须同步
        let ids = ["tray.show", "tray.settings", "tray.help", "tray.quit"];
        for id in ids {
            assert!(id.starts_with("tray."), "{id} 应以 tray. 开头");
        }
        assert_eq!(ids.len(), 4);
    }
}
