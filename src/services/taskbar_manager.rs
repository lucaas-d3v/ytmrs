//! Port of TaskbarManager: taskbar progress (all platforms supported by tao) and
//! thumbnail toolbar buttons (Windows only, see `platform::windows`).

use serde_json::Value;

use crate::app::App;
use crate::shared::player_state::VideoState;

#[derive(Default)]
pub struct TaskbarManager {
    initialized: bool,
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
    if !app.window_manager.has_window("Main") {
        return;
    }
    let state = app.player_state_store.state();
    let has_video = state.video_details.is_some();
    let is_playing = state.track_state == VideoState::Playing;
    let progress = state.video_details.as_ref().map(|d| state.video_progress / d.duration_seconds);

    #[cfg(windows)]
    crate::platform::windows::set_thumbar_buttons(app, has_video, is_playing);
    let _ = has_video;

    let progress_in_taskbar = match playback {
        Some(p) => p.get("progressInTaskbar").and_then(|v| v.as_bool()).unwrap_or(false),
        None => app.config_store.get_bool("playback.progressInTaskbar"),
    };
    match (progress_in_taskbar, progress) {
        (true, Some(p)) if p.is_finite() => app.window_manager.set_progress_bar("Main", p, !is_playing),
        _ => app.window_manager.set_progress_bar("Main", -1.0, false),
    }
}
