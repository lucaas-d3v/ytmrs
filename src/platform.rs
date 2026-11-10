//! freedesktop integration: autostart, `ytmrs://` handler, taskbar progress (Unity launcher
//! entry, as Electron does on Linux) and the system color scheme (XDG desktop portal).

use std::collections::HashMap;
use std::path::PathBuf;

use zbus::export::futures_core::Stream;
use zbus::zvariant::{OwnedValue, Value};

use crate::app::MainHandle;

fn desktop_file_name() -> String {
    format!("{}.desktop", crate::APP_ID)
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "ytmrs".into())
}

fn desktop_entry(extra: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=YtmRs\nComment=YouTube Music desktop app written in Rust\nExec=\"{}\" %U\nIcon=ytmrs\nCategories=AudioVideo;Audio;Player;\nStartupWMClass={}\nMimeType=x-scheme-handler/ytmrs;\n{extra}",
        exe(),
        crate::APP_ID
    )
}

fn applications_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("applications")
}

/// Electron `app.setLoginItemSettings({ openAtLogin })`
pub fn set_login_item(enabled: bool) {
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

/// Electron `app.isDefaultProtocolClient`
pub fn is_default_protocol_client(scheme: &str) -> bool {
    let out = std::process::Command::new("xdg-settings").args(["get", "default-url-scheme-handler", scheme]).output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == desktop_file_name(),
        Err(_) => true, // xdg-utils unavailable, nothing we can do
    }
}

/// Electron `app.setAsDefaultProtocolClient`
pub fn set_as_default_protocol_client(scheme: &str) {
    let dir = applications_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(desktop_file_name());
    let system_entry = PathBuf::from("/usr/share/applications").join(desktop_file_name());
    if !path.exists() && !system_entry.exists() {
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

static SESSION_BUS: tokio::sync::OnceCell<Option<zbus::Connection>> = tokio::sync::OnceCell::const_new();

async fn session_bus() -> Option<zbus::Connection> {
    SESSION_BUS
        .get_or_init(|| async {
            zbus::Connection::session()
                .await
                .map_err(|err| log::warn!("Session bus unavailable: {err}"))
                .ok()
        })
        .await
        .clone()
}

/// Electron `BrowserWindow.setProgressBar` on Linux: `com.canonical.Unity.LauncherEntry`
/// (taskbar progress of KDE Plasma, Dash to Dock, Plank...). `None` hides the progress.
pub fn set_taskbar_progress(rt: &tokio::runtime::Handle, progress: Option<f64>) {
    rt.spawn(async move {
        let Some(conn) = session_bus().await else { return };
        let mut properties: HashMap<&str, Value> = HashMap::new();
        properties.insert("progress", Value::F64(progress.unwrap_or(0.0).clamp(0.0, 1.0)));
        properties.insert("progress-visible", Value::Bool(progress.is_some()));
        let app_uri = format!("application://{}", desktop_file_name());
        if let Err(err) = conn
            .emit_signal(None::<zbus::names::BusName<'_>>, "/app/ytmrs/YtmRs/LauncherEntry", "com.canonical.Unity.LauncherEntry", "Update", &(app_uri, properties))
            .await
        {
            log::debug!("Failed to update the taskbar progress: {err}");
        }
    });
}

/// org.freedesktop.appearance color-scheme: 1 = prefer dark, 2 = prefer light
fn is_dark(value: &OwnedValue) -> Option<bool> {
    let mut v: &Value = value;
    // `Read` (portal v1) wraps the value in another variant
    while let Value::Value(inner) = v {
        v = inner;
    }
    match v {
        Value::U32(scheme) => Some(*scheme == 1),
        _ => None,
    }
}

async fn next<S: Stream>(stream: &mut std::pin::Pin<&mut S>) -> Option<S::Item> {
    std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
}

/// Reports the system dark preference now and whenever it changes
pub fn watch_color_scheme(rt: &tokio::runtime::Handle, main: MainHandle, on_change: fn(&mut crate::app::App, bool)) {
    rt.spawn(async move {
        let Some(conn) = session_bus().await else { return };
        let Ok(proxy) = zbus::Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings").await else { return };
        let current = match proxy.call::<_, _, OwnedValue>("ReadOne", &("org.freedesktop.appearance", "color-scheme")).await {
            Ok(v) => Some(v),
            Err(_) => proxy.call::<_, _, OwnedValue>("Read", &("org.freedesktop.appearance", "color-scheme")).await.ok(),
        };
        if let Some(dark) = current.as_ref().and_then(is_dark) {
            main.run(move |app| on_change(app, dark));
        }
        let Ok(stream) = proxy.receive_signal("SettingChanged").await else { return };
        let mut stream = std::pin::pin!(stream);
        while let Some(message) = next(&mut stream).await {
            let Ok((namespace, key, value)) = message.body().deserialize::<(String, String, OwnedValue)>() else { continue };
            if namespace == "org.freedesktop.appearance" && key == "color-scheme" {
                if let Some(dark) = is_dark(&value) {
                    main.run(move |app| on_change(app, dark));
                }
            }
        }
    });
}
