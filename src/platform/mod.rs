//! Platform integration helpers (login items, protocol client registration, webview hooks)

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

/// Electron `app.setLoginItemSettings({ openAtLogin })`
pub fn set_login_item(enabled: bool) {
    #[cfg(target_os = "linux")]
    linux::set_autostart(enabled);
    #[cfg(windows)]
    windows::set_run_at_login(enabled);
    #[cfg(target_os = "macos")]
    let _ = enabled;
}

/// Electron `app.isDefaultProtocolClient`
pub fn is_default_protocol_client(scheme: &str) -> bool {
    #[cfg(target_os = "linux")]
    return linux::is_default_protocol_client(scheme);
    #[cfg(windows)]
    return windows::is_default_protocol_client(scheme);
    #[cfg(target_os = "macos")]
    {
        let _ = scheme;
        true
    }
}

/// Electron `app.setAsDefaultProtocolClient`
pub fn set_as_default_protocol_client(scheme: &str) {
    #[cfg(target_os = "linux")]
    linux::set_as_default_protocol_client(scheme);
    #[cfg(windows)]
    windows::set_as_default_protocol_client(scheme);
    #[cfg(target_os = "macos")]
    let _ = scheme;
}
