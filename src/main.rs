#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]
#![allow(dead_code)]
//! YtmRs - Rust port of ytmdesktop (Electron) using the
//! operating system's native webview (WebKitGTK / WebView2 / WKWebView).

mod app;
mod assets;
mod clipboard;
mod dialogs;
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
mod util;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

use crate::app::{App, UserEvent};

pub const APP_NAME: &str = "YtmRs";
pub const APP_ID: &str = "app.ytmrs.YtmRs";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    logging::init();
    services::watchdog::install_panic_hook();
    log::info!("Application launched");

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("ytmrs-worker").build().expect("failed to create the async runtime");

    let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::EventLoopBuilderExtUnix;
        builder.with_app_id(APP_ID);
    }
    let event_loop = builder.build();
    let proxy = event_loop.create_proxy();

    // Single instance lock
    if !single_instance::acquire(&runtime, proxy.clone()) {
        log::info!("Another instance is running, forwarded command line");
        return;
    }

    let menu_proxy = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |e| {
        let _ = menu_proxy.send_event(UserEvent::Menu(e));
    }));
    let tray_proxy = proxy.clone();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |e| {
        let _ = tray_proxy.send_event(UserEvent::Tray(e));
    }));

    let mut app = App::new(proxy, runtime.handle().clone());
    lifecycle::pre_ready(&mut app);

    event_loop.run(move |event, target, control_flow| {
        let _keep_runtime_alive = &runtime;
        *control_flow = ControlFlow::Wait;
        app.set_target(target);
        match event {
            Event::NewEvents(StartCause::Init) => lifecycle::on_ready(&mut app),
            Event::UserEvent(user_event) => app.handle_user_event(user_event),
            Event::WindowEvent { window_id, event, .. } => app.handle_window_event(window_id, event),
            Event::LoopDestroyed => lifecycle::on_quit(&mut app),
            _ => {}
        }
        if app.exit_requested {
            *control_flow = ControlFlow::Exit;
        }
    });
}
