//! Port of `src/main/util.ts` (v1 config migration) + helpers

use serde_json::Value;

use crate::app::App;
use crate::dialogs::{self, Kind, MessageBox};
use crate::services::config_store;

pub fn render_markdown(md: &str) -> String {
    let mut options = comrak::Options::default();
    options.extension.autolink = true;
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.render.r#unsafe = true;
    comrak::markdown_to_html(md, &options)
}

/// Settings migration from YTMDesktop v1 (only offered on the first run)
pub fn v1_config_migration(app: &mut App) {
    let paths = crate::paths::get();
    let first_run = paths.user_data.join(".first-run");
    if first_run.exists() {
        return;
    }
    let _ = std::fs::write(&first_run, "");
    let v1_dir = paths.app_data.join("youtube-music-desktop-app");
    let v1_path = v1_dir.join("config.json");
    let Ok(data) = std::fs::read_to_string(&v1_path) else {
        // No v1 configuration (the original shows a "not migrated" warning here; skipped intentionally)
        return;
    };
    let Ok(v1): Result<Value, _> = serde_json::from_str(&data) else {
        not_migrated(app);
        return;
    };
    dialogs::show_message_box(
        app,
        None,
        MessageBox::new(
            Kind::Question,
            "YtmRs",
            "Would you like to migrate your settings?",
            "A configuration file for YTMDesktop v1 was found. Your settings can be migrated.\n\nWARNING: Not all settings will be migrated as they may no longer be available in v2.",
            &["No", "Migrate Settings"],
        ),
        move |app, result| {
            if result != 1 {
                return;
            }
            if let Err(err) = migrate(app, &v1, &v1_dir) {
                log::warn!("v1 migration failed: {err}");
                not_migrated(app);
                return;
            }
            let lastfm = v1.get("last-fm-login");
            let non_empty = |k: &str| lastfm.and_then(|l| l.get(k)).and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);
            let lastfm_found = non_empty("username") && non_empty("password");
            if lastfm_found {
                config_store::set(app, "integrations.lastFMEnabled", Value::from(true));
                dialogs::show_message_box(
                    app,
                    None,
                    MessageBox::new(Kind::Info, "YtmRs", "Last.fm", "Last.fm configuration was found and has NOT been migrated. Re-authentication is required.", &["OK"]),
                    |app, _| migrated(app),
                );
            } else {
                migrated(app);
            }
        },
    );
}

fn migrated(app: &mut App) {
    dialogs::show_message_box(app, None, MessageBox::new(Kind::Info, "YtmRs", "Settings migrated.", "Your settings have been migrated.", &["OK"]), |_, _| {});
}

fn not_migrated(app: &mut App) {
    dialogs::show_message_box(app, None, MessageBox::new(Kind::Warning, "YtmRs", "Settings not migrated!", "Your settings could not be fully migrated.", &["OK"]), |_, _| {});
}

fn migrate(app: &mut App, v1: &Value, v1_dir: &std::path::Path) -> anyhow::Result<()> {
    let map = [
        ("settings-companion-server", "integrations.companionServerEnabled"),
        ("settings-continue-where-left-of", "playback.continueWhereYouLeftOff"),
        ("settings-decibel-volume", "playback.ratioVolume"),
        ("settings-discord-rich-presence", "integrations.discordPresenceEnabled"),
        ("settings-page-zoom", "appearance.zoom"),
        ("settings-keep-background", "general.hideToTrayOnClose"),
        ("settings-show-notifications", "general.showNotificationOnSongChange"),
        ("settings-start-minimized", "general.startMinimized"),
        ("settings-start-on-boot", "general.startOnBoot"),
        ("settings-surround-sound", "playback.enableSpeakerFill"),
    ];
    for (v1_key, key) in map {
        if let Some(v) = v1.get(v1_key) {
            config_store::set(app, key, v.clone());
        }
    }
    if v1.get("settings-custom-css-page").and_then(|v| v.as_bool()).unwrap_or(false) {
        let src = v1_dir.join("custom").join("css").join("page.css");
        let dst = crate::paths::get().user_data.join("custom_css.css");
        std::fs::copy(&src, &dst)?;
        config_store::set(app, "appearance.customCSSPath", Value::from(dst.to_string_lossy().to_string()));
        config_store::set(app, "appearance.customCSSEnabled", Value::from(true));
    }
    if let Some(acc) = v1.get("settings-accelerators") {
        let map = [
            ("media-play-pause", "shortcuts.playPause"),
            ("media-track-next", "shortcuts.next"),
            ("media-track-previous", "shortcuts.previous"),
            ("media-track-like", "shortcuts.thumbsUp"),
            ("media-track-dislike", "shortcuts.thumbsDown"),
            ("media-volume-up", "shortcuts.volumeUp"),
            ("media-volume-down", "shortcuts.volumeDown"),
        ];
        for (v1_key, key) in map {
            if let Some(s) = acc.get(v1_key).and_then(|v| v.as_str()) {
                if s.to_lowercase() != "disabled" {
                    config_store::set(app, key, Value::from(s));
                }
            }
        }
    }
    Ok(())
}
