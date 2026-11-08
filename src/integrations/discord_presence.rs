//! Port of DiscordPresence + minimal-discord-client (Discord RPC over local IPC)

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::app::{App, MainHandle, TimerHandle};
use crate::events::ListenerId;
use crate::services::integration_manager::IntegrationDescriptor;
use crate::services::memory_store;
use crate::shared::player_state::{highest_res_thumbnail_reduce, PlayerState, VideoDetails, VideoState};

const DISCORD_CLIENT_ID: &str = "1143202598460076053";

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpCode {
    Handshake = 0,
    Frame = 1,
    Close = 2,
    Ping = 3,
    Pong = 4,
}

pub fn encode_frame(op: u32, data: &Value) -> Vec<u8> {
    let json = data.to_string();
    let mut buf = Vec::with_capacity(8 + json.len());
    buf.extend_from_slice(&op.to_le_bytes());
    buf.extend_from_slice(&(json.len() as u32).to_le_bytes());
    buf.extend_from_slice(json.as_bytes());
    buf
}

enum Command {
    Connect,
    Send(Value),
}

/// Handle to the connection task
struct DiscordClient {
    tx: mpsc::UnboundedSender<Command>,
    task: tokio::task::JoinHandle<()>,
}

impl DiscordClient {
    fn new(app: &App, generation: u64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let task = app.spawn(client_task(rx, app.main.clone(), generation));
        Self { tx, task }
    }

    fn connect(&self) {
        let _ = self.tx.send(Command::Connect);
    }

    fn set_activity(&self, activity: Value) {
        let _ = self.tx.send(Command::Send(json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": uuid::Uuid::new_v4().to_string()
        })));
    }

    fn clear_activity(&self) {
        let _ = self.tx.send(Command::Send(json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id() },
            "nonce": uuid::Uuid::new_v4().to_string()
        })));
    }
}

impl Drop for DiscordClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(unix)]
fn directory_exists(p: &str) -> bool {
    std::path::Path::new(p).is_dir()
}

