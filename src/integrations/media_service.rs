//! Port of EnhancedMediaService (xosms). Linux: MPRIS2 over D-Bus (zbus).
//! Windows/macOS: not available in this port yet (the original disables it on macOS).

use serde_json::Value;

use crate::app::App;
use crate::events::ListenerId;
use crate::services::integration_manager::IntegrationDescriptor;
use crate::services::protected_api::remote_execute;
use crate::shared::player_state::{PlayerState, RepeatMode, Thumbnail, VideoState};

/// Highest resolution thumbnail (strictly larger in both dimensions)
pub fn highest_res_thumbnail(thumbnails: &[Thumbnail]) -> Option<String> {
    let (mut cw, mut ch) = (0.0, 0.0);
    let mut url = None;
    for t in thumbnails {
        if t.w() > cw && t.h() > ch {
            cw = t.w();
            ch = t.h();
            url = t.url.clone();
        }
    }
    url
}

/// Commands coming from the OS media controls
#[derive(Debug)]
pub enum MediaCommand {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SetPosition(f64),
    Seek(f64),
    Loop(&'static str),
    Shuffle,
    Volume(f64),
    Raise,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaSnapshot {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub track_id: String,
    pub art_url: Option<String>,
    pub duration: f64,
    pub position: f64,
    pub status: &'static str,
    pub volume: f64,
    pub shuffle: bool,
    pub loop_status: &'static str,
}

#[derive(Default)]
pub struct EnhancedMediaService {
    listener: Option<ListenerId>,
    #[cfg(target_os = "linux")]
    player: Option<mpris::MprisHandle>,
    last: MediaSnapshot,
}

pub fn descriptor() -> IntegrationDescriptor {
    IntegrationDescriptor {
        name: "EnhancedMediaService",
        store_enable_property: "integrations.enhancedMediaServiceEnabled",
        dependent_store_properties: &[],
        on_setup: |_| {},
        on_enabled,
        on_disabled,
    }
}

pub fn handle_command(app: &mut App, command: MediaCommand) {
    match command {
        MediaCommand::PlayPause => remote_execute(app, "playPause", vec![]),
        MediaCommand::Play => remote_execute(app, "play", vec![]),
        MediaCommand::Pause | MediaCommand::Stop => remote_execute(app, "pause", vec![]),
        MediaCommand::Next => remote_execute(app, "next", vec![]),
        MediaCommand::Previous => remote_execute(app, "previous", vec![]),
        MediaCommand::SetPosition(position) => {
            let duration = app.player_state_store.state().video_details.as_ref().map(|d| d.duration_seconds);
            if let Some(d) = duration {
                if position >= 0.0 && position <= d {
                    remote_execute(app, "seekTo", vec![Value::from(position)]);
                }
            }
        }
        MediaCommand::Seek(offset) => {
            let state = app.player_state_store.state();
            let new_progress = (state.video_progress + offset).max(0.0);
            // Behavior aligns with MPRIS documentation
            if state.video_details.as_ref().map(|d| new_progress > d.duration_seconds).unwrap_or(false) {
                remote_execute(app, "next", vec![]);
            } else {
                remote_execute(app, "seekTo", vec![Value::from(new_progress)]);
            }
        }
        MediaCommand::Loop(mode) => remote_execute(app, "repeatMode", vec![Value::from(mode)]),
        MediaCommand::Shuffle => remote_execute(app, "shuffle", vec![]),
        MediaCommand::Volume(volume) => remote_execute(app, "setVolume", vec![Value::from((volume * 100.0).round())]),
        MediaCommand::Raise => crate::ui::main_window::show_and_focus(app),
    }
}

fn snapshot(state: &PlayerState, last: &MediaSnapshot) -> Option<MediaSnapshot> {
    let d = state.video_details.as_ref()?;
    let status = match state.track_state {
        VideoState::Playing => "Playing",
        // No buffering indicator for the media service, paused keeps controllers happy
        VideoState::Paused | VideoState::Buffering => "Paused",
        VideoState::Unknown => "Stopped",
    };
    let (shuffle, loop_status) = match &state.queue {
        Some(q) => (
            q.shuffle(),
            match q.repeat_mode {
                RepeatMode::All => "Playlist",
                RepeatMode::One => "Track",
                _ => "None",
            },
        ),
        None => (last.shuffle, last.loop_status),
    };
    Some(MediaSnapshot {
        title: d.title().to_string(),
        artist: d.author().to_string(),
        album: d.album.clone().or_else(|| last.album.clone()),
        track_id: d.id().to_string(),
        art_url: highest_res_thumbnail(&d.thumbnails).or_else(|| last.art_url.clone()),
        duration: if d.duration_seconds.is_finite() { d.duration_seconds } else { 0.0 },
        position: state.video_progress.max(0.0).min(if d.duration_seconds.is_finite() { d.duration_seconds } else { f64::MAX }),
        status,
        volume: state.volume / 100.0,
        shuffle,
        loop_status: if loop_status.is_empty() { "None" } else { loop_status },
    })
}

fn player_state_changed(app: &mut App, state: &PlayerState) {
    if !app.integration_manager.is_enabled("EnhancedMediaService") {
        return;
    }
    if state.video_details.is_some() && !state.has_full_metadata {
        return;
    }
    let snap = match snapshot(state, &app.integrations.media_service.last) {
        Some(s) => s,
        None => MediaSnapshot { status: "Stopped", loop_status: "None", ..app.integrations.media_service.last.clone() },
    };
    if snap == app.integrations.media_service.last {
        return;
    }
    app.integrations.media_service.last = snap.clone();
    #[cfg(target_os = "linux")]
    if let Some(p) = &app.integrations.media_service.player {
        p.update(snap);
    }
}

fn on_enabled(app: &mut App) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    if app.integrations.media_service.player.is_none() {
        app.integrations.media_service.player = Some(mpris::MprisHandle::start(app));
    }
    #[cfg(not(target_os = "linux"))]
    log::info!("EnhancedMediaService is not available on this platform in the Rust port");
    let id = app.player_state_store.state_changed.on(player_state_changed);
    app.integrations.media_service.listener = Some(id);
    Ok(())
}

fn on_disabled(app: &mut App) {
    if let Some(id) = app.integrations.media_service.listener.take() {
        app.player_state_store.state_changed.off(id);
    }
    #[cfg(target_os = "linux")]
    {
        app.integrations.media_service.player = None;
    }
    app.integrations.media_service.last = MediaSnapshot::default();
}

#[cfg(target_os = "linux")]
pub mod mpris {
    use std::collections::HashMap;
    use std::sync::Arc;

