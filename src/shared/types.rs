use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowsEventArguments {
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub always_on_top: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YTMViewStatus {
    Loading = 0,
    Hooking = 1,
    Ready = 2,
    LoadFailed = 3,
}

pub mod setup_flags {
    pub const LOCATION_NOT_APPLICABLE: u32 = 1;
    pub const NAMES: [(&str, u32); 7] = [
        ("LocationNotApplicable", 1),
        ("Early", 2),
        ("Styles", 4),
        ("Navigation", 8),
        ("Hooks", 16),
        ("Remote", 32),
        ("Extras", 64),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIconStyle {
    Auto = 0,
    White = 1,
    Black = 2,
}

impl TrayIconStyle {
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::White,
            2 => Self::Black,
            _ => Self::Auto,
        }
    }
}

/// Electron.Rectangle (DIP coordinates)
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rectangle {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rectangle {
    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        let o = v.as_object()?;
        let n = |k: &str| o.get(k).and_then(|v| v.as_f64()).map(|f| f.round() as i32);
        Some(Self {
            x: n("x")?,
            y: n("y")?,
            width: n("width")?,
            height: n("height")?,
        })
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({ "x": self.x, "y": self.y, "width": self.width, "height": self.height })
    }
}

/// AuthToken stored (encrypted) in the config store
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthToken {
    pub app_id: String,
    pub app_version: String,
    pub app_name: String,
    pub id: String,
    pub token: String,
    pub metadata: AuthTokenMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthTokenMetadata {
    pub version: u32,
}