fn ipc_path(id: u32) -> String {
    #[cfg(windows)]
    {
        format!(r"\\?\pipe\discord-ipc-{id}")
    }
    #[cfg(unix)]
    {
        let prefix = crate::paths::runtime_prefix();
        if directory_exists(&format!("{prefix}/snap.discord")) {
            format!("{prefix}/snap.discord/discord-ipc-{id}")
        } else if directory_exists(&format!("{prefix}/com.discordapp.Discord")) || directory_exists(&format!("{prefix}/app/com.discordapp.Discord")) {
            format!("{prefix}/app/com.discordapp.Discord/discord-ipc-{id}")
        } else {
            format!("{prefix}/discord-ipc-{id}")
        }
    }
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

async fn open(path: &str) -> std::io::Result<Box<dyn Stream>> {
    #[cfg(unix)]
    {
        Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
    }
    #[cfg(windows)]
    {
        Ok(Box::new(tokio::net::windows::named_pipe::ClientOptions::new().open(path)?))
    }
}

async fn client_task(mut rx: mpsc::UnboundedReceiver<Command>, main: MainHandle, generation: u64) {
    loop {
        // Wait for a connect command
        match rx.recv().await {
            Some(Command::Connect) => {}
            Some(Command::Send(_)) => continue,
            None => return,
        }
        log::debug!("dipc: initiated connection loop over 10 ids");
        let mut stream = None;
        for id in 0..10 {
            let path = ipc_path(id);
            log::debug!("dipc: connecting to discord at {path}");
            match open(&path).await {
                Ok(s) => {
                    log::debug!("dipc: connected to discord at {path}");
                    stream = Some(s);
                    break;
                }
                Err(_) => log::debug!("dipc: failed to connect to discord at {path}"),
            }
        }
        let Some(stream) = stream else {
            main.run(move |app| on_connect_failed(app, generation));
            continue;
        };
        let (mut reader, mut writer) = tokio::io::split(stream);
        if writer.write_all(&encode_frame(OpCode::Handshake as u32, &json!({ "v": 1, "client_id": DISCORD_CLIENT_ID }))).await.is_err() {
            main.run(move |app| on_connect_failed(app, generation));
            continue;
        }
        main.run(move |app| on_connect(app, generation));

        let mut header = [0u8; 8];
        loop {
            tokio::select! {
                read = reader.read_exact(&mut header) => {
                    if read.is_err() { break; }
                    let op = u32::from_le_bytes(header[0..4].try_into().unwrap());
                    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
                    let mut body = vec![0u8; len.min(1 << 20)];
                    if reader.read_exact(&mut body).await.is_err() { break; }
                    if op == OpCode::Ping as u32 {
                        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                        let _ = writer.write_all(&encode_frame(OpCode::Pong as u32, &json)).await;
                    } else if op == OpCode::Close as u32 {
                        break;
                    }
                }
                cmd = rx.recv() => match cmd {
                    Some(Command::Send(v)) => { if writer.write_all(&encode_frame(OpCode::Frame as u32, &v)).await.is_err() { break; } }
                    Some(Command::Connect) => {}
                    None => return,
                }
            }
        }
        main.run(move |app| on_close(app, generation));
    }
}

#[derive(Default)]
pub struct DiscordPresence {
    client: Option<DiscordClient>,
    generation: u64,
    ready: bool,
    activity_debounce: Option<TimerHandle>,
    pause_timeout: Option<TimerHandle>,
    retry_timeout: Option<TimerHandle>,
    listener: Option<ListenerId>,
    video_state: Option<VideoState>,
    video_details: Option<VideoDetails>,
    progress: f64,
    connection_retries: u32,
}

/// String.length based limit with zero width space padding (UTF-16 semantics)
pub fn string_limit(s: &str, limit: usize, minimum: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.len() > limit {
        return format!("{}...", String::from_utf16_lossy(&units[..limit - 3]).trim());
    }
    if units.len() < minimum {
        let mut out = s.to_string();
        for _ in units.len()..minimum {
            out.push('\u{200B}');
        }
        return out;
    }
    s.to_string()
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

pub fn build_activity(details: &VideoDetails, state: VideoState, progress: f64, now: f64) -> Value {
    let thumbnail = highest_res_thumbnail_reduce(&details.thumbnails);
    let playing = state == VideoState::Playing;
    let mut timestamps = Map::new();
    if playing {
        timestamps.insert("start".into(), json!((now - progress * 1000.0).round() as i64));
        timestamps.insert("end".into(), json!((now + (details.duration_seconds - progress) * 1000.0).round() as i64));
    }
    let mut assets = Map::new();
    match &thumbnail {
        Some(t) if t.encode_utf16().count() <= 256 => {
            assets.insert("large_image".into(), json!(t));
        }
        Some(_) => {
            assets.insert("large_image".into(), json!("ytmd-logo"));
        }
        None => {}
    }
    if let Some(album) = &details.album {
        assets.insert("large_text".into(), json!(string_limit(album, 128, 2)));
    }
    if let Some(album_id) = &details.album_id {
        assets.insert("large_url".into(), json!(format!("https://music.youtube.com/browse/{album_id}")));
    }
    assets.insert("small_image".into(), json!(if playing { "play-border" } else { "pause-border" }));
    assets.insert("small_text".into(), json!(if playing { "Playing" } else { "Paused" }));
    let mut activity = json!({
        "type": 2,
        "status_display_type": 1,
        "details": string_limit(details.title(), 128, 2),
        "details_url": format!("https://music.youtube.com/watch?v={}", details.id()),
        "state": string_limit(details.author(), 128, 2),
        "timestamps": timestamps,
        "assets": assets,
        "instance": false,
        "buttons": [{ "label": "Play on YtmRs", "url": format!("ytmrs://play/{}", details.id()) }]
    });
    if let Some(channel) = &details.channel_id {
        activity["state_url"] = json!(format!("https://music.youtube.com/channel/{channel}"));
    }
    activity
}

pub fn descriptor() -> IntegrationDescriptor {
    IntegrationDescriptor {
        name: "DiscordPresence",
        store_enable_property: "integrations.discordPresenceEnabled",
        dependent_store_properties: &[],
        on_setup: |_| {},
        on_enabled: |app| {
            on_enabled(app);
            Ok(())
        },
        on_disabled,
    }
}

fn update_activity(app: &mut App) {
    if app.integrations.discord_presence.activity_debounce.is_some() {
        return;
    }
    let timer = app.set_timeout(Duration::from_secs(1), |app| {
        let dp = &mut app.integrations.discord_presence;
        dp.activity_debounce = None;
        let Some(client) = &dp.client else { return };
        match &dp.video_details {
            None => client.clear_activity(),
            Some(details) => client.set_activity(build_activity(details, dp.video_state.unwrap_or(VideoState::Unknown), dp.progress, now_ms())),
        }
    });
    app.integrations.discord_presence.activity_debounce = Some(timer);
}

fn player_state_changed(app: &mut App, state: &PlayerState) {
    let dp = &mut app.integrations.discord_presence;
    if !dp.ready {
        return;
    }
    let Some(details) = &state.video_details else {
        if let Some(c) = &dp.client {
            c.clear_activity();
        }
        return;
    };
    let old_state = dp.video_state;
    let old_id = dp.video_details.as_ref().map(|d| d.id().to_string());
    let old_progress = dp.progress;
    dp.video_state = Some(state.track_state);
    dp.video_details = Some(details.clone());
    dp.progress = state.video_progress.floor();
    let changed = old_state != dp.video_state || old_id.as_deref() != Some(details.id()) || (dp.progress - old_progress).abs() > 1.0 || old_progress > dp.progress;
    if state.has_full_metadata && changed {
        update_activity(app);
    }

    if let Some(t) = app.integrations.discord_presence.pause_timeout.take() {
        t.cancel();
    }
    if state.track_state == VideoState::Playing {
        return;
    }
    let timer = app.set_timeout(Duration::from_secs(30), |app| {
        let dp = &mut app.integrations.discord_presence;
        dp.pause_timeout = None;
        if let Some(c) = &dp.client {
            c.clear_activity();
        }
    });
    app.integrations.discord_presence.pause_timeout = Some(timer);
}

fn retry_discord_connection(app: &mut App) {
    if !app.integration_manager.is_enabled("DiscordPresence") {
        return;
    }
    if app.integrations.discord_presence.connection_retries >= 30 {
        memory_store::set(app, "discordPresenceConnectionFailed", true);
        return;
    }
    app.integrations.discord_presence.connection_retries += 1;
    log::info!("Connecting to Discord attempt {}/30", app.integrations.discord_presence.connection_retries);
    if let Some(t) = app.integrations.discord_presence.retry_timeout.take() {
        t.cancel();
    }
    let timer = app.set_timeout(Duration::from_secs(5), |app| {
        if let Some(c) = &app.integrations.discord_presence.client {
            c.connect();
        }
    });
    app.integrations.discord_presence.retry_timeout = Some(timer);
}

fn on_connect(app: &mut App, generation: u64) {
    if generation != app.integrations.discord_presence.generation {
        return;
    }
    app.integrations.discord_presence.ready = true;
    app.integrations.discord_presence.connection_retries = 0;
    memory_store::set(app, "discordPresenceConnectionFailed", false);
}

fn on_connect_failed(app: &mut App, generation: u64) {
    if generation == app.integrations.discord_presence.generation {
        retry_discord_connection(app);
    }
}

fn on_close(app: &mut App, generation: u64) {
    if generation != app.integrations.discord_presence.generation {
        return;
    }
    log::info!("Discord connection closed");
    app.integrations.discord_presence.ready = false;
    retry_discord_connection(app);
}

fn on_enabled(app: &mut App) {
    if app.integrations.discord_presence.client.is_some() {
        return;
    }
    app.integrations.discord_presence.generation += 1;
    let client = DiscordClient::new(app, app.integrations.discord_presence.generation);
    client.connect();
    app.integrations.discord_presence.client = Some(client);
    let id = app.player_state_store.state_changed.on(player_state_changed);
    app.integrations.discord_presence.listener = Some(id);
}

fn on_disabled(app: &mut App) {
    app.integrations.discord_presence.connection_retries = 0;
    memory_store::set(app, "discordPresenceConnectionFailed", false);
    let dp = &mut app.integrations.discord_presence;
    for t in [dp.activity_debounce.take(), dp.pause_timeout.take(), dp.retry_timeout.take()].into_iter().flatten() {
        t.cancel();
    }
    if let Some(id) = dp.listener.take() {
        app.player_state_store.state_changed.off(id);
    }
    let dp = &mut app.integrations.discord_presence;
    dp.ready = false;
    dp.generation += 1;
    dp.client = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_strings() {
        assert_eq!(string_limit("a", 128, 2), "a\u{200B}");
        assert_eq!(string_limit("ab", 128, 2), "ab");
        let long = "x".repeat(200);
        let limited = string_limit(&long, 128, 2);
        assert_eq!(limited.encode_utf16().count(), 128);
        assert!(limited.ends_with("..."));
    }

    #[test]
    fn frames() {
        let f = encode_frame(1, &json!({"a": 1}));
        assert_eq!(&f[0..4], &1u32.to_le_bytes());
        assert_eq!(&f[4..8], &7u32.to_le_bytes());
        assert_eq!(&f[8..], b"{\"a\":1}");
    }
}
