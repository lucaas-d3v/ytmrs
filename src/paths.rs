use std::path::PathBuf;
use std::sync::OnceLock;

use crate::APP_NAME;

pub struct Paths {
    /// Electron `app.getPath("userData")`
    pub user_data: PathBuf,
    /// Electron `app.getPath("appData")`
    pub app_data: PathBuf,
    pub logs: PathBuf,
    pub cache: PathBuf,
}

static PATHS: OnceLock<Paths> = OnceLock::new();

pub fn get() -> &'static Paths {
    PATHS.get_or_init(|| {
        let app_data = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        let user_data = std::env::var_os("YTMD_USER_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| app_data.join(APP_NAME));
        #[cfg(target_os = "macos")]
        let logs = dirs::home_dir().unwrap_or_default().join("Library/Logs").join(APP_NAME);
        #[cfg(not(target_os = "macos"))]
        let logs = user_data.join("logs");
        let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join(APP_NAME);
        let _ = std::fs::create_dir_all(&user_data);
        Paths { user_data, app_data, logs, cache }
    })
}

/// Electron `app.isPackaged`
pub fn is_packaged() -> bool {
    !cfg!(debug_assertions)
}

/// `$XDG_RUNTIME_DIR || $TMPDIR || $TMP || $TEMP || /tmp` without trailing slash
#[cfg(unix)]
pub fn runtime_prefix() -> String {
    let dirty = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .next()
        .unwrap_or_else(|| "/tmp".to_string());
    dirty.trim_end_matches('/').to_string()
}
