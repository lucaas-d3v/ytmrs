//! Main window: native header bar, the YouTube Music view and the loading overlay
//! (port of windows/main, the titlebar and components/YTMViewLoading).

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::Value;

use super::{add_action, icon, icon_button};
use crate::app::{App, TimerHandle};
use crate::services::{state_manager, ytm_view_manager};
use crate::shared::types::YTMViewStatus;

pub struct MainWindow {
    pub window: gtk::Window,
    header: gtk::HeaderBar,
    overlay: gtk::Overlay,
    actions: gio::SimpleActionGroup,
    loading: gtk::Revealer,
    spinner: gtk::Spinner,
    status: gtk::Label,
    error_box: gtk::Box,
    error: gtk::Label,
    timeout: gtk::Label,
    unresponsive: gtk::Revealer,
    timeout_timer: Option<TimerHandle>,
    timed_out: bool,
    maximized: bool,
    fullscreen: bool,
}

const SLEEP_TIMER_OPTIONS: [(&str, i32); 7] =
    [("Off", 0), ("5 minutes", 5), ("10 minutes", 10), ("15 minutes", 15), ("30 minutes", 30), ("45 minutes", 45), ("1 hour", 60)];

fn menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    let player = gio::Menu::new();
    player.append(Some("Miniplayer"), Some("win.miniplayer"));
    player.append(Some("Lyrics"), Some("win.lyrics"));
    let sleep = gio::Menu::new();
    for (label, minutes) in SLEEP_TIMER_OPTIONS {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.sleep-timer"), Some(&minutes.to_variant()));
        sleep.append_item(&item);
    }
    player.append_submenu(Some("Sleep timer"), &sleep);
    menu.append_section(None, &player);
    let app_section = gio::Menu::new();
    app_section.append(Some("Reload"), Some("win.reload"));
    app_section.append(Some("Settings"), Some("win.settings"));
    menu.append_section(None, &app_section);
    let quit = gio::Menu::new();
    quit.append(Some("Quit"), Some("win.quit"));
    menu.append_section(None, &quit);
    menu
}

