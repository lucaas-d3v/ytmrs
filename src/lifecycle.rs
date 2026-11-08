//! Port of `src/main/index.ts`: application startup, windows, tray and IPC glue.

use std::time::Duration;

use serde_json::{json, Value};

use crate::app::App;
use crate::dialogs::{self, Kind, MessageBox};
use crate::ipc::IpcReply;
use crate::services::service_host::{run_next_lifecycle, ServiceHostLifecycle};
use crate::services::window_manager::{self as wm, AppWindowOptions, NavPolicy, NewWindowPolicy};
use crate::services::{auto_updater, config_store, memory_store, state_manager, ytm_view_manager};
use crate::shared::types::{setup_flags, Rectangle, YTMViewStatus};

#[derive(Default)]
pub struct UiState {
    pub tray: Option<crate::tray::Tray>,
    pub context_menu: Option<muda::Menu>,
    pub login_item_enabled: Option<bool>,
    startup_continued: bool,
}

pub fn pre_ready(app: &mut App) {
    app.host.set_services(crate::services::collection());
    // At this stage the next lifecycle is PreInitialized
    run_next_lifecycle(app);
    app.integration_manager.create_integrations(crate::integrations::all());
    crate::services::integration_manager::run_integration_setups(app);
}

pub fn on_ready(app: &mut App) {
    log::info!("Application ready");
    crate::services::watchdog::set_app_ready();
    // At this stage the next lifecycle is Initialized
    run_next_lifecycle(app);
    install_shutdown_handler(app);

    //#region Updater Check
    let mut opts = AppWindowOptions::new("Updater", "updater", "preload/updater.js", 256.0, 320.0);
    opts.min_size = Some((256.0, 320.0));
    opts.resizable = false;
    opts.title = "YtmRs - Updater".into();
    if let Err(err) = wm::create_window(app, opts) {
        log::error!("Failed to create updater window: {err:?}");
    }

    if !app.auto_updater.is_disabled() {
        let forward = |channel: &'static str| move |app: &mut App, _: &()| app.window_manager.send("Updater", channel, &[]);
        let ids = [
            app.auto_updater.checking.once(forward("autoUpdater:checking")),
            app.auto_updater.available.once(forward("autoUpdater:available")),
            app.auto_updater.not_available.once(forward("autoUpdater:not-available")),
            app.auto_updater.downloaded.once(forward("autoUpdater:downloaded")),
            app.auto_updater.error.once(forward("autoUpdater:error")),
        ];
        let _ = ids;
        app.auto_updater.downloaded.once(|app, _| auto_updater::quit_and_install(app));
        app.auto_updater.not_available.once(|app, _| continue_startup(app));
        app.auto_updater.error.once(|app, _| continue_startup(app));
        auto_updater::check_for_updates(app);
    } else if !crate::paths::is_packaged() {
        // Better visual feedback for the updater window when in development mode
        app.set_timeout(Duration::from_millis(1500), continue_startup);
    } else {
        continue_startup(app);
    }
}

fn continue_startup(app: &mut App) {
    if app.ui.startup_continued {
        return;
    }
    app.ui.startup_continued = true;
    // At this stage the next lifecycle is PostInitialized
    run_next_lifecycle(app);

    crate::util::v1_config_migration(app);

    // safeStorage setup and checks (may talk to the keyring daemon)
    let main = app.main.clone();
    app.spawn(async move {
        let storage = tokio::task::spawn_blocking(crate::services::safe_storage::SafeStorage::load).await.unwrap_or_default();
        main.run(move |app| {
            let available = storage.is_available();
            app.safe_storage = std::sync::Arc::new(storage);
            memory_store::set(app, "safeStorageAvailable", available);
            create_main_ui(app);
        });
    });
}

