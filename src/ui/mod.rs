//! Native GTK interface. YouTube Music (inside the main window) is the only web content.

pub mod changelog;
pub mod companion_auth;
pub mod lyrics;
pub mod main_window;
pub mod miniplayer;
pub mod settings;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::app::{App, TimerHandle};
use crate::services::memory_store::MemoryChange;
use crate::services::protected_api::remote_execute;
use crate::shared::player_state::{PlayerState, VideoState};

#[derive(Default)]
pub struct Ui {
    pub main: Option<main_window::MainWindow>,
    pub settings: Option<settings::SettingsWindow>,
    pub miniplayer: Option<miniplayer::Miniplayer>,
    pub lyrics: Option<lyrics::LyricsWindow>,
    pub changelog: Option<gtk::Window>,
    pub auth_windows: HashMap<String, gtk::Window>,
    pub tray: Option<crate::tray::Tray>,
    pub login_item_enabled: Option<bool>,
    pub sleep_timer: Option<(TimerHandle, Instant)>,
    /// System color scheme (tray icon style "Auto")
    pub dark_theme: bool,
    pub startup_continued: bool,
    pub timed_lyrics: serde_json::Value,
}

const CSS: &str = r#"
.ytmrs-loading { background-color: #000000; color: #ffffff; }
.ytmrs-loading .status { font-size: 15px; opacity: 0.8; }
.ytmrs-loading .error { color: #ff6b6b; }
.ytmrs-unresponsive { background-color: rgba(127, 127, 127, 0.5); }
.ytmrs-miniplayer { background-color: #000000; color: #ffffff; }
.ytmrs-miniplayer .title { font-weight: bold; text-shadow: 0 1px 3px rgba(0, 0, 0, 0.9); }
.ytmrs-miniplayer .message { opacity: 0.8; }
.ytmrs-miniplayer button { color: #ffffff; text-shadow: 0 1px 3px rgba(0, 0, 0, 0.9); -gtk-icon-shadow: 0 1px 3px rgba(0, 0, 0, 0.9); }
.ytmrs-miniplayer .controls { background-image: linear-gradient(to bottom, rgba(0, 0, 0, 0), rgba(0, 0, 0, 0.85)); }
.ytmrs-miniplayer .topbar { background-image: linear-gradient(to top, rgba(0, 0, 0, 0), rgba(0, 0, 0, 0.6)); }
.ytmrs-lyrics { background-color: #000000; }
.ytmrs-lyrics .line { color: rgba(255, 255, 255, 0.5); font-size: 20px; font-weight: bold; padding: 6px 12px; }
.ytmrs-lyrics .line.active { color: #ffffff; font-size: 23px; }
.ytmrs-lyrics .source { color: rgba(255, 255, 255, 0.5); padding: 12px; }
.ytmrs-lyrics .empty { color: rgba(255, 255, 255, 0.6); font-size: 16px; }
.ytmrs-settings .description { opacity: 0.7; font-size: 90%; }
.ytmrs-settings .disabled-message { color: #ff6b6b; font-size: 90%; }
.ytmrs-settings .beta { color: #ffb74d; font-size: 80%; font-weight: bold; }
.ytmrs-settings .ok { color: #4caf50; }
.ytmrs-settings .fail { color: #ff1100; }
"#;

/// GTK theming for the whole app (YouTube Music is dark only, so is the app)
pub fn init_style() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let provider = gtk::CssProvider::new();
    if let Err(err) = provider.load_from_data(CSS.as_bytes()) {
        log::warn!("Failed to load the app stylesheet: {err}");
    }
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(&screen, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    gtk::Window::set_default_icon_list(&[crate::assets::icon_pixbuf("ytmrs.png", Some(256))].into_iter().flatten().collect::<Vec<_>>());
}

/// Image of the first icon of `names` available in the icon theme
pub fn icon(names: &[&str]) -> gtk::Image {
    let theme = gtk::IconTheme::default();
    let name = names.iter().copied().find(|n| theme.as_ref().map(|t| t.has_icon(n)).unwrap_or(false)).unwrap_or(names[0]);
    gtk::Image::from_icon_name(Some(name), gtk::IconSize::Button)
}

pub fn icon_button(names: &[&str], tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_image(Some(&icon(names)));
    button.set_tooltip_text(Some(tooltip));
    button.set_focus_on_click(false);
    button
}

/// Link that opens in the system browser through [`crate::util::open_external`]
pub fn link_button(uri: &str, label: &str) -> gtk::LinkButton {
    let button = gtk::LinkButton::with_label(uri, label);
    button.connect_activate_link(|button| {
        if let Some(uri) = button.uri() {
            crate::util::open_external(&uri);
        }
        glib::Propagation::Stop
    });
    button
}

pub fn format_time(seconds: f64) -> String {
    let s = if seconds.is_finite() && seconds > 0.0 { seconds as u64 } else { 0 };
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn on_player_state(app: &mut App, state: &PlayerState) {
    let title = match (&state.video_details, state.has_full_metadata) {
        (Some(d), true) => format!("{} - {} | YtmRs", d.title(), d.author()),
        _ => "YtmRs".to_string(),
    };
    main_window::set_title(app, &title);
    miniplayer::update(app, state);
    lyrics::update_progress(app, state);
}

pub fn on_memory_changed(app: &mut App, change: &MemoryChange) {
    if change.new.get("ytmViewUnresponsive") != change.old.get("ytmViewUnresponsive") {
        main_window::update_loading(app);
    }
    settings::sync(app);
}

/// Sleep timer (originally a YTM player bar extra): pauses playback after `minutes`
pub fn set_sleep_timer(app: &mut App, minutes: u32) {
    if let Some((timer, _)) = app.ui.sleep_timer.take() {
        timer.cancel();
    }
    if minutes > 0 {
        let timer = app.set_timeout(Duration::from_secs(minutes as u64 * 60), |app| {
            app.ui.sleep_timer = None;
            main_window::sync_sleep_timer(app);
            let state = app.player_state_store.state();
            // An ad reports an unknown track state: everything but "paused" counts as playing
            if state.video_details.is_some() && state.track_state != VideoState::Paused {
                remote_execute(app, "pause", vec![]);
                app.rt.spawn_blocking(|| {
                    let _ = notify_rust::Notification::new()
                        .appname("YtmRs")
                        .summary("Music paused")
                        .body("Sleep timer expired and your music has been paused")
                        .icon("ytmrs")
                        .show();
                });
            }
        });
        app.ui.sleep_timer = Some((timer, Instant::now() + Duration::from_secs(minutes as u64 * 60)));
    }
    main_window::sync_sleep_timer(app);
}

/// Every top level window (parents for dialogs)
pub fn main_gtk_window(app: &App) -> Option<gtk::Window> {
    app.ui.main.as_ref().map(|m| m.window.clone())
}

pub fn destroy_all(app: &mut App) {
    if let Some((timer, _)) = app.ui.sleep_timer.take() {
        timer.cancel();
    }
    for (_, w) in app.ui.auth_windows.drain() {
        unsafe { w.destroy() };
    }
    if let Some(w) = app.ui.changelog.take() {
        unsafe { w.destroy() };
    }
    if let Some(s) = app.ui.settings.take() {
        unsafe { s.window.destroy() };
    }
    if let Some(m) = app.ui.miniplayer.take() {
        unsafe { m.window.destroy() };
    }
    if let Some(l) = app.ui.lyrics.take() {
        unsafe { l.window.destroy() };
    }
    if let Some(m) = app.ui.main.take() {
        unsafe { m.window.destroy() };
    }
    app.ui.tray = None;
}

/// Window bounds as stored in the `state` config section
pub fn window_bounds(window: &gtk::Window) -> serde_json::Value {
    let (x, y) = window.position();
    let (width, height) = window.size();
    serde_json::json!({ "x": x, "y": y, "width": width, "height": height })
}

pub fn restore_bounds(window: &gtk::Window, bounds: &serde_json::Value, default: (i32, i32)) {
    let get = |k: &str| bounds.get(k).and_then(|v| v.as_i64()).map(|v| v as i32);
    match (get("width"), get("height")) {
        (Some(w), Some(h)) if w > 0 && h > 0 => window.set_default_size(w, h),
        _ => window.set_default_size(default.0, default.1),
    }
    // Ignored on Wayland, where the compositor places windows
    if let (Some(x), Some(y)) = (get("x"), get("y")) {
        window.move_(x, y);
    }
}

/// Simple action helper: activation is forwarded to the App on the main loop
pub fn add_action(group: &gio::SimpleActionGroup, main: &crate::app::MainHandle, name: &str, f: fn(&mut App)) {
    let action = gio::SimpleAction::new(name, None);
    let main = main.clone();
    action.connect_activate(move |_, _| main.run(f));
    group.add_action(&action);
}