    use parking_lot::Mutex;
    use tokio::sync::mpsc;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{ObjectPath, OwnedValue, Value as ZValue};

    use super::{MediaCommand, MediaSnapshot};
    use crate::app::{App, MainHandle};

    type Shared = Arc<Mutex<MediaSnapshot>>;

    pub struct MprisHandle {
        tx: mpsc::UnboundedSender<MediaSnapshot>,
        task: tokio::task::JoinHandle<()>,
    }

    impl MprisHandle {
        pub fn start(app: &App) -> Self {
            let (tx, rx) = mpsc::unbounded_channel();
            let main = app.main.clone();
            let task = app.spawn(async move {
                if let Err(err) = run(rx, main).await {
                    log::warn!("mpris: failed to start media service: {err}");
                }
            });
            Self { tx, task }
        }

        pub fn update(&self, snapshot: MediaSnapshot) {
            let _ = self.tx.send(snapshot);
        }
    }

    impl Drop for MprisHandle {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct Root {
        main: MainHandle,
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        fn raise(&self) {
            self.main.run(|app| super::handle_command(app, MediaCommand::Raise));
        }
        fn quit(&self) {}
        #[zbus(property)]
        fn can_quit(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn can_raise(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn has_track_list(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn identity(&self) -> String {
            "YtmRs".into()
        }
        #[zbus(property)]
        fn desktop_entry(&self) -> String {
            crate::APP_ID.into()
        }
        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<String> {
            vec![]
        }
        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<String> {
            vec![]
        }
    }

    struct Player {
        main: MainHandle,
        state: Shared,
    }

    impl Player {
        fn send(&self, c: MediaCommand) {
            self.main.run(move |app| super::handle_command(app, c));
        }
    }

    fn track_path(id: &str) -> ObjectPath<'static> {
        let clean: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        ObjectPath::try_from(format!("/app/ytmdesktop/track/{}", if clean.is_empty() { "none".into() } else { clean })).unwrap_or_else(|_| ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack").unwrap())
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        fn next(&self) {
            self.send(MediaCommand::Next)
        }
        fn previous(&self) {
            self.send(MediaCommand::Previous)
        }
        fn pause(&self) {
            self.send(MediaCommand::Pause)
        }
        fn play_pause(&self) {
            self.send(MediaCommand::PlayPause)
        }
        fn stop(&self) {
            self.send(MediaCommand::Stop)
        }
        fn play(&self) {
            self.send(MediaCommand::Play)
        }
        fn seek(&self, offset: i64) {
            self.send(MediaCommand::Seek(offset as f64 / 1_000_000.0))
        }
        fn set_position(&self, _track_id: ObjectPath<'_>, position: i64) {
            self.send(MediaCommand::SetPosition(position as f64 / 1_000_000.0))
        }
        fn open_uri(&self, _uri: String) {}

        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.state.lock().status.to_string()
        }
        #[zbus(property)]
        fn loop_status(&self) -> String {
            self.state.lock().loop_status.to_string()
        }
        #[zbus(property)]
        fn set_loop_status(&self, value: String) {
            let mode = match value.as_str() {
                "Track" => "ONE",
                "Playlist" => "ALL",
                _ => "NONE",
            };
            self.send(MediaCommand::Loop(mode));
        }
        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn set_rate(&self, _value: f64) {}
        #[zbus(property)]
        fn shuffle(&self) -> bool {
            self.state.lock().shuffle
        }
        #[zbus(property)]
        fn set_shuffle(&self, value: bool) {
            if value != self.state.lock().shuffle {
                self.send(MediaCommand::Shuffle);
            }
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            let s = self.state.lock().clone();
            let mut m: HashMap<String, OwnedValue> = HashMap::new();
            let ins = |m: &mut HashMap<String, OwnedValue>, k: &str, v: ZValue<'_>| {
                if let Ok(v) = OwnedValue::try_from(v) {
                    m.insert(k.to_string(), v);
                }
            };
            ins(&mut m, "mpris:trackid", ZValue::from(track_path(&s.track_id)));
            ins(&mut m, "mpris:length", ZValue::from((s.duration * 1_000_000.0) as i64));
            ins(&mut m, "xesam:title", ZValue::from(s.title.clone()));
            ins(&mut m, "xesam:artist", ZValue::from(vec![s.artist.clone()]));
            if let Some(album) = &s.album {
                ins(&mut m, "xesam:album", ZValue::from(album.clone()));
            }
            if let Some(art) = &s.art_url {
                ins(&mut m, "mpris:artUrl", ZValue::from(art.clone()));
            }
            if !s.track_id.is_empty() {
                ins(&mut m, "xesam:url", ZValue::from(format!("https://music.youtube.com/watch?v={}", s.track_id)));
            }
            m
        }
        #[zbus(property)]
        fn volume(&self) -> f64 {
            self.state.lock().volume
        }
        #[zbus(property)]
        fn set_volume(&self, value: f64) {
            self.send(MediaCommand::Volume(value.clamp(0.0, 1.0)));
        }
        #[zbus(property(emits_changed_signal = "false"))]
        fn position(&self) -> i64 {
            (self.state.lock().position * 1_000_000.0) as i64
        }
        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_pause(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_seek(&self) -> bool {
            true
        }
        #[zbus(property(emits_changed_signal = "const"))]
        fn can_control(&self) -> bool {
            true
        }

        #[zbus(signal)]
        async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
    }

    async fn run(mut rx: mpsc::UnboundedReceiver<MediaSnapshot>, main: MainHandle) -> zbus::Result<()> {
        let state: Shared = Arc::new(Mutex::new(MediaSnapshot { status: "Stopped", loop_status: "None", ..Default::default() }));
        let name = format!("org.mpris.MediaPlayer2.ytmrs.instance{}", std::process::id());
        let connection = zbus::connection::Builder::session()?
            .name(name)?
            .serve_at("/org/mpris/MediaPlayer2", Root { main: main.clone() })?
            .serve_at("/org/mpris/MediaPlayer2", Player { main, state: state.clone() })?
            .build()
            .await?;
        let iface = connection.object_server().interface::<_, Player>("/org/mpris/MediaPlayer2").await?;
        while let Some(snapshot) = rx.recv().await {
            let previous = std::mem::replace(&mut *state.lock(), snapshot.clone());
            let player = iface.get().await;
            let emitter = iface.signal_emitter();
            if previous.status != snapshot.status {
                let _ = player.playback_status_changed(emitter).await;
            }
            if previous.title != snapshot.title || previous.artist != snapshot.artist || previous.album != snapshot.album || previous.art_url != snapshot.art_url || previous.duration != snapshot.duration || previous.track_id != snapshot.track_id {
                let _ = player.metadata_changed(emitter).await;
            }
            if previous.volume != snapshot.volume {
                let _ = player.volume_changed(emitter).await;
            }
            if previous.shuffle != snapshot.shuffle {
                let _ = player.shuffle_changed(emitter).await;
            }
            if previous.loop_status != snapshot.loop_status {
                let _ = player.loop_status_changed(emitter).await;
            }
            // Seeked signal when the position jumps (not regular progress)
            let expected = previous.position;
            if previous.track_id == snapshot.track_id && (snapshot.position - expected).abs() > 2.0 {
                let _ = Player::seeked(emitter, (snapshot.position * 1_000_000.0) as i64).await;
            }
        }
        Ok(())
    }
}
