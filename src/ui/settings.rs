//! Native settings window (port of windows/settings)

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::Value;

use super::{icon, icon_button};
use crate::app::{App, MainHandle};
use crate::services::{config_store, memory_store};

type Updater = Box<dyn Fn(&App)>;

pub struct SettingsWindow {
    pub window: gtk::Window,
    updaters: Vec<Updater>,
    syncing: Rc<Cell<bool>>,
}

const SAFE_STORAGE_UNAVAILABLE: &str = "This integration cannot be enabled due to safeStorage being unavailable";

#[derive(Default, Clone, Copy)]
struct Opts {
    description: Option<&'static str>,
    indented: bool,
    restart: bool,
    beta: bool,
    /// (predicate, message)
    disabled: Option<(fn(&App) -> bool, &'static str)>,
    show_if: Option<fn(&App) -> bool>,
}

struct Builder {
    main: MainHandle,
    updaters: Vec<Updater>,
    syncing: Rc<Cell<bool>>,
    restart_bar: gtk::InfoBar,
    capture: Rc<RefCell<Option<(&'static str, gtk::Button)>>>,
}

fn page() -> (gtk::ScrolledWindow, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let scrolled = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.add(&content);
    (scrolled, content)
}

fn label(text: &str, class: Option<&str>) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_line_wrap(true);
    if let Some(c) = class {
        l.style_context().add_class(c);
    }
    l
}

impl Builder {
    fn on_change(&self, restart: bool) -> impl Fn() + 'static {
        let bar = self.restart_bar.clone();
        move || {
            if restart {
                bar.set_revealed(true);
                bar.show_all();
            }
        }
    }

