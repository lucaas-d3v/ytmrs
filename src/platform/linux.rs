//! Linux (GTK / WebKitGTK / freedesktop) specifics

use std::path::PathBuf;

use webkit2gtk::{LoadEvent, SettingsExt, WebViewExt};
use wry::WebViewExtUnix;

use crate::app::MainHandle;
use crate::services::ytm_view_manager::{self, ViewSignal};

fn desktop_file_name() -> String {
    format!("{}.desktop", crate::APP_ID)
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "ytmrs".into())
}

fn desktop_entry(extra: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=YtmRs\nComment=An unofficial cross platform desktop app for YouTube Music\nExec=\"{}\" %U\nIcon=ytmrs\nCategories=AudioVideo;Audio;\nStartupWMClass={}\nMimeType=x-scheme-handler/ytmrs;\n{extra}",
        exe(),
        crate::APP_ID
    )
}

fn applications_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("applications")
}

pub fn set_autostart(enabled: bool) {
    let dir = dirs::config_dir().unwrap_or_default().join("autostart");
    let path = dir.join(desktop_file_name());
    if enabled {
        let _ = std::fs::create_dir_all(&dir);
        if let Err(err) = std::fs::write(&path, desktop_entry("X-GNOME-Autostart-enabled=true\n")) {
            log::warn!("Failed to enable start on boot: {err}");
        }
    } else if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn is_default_protocol_client(scheme: &str) -> bool {
    let out = std::process::Command::new("xdg-settings").args(["get", "default-url-scheme-handler", scheme]).output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == desktop_file_name(),
        Err(_) => true, // xdg-utils unavailable, nothing we can do
    }
}

pub fn set_as_default_protocol_client(scheme: &str) {
    let dir = applications_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(desktop_file_name());
    if !path.exists() {
        let _ = std::fs::write(&path, desktop_entry(""));
        // Install the icon for the desktop entry
        if let Some(icon) = crate::assets::icon_bytes("ytmrs.png") {
            let icon_dir = dirs::data_dir().unwrap_or_default().join("icons/hicolor/512x512/apps");
            let _ = std::fs::create_dir_all(&icon_dir);
            let _ = std::fs::write(icon_dir.join("ytmrs.png"), icon);
        }
    }
    let _ = std::process::Command::new("xdg-settings").args(["set", "default-url-scheme-handler", scheme, &desktop_file_name()]).status();
}

pub fn configure_webview(webview: &wry::WebView, disable_hardware_acceleration: bool) {
    if disable_hardware_acceleration {
        if let Some(settings) = WebViewExt::settings(&webview.webview()) {
            settings.set_hardware_acceleration_policy(webkit2gtk::HardwareAccelerationPolicy::Never);
        }
    }
}

/// WebKit signal wiring for the YouTube Music view (navigation policy, load errors,
/// crashes, responsiveness, HTML fullscreen and in-page navigations)
pub fn attach_ytm_view_hooks(webview: &wry::WebView, main: MainHandle) {
    let wv = webview.webview();

    let m = main.clone();
    wv.connect_load_changed(move |wv, event| {
        let uri = wv.uri().map(|u| u.to_string()).unwrap_or_default();
        log::trace!("ytmview: load {event:?} {uri}");
        match event {
            LoadEvent::Started => {
                m.run(|app| ytm_view_manager::handle_signal(app, ViewSignal::LoadStarted));
                if let Ok(url) = url::Url::parse(&uri) {
                    if (url.scheme() == "http" || url.scheme() == "https") && ytm_view_manager::is_prevented_nav_or_redirect(&url) {
                        log::info!("Blocking YTM View navigation to {uri}");
                        wv.stop_loading();
                        let u = uri.clone();
                        m.run(move |_| ytm_view_manager::open_external_from_ytm_view(&u));
                    }
                }
            }
            LoadEvent::Redirected => {
                if let Ok(url) = url::Url::parse(&uri) {
                    if ytm_view_manager::is_prevented_nav_or_redirect(&url) {
                        log::info!("Blocking YTM View redirect to {uri}");
                        wv.stop_loading();
                    }
                    if ytm_view_manager::is_premium_redirect(&url) {
                        // This users region requires a premium subscription to use YTM
                        let wv = wv.clone();
                        gtk::glib::idle_add_local_once(move || wv.load_uri(ytm_view_manager::PREMIUM_LOGIN_URL));
                    }
                }
            }
            _ => {}
        }
    });

    let m = main.clone();
    wv.connect_load_failed(move |_wv, _event, _uri, error| {
        // Cancellations / policy interruptions are not load failures
        if error.matches(webkit2gtk::NetworkError::Cancelled) || error.matches(webkit2gtk::PolicyError::FrameLoadInterruptedByPolicyChange) {
            return false;
        }
        log::debug!("ytmview: load failed {_uri}: {error}");
        let code: i64 = if error.kind::<webkit2gtk::NetworkError>().is_some() { -2 } else if error.kind::<webkit2gtk::PolicyError>().is_some() { -3 } else { -1 };
        let message = error.message().to_string();
        m.run(move |app| ytm_view_manager::handle_signal(app, ViewSignal::LoadFailed(code, message)));
        false
    });

    let m = main.clone();
    wv.connect_web_process_terminated(move |_wv, _reason| {
        m.run(|app| ytm_view_manager::handle_signal(app, ViewSignal::Crashed));
    });

    let m = main.clone();
    wv.connect_is_web_process_responsive_notify(move |wv| {
        let responsive = wv.is_web_process_responsive();
        m.run(move |app| ytm_view_manager::handle_signal(app, if responsive { ViewSignal::Responsive } else { ViewSignal::Unresponsive }));
    });

    let m = main.clone();
    wv.connect_enter_fullscreen(move |_| {
        m.run(|app| ytm_view_manager::handle_signal(app, ViewSignal::EnterFullscreen));
        false
    });
    let m = main.clone();
    wv.connect_leave_fullscreen(move |_| {
        m.run(|app| ytm_view_manager::handle_signal(app, ViewSignal::LeaveFullscreen));
        false
    });

    let m = main;
    wv.connect_uri_notify(move |wv| {
        if wv.is_loading() && wv.estimated_load_progress() < 0.1 {
            // provisional, wait for commit
        }
        if let Some(uri) = wv.uri() {
            let uri = uri.to_string();
            m.run(move |app| ytm_view_manager::handle_signal(app, ViewSignal::Navigated(uri)));
        }
    });
}

/// User level stylesheet (Electron `insertCSS(css, { cssOrigin: "user" })`)
pub fn insert_user_css(webview: &wry::WebView, css: &str) -> Option<webkit2gtk::UserStyleSheet> {
    use webkit2gtk::UserContentManagerExt;
    let manager = webview.webview().user_content_manager()?;
    let sheet = webkit2gtk::UserStyleSheet::new(css, webkit2gtk::UserContentInjectedFrames::TopFrame, webkit2gtk::UserStyleLevel::User, &[], &[]);
    manager.add_style_sheet(&sheet);
    Some(sheet)
}

pub fn remove_user_css(webview: &wry::WebView, sheet: &webkit2gtk::UserStyleSheet) {
    use webkit2gtk::UserContentManagerExt;
    if let Some(manager) = webview.webview().user_content_manager() {
        manager.remove_style_sheet(sheet);
    }
}
