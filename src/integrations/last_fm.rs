//! Port of the LastFM integration + LastFMClient

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde_json::Value;

use crate::app::App;
use crate::events::ListenerId;
use crate::services::integration_manager::IntegrationDescriptor;
use crate::services::{config_store, memory_store};
use crate::shared::player_state::{PlayerSignal, PlayerState, VideoState};

const LASTFM_ENDPOINT: &str = "https://ws.audioscrobbler.com/2.0/";
const LASTFM_API_KEY: Option<&str> = option_env!("LASTFM_API_KEY");
const LASTFM_API_SECRET: Option<&str> = option_env!("LASTFM_API_SECRET");

const ERR_OPERATION_FAILED: i64 = 8;
const ERR_INVALID_SESSION_KEY: i64 = 9;
const ERR_SERVICE_OFFLINE: i64 = 11;
const ERR_TEMPORARILY_UNAVAILABLE: i64 = 16;
const ERR_API_KEY_SUSPENDED: i64 = 26;
const ERR_RATE_LIMIT_EXCEEDED: i64 = 29;

pub fn sign(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut s = String::new();
    for (k, v) in params {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    hex::encode(Md5::digest(s.as_bytes()))
}

#[derive(Clone)]
pub struct LastFmClient {
    http: reqwest::Client,
    api_key: String,
    api_secret: String,
    session_key: std::sync::Arc<parking_lot::Mutex<Option<String>>>,
    api_key_suspended: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl LastFmClient {
    pub fn new(http: reqwest::Client, api_key: &str, api_secret: &str, session_key: Option<String>) -> Self {
        Self {
            http,
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            session_key: std::sync::Arc::new(parking_lot::Mutex::new(session_key)),
            api_key_suspended: Default::default(),
        }
    }

    fn retry_delay(retry_after: Option<&str>, attempt: u32) -> Duration {
        if let Some(r) = retry_after {
            if let Ok(secs) = r.trim().parse::<f64>() {
                if secs >= 0.0 {
                    return Duration::from_secs_f64(secs);
                }
            }
            if let Ok(date) = chrono::DateTime::parse_from_rfc2822(r) {
                let ms = date.timestamp_millis() - chrono::Utc::now().timestamp_millis();
                return Duration::from_millis(ms.max(0) as u64);
            }
        }
        let base = (1000u64 * 2u64.pow(attempt)).min(60000);
        let jitter = rand::random::<f64>() * (base as f64 * 0.2).min(1000.0);
        Duration::from_millis(base + jitter as u64)
    }

    async fn call(&self, method: &str, params: Vec<(&str, Option<String>)>, authenticated: bool) -> anyhow::Result<Value> {
        for attempt in 0..5 {
            if self.api_key_suspended.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("LastFM API key suspended");
            }
            let mut body: BTreeMap<String, String> = params.iter().filter_map(|(k, v)| v.clone().map(|v| (k.to_string(), v))).collect();
            body.insert("method".into(), method.into());
            body.insert("api_key".into(), self.api_key.clone());
            if authenticated {
                body.insert("sk".into(), self.session_key.lock().clone().unwrap_or_default());
            }
            let sig = sign(&body, &self.api_secret);
            body.insert("api_sig".into(), sig);
            body.insert("format".into(), "json".into());

            let response = self.http.post(LASTFM_ENDPOINT).form(&body).timeout(Duration::from_secs(15)).send().await?;
            let status = response.status().as_u16();
            let retry_after = response.headers().get("Retry-After").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
            let data: Value = response.json().await?;
            if let Some(error) = data.get("error").and_then(|v| v.as_i64()) {
                if error == ERR_INVALID_SESSION_KEY {
                    *self.session_key.lock() = None;
                }
                if error == ERR_API_KEY_SUSPENDED {
                    log::warn!("lastfm: api key suspended forbidding any future requests to lastfm");
                    self.api_key_suspended.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                let rate_limited = status == 429 || error == ERR_RATE_LIMIT_EXCEEDED;
                let transient = (500..=599).contains(&status) || [ERR_OPERATION_FAILED, ERR_SERVICE_OFFLINE, ERR_TEMPORARILY_UNAVAILABLE].contains(&error);
                if rate_limited || transient {
                    tokio::time::sleep(Self::retry_delay(retry_after.as_deref(), attempt)).await;
                    continue;
                }
                anyhow::bail!("{} ({error})", data.get("message").and_then(|v| v.as_str()).unwrap_or("LastFM returned an error"));
            }
            return Ok(data);
        }
        anyhow::bail!("LastFM request failed after 5 retries")
    }

    pub async fn get_token(&self) -> anyhow::Result<String> {
        let data = self.call("auth.getToken", vec![], false).await?;
        Ok(data.get("token").and_then(|v| v.as_str()).unwrap_or_default().to_string())
    }

    pub async fn get_session(&self, token: &str) -> anyhow::Result<String> {
        let data = self.call("auth.getSession", vec![("token", Some(token.to_string()))], false).await?;
        let key = data["session"]["key"].as_str().ok_or_else(|| anyhow::anyhow!("invalid session response"))?.to_string();
        *self.session_key.lock() = Some(key.clone());
        Ok(key)
    }

    pub async fn update_now_playing(&self, artist: String, track: String, album: Option<String>, duration: f64) -> anyhow::Result<()> {
        self.call("track.updateNowPlaying", vec![("artist", Some(artist)), ("track", Some(track)), ("album", album), ("duration", fmt_num(duration))], true).await?;
        Ok(())
    }

    pub async fn scrobble(&self, artist: String, track: String, timestamp: i64, album: Option<String>, duration: f64) -> anyhow::Result<()> {
        self.call(
            "track.scrobble",
            vec![("artist", Some(artist)), ("track", Some(track)), ("timestamp", Some(timestamp.to_string())), ("album", album), ("duration", fmt_num(duration))],
            true,
        )
        .await?;
        Ok(())
    }
}

fn fmt_num(v: f64) -> Option<String> {
    if v.is_finite() {
        Some(if v.fract() == 0.0 { format!("{}", v as i64) } else { v.to_string() })
    } else {
        Some("NaN".into())
    }
}

#[derive(Default)]
pub struct LastFm {
    client: Option<LastFmClient>,
    session_key: Option<String>,
    state_listener: Option<ListenerId>,
    signal_listener: Option<ListenerId>,
    current_video_id: Option<String>,
    scrobble_time_required: i64,
    accumulated_playback_time: i64,
    has_scrobbled: bool,
    playback_start_timestamp: i64,
    last_playback_accumulation_time: i64,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn descriptor() -> IntegrationDescriptor {
    IntegrationDescriptor { name: "LastFM", store_enable_property: "integrations.lastFMEnabled", dependent_store_properties: &[], on_setup: |_| {}, on_enabled, on_disabled }
}

fn on_enabled(app: &mut App) -> anyhow::Result<()> {
    let (Some(key), Some(secret)) = (LASTFM_API_KEY.filter(|s| !s.trim().is_empty()), LASTFM_API_SECRET.filter(|s| !s.trim().is_empty())) else {
        log::info!("Refusing to enable LastFM Integration with reason: invalid api key or secret");
        return Ok(());
    };
    if !app.memory_store.get_bool("safeStorageAvailable") {
        log::info!("Refusing to enable LastFM Integration with reason: safeStorage unavailable");
        return Ok(());
    }
    let session_key = app.config_store.get_str("lastfm.sessionKey").and_then(|enc| match app.safe_storage.decrypt_string(&enc) {
        Ok(v) => Some(v),
        Err(err) => {
            log::error!("{err}");
            None
        }
    });
    app.integrations.last_fm.client = Some(LastFmClient::new(app.http.clone(), key, secret, session_key.clone()));
    app.integrations.last_fm.session_key = session_key.clone();
    memory_store::set(app, "lastFMAuthenticationStep", if session_key.is_some() { 2 } else { 0 });

    let id = app.player_state_store.state_changed.on(update_player_state);
    app.integrations.last_fm.state_listener = Some(id);
    let id = app.player_state_store.player_signal.on(|app, signal| {
        // NewPlayback signal is considered a fresh playback e.g. a new video or a looped video
        if *signal == PlayerSignal::NewPlayback {
            app.integrations.last_fm.current_video_id = None;
        }
    });
    app.integrations.last_fm.signal_listener = Some(id);

    Ok(())
}

fn on_disabled(app: &mut App) {
    if let Some(id) = app.integrations.last_fm.state_listener.take() {
        app.player_state_store.state_changed.off(id);
    }
    if let Some(id) = app.integrations.last_fm.signal_listener.take() {
        app.player_state_store.player_signal.off(id);
    }
}

/// Settings "Authorize Last.fm in browser" / "Finish Last.fm authentication"
pub fn authenticate_step(app: &mut App) {
    if !app.integration_manager.is_enabled("LastFM") {
        return;
    }
    let Some(client) = app.integrations.last_fm.client.clone() else { return };
    let step = app.memory_store.get("lastFMAuthenticationStep").as_i64().unwrap_or(-1);
    let main = app.main.clone();
    if step == 0 {
        memory_store::set(app, "lastFMAuthenticationStep", 1);
        app.spawn(async move {
            match client.get_token().await {
                Ok(token) => {
                    let url = format!("https://www.last.fm/api/auth/?api_key={}&token={}", urlencode(LASTFM_API_KEY.unwrap_or("")), urlencode(&token));
                    main.run(move |app| {
                        memory_store::set(app, "lastFMToken", token);
                        crate::util::open_external(&url);
                    });
                }
                Err(err) => log::error!("lastfm: failed to get token {err}"),
            }
        });
    } else if step == 1 {
        let token = app.memory_store.get("lastFMToken").as_str().unwrap_or("").to_string();
        app.spawn(async move {
            match client.get_session(&token).await {
                Ok(session) => main.run(move |app| {
                    app.integrations.last_fm.session_key = Some(session.clone());
                    if let Ok(enc) = app.safe_storage.encrypt_string(&session) {
                        config_store::set(app, "lastfm.sessionKey", Value::from(enc));
                    }
                    memory_store::set(app, "lastFMAuthenticationStep", 2);
                }),
                Err(err) => log::error!("lastfm: failed to get session {err}"),
            }
        });
    }
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn maybe_update_scrobble(app: &mut App, state: &PlayerState) {
    let Some(details) = &state.video_details else { return };
    let lf = &mut app.integrations.last_fm;
    if lf.current_video_id.as_deref() == Some(details.id()) {
        return;
    }
    if !state.has_full_metadata {
        return;
    }
    // Track must be longer than 30 seconds https://www.last.fm/api/scrobbling
    if details.duration_seconds < 30.0 {
        return;
    }
    lf.current_video_id = Some(details.id().to_string());
    let percent = app.config_store.get_f64("lastfm.scrobblePercent").unwrap_or(50.0);
    let decimal = percent.clamp(50.0, 100.0) / 100.0;
    let lf = &mut app.integrations.last_fm;
    lf.scrobble_time_required = ((details.duration_seconds * decimal).round() as i64).min(240);
    lf.accumulated_playback_time = -1;
    lf.has_scrobbled = false;
    lf.playback_start_timestamp = now_ms();
}

fn update_player_state(app: &mut App, state: &PlayerState) {
    if !app.integration_manager.is_enabled("LastFM") || app.integrations.last_fm.session_key.is_none() {
        return;
    }
    maybe_update_scrobble(app, state);
    let Some(details) = &state.video_details else { return };
    if state.track_state != VideoState::Playing {
        return;
    }
    let Some(client) = app.integrations.last_fm.client.clone() else { return };
    let lf = &mut app.integrations.last_fm;
    let (artist, track, album, duration) = (details.author().to_string(), details.title().to_string(), details.album.clone(), details.duration_seconds);
    if lf.accumulated_playback_time == -1 {
        lf.accumulated_playback_time = 0;
        let c = client.clone();
        let (a, t, al) = (artist.clone(), track.clone(), album.clone());
        app.spawn(async move {
            if let Err(err) = c.update_now_playing(a, t, al, duration).await {
                log::error!("lastfm: failed to update now playing {err}");
            }
        });
    }
    let lf = &mut app.integrations.last_fm;
    if !lf.has_scrobbled && lf.scrobble_time_required >= 0 {
        let now = now_ms();
        if now - lf.last_playback_accumulation_time >= 1000 {
            lf.accumulated_playback_time += 1;
            lf.last_playback_accumulation_time = now;
        }
        if lf.accumulated_playback_time >= lf.scrobble_time_required {
            lf.has_scrobbled = true;
            let timestamp = lf.playback_start_timestamp / 1000;
            app.spawn(async move {
                if let Err(err) = client.scrobble(artist, track, timestamp, album, duration).await {
                    log::error!("lastfm: failed to scrobble {err}");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_sorted_md5() {
        let mut p = BTreeMap::new();
        p.insert("method".to_string(), "auth.getToken".to_string());
        p.insert("api_key".to_string(), "key".to_string());
        // md5("api_keykeymethodauth.getTokensecret")
        assert_eq!(sign(&p, "secret"), hex::encode(Md5::digest(b"api_keykeymethodauth.getTokensecret")));
    }
}
