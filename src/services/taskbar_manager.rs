//! Port of TaskbarManager: track progress in the taskbar (Unity launcher entry on Linux).
//! The Windows thumbnail toolbar buttons have no Linux equivalent.

use serde_json::Value;

use crate::app::App;
use crate::shared::player_state::VideoState;

#[derive(Default)]
pub struct TaskbarManager {
    initialized: bool,
    /// Last progress sent (avoids a D-Bus signal per progress tick)
    last: Option<Option<i64>>,
}

pub fn on_initialized(app: &mut App) {
    if app.taskbar_manager.initialized {
        panic!("TaskbarManager is already initialized!");
    }
    app.taskbar_manager.initialized = true;
    log::info!("TaskbarManager initialized");
}

pub fn on_post_initialized(app: &mut App) {
    app.player_state_store.state_changed.on(|app, _| reconcile_taskbar(app, None));
    app.config_store.on_did_change("playback", |app, new, _| reconcile_taskbar(app, Some(new.clone())));
    reconcile_taskbar(app, None);
}

pub fn reconcile_taskbar(app: &mut App, playback: Option<Value>) {
    let state = app.player_state_store.state();
    let progress = state.video_details.as_ref().map(|d| state.video_progress / d.duration_seconds);
    let _is_playing = state.track_state == VideoState::Playing;
    let progress_in_taskbar = match playback {
        Some(p) => p.get("progressInTaskbar").and_then(|v| v.as_bool()).unwrap_or(false),
        None => app.config_store.get_bool("playback.progressInTaskbar"),
    };
    let progress = match (progress_in_taskbar, progress) {
        (true, Some(p)) if p.is_finite() => Some(p.clamp(0.0, 1.0)),
        _ => None,
    };
    // Per mille resolution is plenty for a taskbar
    let key = progress.map(|p| (p * 1000.0) as i64);
    if app.taskbar_manager.last == Some(key) {
        return;
    }
    app.taskbar_manager.last = Some(key);
    crate::platform::set_taskbar_progress(&app.rt, progress);
}