pub fn create(app: &mut App) {
    let main = app.main.clone();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("YtmRs");
    window.set_size_request(156, 180);
    super::restore_bounds(&window, &app.config_store.get("state.windowBounds"), (1280, 720));
    let maximized = app.config_store.get_bool("state.windowMaximized");
    if maximized {
        window.maximize();
    }

    //#region Header bar
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    header.set_title(Some("YtmRs"));
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    nav.style_context().add_class("linked");
    let back = icon_button(&["go-previous-symbolic"], "Back");
    back.set_action_name(Some("win.back"));
    let forward = icon_button(&["go-next-symbolic"], "Forward");
    forward.set_action_name(Some("win.forward"));
    nav.add(&back);
    nav.add(&forward);
    header.pack_start(&nav);
    let home = icon_button(&["go-home-symbolic", "user-home-symbolic"], "Home");
    home.set_action_name(Some("win.home"));
    header.pack_start(&home);

    let menu_button = gtk::MenuButton::new();
    menu_button.set_image(Some(&icon(&["open-menu-symbolic"])));
    menu_button.set_menu_model(Some(&menu_model()));
    menu_button.set_tooltip_text(Some("Menu"));
    header.pack_end(&menu_button);
    let settings = icon_button(&["emblem-system-symbolic", "preferences-system-symbolic"], "Settings");
    settings.set_action_name(Some("win.settings"));
    header.pack_end(&settings);
    let miniplayer = icon_button(&["view-restore-symbolic", "window-restore-symbolic", "view-paged-symbolic"], "Miniplayer");
    miniplayer.set_action_name(Some("win.miniplayer"));
    header.pack_end(&miniplayer);
    window.set_titlebar(Some(&header));

    //#region Actions
    let actions = gio::SimpleActionGroup::new();
    add_action(&actions, &main, "back", |app| ytm_view_manager::go_back(app));
    add_action(&actions, &main, "forward", |app| ytm_view_manager::go_forward(app));
    add_action(&actions, &main, "home", |app| ytm_view_manager::load_default(app));
    add_action(&actions, &main, "reload", |app| ytm_view_manager::retry(app));
    add_action(&actions, &main, "settings", super::settings::open);
    add_action(&actions, &main, "miniplayer", super::miniplayer::open);
    add_action(&actions, &main, "quit", |app| app.quit());
    let lyrics = gio::SimpleAction::new_stateful("lyrics", None, &false.to_variant());
    let m = main.clone();
    lyrics.connect_activate(move |_, _| m.run(super::lyrics::toggle));
    actions.add_action(&lyrics);
    let sleep = gio::SimpleAction::new_stateful("sleep-timer", Some(glib::VariantTy::INT32), &0i32.to_variant());
    let m = main.clone();
    sleep.connect_activate(move |_, param| {
        let minutes = param.and_then(|p| p.get::<i32>()).unwrap_or(0).max(0) as u32;
        m.run(move |app| super::set_sleep_timer(app, minutes));
    });
    actions.add_action(&sleep);
    window.insert_action_group("win", Some(&actions));

    //#region Content: YTM view + loading overlay
    let overlay = gtk::Overlay::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Center);
    if let Some(logo) = crate::assets::icon_pixbuf("ytmrs.png", Some(128)) {
        content.add(&gtk::Image::from_pixbuf(Some(&logo)));
    }
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(32, 32);
    spinner.start();
    content.add(&spinner);
    let status = gtk::Label::new(Some("Loading YouTube Music..."));
    status.style_context().add_class("status");
    content.add(&status);
    let error_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let error = gtk::Label::new(None);
    error.set_line_wrap(true);
    error.set_justify(gtk::Justification::Center);
    error.style_context().add_class("status");
    error.style_context().add_class("error");
    error_box.add(&error);
    let refresh = gtk::Button::with_label("Refresh YouTube Music");
    refresh.set_halign(gtk::Align::Center);
    refresh.set_action_name(Some("win.reload"));
    error_box.add(&refresh);
    content.add(&error_box);
    let timeout = gtk::Label::new(Some("YouTube Music is taking longer than usual to load"));
    timeout.style_context().add_class("status");
    timeout.set_no_show_all(true);
    content.add(&timeout);
    let loading_bg = gtk::EventBox::new();
    loading_bg.style_context().add_class("ytmrs-loading");
    loading_bg.add(&content);
    let loading = gtk::Revealer::new();
    loading.set_transition_type(gtk::RevealerTransitionType::Crossfade);
    loading.set_transition_duration(250);
    loading.add(&loading_bg);
    loading.set_reveal_child(true);
    overlay.add_overlay(&loading);

    let unresponsive_bg = gtk::EventBox::new();
    unresponsive_bg.style_context().add_class("ytmrs-unresponsive");
    let unresponsive = gtk::Revealer::new();
    unresponsive.set_transition_type(gtk::RevealerTransitionType::Crossfade);
    unresponsive.set_transition_duration(250);
    unresponsive.add(&unresponsive_bg);
    overlay.add_overlay(&unresponsive);
    window.add(&overlay);

    //#region Window events
    let m = main.clone();
    window.connect_delete_event(move |_, _| {
        m.run(on_close_requested);
        glib::Propagation::Stop
    });
    let m = main.clone();
    window.connect_configure_event(move |_, _| {
        m.run(save_bounds);
        false
    });
    let m = main.clone();
    window.connect_window_state_event(move |_, event| {
        let state = event.new_window_state();
        let maximized = state.contains(gtk::gdk::WindowState::MAXIMIZED);
        let fullscreen = state.contains(gtk::gdk::WindowState::FULLSCREEN);
        m.run(move |app| {
            let Some(w) = app.ui.main.as_mut() else { return };
            w.fullscreen = fullscreen;
            if w.maximized != maximized {
                w.maximized = maximized;
                state_manager::update_state(app, vec![("windowMaximized", Value::from(maximized))]);
            }
        });
        glib::Propagation::Proceed
    });
    // Alt+Left / Alt+Right history navigation
    let m = main;
    window.connect_key_press_event(move |_, event| {
        if event.state().contains(gtk::gdk::ModifierType::MOD1_MASK) {
            match event.keyval() {
                gtk::gdk::keys::constants::Left => m.run(|app| ytm_view_manager::go_back(app)),
                gtk::gdk::keys::constants::Right => m.run(|app| ytm_view_manager::go_forward(app)),
                _ => return glib::Propagation::Proceed,
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });

    overlay.show_all();
    // A revealer that is not revealed still covers the page and swallows its clicks:
    // hide it once the fade out ends
    for revealer in [&loading, &unresponsive] {
        revealer.connect_child_revealed_notify(|r| {
            if !r.is_child_revealed() && !r.reveals_child() {
                r.hide();
            }
        });
    }
    unresponsive.hide();
    error_box.hide();
    header.show_all();
    app.ui.main = Some(MainWindow {
        window,
        header,
        overlay,
        actions,
        loading,
        spinner,
        status,
        error_box,
        error,
        timeout,
        unresponsive,
        timeout_timer: None,
        timed_out: false,
        maximized,
        fullscreen: false,
    });
    restart_loading_timeout(app);
}

/// Places the YouTube Music view below the loading overlay
pub fn attach_view(app: &mut App, view: &webkit2gtk::WebView) {
    let Some(w) = app.ui.main.as_ref() else { return };
    w.overlay.add(view);
    view.show();
    view.grab_focus();
}

pub fn show(app: &mut App) {
    if let Some(w) = app.ui.main.as_ref() {
        w.window.show();
    }
}

pub fn show_and_focus(app: &mut App) {
    if let Some(w) = app.ui.main.as_ref() {
        w.window.show();
        w.window.deiconify();
        w.window.present();
    }
}

pub fn hide(app: &mut App) {
    if let Some(w) = app.ui.main.as_ref() {
        w.window.hide();
    }
}

pub fn is_visible(app: &App) -> bool {
    app.ui.main.as_ref().map(|w| w.window.is_visible()).unwrap_or(false)
}

pub fn set_title(app: &mut App, title: &str) {
    if let Some(w) = app.ui.main.as_ref() {
        if w.window.title().as_deref() != Some(title) {
            w.window.set_title(title);
            w.header.set_title(Some(title));
        }
    }
}

/// HTML fullscreen (videos): the header bar is hidden
pub fn set_fullscreen(app: &mut App, fullscreen: bool) {
    if let Some(w) = app.ui.main.as_ref() {
        w.header.set_visible(!fullscreen);
        if fullscreen {
            w.window.fullscreen();
        } else {
            w.window.unfullscreen();
        }
    }
}

pub fn update_history_buttons(app: &mut App) {
    let (Some(w), Some(view)) = (app.ui.main.as_ref(), app.ytm_view_manager.view()) else { return };
    use webkit2gtk::WebViewExt;
    let set = |name: &str, enabled: bool| {
        if let Some(action) = w.actions.lookup_action(name).and_then(|a| a.downcast::<gio::SimpleAction>().ok()) {
            action.set_enabled(enabled);
        }
    };
    set("back", view.can_go_back());
    set("forward", view.can_go_forward());
}

pub fn sync_lyrics_toggle(app: &App) {
    if let Some(action) = app.ui.main.as_ref().and_then(|w| w.actions.lookup_action("lyrics")) {
        action.change_state(&app.ui.lyrics.is_some().to_variant());
    }
}

pub fn sync_sleep_timer(app: &App) {
    let Some(w) = app.ui.main.as_ref() else { return };
    let minutes = app.ui.sleep_timer.as_ref().map(|(_, deadline)| {
        let left = deadline.saturating_duration_since(std::time::Instant::now()).as_secs_f64() / 60.0;
        // The selected option (the closest one)
        SLEEP_TIMER_OPTIONS.iter().map(|(_, m)| *m).filter(|m| *m > 0).min_by(|a, b| (*a as f64 - left).abs().total_cmp(&(*b as f64 - left).abs())).unwrap_or(0)
    });
    if let Some(action) = w.actions.lookup_action("sleep-timer") {
        action.change_state(&minutes.unwrap_or(0).to_variant());
    }
}

fn on_close_requested(app: &mut App) {
    save_bounds(app);
    if app.config_store.get_bool("general.hideToTrayOnClose") {
        hide(app);
    } else {
        app.quit();
    }
}

pub fn save_bounds(app: &mut App) {
    let Some(w) = app.ui.main.as_ref() else { return };
    // The maximized/fullscreen size is not the window size to restore
    if w.maximized || w.fullscreen || !w.window.is_visible() {
        return;
    }
    let bounds = super::window_bounds(&w.window);
    state_manager::update_state(app, vec![("windowBounds", bounds)]);
}

fn restart_loading_timeout(app: &mut App) {
    let timer = app.set_timeout(Duration::from_secs(5), |app| {
        if let Some(w) = app.ui.main.as_mut() {
            w.timed_out = true;
        }
        update_loading(app);
    });
    if let Some(w) = app.ui.main.as_mut() {
        if let Some(old) = w.timeout_timer.replace(timer) {
            old.cancel();
        }
        w.timed_out = false;
    }
}

/// YTMViewLoading: reflects the YTM view status (loading, hooking, load error, unresponsive)
pub fn on_view_status_changed(app: &mut App) {
    if app.ytm_view_manager.status() != YTMViewStatus::Ready {
        restart_loading_timeout(app);
    } else if let Some(t) = app.ui.main.as_mut().and_then(|w| w.timeout_timer.take()) {
        t.cancel();
    }
    update_loading(app);
}

pub fn update_loading(app: &mut App) {
    let status = app.ytm_view_manager.status();
    let unresponsive = app.memory_store.get_bool("ytmViewUnresponsive");
    let load_error = if status == YTMViewStatus::LoadFailed { app.ytm_view_manager.load_error.clone() } else { None };
    let Some(w) = app.ui.main.as_ref() else { return };
    for (revealer, reveal) in [(&w.loading, status != YTMViewStatus::Ready), (&w.unresponsive, unresponsive)] {
        if reveal {
            revealer.show();
        }
        revealer.set_reveal_child(reveal);
    }
    w.status.set_text(match status {
        YTMViewStatus::Loading => "Loading YouTube Music...",
        YTMViewStatus::Hooking => "Waiting for YtmRs hooks...",
        YTMViewStatus::Ready => "Ready",
        YTMViewStatus::LoadFailed => "",
    });
    w.status.set_visible(load_error.is_none());
    w.spinner.set_visible(load_error.is_none());
    match &load_error {
        Some((code, description)) => {
            w.error.set_text(&format!("Failed to load YouTube Music: {description} ({code})"));
            w.error_box.show_all();
        }
        None => w.error_box.hide(),
    }
    w.timeout.set_visible(w.timed_out && load_error.is_none() && status != YTMViewStatus::Ready);
}