fn register_app_ipc(app: &mut App) {
    app.ipc.handle("safeStorage:decryptString", |app, _, args| {
        if !app.memory_store.get_bool("safeStorageAvailable") {
            return IpcReply::Err("safeStorage is unavailable".into());
        }
        match args.first().and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            Some(v) => match app.safe_storage.decrypt_string(v) {
                Ok(s) => IpcReply::Ok(Value::from(s)),
                Err(err) => IpcReply::Err(err.to_string()),
            },
            None => IpcReply::Ok(Value::Null),
        }
    });
    app.ipc.handle("safeStorage:encryptString", |app, _, args| {
        if !app.memory_store.get_bool("safeStorageAvailable") {
            return IpcReply::Err("safeStorage is unavailable".into());
        }
        let v = args.first().and_then(|v| v.as_str()).unwrap_or("");
        match app.safe_storage.encrypt_string(v) {
            Ok(s) => IpcReply::Ok(Value::from(s)),
            Err(err) => IpcReply::Err(err.to_string()),
        }
    });
    app.ipc.handle("app:getVersion", |_, _, _| IpcReply::Ok(Value::from(crate::APP_VERSION)));
    app.ipc.on("app:relaunch", |app, _, _| app.relaunch());
    app.ipc.handle("app:getBuildInfo", |_, _, _| {
        IpcReply::Ok(json!({
            "branch": option_env!("YTMD_GIT_BRANCH"),
            "commit": option_env!("YTMD_GIT_COMMIT_HASH").map(|h| h.chars().take(7).collect::<String>()),
            "webview": wry::webview_version().unwrap_or_default(),
            "rust": "tao/wry",
        }))
    });
    app.ipc.on("dialog:selectCustomCss", |app, event, _| {
        let parent = match &event.owner {
            crate::ipc::WebviewOwner::Window(n) => Some(n.clone()),
            _ => None,
        };
        dialogs::pick_css_file(app, parent.as_deref(), |app, path| {
            config_store::set(app, "appearance.customCSSPath", path.map(|p| Value::from(p.to_string_lossy().to_string())).unwrap_or(Value::Null));
        });
    });
}

fn primary_scale_factor(app: &App) -> f64 {
    app.target().primary_monitor().map(|m| m.scale_factor()).unwrap_or(1.0)
}

