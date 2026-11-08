//! Port of ProtocolManager: `ytmrs://` system protocol (the in-app `ytmd-app://`
//! protocol is served by [`crate::assets::serve_protocol`]).

use serde_json::json;

use crate::app::App;

#[derive(Default)]
pub struct ProtocolManager {
    initialized: bool,
}

pub fn on_initialized(app: &mut App) {
    if app.protocol_manager.initialized {
        panic!("ProtocolManager is already initialized!");
    }
    app.protocol_manager.initialized = true;
    // Registered in development too, intentionally (same as the original)
    if std::env::var_os("YTMD_SKIP_SYSTEM_INTEGRATION").is_some() {
        log::info!("ProtocolManager initialized");
        return;
    }
    app.spawn(async {
        let _ = tokio::task::spawn_blocking(|| {
            if !crate::platform::is_default_protocol_client("ytmrs") {
                log::info!("Application set as default protcol client for 'ytmrs'");
                crate::platform::set_as_default_protocol_client("ytmrs");
            }
        })
        .await;
    });
    log::info!("ProtocolManager initialized");
}

/// Parse `ytmrs://play/<videoId>/<playlistId>` -> (videoId, playlistId)
pub fn parse_play_url(url: &str) -> Option<(String, Option<String>)> {
    let paths = url.split("://").nth(1)?;
    let parts: Vec<&str> = paths.split('/').collect();
    if parts.first() != Some(&"play") || parts.len() < 2 {
        return None;
    }
    Some((parts[1].to_string(), parts.get(2).map(|s| s.to_string())))
}

pub fn handle_ytmd_protocol(app: &mut App, url: &str) {
    log::info!("Handling ytmrs protocol url {url}");
    if let Some((video_id, playlist_id)) = parse_play_url(url) {
        if app.ytm_view_manager.is_initialized() {
            log::debug!("Navigating to videoId: {video_id}, playlistId: {playlist_id:?}");
            crate::services::protected_api::remote_execute(
                app,
                "navigate",
                vec![json!({ "watchEndpoint": { "videoId": video_id, "playlistId": playlist_id } })],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_play() {
        assert_eq!(super::parse_play_url("ytmrs://play/abc/PL1"), Some(("abc".into(), Some("PL1".into()))));
        assert_eq!(super::parse_play_url("ytmrs://play/abc"), Some(("abc".into(), None)));
        assert_eq!(super::parse_play_url("ytmrs://other/abc"), None);
        assert_eq!(super::parse_play_url("/usr/bin/app"), None);
    }
}