    /// YTMDSetting: name, tags, description, disabled message and the control
    fn row(&mut self, page: &gtk::Box, name: &str, opts: Opts, control: Option<&gtk::Widget>) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        if opts.indented {
            row.set_margin_start(32);
        }
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        title.add(&label(name, None));
        if opts.restart {
            let i = icon(&["view-refresh-symbolic"]);
            i.set_tooltip_text(Some("Requires a restart"));
            title.add(&i);
        }
        if opts.beta {
            let b = label("BETA", Some("beta"));
            b.set_tooltip_text(Some("This is a beta feature and may not work correctly yet."));
            title.add(&b);
        }
        text.add(&title);
        if let Some(d) = opts.description.filter(|d| !d.trim().is_empty()) {
            text.add(&label(d, Some("description")));
        }
        if let Some((predicate, message)) = opts.disabled {
            let m = label(&format!("DISABLED: {message}"), Some("disabled-message"));
            text.add(&m);
            let control = control.cloned();
            self.updaters.push(Box::new(move |app| {
                let disabled = predicate(app);
                m.set_visible(disabled);
                if let Some(c) = &control {
                    c.set_sensitive(!disabled);
                }
            }));
        }
        row.add(&text);
        if let Some(c) = control {
            c.set_valign(gtk::Align::Center);
            row.add(c);
        }
        page.add(&row);
        if let Some(show_if) = opts.show_if {
            let r = row.clone();
            self.updaters.push(Box::new(move |app| r.set_visible(show_if(app))));
        }
        row
    }

    fn switch(&mut self, page: &gtk::Box, path: &'static str, name: &str, opts: Opts) -> gtk::Box {
        let sw = gtk::Switch::new();
        let (main, syncing, changed) = (self.main.clone(), self.syncing.clone(), self.on_change(opts.restart));
        sw.connect_active_notify(move |sw| {
            if syncing.get() {
                return;
            }
            let active = sw.is_active();
            main.run(move |app| config_store::set(app, path, Value::from(active)));
            changed();
        });
        let s = sw.clone();
        self.updaters.push(Box::new(move |app| s.set_active(app.config_store.get_bool(path))));
        self.row(page, name, opts, Some(sw.upcast_ref()))
    }

    fn memory_switch(&mut self, page: &gtk::Box, key: &'static str, name: &str, opts: Opts) -> gtk::Box {
        let sw = gtk::Switch::new();
        let (main, syncing) = (self.main.clone(), self.syncing.clone());
        sw.connect_active_notify(move |sw| {
            if syncing.get() {
                return;
            }
            let active = sw.is_active();
            main.run(move |app| memory_store::set(app, key, active));
        });
        let s = sw.clone();
        self.updaters.push(Box::new(move |app| s.set_active(app.memory_store.get_bool(key))));
        self.row(page, name, opts, Some(sw.upcast_ref()))
    }

    fn range(&mut self, page: &gtk::Box, path: &'static str, name: &str, (min, max, step): (f64, f64, f64), opts: Opts) -> gtk::Box {
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
        scale.set_digits(0);
        scale.set_value_pos(gtk::PositionType::Right);
        scale.set_size_request(220, -1);
        let (main, syncing) = (self.main.clone(), self.syncing.clone());
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        scale.connect_value_changed(move |scale| {
            if syncing.get() {
                return;
            }
            let value = (scale.value() / step).round() * step;
            // Debounced: every change is written to disk
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (main, p) = (main.clone(), pending.clone());
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                p.borrow_mut().take();
                main.run(move |app| config_store::set(app, path, Value::from(value as i64)));
            });
            *pending.borrow_mut() = Some(id);
        });
        let s = scale.clone();
        self.updaters.push(Box::new(move |app| {
            if let Some(v) = app.config_store.get_f64(path) {
                s.set_value(v);
            }
        }));
        self.row(page, name, opts, Some(scale.upcast_ref()))
    }

    fn select(&mut self, page: &gtk::Box, path: &'static str, name: &str, options: &[(i64, &str)], opts: Opts) -> gtk::Box {
        let combo = gtk::ComboBoxText::new();
        for (id, text) in options {
            combo.append(Some(&id.to_string()), text);
        }
        let (main, syncing) = (self.main.clone(), self.syncing.clone());
        combo.connect_changed(move |combo| {
            if syncing.get() {
                return;
            }
            if let Some(id) = combo.active_id().and_then(|id| id.parse::<i64>().ok()) {
                main.run(move |app| config_store::set(app, path, Value::from(id)));
            }
        });
        let c = combo.clone();
        self.updaters.push(Box::new(move |app| {
            let id = app.config_store.get(path).as_i64().unwrap_or(0).to_string();
            c.set_active_id(Some(&id));
        }));
        self.row(page, name, opts, Some(combo.upcast_ref()))
    }

    fn button_row(&mut self, page: &gtk::Box, text: &str, button: &str, f: fn(&mut App), opts: Opts) -> gtk::Box {
        let b = gtk::Button::with_label(button);
        let main = self.main.clone();
        b.connect_clicked(move |_| main.run(f));
        self.row(page, text, opts, Some(b.upcast_ref()))
    }

    /// Keybind capture (port of the keybind input)
    fn keybind(&mut self, page: &gtk::Box, path: &'static str, title: &str, failed_flag: &'static str) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let name = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        name.set_hexpand(true);
        name.add(&label(title, None));
        let error = icon(&["dialog-warning-symbolic"]);
        error.set_tooltip_text(Some("Failed to register keybind. Does another application have this keybind?"));
        name.add(&error);
        row.add(&name);
        let button = gtk::Button::new();
        button.set_size_request(200, -1);
        let capture = self.capture.clone();
        button.connect_clicked(move |b| {
            b.set_label("Press a key combination...");
            *capture.borrow_mut() = Some((path, b.clone()));
        });
        row.add(&button);
        let remove = icon_button(&["edit-delete-symbolic", "user-trash-symbolic"], "Remove keybind");
        let main = self.main.clone();
        remove.connect_clicked(move |_| main.run(move |app| config_store::set(app, path, Value::from(""))));
        row.add(&remove);
        page.add(&row);
        self.updaters.push(Box::new(move |app| {
            let current = app.config_store.get_str(path).filter(|s| !s.is_empty());
            button.set_label(current.as_deref().unwrap_or("None"));
            remove.set_sensitive(current.is_some());
            error.set_visible(app.memory_store.get_bool(failed_flag));
        }));
    }
}

