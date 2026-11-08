//! Port of AutoUpdater. The original relies on Squirrel.Windows through
//! update.electronjs.org; this port keeps the same feed URL scheme, state
//! machine and memory store flags, and downloads/self-replaces the executable.

use serde_json::Value;

use crate::app::App;
use crate::emit;
use crate::events::Emitter;
use crate::ipc::IpcReply;
use crate::services::memory_store;

const FEED_OWNER: Option<&str> = option_env!("YTMD_UPDATE_FEED_OWNER");
const FEED_REPOSITORY: Option<&str> = option_env!("YTMD_UPDATE_FEED_REPOSITORY");
const DISABLE_UPDATES: bool = cfg!(debug_assertions) || option_env!("YTMD_DISABLE_UPDATES").is_some();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoUpdaterState {
    Checking,
    Available,
    #[default]
    NotAvailable,
    Downloaded,
    Error,
}

#[derive(Default)]
pub struct AutoUpdater {
    enabled: bool,
    pub state: AutoUpdaterState,
    invalid_update_feed: bool,
    feed_no_platform: Option<String>,
    feed: Option<String>,
    downloaded_file: Option<std::path::PathBuf>,
    initialized: bool,
    pub checking: Emitter<()>,
    pub available: Emitter<()>,
    pub not_available: Emitter<()>,
    pub downloaded: Emitter<()>,
    pub error: Emitter<()>,
}

impl AutoUpdater {
    pub fn is_disabled(&self) -> bool {
        // macOS can't have auto updates without a code signature, linux is not supported on the update server
        self.invalid_update_feed || !crate::paths::is_packaged() || DISABLE_UPDATES || !cfg!(windows)
    }

    pub fn feed_url_no_platform(&self) -> anyhow::Result<String> {
        if self.invalid_update_feed {
            anyhow::bail!("Update feed is marked invalid");
        }
        Ok(self.feed_no_platform.clone().unwrap_or_default())
    }
}

pub fn on_initialized(app: &mut App) {
    if app.auto_updater.initialized {
        panic!("AutoUpdater is already initialized!");
    }
    app.auto_updater.initialized = true;
    match (FEED_OWNER.filter(|s| !s.trim().is_empty()), FEED_REPOSITORY.filter(|s| !s.trim().is_empty())) {
        (Some(owner), Some(repo)) => {
            let no_platform = format!("https://update.electronjs.org/{owner}/{repo}");
            let arch = match std::env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                other => other,
            };
            let platform = if cfg!(windows) { "win32" } else { std::env::consts::OS };
            app.auto_updater.feed = Some(format!("{no_platform}/{platform}-{arch}/{}", crate::APP_VERSION));
            app.auto_updater.feed_no_platform = Some(no_platform);
        }
        _ => app.auto_updater.invalid_update_feed = true,
    }

    app.ipc.handle("autoUpdater:isUpdateAvailable", |app, _, _| IpcReply::Ok(Value::from(app.auto_updater.state == AutoUpdaterState::Available)));
    app.ipc.handle("autoUpdater:isUpdateDownloaded", |app, _, _| IpcReply::Ok(Value::from(app.auto_updater.state == AutoUpdaterState::Downloaded)));
    app.ipc.on("app:checkForUpdates", |app, _, _| check_for_updates(app));
    app.ipc.on("app:restartApplicationForUpdate", |app, _, _| quit_and_install(app));

    if app.auto_updater.is_disabled() {
        memory_store::set(app, "autoUpdaterDisabled", true);
        return;
    }
    app.auto_updater.enabled = true;
    log::debug!("AutoUpdater enabled with at feed {:?}", app.auto_updater.feed);
    log::info!("AutoUpdater initialized");
}

fn set_state(app: &mut App, state: AutoUpdaterState) {
    app.auto_updater.state = state;
    use AutoUpdaterState::*;
    for (key, s) in [("autoUpdaterChecking", Checking), ("autoUpdaterNotAvailable", NotAvailable), ("autoUpdaterAvailable", Available), ("autoUpdaterDownloaded", Downloaded), ("autoUpdaterErrored", Error)] {
        memory_store::set(app, key, state == s);
    }
    match state {
        Checking => emit!(app, app.auto_updater.checking, &()),
        Available => emit!(app, app.auto_updater.available, &()),
        NotAvailable => emit!(app, app.auto_updater.not_available, &()),
        Downloaded => emit!(app, app.auto_updater.downloaded, &()),
        Error => emit!(app, app.auto_updater.error, &()),
    }
}

/// Start an update check; listen to the emitters for the result
pub fn check_for_updates(app: &mut App) {
    if !app.auto_updater.enabled || app.auto_updater.state == AutoUpdaterState::Checking {
        return;
    }
    let Some(feed) = app.auto_updater.feed.clone() else { return };
    log::debug!("AutoUpdater checking for updates...");
    set_state(app, AutoUpdaterState::Checking);
    let client = app.http.clone();
    let main = app.main.clone();
    app.spawn(async move {
        let result: anyhow::Result<Option<std::path::PathBuf>> = async {
            let res = client.get(&feed).send().await?;
            if res.status() == 204 || !res.status().is_success() {
                return Ok(None);
            }
            let json: Value = res.json().await?;
            let url = json.get("url").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("invalid feed response"))?.to_string();
            main.run(|app| {
                log::debug!("AutoUpdater found an update, downloading...");
                set_state(app, AutoUpdaterState::Available)
            });
            let bytes = client.get(&url).send().await?.error_for_status()?.bytes().await?;
            let path = std::env::temp_dir().join(format!("ytmdesktop-update-{}", uuid::Uuid::new_v4()));
            tokio::fs::write(&path, &bytes).await?;
            Ok(Some(path))
        }
        .await;
        main.run(move |app| match result {
            Ok(Some(path)) => {
                log::debug!("AutoUpdater update downloaded");
                app.auto_updater.downloaded_file = Some(path);
                set_state(app, AutoUpdaterState::Downloaded);
            }
            Ok(None) => {
                log::debug!("AutoUpdater did not find an update");
                set_state(app, AutoUpdaterState::NotAvailable);
            }
            Err(err) => {
                log::debug!("AutoUpdater errored {err}");
                set_state(app, AutoUpdaterState::Error);
            }
        });
    });
}

pub fn quit_and_install(app: &mut App) {
    let Some(path) = app.auto_updater.downloaded_file.take() else { return };
    if let Err(err) = self_replace::self_replace(&path) {
        log::error!("Failed to install update: {err}");
        return;
    }
    let _ = std::fs::remove_file(&path);
    app.relaunch();
}
