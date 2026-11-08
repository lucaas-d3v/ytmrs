//! Windows specifics (registry based login item / protocol registration).
//! NOTE: not compiled/tested in the Linux development environment of this port.

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
}

pub fn set_run_at_login(enabled: bool) {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run") {
        if enabled {
            let _ = key.set_value("YtmRs", &format!("\"{}\"", exe()));
        } else {
            let _ = key.delete_value("YtmRs");
        }
    }
}

pub fn is_default_protocol_client(scheme: &str) -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(format!("Software\\Classes\\{scheme}\\shell\\open\\command"))
        .and_then(|k| k.get_value::<String, _>(""))
        .map(|v| v.contains(&exe()))
        .unwrap_or(false)
}

pub fn set_as_default_protocol_client(scheme: &str) {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(format!("Software\\Classes\\{scheme}")) {
        let _ = key.set_value("", &format!("URL:{scheme}"));
        let _ = key.set_value("URL Protocol", &"");
        if let Ok((cmd, _)) = key.create_subkey("shell\\open\\command") {
            let _ = cmd.set_value("", &format!("\"{}\" \"%1\"", exe()));
        }
    }
}

/// Thumbnail toolbar buttons (Previous / Play-Pause / Next).
/// TODO: ITaskbarList3::ThumbBarAddButtons implementation; currently a no-op.
pub fn set_thumbar_buttons(_app: &mut crate::app::App, _has_video: bool, _is_playing: bool) {}