fn companion_enabled(app: &App) -> bool {
    app.config_store.get_bool("integrations.companionServerEnabled") && app.memory_store.get_bool("safeStorageAvailable")
}

fn safe_storage_unavailable(app: &App) -> bool {
    !app.memory_store.get_bool("safeStorageAvailable")
}

pub fn open(app: &mut App) {
    if let Some(s) = app.ui.settings.as_ref() {
        s.window.present();
        return;
    }
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("YtmRs - Settings");
    window.set_default_size(800, 600);
    window.set_modal(true);
    window.set_destroy_with_parent(true);
    window.set_type_hint(gdk::WindowTypeHint::Dialog);
    if let Some(parent) = super::main_gtk_window(app) {
        window.set_transient_for(Some(&parent));
        window.set_position(gtk::WindowPosition::CenterOnParent);
    }
    window.style_context().add_class("ytmrs-settings");
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    header.set_title(Some("Settings"));
    window.set_titlebar(Some(&header));

    let restart_bar = gtk::InfoBar::new();
    restart_bar.set_message_type(gtk::MessageType::Info);
    restart_bar.content_area().add(&gtk::Label::new(Some("Some changes require a restart to take effect")));
    restart_bar.add_button("Restart application", gtk::ResponseType::Accept);
    let main = app.main.clone();
    restart_bar.connect_response(move |_, _| main.run(|app| app.relaunch()));
    restart_bar.set_revealed(false);
    restart_bar.set_no_show_all(true);

    let mut b = Builder {
        main: app.main.clone(),
        updaters: Vec::new(),
        syncing: Rc::new(Cell::new(false)),
        restart_bar: restart_bar.clone(),
        capture: Rc::default(),
    };
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let o = Opts::default();

    //#region General
    let (scrolled, p) = page();
    b.switch(&p, "general.hideToTrayOnClose", "Hide to tray on close", o);
    b.switch(&p, "general.showNotificationOnSongChange", "Show notification on song change", o);
    b.switch(&p, "general.startOnBoot", "Start on boot", o);
    b.switch(&p, "general.disableHardwareAcceleration", "Disable hardware acceleration", Opts { restart: true, ..o });
    b.switch(
        &p,
        "general.lowMemoryMode",
        "Low memory mode",
        Opts { restart: true, description: Some("Runs YouTube Music without the JavaScript JIT compiler: about 30% less memory, more processor time while the page is busy"), ..o },
    );
    stack.add_titled(&scrolled, "general", "General");

    //#region Appearance
    let (scrolled, p) = page();
    b.switch(&p, "appearance.alwaysShowVolumeSlider", "Always show volume slider", Opts { description: Some("This feature only works on the old player bar and may not work for you"), ..o });
    b.switch(&p, "appearance.customCSSEnabled", "Custom CSS", o);
    {
        let path_label = label("No file chosen", Some("description"));
        path_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        path_label.set_max_width_chars(40);
        let choose = gtk::Button::with_label("Choose file");
        let main = b.main.clone();
        choose.connect_clicked(move |_| {
            main.run(move |app| {
                let parent = app.ui.settings.as_ref().map(|s| s.window.clone());
                crate::dialogs::pick_css_file(app, parent.as_ref(), |app, path| {
                    if let Some(path) = path {
                        config_store::set(app, "appearance.customCSSPath", Value::from(path.to_string_lossy().to_string()));
                    }
                })
            });
        });
        let control = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        control.add(&path_label);
        control.add(&choose);
        b.row(&p, "Custom CSS file path", Opts { indented: true, show_if: Some(|app| app.config_store.get_bool("appearance.customCSSEnabled")), ..o }, Some(control.upcast_ref()));
        b.updaters.push(Box::new(move |app| {
            path_label.set_text(app.config_store.get_str("appearance.customCSSPath").filter(|s| !s.is_empty()).as_deref().unwrap_or("No file chosen"));
        }));
    }
    b.range(&p, "appearance.zoom", "Zoom", (30.0, 300.0, 10.0), o);
    b.select(&p, "appearance.trayIconStyle", "Tray icon style", &[(0, "Auto"), (1, "White"), (2, "Black")], o);
    stack.add_titled(&scrolled, "appearance", "Appearance");

    //#region Playback
    let (scrolled, p) = page();
    b.switch(&p, "playback.continueWhereYouLeftOff", "Continue where you left off", Opts { description: Some("If you're signed in this feature can be turned off"), ..o });
    b.switch(&p, "playback.continueWhereYouLeftOffPaused", "Pause on application launch", Opts { indented: true, show_if: Some(|app| app.config_store.get_bool("playback.continueWhereYouLeftOff")), ..o });
    b.switch(&p, "playback.progressInTaskbar", "Show track progress on taskbar", o);
    b.switch(&p, "playback.ratioVolume", "Ratio volume", o);
    stack.add_titled(&scrolled, "playback", "Playback");

    //#region Integrations
    let (scrolled, p) = page();
    let safe = Some((safe_storage_unavailable as fn(&App) -> bool, SAFE_STORAGE_UNAVAILABLE));
    b.switch(&p, "integrations.companionServerEnabled", "Companion server", Opts { disabled: safe, ..o });
    b.switch(
        &p,
        "integrations.companionServerCORSWildcardEnabled",
        "Allow browser communication",
        Opts { indented: true, description: Some("This setting could be dangerous as it allows any website you visit to communicate with the companion server"), show_if: Some(companion_enabled), ..o },
    );
    b.memory_switch(
        &p,
        "companionServerAuthWindowEnabled",
        "Enable companion authorization",
        Opts { indented: true, description: Some("Automatically disables after the first successful authorization or 5 minutes has passed"), show_if: Some(companion_enabled), ..o },
    );
    {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        list.set_margin_start(32);
        list.add(&label("Authorized companions", None));
        list.add(&label("This is a list of companions that currently have access to the companion server", Some("description")));
        let rows = gtk::ListBox::new();
        rows.set_selection_mode(gtk::SelectionMode::None);
        let empty = label("No authorized companions", Some("description"));
        rows.set_placeholder(Some(&empty));
        empty.show();
        list.add(&rows);
        p.add(&list);
        let rendered: Rc<RefCell<Option<String>>> = Rc::default();
        let main = b.main.clone();
        b.updaters.push(Box::new(move |app| {
            list.set_visible(companion_enabled(app));
            let encrypted = app.config_store.get_str("integrations.companionServerAuthTokens");
            if *rendered.borrow() == encrypted && encrypted.is_some() {
                return;
            }
            *rendered.borrow_mut() = encrypted.clone();
            for child in rows.children() {
                rows.remove(&child);
            }
            let tokens = crate::integrations::companion_server::auth::decrypt_tokens(&app.safe_storage, encrypted.as_deref());
            for token in tokens {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                row.set_margin_top(6);
                row.set_margin_bottom(6);
                row.set_margin_start(6);
                row.set_margin_end(6);
                let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
                names.set_hexpand(true);
                names.add(&label(&token.app_name, None));
                names.add(&label(&token.app_id, Some("description")));
                row.add(&names);
                row.add(&label(&token.app_version, Some("description")));
                let delete = icon_button(&["edit-delete-symbolic", "user-trash-symbolic"], "Remove companion");
                let (main, app_id) = (main.clone(), token.app_id.clone());
                delete.connect_clicked(move |_| {
                    let app_id = app_id.clone();
                    main.run(move |app| delete_companion_token(app, &app_id));
                });
                row.add(&delete);
                row.show_all();
                rows.add(&row);
            }
        }));
    }

    b.switch(&p, "integrations.discordPresenceEnabled", "Discord rich presence", o);
    {
        let retry = gtk::Button::with_label("Retry");
        let main = b.main.clone();
        retry.connect_clicked(move |_| {
            main.run(|app| {
                config_store::set(app, "integrations.discordPresenceEnabled", Value::from(false));
                config_store::set(app, "integrations.discordPresenceEnabled", Value::from(true));
            })
        });
        let row = b.row(&p, "Discord connection could not be established after 30 attempts", Opts { indented: true, ..o }, Some(retry.upcast_ref()));
        b.updaters.push(Box::new(move |app| {
            row.set_visible(app.config_store.get_bool("integrations.discordPresenceEnabled") && app.memory_store.get_bool("discordPresenceConnectionFailed"))
        }));
    }

    b.switch(&p, "integrations.lastFMEnabled", "Last.fm scrobbling", Opts { disabled: safe, ..o });
    {
        let status = label("", Some("description"));
        let logout = gtk::Button::with_label("Logout");
        let main = b.main.clone();
        logout.connect_clicked(move |_| {
            main.run(|app| {
                config_store::set(app, "lastfm.sessionKey", Value::Null);
                config_store::set(app, "integrations.lastFMEnabled", Value::from(false));
            })
        });
        let control = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        control.add(&status);
        control.add(&logout);
        let row = b.row(&p, "User is authenticated", Opts { indented: true, show_if: Some(|app| app.config_store.get_bool("integrations.lastFMEnabled")), ..o }, Some(control.upcast_ref()));
        let _ = row;
        b.updaters.push(Box::new(move |app| {
            let authenticated = !app.config_store.get("lastfm.sessionKey").is_null();
            status.set_text(if authenticated { "Yes" } else { "No" });
            status.style_context().remove_class(if authenticated { "fail" } else { "ok" });
            status.style_context().add_class(if authenticated { "ok" } else { "fail" });
            logout.set_visible(authenticated);
        }));
        let step0: fn(&App) -> bool = |app| app.config_store.get_bool("integrations.lastFMEnabled") && app.memory_store.get("lastFMAuthenticationStep").as_i64() == Some(0);
        let step1: fn(&App) -> bool = |app| app.config_store.get_bool("integrations.lastFMEnabled") && app.memory_store.get("lastFMAuthenticationStep").as_i64() == Some(1);
        b.button_row(&p, "Continue setup in your browser then come back here when you're done", "Authorize Last.fm in browser", crate::integrations::last_fm::authenticate_step, Opts { indented: true, show_if: Some(step0), ..o });
        b.button_row(&p, "After you're done in the browser click \"Finish Last.fm authentication\"", "Finish Last.fm authentication", crate::integrations::last_fm::authenticate_step, Opts { indented: true, show_if: Some(step1), ..o });
        b.range(&p, "lastfm.scrobblePercent", "Scrobble percent", (50.0, 95.0, 5.0), Opts { indented: true, description: Some("Determines when a song is scrobbled"), show_if: Some(|app| app.config_store.get_bool("integrations.lastFMEnabled")), ..o });
    }
    b.switch(
        &p,
        "integrations.enhancedMediaServiceEnabled",
        "Enhanced media service",
        Opts { restart: true, beta: true, description: Some("An enhanced media service that can provide better media button controls"), ..o },
    );
    stack.add_titled(&scrolled, "integrations", "Integrations");

    //#region Shortcuts
    let (scrolled, p) = page();
    for (key, title, flag) in [
        ("shortcuts.playPause", "Play/Pause", "shortcutsPlayPauseRegisterFailed"),
        ("shortcuts.next", "Next", "shortcutsNextRegisterFailed"),
        ("shortcuts.previous", "Previous", "shortcutsPreviousRegisterFailed"),
        ("shortcuts.thumbsUp", "Thumbs Up", "shortcutsThumbsUpRegisterFailed"),
        ("shortcuts.thumbsDown", "Thumbs Down", "shortcutsThumbsDownRegisterFailed"),
        ("shortcuts.volumeUp", "Increase Volume", "shortcutsVolumeUpRegisterFailed"),
        ("shortcuts.volumeDown", "Decrease Volume", "shortcutsVolumeDownRegisterFailed"),
        ("shortcuts.volumeMute", "Mute Volume", "shortcutsVolumeMuteRegisterFailed"),
        ("shortcuts.toggleShuffle", "Toggle Shuffle", "shortcutsToggleShuffleRegisterFailed"),
        ("shortcuts.toggleRepeat", "Toggle Repeat", "shortcutsToggleRepeatRegisterFailed"),
    ] {
        b.keybind(&p, key, title, flag);
    }
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        p.add(&label("Global shortcuts are only available on X11 sessions (and XWayland applications).", Some("description")));
    }
    stack.add_titled(&scrolled, "shortcuts", "Shortcuts");

    //#region About
    let (scrolled, p) = page();
    p.set_halign(gtk::Align::Center);
    if let Some(logo) = crate::assets::icon_pixbuf("ytmrs.png", Some(96)) {
        p.add(&gtk::Image::from_pixbuf(Some(&logo)));
    }
    let name = gtk::Label::new(None);
    name.set_markup("<span size='x-large' weight='bold'>YtmRs</span>");
    p.add(&name);
    // SAFETY: plain version getters without arguments
    let webkit = unsafe { format!("WebKitGTK {}.{}.{}", webkit2gtk::ffi::webkit_get_major_version(), webkit2gtk::ffi::webkit_get_minor_version(), webkit2gtk::ffi::webkit_get_micro_version()) };
    let info = gtk::Box::new(gtk::Orientation::Vertical, 4);
    p.add(&info);
    for line in [
        format!("Version: {}", crate::APP_VERSION),
        format!("Branch: {}", option_env!("YTMD_GIT_BRANCH").unwrap_or("None")),
        format!("Commit: {}", option_env!("YTMD_GIT_COMMIT_HASH").map(|h| h.chars().take(7).collect::<String>()).unwrap_or_else(|| "None".into())),
        format!("Webview: {webkit}"),
    ] {
        let l = gtk::Label::new(Some(&line));
        l.style_context().add_class("description");
        info.add(&l);
    }
    let links = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    links.set_halign(gtk::Align::Center);
    links.add(&super::link_button("https://github.com/lucaas-d3v/ytmrs", "GitHub"));
    links.add(&super::link_button("https://github.com/ytmdesktop/ytmdesktop", "Based on YTMDesktop"));
    p.add(&links);
    let restart = gtk::Button::with_label("Restart application");
    restart.set_halign(gtk::Align::Center);
    let main = b.main.clone();
    restart.connect_clicked(move |_| main.run(|app| app.relaunch()));
    p.add(&restart);
    stack.add_titled(&scrolled, "about", "About");

    let sidebar = gtk::StackSidebar::new();
    sidebar.set_stack(&stack);
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.add(&sidebar);
    body.add(&gtk::Separator::new(gtk::Orientation::Vertical));
    stack.set_hexpand(true);
    body.add(&stack);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add(&restart_bar);
    body.set_vexpand(true);
    root.add(&body);
    window.add(&root);

    // Keybind capture
    let capture = b.capture.clone();
    let main = b.main.clone();
    window.connect_key_press_event(move |_, event| {
        let Some((path, _button)) = capture.borrow().clone() else { return glib::Propagation::Proceed };
        let Some(keybind) = keybind_from_event(event) else { return glib::Propagation::Stop };
        capture.borrow_mut().take();
        main.run(move |app| {
            config_store::set(app, path, Value::from(keybind));
            sync(app);
        });
        glib::Propagation::Stop
    });
    let capture = b.capture.clone();
    let main = b.main.clone();
    window.connect_button_press_event(move |_, _| {
        if capture.borrow_mut().take().is_some() {
            main.run(sync);
        }
        glib::Propagation::Proceed
    });
    let main = b.main.clone();
    window.connect_destroy(move |_| main.run(|app| app.ui.settings = None));

    window.show_all();
    let Builder { updaters, syncing, .. } = b;
    app.ui.settings = Some(SettingsWindow { window, updaters, syncing });
    sync(app);
}

