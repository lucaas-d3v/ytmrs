use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YTMViewStatus {
    Loading = 0,
    Hooking = 1,
    Ready = 2,
    LoadFailed = 3,
}

pub mod setup_flags {
    pub const LOCATION_NOT_APPLICABLE: u32 = 1;
    pub const NAMES: [(&str, u32); 5] = [("LocationNotApplicable", 1), ("Early", 2), ("Hooks", 16), ("Remote", 32), ("Extras", 64)];
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
