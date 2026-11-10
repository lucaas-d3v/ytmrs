//! Port of PlayerStateStore

use serde_json::Value;

use crate::app::App;
use crate::emit;
use crate::events::Emitter;
use crate::shared::player_state::*;

#[derive(Default)]
pub struct PlayerStateStore {
    state: PlayerState,
    pub state_changed: Emitter<PlayerState>,
    pub playlist_created: Emitter<Value>,
    pub playlist_deleted: Emitter<String>,
    pub player_signal: Emitter<PlayerSignal>,
}

impl PlayerStateStore {
    pub fn get_state(&self) -> PlayerState {
        self.state.clone()
    }

    pub fn state(&self) -> &PlayerState {
        &self.state
    }
}

pub fn on_initialized(app: &mut App) {
    let api = &mut app.protected_api;
    api.on("PlayerState", "updateVideoProgress", |app, args| {
        app.player_state_store.state.video_progress = args.first().map(crate::shared::js_number).unwrap_or(0.0);
        state_changed(app);
    });
    api.on("PlayerState", "updateVideoState", |app, args| {
        app.player_state_store.state.track_state = transform_video_state(args.first().unwrap_or(&Value::Null));
        state_changed(app);
    });
    api.on("PlayerState", "updateVideoDetails", |app, args| {
        let null = Value::Null;
        let details = args.first().unwrap_or(&null);
        let playlist_id = args.get(1).and_then(|v| v.as_str()).map(|s| s.to_string());
        let album = args.get(2).unwrap_or(&null);
        let like_status = args.get(3).unwrap_or(&null);
        let has_full_metadata = args.get(4).and_then(|v| v.as_bool()).unwrap_or(false);
        let s = &mut app.player_state_store.state;
        s.video_details = Some(video_details_from_ytm(details, album, like_status));
        s.playlist_id = playlist_id;
        s.has_full_metadata = has_full_metadata;
        state_changed(app);
    });
    api.on("PlayerState", "updateFromStore", |app, args| {
        let null = Value::Null;
        let queue = args.first().unwrap_or(&null);
        let like_status = args.get(1).and_then(|v| v.as_str()).unwrap_or("INDIFFERENT").to_string();
        let volume = args.get(2).and_then(|v| v.as_f64());
        let muted = args.get(3).and_then(|v| v.as_bool());
        let ad_playing = args.get(4).and_then(|v| v.as_bool());
        let s = &mut app.player_state_store.state;
        s.queue = queue_from_ytm(queue);
        if let Some(d) = s.video_details.as_mut() {
            d.like_status = transform_like_status(&like_status);
        }
        s.ad_playing = ad_playing == Some(true);
        s.muted = muted == Some(true);
        if let Some(v) = volume {
            if v >= 0.0 {
                s.volume = v;
            }
        }
        state_changed(app);
    });
    api.on("PlayerState", "playerSignal", |app, args| {
        if args.first().and_then(|v| v.as_str()) == Some("NEW_PLAYBACK") {
            emit!(app, app.player_state_store.player_signal, &PlayerSignal::NewPlayback);
        }
    });
    api.on("PlayerState", "playlistCreated", |app, args| {
        let playlist = args.first().cloned().unwrap_or(Value::Null);
        emit!(app, app.player_state_store.playlist_created, &playlist);
    });
    api.on("PlayerState", "playlistDeleted", |app, args| {
        let id = args.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
        emit!(app, app.player_state_store.playlist_deleted, &id);
    });
    log::info!("PlayerStateStore initialized");
}

fn state_changed(app: &mut App) {
    let state = app.player_state_store.get_state();
    crate::ui::on_player_state(app, &state);

    crate::services::state_manager::update_state(
        app,
        vec![
            ("lastVideoId", Value::from(state.video_details.as_ref().map(|d| d.id().to_string()).unwrap_or_default())),
            ("lastPlaylistId", Value::from(state.playlist_id.clone().unwrap_or_default())),
        ],
    );

    emit!(app, app.player_state_store.state_changed, &state);
}