fn create_main_ui(app: &mut App) {
    register_app_ipc(app);

    //#region Main Window
    let scale_factor = primary_scale_factor(app);
    let bounds = Rectangle::from_value(&app.config_store.get("state.windowBounds"));
    let mut opts = AppWindowOptions::new("Main", "main", "preload/main.js", bounds.map(|b| b.width as f64).unwrap_or(1280.0 / scale_factor), bounds.map(|b| b.height as f64).unwrap_or(720.0 / scale_factor));
    opts.position = bounds.map(|b| (b.x as f64, b.y as f64));
    opts.min_size = Some((156.0, 180.0));
    opts.maximized = app.config_store.get_bool("state.windowMaximized");
    if let Err(err) = wm::create_window(app, opts) {
        panic!("Failed to create the main window: {err:?}");
    }

    let save_bounds = |app: &mut App, _: &()| {
        if let Some(b) = app.window_manager.bounds("Main") {
            state_manager::update_state(app, vec![("windowBounds", b.to_value())]);
        }
    };
    let main_window = app.window_manager.get_mut("Main").unwrap();
    main_window.events.resize.on(save_bounds);
    main_window.events.moved.on(save_bounds);
    main_window.events.maximize.on(|app, _| state_manager::update_state(app, vec![("windowMaximized", Value::from(true))]));
    main_window.events.unmaximize.on(|app, _| state_manager::update_state(app, vec![("windowMaximized", Value::from(false))]));
    // This event is not called if the app is quitting
    main_window.events.close.on(|app, event| {
        if let Some(b) = app.window_manager.bounds("Main") {
            state_manager::update_state(app, vec![("windowBounds", b.to_value())]);
        }
        let maximized = app.window_manager.is_maximized("Main");
        state_manager::update_state(app, vec![("windowMaximized", Value::from(maximized))]);
        if app.config_store.get_bool("general.hideToTrayOnClose") {
            event.prevent_default();
            app.window_manager.hide("Main");
        }
    });

    let ipc = &mut app.window_manager.get_mut("Main").unwrap().ipc;
    ipc.on("window:openSettings", |app, _, _| open_settings(app));
    ipc.on("window:openMiniplayer", |app, _, _| open_miniplayer(app));
    ipc.on("ytmView:navigateDefault", |app, _, _| {
        if let Some(v) = app.window_manager.view(ytm_view_manager::VIEW) {
            let _ = v.webview.webview.load_url("https://music.youtube.com/");
        }
    });
    ipc.on("ytmView:reload", |app, _, _| {
        if let Some(v) = app.window_manager.view(ytm_view_manager::VIEW) {
            let _ = v.webview.webview.reload();
        }
    });
    ipc.on("ytmView:recreate", |app, _, _| ytm_view_manager::handle_signal(app, ytm_view_manager::ViewSignal::Crashed));
    ipc.on("ytmView:switchFocus", |app, _, args| {
        if args.first().and_then(|v| v.as_str()) == Some("ytm") {
            if let Some(v) = app.window_manager.view(ytm_view_manager::VIEW) {
                let _ = v.webview.webview.focus();
            }
        }
    });

    // Late updater window closure to prevent quitting before creating the main window
    wm::destroy_window(app, "Updater");

    //#region Tray creation
    crate::tray::create_tray(app);
    app.config_store.on_did_change("appearance", |app, _, _| crate::tray::update_icon(app));

    // Attach events for the ytmviewmanager
    app.ytm_view_manager.status_changed.on(|app, _| {
        let status = app.ytm_view_manager.status();
        app.window_manager.send_view(MAIN_VIEW, "ytmView:statusChanged", &[Value::from(status as i64)]);
        if status == YTMViewStatus::Ready {
            main_view_hide(app);
            if app.ytm_view_manager.has_error() {
                show_hook_error(app);
            }
        } else {
            main_view_show(app);
        }
    });
    app.ytm_view_manager.view_recreated.on(|app, _| main_view_show(app));
    app.ytm_view_manager.unresponsive.on(|app, _| main_view_show(app));
    app.ytm_view_manager.responsive.on(|app, _| main_view_hide(app));
    app.ytm_view_manager.load_errored.on(|app, _| {
        let error = app.ytm_view_manager.load_error.clone().map(|(code, description)| json!({ "code": code, "description": description })).unwrap_or(Value::Null);
        app.window_manager.send_view(MAIN_VIEW, "ytmView:loadError", &[error]);
    });

    // Initially create the YTM view and attach it
    if let Err(err) = ytm_view_manager::create_view(app) {
        log::error!("Failed to create the YTM view: {err:?}");
    }

    // The "Main" view: loading / unresponsive overlay stacked above the YTM view
    let main_view = wm::ViewOptions {
        name: MAIN_VIEW.into(),
        url: crate::assets::app_url("windows/main-view/index.html"),
        preload: Some("preload/main-view.js"),
        receive_broadcasts: true,
        init_scripts: vec![],
        data_dir: None,
        autoplay: false,
        offset_top: ytm_view_manager::TITLEBAR_HEIGHT,
        nav: NavPolicy::AppOnly,
        new_window: NewWindowPolicy::Deny,
        obscured: false,
    };
    if let Err(err) = wm::create_view(app, "Main", main_view) {
        log::error!("Failed to create the main view: {err:?}");
    }
    if let Some(v) = app.window_manager.view_mut(MAIN_VIEW) {
        v.ipc.on("appView:hide", |app, _, _| wm::set_view_visible(app, MAIN_VIEW, false));
        v.ipc.on("ytmView:reload", |app, _, _| {
            if let Some(v) = app.window_manager.view(ytm_view_manager::VIEW) {
                let _ = v.webview.webview.reload();
            }
        });
        v.ipc.on("__ytmd:domReady", |app, _, _| {
            let status = app.ytm_view_manager.status();
            app.window_manager.send_view(MAIN_VIEW, "ytmView:statusChanged", &[Value::from(status as i64)]);
        });
    }

    let after_ready = |app: &mut App| {
        crate::services::integration_manager::app_ready(app);
        maybe_show_changelog(app);
    };
    if !app.window_manager.when_ready("Main", after_ready) {
        after_ready(app);
    }
}

const MAIN_VIEW: &str = "Main";

fn main_view_show(app: &mut App) {
    app.window_manager.send_view(MAIN_VIEW, "appView:show", &[]);
    wm::set_view_visible(app, MAIN_VIEW, true);
}

/// mainView.hide(true): the page fades out and replies with appView:hide
fn main_view_hide(app: &mut App) {
    app.window_manager.send_view(MAIN_VIEW, "appView:hide", &[]);
}

