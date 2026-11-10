//! YtmRs - YouTube Music desktop app (Rust port of YTMDesktop).
//!
//! The interface is native GTK; the only web content is YouTube Music itself, running in the
//! system WebKitGTK webview.


#[cfg(not(target_os = "linux"))]
compile_error!("YtmRs currently targets Linux only (native GTK interface + WebKitGTK).");

mod app;
mod assets;
mod clipboard;
mod dialogs;
mod engine;
mod events;
mod integrations;
mod ipc;
mod lifecycle;
mod logging;
mod paths;
mod platform;
mod services;
mod shared;
mod single_instance;
mod tray;
mod ui;
mod util;

use std::cell::RefCell;

use gtk::glib;

use crate::app::{App, MainHandle, UserEvent};

pub const APP_NAME: &str = "YtmRs";
pub const APP_ID: &str = "app.ytmrs.YtmRs";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` with the App (main thread only)
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => {
            log::error!("App re-entrancy detected, dropping event");
            None
        }
    })
}

fn main() {
    // glibc creates one malloc arena per thread (up to 8x cores): cap it to keep RSS low
    #[cfg(target_env = "gnu")]
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
    logging::init();
    services::watchdog::install_panic_hook();
    log::info!("Application launched");
    // Before any thread exists: decides the environment of the WebKit processes
    engine::configure();

    // Wayland app_id / X11 WM_CLASS (matches the desktop entry)
    glib::set_prgname(Some(APP_ID));
    glib::set_application_name(APP_NAME);
    if let Err(err) = gtk::init() {
        eprintln!("Failed to initialize GTK: {err}");
        std::process::exit(1);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .thread_name("ytmrs-worker")
        .enable_all()
        .build()
        .expect("failed to create the async runtime");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<UserEvent>();
    let main_handle = MainHandle::new(tx);

    // Single instance lock
    if !single_instance::acquire(&runtime, main_handle.clone()) {
        log::info!("Another instance is running, forwarded command line");
        return;
    }

    let menu_handle = main_handle.clone();
    tray_icon::menu::MenuEvent::set_event_handler(Some(move |e| menu_handle.send(UserEvent::Menu(e))));
    let tray_handle = main_handle.clone();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |e| tray_handle.send(UserEvent::Tray(e))));

    let mut app = App::new(main_handle.clone(), runtime.handle().clone());
    lifecycle::pre_ready(&mut app);
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    main_handle.run(lifecycle::on_ready);
    glib::MainContext::default().spawn_local(async move {
        while let Some(event) = rx.recv().await {
            let exit = with_app(|app| {
                app.handle_user_event(event);
                app.exit_requested
            })
            .unwrap_or(false);
            if exit {
                gtk::main_quit();
                break;
            }
        }
    });

    gtk::main();
    APP.with(|cell| drop(cell.borrow_mut().take()));
    runtime.shutdown_timeout(std::time::Duration::from_millis(500));
}
