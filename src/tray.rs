//! Tray icon + context menu (index.ts "Tray creation" region), StatusNotifierItem via ksni

use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::app::App;
use crate::services::protected_api::remote_execute;
use crate::shared::types::TrayIconStyle;
use crate::ui::{main_window, miniplayer};

pub struct Tray {
    icon: TrayIcon,
    _menu: Menu,
    current_file: String,
}

fn tray_icon_file_name(app: &App) -> &'static str {
    let style = TrayIconStyle::from_i64(app.config_store.get("appearance.trayIconStyle").as_i64().unwrap_or(0));
    let white = match style {
        TrayIconStyle::White => true,
        TrayIconStyle::Black => false,
        TrayIconStyle::Auto => app.ui.dark_theme,
    };
    if white {
        "ytmrs_white.png"
    } else {
        "ytmrs_black.png"
    }
}

fn load_icon(file: &str) -> Option<Icon> {
    let (rgba, w, h) = crate::assets::icon_rgba(file, 64)?;
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
        if miniplayer::is_open(app) {
            // Closing the miniplayer automatically shows and focuses the main window
            miniplayer::close(app);
        } else if main_window::is_visible(app) {
            main_window::hide(app);
        } else {
            main_window::show_and_focus(app);
        }
    }));
    app.menu_actions.insert("tray:playPause".into(), Box::new(|app| remote_execute(app, "playPause", vec![])));
    app.menu_actions.insert("tray:previous".into(), Box::new(|app| remote_execute(app, "previous", vec![])));
    app.menu_actions.insert("tray:next".into(), Box::new(|app| remote_execute(app, "next", vec![])));
    app.menu_actions.insert("tray:quit".into(), Box::new(|app| app.quit()));

    let file = tray_icon_file_name(app).to_string();
    let mut builder = TrayIconBuilder::new().with_id("ytmrs").with_menu(Box::new(menu.clone())).with_tooltip("YtmRs").with_title("YtmRs").with_menu_on_left_click(false);
    if let Some(icon) = load_icon(&file) {
        builder = builder.with_icon(icon);
    }
    match builder.build() {
        Ok(icon) => {
            app.ui.tray = Some(Tray { icon, _menu: menu, current_file: file });
            log::info!("Created tray icon");
        }
        Err(err) => log::warn!("Failed to create tray icon: {err}"),
    }
}

/// System color scheme or tray style changed
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
        if miniplayer::is_open(app) {
            miniplayer::close(app);
        } else {
            main_window::show_and_focus(app);
        }
    }
}
