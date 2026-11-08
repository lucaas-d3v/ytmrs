//! Port of ShortcutManager (Electron globalShortcut -> global-hotkey)

use std::collections::HashMap;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde_json::Value;

use crate::app::App;
use crate::services::memory_store;

#[derive(Default)]
pub struct ShortcutManager {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    commands: HashMap<u32, &'static str>,
    initialized: bool,
}

/// (config key, remote control command, memory store fail flag)
const SHORTCUTS: [(&str, &str, &str); 10] = [
    ("playPause", "playPause", "shortcutsPlayPauseRegisterFailed"),
    ("next", "next", "shortcutsNextRegisterFailed"),
    ("previous", "previous", "shortcutsPreviousRegisterFailed"),
    ("thumbsUp", "toggleLike", "shortcutsThumbsUpRegisterFailed"),
    ("thumbsDown", "toggleDislike", "shortcutsThumbsDownRegisterFailed"),
    ("volumeUp", "volumeUp", "shortcutsVolumeUpRegisterFailed"),
    ("volumeDown", "volumeDown", "shortcutsVolumeDownRegisterFailed"),
    ("volumeMute", "toggleMute", "shortcutsVolumeMuteRegisterFailed"),
    ("toggleShuffle", "shuffle", "shortcutsToggleShuffleRegisterFailed"),
    ("toggleRepeat", "cycleRepeatMode", "shortcutsToggleRepeatRegisterFailed"),
];

pub fn on_initialized(app: &mut App) {
    if app.shortcut_manager.initialized {
        panic!("ShortcutManager is already initialized!");
    }
    app.shortcut_manager.initialized = true;
    match GlobalHotKeyManager::new() {
        Ok(m) => app.shortcut_manager.manager = Some(m),
        Err(err) => log::warn!("Global shortcuts unavailable: {err}"),
    }
    let main = app.main.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |e| main.send(crate::app::UserEvent::HotKey(e))));
    log::info!("ShortcutManager initialized");
}

pub fn on_post_initialized(app: &mut App) {
    app.config_store.on_did_change("shortcuts", |app, new, _| reconcile_shortcuts(app, Some(new.clone())));
    reconcile_shortcuts(app, None);
}

pub fn reconcile_shortcuts(app: &mut App, state: Option<Value>) {
    let shortcuts = state.unwrap_or_else(|| app.config_store.get("shortcuts"));
    let sm = &mut app.shortcut_manager;
    if let Some(m) = &sm.manager {
        let _ = m.unregister_all(&sm.registered);
    }
    sm.registered.clear();
    sm.commands.clear();
    log::info!("Unregistered shortcuts");

    for (key, command, flag) in SHORTCUTS {
        let accelerator = shortcuts.get(key).and_then(|v| v.as_str()).unwrap_or("");
        if accelerator.is_empty() {
            memory_store::set(app, flag, false);
            continue;
        }
        let registered = match (parse_accelerator(accelerator), &app.shortcut_manager.manager) {
            (Some(hotkey), Some(m)) => match m.register(hotkey) {
                Ok(()) => {
                    app.shortcut_manager.registered.push(hotkey);
                    app.shortcut_manager.commands.insert(hotkey.id(), command);
                    true
                }
                Err(_) => false,
            },
            _ => false,
        };
        if registered {
            log::info!("Registered shortcut: {command}");
        } else {
            log::info!("Failed to register shortcut: {command}");
        }
        memory_store::set(app, flag, !registered);
    }
    log::info!("Registered shortcuts");
}

pub fn handle_hotkey(app: &mut App, event: GlobalHotKeyEvent) {
    if event.state() != HotKeyState::Pressed {
        return;
    }
    if let Some(command) = app.shortcut_manager.commands.get(&event.id()).copied() {
        crate::services::protected_api::remote_execute(app, command, vec![]);
    }
}

/// Converts an Electron accelerator (as produced by the KeybindInput) into a HotKey
pub fn parse_accelerator(accelerator: &str) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;
    // "Shift++" style is not produced by the UI ("Plus" is used instead)
    let tokens: Vec<&str> = accelerator.split('+').filter(|t| !t.is_empty()).collect();
    for (i, token) in tokens.iter().enumerate() {
        let lower = token.to_ascii_lowercase();
        let is_last = i == tokens.len() - 1;
        match lower.as_str() {
            "cmdorctrl" | "commandorcontrol" | "cmdorcontrol" | "commandorctrl" => {
                if cfg!(target_os = "macos") {
                    mods |= Modifiers::META
                } else {
                    mods |= Modifiers::CONTROL
                }
            }
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" | "altgr" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "meta" | "super" | "cmd" | "command" if !is_last => mods |= Modifiers::META,
            _ => {
                if !is_last {
                    return None;
                }
                key = Some(parse_key(token)?);
            }
        }
    }
    Some(HotKey::new(if mods.is_empty() { None } else { Some(mods) }, key?))
}