/// Refresh every control from the config/memory stores
pub fn sync(app: &mut App) {
    let Some(s) = app.ui.settings.as_ref() else { return };
    s.syncing.set(true);
    for updater in &s.updaters {
        updater(app);
    }
    s.syncing.set(false);
}

fn delete_companion_token(app: &mut App, app_id: &str) {
    if !app.memory_store.get_bool("safeStorageAvailable") {
        return;
    }
    let encrypted = app.config_store.get_str("integrations.companionServerAuthTokens");
    let mut tokens = crate::integrations::companion_server::auth::decrypt_tokens(&app.safe_storage, encrypted.as_deref());
    tokens.retain(|t| t.app_id != app_id);
    match serde_json::to_string(&tokens).map_err(anyhow::Error::from).and_then(|json| app.safe_storage.encrypt_string(&json)) {
        Ok(encrypted) => config_store::set(app, "integrations.companionServerAuthTokens", Value::from(encrypted)),
        Err(err) => log::error!("Failed to remove companion token: {err}"),
    }
}

/// Electron accelerator string from a key press (same format as the original keybind input)
fn keybind_from_event(event: &gdk::EventKey) -> Option<String> {
    use gdk::keys::constants as k;
    let keyval = event.keyval();
    // Modifier only / disallowed keys keep capturing
    let disallowed = [
        k::Shift_L, k::Shift_R, k::Control_L, k::Control_R, k::Alt_L, k::Alt_R, k::Super_L, k::Super_R, k::Meta_L, k::Meta_R, k::ISO_Level3_Shift,
        k::Pause, k::Tab, k::ISO_Left_Tab, k::Menu, k::Cancel, k::AudioPlay, k::AudioPrev, k::AudioNext, k::AudioStop, k::AudioRaiseVolume,
        k::AudioLowerVolume, k::AudioMute, k::Caps_Lock, k::Num_Lock,
    ];
    if disallowed.contains(&keyval) {
        return None;
    }
    if keyval == k::Escape {
        return Some(String::new());
    }
    let state = event.state();
    // The unshifted key (Shift+1 is "Shift+1", not "Shift+!")
    let base = gdk::Display::default()
        .and_then(|d| gdk::Keymap::for_display(&d))
        .and_then(|keymap| keymap.translate_keyboard_state(event.hardware_keycode() as u32, gdk::ModifierType::empty(), event.group() as i32))
        .map(|(kv, ..)| gdk::keys::Key::from(kv))
        .unwrap_or(keyval);
    let name = base.name()?.to_string();
    let key = match name.as_str() {
        "space" => "Space".to_string(),
        "Return" | "KP_Enter" => "Enter".to_string(),
        "BackSpace" => "Backspace".to_string(),
        "Page_Up" => "PageUp".to_string(),
        "Page_Down" => "PageDown".to_string(),
        "KP_Add" => "NumAdd".to_string(),
        "KP_Subtract" => "NumSub".to_string(),
        "KP_Multiply" => "NumMult".to_string(),
        "KP_Divide" => "NumDiv".to_string(),
        "KP_Decimal" => "NumDec".to_string(),
        "plus" => "Plus".to_string(),
        "comma" => ",".to_string(),
        "period" => ".".to_string(),
        "slash" => "/".to_string(),
        "semicolon" => ";".to_string(),
        "apostrophe" => "'".to_string(),
        "bracketleft" => "[".to_string(),
        "bracketright" => "]".to_string(),
        "backslash" => "\\".to_string(),
        "minus" => "-".to_string(),
        "equal" => "=".to_string(),
        "grave" => "`".to_string(),
        n if n.starts_with("KP_") && n.len() == 4 && n.as_bytes()[3].is_ascii_digit() => format!("Num{}", &n[3..]),
        n if n.chars().count() == 1 => n.to_uppercase(),
        n => n.to_string(),
    };
    let mut keybind = String::new();
    if state.intersects(gdk::ModifierType::SUPER_MASK | gdk::ModifierType::MOD4_MASK) {
        keybind.push_str("Meta+");
    }
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        keybind.push_str("CmdOrCtrl+");
    }
    if state.contains(gdk::ModifierType::MOD1_MASK) {
        keybind.push_str("Alt+");
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        keybind.push_str("Shift+");
    }
    keybind.push_str(&key);
    Some(keybind)
}