fn show_hook_error(app: &mut App) {
    let flags = app.ytm_view_manager.setup_flags();
    let names: Vec<(&str, u32)> = setup_flags::NAMES.iter().copied().filter(|(n, _)| *n != "LocationNotApplicable").collect();
    let mut log_lines = String::new();
    for (n, f) in names.iter().filter(|(_, f)| flags & f != 0) {
        let _ = f;
        log_lines.push_str(&format!("    {n}... OK\n"));
    }
    for (n, f) in names.iter().filter(|(_, f)| flags & f == 0) {
        let _ = f;
        log_lines.push_str(&format!("    {n}... FAIL\n"));
    }
    let error = app.ytm_view_manager.error().cloned().unwrap_or(Value::Null);
    let stack = error.get("stack").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| error.to_string());
    let message = format!(
        "Features from YtmRs may not be present or function correctly\n\nThis usually means there's a bug and a bug report should be filed on the GitHub\n\nHook log:\n{log_lines}\n{stack}"
    );
    let copy = message.clone();
    dialogs::show_message_box(
        app,
        Some("Main"),
        MessageBox::new(Kind::Warning, "Hook Error", "YtmRs could not hook YouTube Music", &message, &["Copy to Clipboard and I understand", "I understand"]),
        move |_, result| {
            if result == 0 {
                crate::clipboard::write_text(&copy);
            }
        },
    );
}

fn centered_on_main(app: &App, width: f64, height: f64) -> Option<(f64, f64)> {
    let b = app.window_manager.bounds("Main")?;
    Some(((b.x as f64 + (b.width as f64 / 2.0 - width / 2.0)).round(), (b.y as f64 + (b.height as f64 / 2.0 - height / 2.0)).round()))
}

fn open_settings(app: &mut App) {
    if app.window_manager.has_window("Settings") {
        app.window_manager.show_and_focus("Settings");
        return;
    }
    let mut opts = AppWindowOptions::new("Settings", "settings", "preload/settings.js", 800.0, 600.0);
    opts.title = "YtmRs - Settings".into();
    opts.position = centered_on_main(app, 800.0, 600.0);
    opts.minimizable = false;
    opts.maximizable = false;
    opts.resizable = false;
    opts.parent = Some("Main".into());
    opts.modal = !cfg!(target_os = "macos");
    opts.new_window = NewWindowPolicy::External(|url| url == "https://github.com/ytmdesktop/ytmdesktop" || url == "https://ytmdesktop.github.io/");
    if let Err(err) = wm::create_window(app, opts) {
        log::error!("Failed to open settings: {err:?}");
    }
}

fn open_miniplayer(app: &mut App) {
    if app.window_manager.has_window("Miniplayer") {
        app.window_manager.show_and_focus("Miniplayer");
        return;
    }
    let scale_factor = primary_scale_factor(app);
    let bounds = Rectangle::from_value(&app.config_store.get("state.miniplayerWindowBounds"));
    let mut opts = AppWindowOptions::new("Miniplayer", "miniplayer", "preload/miniplayer.js", bounds.map(|b| b.width as f64).unwrap_or(600.0 / scale_factor), bounds.map(|b| b.height as f64).unwrap_or(400.0 / scale_factor));
    opts.title = "YtmRs - Miniplayer".into();
    opts.min_size = Some((240.0, 240.0));
    opts.position = bounds.map(|b| (b.x as f64, b.y as f64)).or_else(|| centered_on_main(app, 800.0, 600.0));
    opts.minimizable = false;
    opts.maximizable = false;
    // On mac and windows `pop-up-menu` will keep the miniplayer above the dock or taskbar
    opts.always_on_top = true;
    if let Err(err) = wm::create_window(app, opts) {
        log::error!("Failed to open miniplayer: {err:?}");
        return;
    }
    let save = |app: &mut App, _: &()| {
        if let Some(b) = app.window_manager.bounds("Miniplayer") {
            state_manager::update_state(app, vec![("miniplayerWindowBounds", b.to_value())]);
        }
    };
    let w = app.window_manager.get_mut("Miniplayer").unwrap();
    w.events.resize.on(save);
    w.events.moved.on(save);
    w.events.close.once(|app, _| {
        if let Some(b) = app.window_manager.bounds("Miniplayer") {
            state_manager::update_state(app, vec![("miniplayerWindowBounds", b.to_value())]);
        }
        app.window_manager.show_and_focus("Main");
    });
    w.ipc.on("remoteControl:execute", |app, _, args| {
        let Some(command) = args.first().and_then(|v| v.as_str()).map(|s| s.to_string()) else { return };
        crate::services::protected_api::remote_execute(app, &command, args[1..].to_vec());
    });
    w.ipc.handle("playerStateStore:getState", |app, _, _| IpcReply::Ok(serde_json::to_value(app.player_state_store.get_state()).unwrap_or(Value::Null)));

    app.window_manager.hide("Main");
}