fn parse_key(token: &str) -> Option<Code> {
    use Code::*;
    let upper = token.to_ascii_uppercase();
    let code = match upper.as_str() {
        "SPACE" | " " => Space,
        "TAB" => Tab,
        "ENTER" | "RETURN" => Enter,
        "BACKSPACE" => Backspace,
        "DELETE" | "DEL" => Delete,
        "INSERT" => Insert,
        "HOME" => Home,
        "END" => End,
        "PAGEUP" => PageUp,
        "PAGEDOWN" => PageDown,
        "ESCAPE" | "ESC" => Escape,
        "UP" | "ARROWUP" => ArrowUp,
        "DOWN" | "ARROWDOWN" => ArrowDown,
        "LEFT" | "ARROWLEFT" => ArrowLeft,
        "RIGHT" | "ARROWRIGHT" => ArrowRight,
        "PLUS" | "=" => Equal,
        "-" | "_" => Minus,
        "," | "<" => Comma,
        "." | ">" => Period,
        "/" | "?" => Slash,
        ";" | ":" => Semicolon,
        "'" | "\"" => Quote,
        "[" | "{" => BracketLeft,
        "]" | "}" => BracketRight,
        "\\" | "|" => Backslash,
        "`" | "~" => Backquote,
        "!" => Digit1,
        "@" => Digit2,
        "#" => Digit3,
        "$" => Digit4,
        "%" => Digit5,
        "^" => Digit6,
        "&" => Digit7,
        "*" => Digit8,
        "(" => Digit9,
        ")" => Digit0,
        "NUMADD" => NumpadAdd,
        "NUMSUB" => NumpadSubtract,
        "NUMMULT" => NumpadMultiply,
        "NUMDIV" => NumpadDivide,
        "NUMDEC" => NumpadDecimal,
        "CAPSLOCK" => CapsLock,
        "NUMLOCK" => NumLock,
        "SCROLLLOCK" => ScrollLock,
        "PRINTSCREEN" => PrintScreen,
        "VOLUMEUP" => AudioVolumeUp,
        "VOLUMEDOWN" => AudioVolumeDown,
        "VOLUMEMUTE" => AudioVolumeMute,
        "MEDIANEXTTRACK" => MediaTrackNext,
        "MEDIAPREVIOUSTRACK" => MediaTrackPrevious,
        "MEDIASTOP" => MediaStop,
        "MEDIAPLAYPAUSE" => MediaPlayPause,
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => {
            return format!("Key{s}").parse().ok();
        }
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_digit() => {
            return format!("Digit{s}").parse().ok();
        }
        s if s.starts_with("NUM") && s.len() == 4 && s.as_bytes()[3].is_ascii_digit() => {
            return format!("Numpad{}", &s[3..]).parse().ok();
        }
        s if s.starts_with('F') && s[1..].parse::<u8>().map(|n| (1..=24).contains(&n)).unwrap_or(false) => {
            return s.parse().ok();
        }
        _ => return None,
    };
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keybind_input_output() {
        let h = parse_accelerator("CmdOrCtrl+Shift+P").unwrap();
        assert_eq!(h.key, Code::KeyP);
        assert!(h.mods.contains(Modifiers::SHIFT));
        assert_eq!(parse_accelerator("Alt+Up").unwrap().key, Code::ArrowUp);
        assert_eq!(parse_accelerator("Num5").unwrap().key, Code::Numpad5);
        assert_eq!(parse_accelerator("NumAdd").unwrap().key, Code::NumpadAdd);
        assert_eq!(parse_accelerator("Shift+Plus").unwrap().key, Code::Equal);
        assert_eq!(parse_accelerator("F12").unwrap().key, Code::F12);
        assert_eq!(parse_accelerator("Meta+Space").unwrap().key, Code::Space);
        assert_eq!(parse_accelerator("Shift+!").unwrap().key, Code::Digit1);
        assert!(parse_accelerator("Ctrl+Nonsense").is_none());
        assert!(parse_accelerator("Ctrl").is_none());
    }
}
