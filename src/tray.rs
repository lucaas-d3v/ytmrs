//! Tray icon + context menu (index.ts "Tray creation" region)

use muda::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::app::App;
use crate::services::protected_api::remote_execute;
use crate::shared::types::TrayIconStyle;

pub struct Tray {
    icon: TrayIcon,
    _menu: Menu,
    current_file: String,
}

fn tray_icon_file_name(app: &App) -> &'static str {
    if cfg!(windows) {
        return "ytmrs.png";
    }
    if cfg!(target_os = "macos") {
        return "trayTemplate@2x.png";
    }
    let style = TrayIconStyle::from_i64(app.config_store.get("appearance.trayIconStyle").as_i64().unwrap_or(0));
    let white = match style {
        TrayIconStyle::White => true,
        TrayIconStyle::Black => false,
        TrayIconStyle::Auto => app.window_manager.get("Main").map(|w| w.window.theme() == tao::window::Theme::Dark).unwrap_or(true),
    };
    if white {
        "ytmrs_white.png"
    } else {
        "ytmrs_black.png"
    }
}

fn load_icon(file: &str) -> Option<Icon> {
    let (rgba, w, h) = crate::assets::decode_png(crate::assets::icon_bytes(file)?)?;
    Icon::from_rgba(rgba, w, h).ok()
}

pub fn create_tray(app: &mut App) {
    let menu = Menu::new();
    let title = MenuItem::with_id("tray:title", "YtmRs", false, None);
    let show_hide = MenuItem::with_id("tray:showHide", "Show/Hide Window", true, None);
    let play_pause = MenuItem::with_id("tray:playPause", "Play/Pause", true, None);
    let previous = MenuItem::with_id("tray:previous", "Previous", true, None);
    let next = MenuItem::with_id("tray:next", "Next", true, None);
    let quit = MenuItem::with_id("tray:quit", "Quit", true, None);
    let _ = menu.append_items(&[&title, &PredefinedMenuItem::separator(), &show_hide, &play_pause, &previous, &next, &PredefinedMenuItem::separator(), &quit]);

    app.menu_actions.insert("tray:showHide".into(), Box::new(|app| {
        if app.window_manager.has_window("Miniplayer") {
            // Closing the miniplayer automatically shows and focuses the main window
            crate::services::window_manager::close_window(app, "Miniplayer");
            return;
        }
        if app.window_manager.is_visible("Main") {
            app.window_manager.hide("Main");
        } else {
            app.window_manager.show_and_focus("Main");
        }
    }));
    app.menu_actions.insert("tray:playPause".into(), Box::new(|app| remote_execute(app, "playPause", vec![])));
    app.menu_actions.insert("tray:previous".into(), Box::new(|app| remote_execute(app, "previous", vec![])));
    app.menu_actions.insert("tray:next".into(), Box::new(|app| remote_execute(app, "next", vec![])));
    app.menu_actions.insert("tray:quit".into(), Box::new(|app| app.quit()));

    let file = tray_icon_file_name(app).to_string();
    let mut builder = TrayIconBuilder::new().with_id("ytmrs").with_menu(Box::new(menu.clone())).with_tooltip("YtmRs").with_title("YtmRs").with_menu_on_left_click(false);
    if let Some(icon) = load_icon(&file) {
        #[cfg(target_os = "macos")]
        {
            builder = builder.with_icon_templated(icon);
        }
        #[cfg(not(target_os = "macos"))]
        {
            builder = builder.with_icon(icon);
        }
    }
    match builder.build() {
        Ok(icon) => {
            app.ui.tray = Some(Tray { icon, _menu: menu, current_file: file });
            log::info!("Created tray icon");
        }
        Err(err) => log::warn!("Failed to create tray icon: {err}"),
    }
}

/// nativeTheme "updated" / tray style change
pub fn update_icon(app: &mut App) {
    let file = tray_icon_file_name(app).to_string();
    if let Some(tray) = app.ui.tray.as_mut() {
        if tray.current_file != file {
            if let Some(icon) = load_icon(&file) {
                let _ = tray.icon.set_icon(Some(icon));
            }
            tray.current_file = file;
        }
    }
}

pub fn handle_tray_event(app: &mut App, event: TrayIconEvent) {
    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
        app.window_manager.show_and_focus("Main");
    }
}
