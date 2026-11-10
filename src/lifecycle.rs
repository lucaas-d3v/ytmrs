//! Port of `src/main/index.ts`: application startup, main window, tray and glue.

use std::time::Duration;

use crate::app::App;
use crate::dialogs::{self, Kind, MessageBox};
use crate::services::service_host::{run_next_lifecycle, ServiceHostLifecycle};
use crate::services::{memory_store, state_manager, ytm_view_manager};
use crate::shared::types::{setup_flags, YTMViewStatus};
use crate::ui::{self, main_window, miniplayer};

pub fn pre_ready(app: &mut App) {
    app.host.set_services(crate::services::collection());
    // At this stage the next lifecycle is PreInitialized
    run_next_lifecycle(app);
    app.integration_manager.create_integrations(crate::integrations::all());
    crate::services::integration_manager::run_integration_setups(app);
}

pub fn on_ready(app: &mut App) {
    log::info!("Application ready");
    // Must happen before any WebKit process starts
    ytm_view_manager::configure_network_process();
    ui::init_style();
    crate::services::watchdog::set_app_ready();
    // At this stage the next lifecycle is Initialized
    run_next_lifecycle(app);
    install_shutdown_handler(app);
    schedule_heap_trim(app);
    // No updater window: the original update server does not serve Linux builds
    continue_startup(app);
}

fn continue_startup(app: &mut App) {
    if app.ui.startup_continued {
        return;
    }
    app.ui.startup_continued = true;
    // At this stage the next lifecycle is PostInitialized
    run_next_lifecycle(app);

    crate::util::v1_config_migration(app);
    create_main_ui(app);

    // safeStorage setup and checks (may talk to the keyring daemon): integrations wait for it
    let main = app.main.clone();
    app.spawn(async move {
        let storage = tokio::task::spawn_blocking(crate::services::safe_storage::SafeStorage::load).await.unwrap_or_default();
        main.run(move |app| {
            let available = storage.is_available();
            app.safe_storage = std::sync::Arc::new(storage);
            memory_store::set(app, "safeStorageAvailable", available);
            crate::services::integration_manager::app_ready(app);
        });
    });
}

fn create_main_ui(app: &mut App) {
    main_window::create(app);

    //#region Tray creation
    app.ui.dark_theme = true;
    crate::tray::create_tray(app);
    app.config_store.on_did_change("appearance", |app, _, _| crate::tray::update_icon(app));
    crate::platform::watch_color_scheme(&app.rt, app.main.clone(), |app, dark| {
        app.ui.dark_theme = dark;
        crate::tray::update_icon(app);
    });

    // Attach events for the ytmviewmanager
    app.ytm_view_manager.status_changed.on(|app, _| {
        main_window::on_view_status_changed(app);
        if app.ytm_view_manager.status() == YTMViewStatus::Ready && app.ytm_view_manager.has_error() {
            show_hook_error(app);
        }
    });
    app.ytm_view_manager.navigated.on(|app, _| {
        main_window::update_history_buttons(app);
        // The back/forward list may be updated right after the URL
        app.set_timeout(Duration::from_millis(150), main_window::update_history_buttons);
    });
    app.ytm_view_manager.timed_lyrics.on(|app, lyrics| ui::lyrics::set_lyrics(app, lyrics.clone()));

    // Create the YTM view and place it in the main window
    let view = ytm_view_manager::create_view(app);
    main_window::attach_view(app, &view);
    main_window::update_history_buttons(app);
    main_window::show(app);
    ui::changelog::maybe_show(app);
}

fn show_hook_error(app: &mut App) {
    let flags = app.ytm_view_manager.setup_flags();
    let names = setup_flags::NAMES.iter().filter(|(n, _)| *n != "LocationNotApplicable");
    let mut log_lines = String::new();
    for (n, _) in names.clone().filter(|(_, f)| flags & f != 0) {
        log_lines.push_str(&format!("    {n}... OK\n"));
    }
    for (n, _) in names.filter(|(_, f)| flags & f == 0) {
        log_lines.push_str(&format!("    {n}... FAIL\n"));
    }
    let error = app.ytm_view_manager.error().cloned().unwrap_or_default();
    let stack = error.get("stack").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| error.to_string());
    let message = format!(
        "Features from YtmRs may not be present or function correctly\n\nThis usually means there's a bug and a bug report should be filed on the GitHub\n\nHook log:\n{log_lines}\n{stack}"
    );
    let copy = message.clone();
    let parent = ui::main_gtk_window(app);
    dialogs::show_message_box(
        app,
        parent.as_ref(),
        MessageBox::new(Kind::Warning, "Hook Error", "YtmRs could not hook YouTube Music", &message, &["Copy to Clipboard and I understand", "I understand"]),
        move |_, result| {
            if result == 0 {
                crate::clipboard::write_text(&copy);
            }
        },
    );
}

pub fn second_instance(app: &mut App, command_line: Vec<String>) {
    // If the service host isn't initialized the app is still starting up
    if !app.host.initialized() {
        return;
    }
    log::debug!("Second instance");
    if miniplayer::is_open(app) {
        log::debug!("Closing miniplayer");
        // Main window will automatically show upon closure of the miniplayer
        miniplayer::close(app);
    } else {
        main_window::show_and_focus(app);
    }
    if let Some(last) = command_line.last() {
        crate::services::protocol_manager::handle_ytmd_protocol(app, &last.clone());
    }
}

/// Return freed heap pages to the OS periodically (page loads churn a lot)
fn schedule_heap_trim(app: &mut App) {
    #[cfg(target_env = "gnu")]
    app.set_timeout(Duration::from_secs(60), |app| {
        unsafe {
            libc::malloc_trim(0);
        }
        schedule_heap_trim(app);
    });
    #[cfg(not(target_env = "gnu"))]
    let _ = app;
}

fn install_shutdown_handler(app: &App) {
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

pub fn on_quit(app: &mut App) {
    if app.host.lifecycle == ServiceHostLifecycle::Terminated {
        return;
    }
    log::debug!("Application quit");
    main_window::save_bounds(app);
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