fn maybe_show_changelog(app: &mut App) {
    if app.config_store.get_str("metadata.appVersion").as_deref() == Some(crate::APP_VERSION) {
        return;
    }
    let mut opts = AppWindowOptions::new("Changelog", "changelog", "preload/changelog.js", 800.0, 600.0);
    opts.title = "YtmRs - Changelog".into();
    opts.position = centered_on_main(app, 800.0, 600.0);
    opts.minimizable = false;
    opts.maximizable = false;
    opts.parent = Some("Main".into());
    opts.modal = !cfg!(target_os = "macos");
    opts.nav = NavPolicy::OpenExternal;
    opts.new_window = NewWindowPolicy::External(|_| true);
    if let Err(err) = wm::create_window(app, opts) {
        log::error!("Failed to open changelog: {err:?}");
        return;
    }
    let w = app.window_manager.get_mut("Changelog").unwrap();
    w.ipc.handle("changelog:getReleaseMetadata", |app, _, _| {
        let failed = json!({ "name": crate::APP_VERSION, "notes": "# Failed to retrieve changelog" });
        let feed = app.auto_updater.feed_url_no_platform();
        let version = app.config_store.get_str("metadata.appVersion").unwrap_or_default();
        let client = app.http.clone();
        IpcReply::Async(Box::pin(async move {
            let result: anyhow::Result<Value> = async {
                let feed = feed?;
                // Not all platforms are supported so we just use win32 to grab the notes
                let res = client.get(format!("{feed}/win32/{version}")).send().await?;
                if res.status().is_success() && res.status() != 204 {
                    return Ok(res.json::<Value>().await?);
                }
                anyhow::bail!("no release metadata")
            }
            .await;
            let mut value = result.unwrap_or_else(|err| {
                log::warn!("Failed to retrieve changelog {err}");
                failed
            });
            let notes = value.get("notes").and_then(|v| v.as_str()).unwrap_or("").to_string();
            value["notesHtml"] = Value::from(crate::util::render_markdown(&notes));
            Ok(value)
        }))
    });
    w.events.close.on(|app, _| config_store::set(app, "metadata.appVersion", Value::from(crate::APP_VERSION)));
}

pub fn second_instance(app: &mut App, command_line: Vec<String>) {
    // If the service host isn't initialized the app is still starting up
    if !app.host.initialized() {
        return;
    }
    log::debug!("Second instance");
    if app.window_manager.has_window("Miniplayer") {
        log::debug!("Closing miniplayer");
        // Main window will automatically show upon closure of the miniplayer
        wm::close_window(app, "Miniplayer");
    } else if app.window_manager.has_window("Main") {
        app.window_manager.show_and_focus("Main");
    }
    if let Some(last) = command_line.last() {
        let last = last.clone();
        crate::services::protocol_manager::handle_ytmd_protocol(app, &last);
    }
}

fn install_shutdown_handler(app: &App) {
    #[cfg(unix)]
    {
        let main = app.main.clone();
        app.spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup())) else { return };
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
                _ = hup.recv() => {}
            }
            main.run(|app| {
                state_manager::force_write(app);
                app.quit();
            });
        });
    }
    #[cfg(not(unix))]
    let _ = app;
}

pub fn on_quit(app: &mut App) {
    if app.host.lifecycle == ServiceHostLifecycle::Terminated {
        return;
    }
    log::debug!("Application quit");
    // Integrations clean up (sockets, media session, presence)
    let enabled: Vec<&'static str> = crate::integrations::all().iter().map(|i| i.name).filter(|n| app.integration_manager.is_enabled(n)).collect();
    for i in crate::integrations::all() {
        if enabled.contains(&i.name) {
            (i.on_disabled)(app);
        }
    }
    // At this stage the next lifecycle is Terminated
    if app.host.lifecycle == ServiceHostLifecycle::PostInitialized {
        run_next_lifecycle(app);
    } else {
        app.host.lifecycle = ServiceHostLifecycle::Terminated;
    }
}
