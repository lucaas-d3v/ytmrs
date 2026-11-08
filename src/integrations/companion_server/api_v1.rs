//! Port of companion-server/api/v1 (+ errors, schemas and the HTTP shell)

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use regex::Regex;
use serde_json::{json, Value};
use socketioxide::extract::{SocketRef, TryData};
use socketioxide::handler::ConnectHandler;
use socketioxide::{SocketIo, TransportType};

use super::auth::{self, CodeData, TemporaryCodes};
use super::rate_limit::{humanize, RateLimiter};
use super::{SharedSafeStorage, API_VERSIONS, REALTIME_NS};
use crate::app::MainHandle;
use crate::services::protected_api::{invoke_message, remote_execute};
use crate::shared::player_state::RepeatMode;

#[derive(Clone)]
pub struct TokenId(pub String);

#[derive(Clone)]
pub struct ServerState {
    main: MainHandle,
    safe: SharedSafeStorage,
    codes: TemporaryCodes,
    limiter: RateLimiter,
    authorization_windows: Arc<AtomicUsize>,
}

impl ServerState {
    pub fn new(main: MainHandle, safe: SharedSafeStorage) -> Self {
        Self { main, safe, codes: TemporaryCodes::default(), limiter: RateLimiter::default(), authorization_windows: Arc::new(AtomicUsize::new(0)) }
    }
}

/// Fastify style error reply
#[derive(Debug)]
pub struct ApiError {
    status: u16,
    code: Option<&'static str>,
    message: String,
}

impl ApiError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code: Some(code), message: message.into() }
    }
    fn validation(message: impl Into<String>) -> Self {
        Self { status: 400, code: Some("FST_ERR_VALIDATION"), message: message.into() }
    }
    fn invalid_volume(v: &Value) -> Self {
        Self::new(400, "INVALID_VOLUME", format!("Volume '{}' is invalid", js_str(v)))
    }
    fn invalid_repeat_mode(v: &Value) -> Self {
        Self::new(400, "INVALID_REPEAT_MODE", format!("Repeat mode '{}' cannot be set", js_str(v)))
    }
    fn invalid_position(v: &Value) -> Self {
        Self::new(400, "INVALID_SEEK_POSITION", format!("Seek position '{}' is invalid", js_str(v)))
    }
    fn invalid_queue_index(v: &Value) -> Self {
        Self::new(400, "INVALID_QUEUE_INDEX", format!("'{}' is an invalid queue index for the current queue", js_str(v)))
    }
    fn invalid_change_request() -> Self {
        Self::new(400, "INVALID_CHANGE_REQUEST", "'videoId', 'playlistId', or both must be provided")
    }
    fn invalid_queue_add_request() -> Self {
        Self::new(400, "INVALID_QUEUE_ADD_REQUEST", "'videoId' or 'playlistId', must be provided")
    }
    fn authorization_disabled() -> Self {
        Self::new(403, "AUTHORIZATION_DISABLED", "Authorization requests are disabled")
    }
    fn authorization_invalid() -> Self {
        Self::new(400, "AUTHORIZATION_INVALID", "Authorization invalid")
    }
    fn authorization_time_out() -> Self {
        Self::new(504, "AUTHORIZATION_TIME_OUT", "Authorization timed out")
    }
    fn authorization_denied() -> Self {
        Self::new(403, "AUTHORIZATION_DENIED", "Authorization request denied")
    }
    fn authorization_too_many() -> Self {
        Self::new(503, "AUTHORIZATION_TOO_MANY", "Too many authorization requests currently active")
    }
    fn youtube_music_time_out() -> Self {
        Self::new(504, "YOUTUBE_MUSIC_TIME_OUT", "Response from YouTube Music took too long")
    }
    fn internal() -> Self {
        Self { status: 500, code: None, message: "An internal server error occurred".into() }
    }
}

fn js_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn reason(status: u16) -> &'static str {
    StatusCode::from_u16(status).ok().and_then(|s| s.canonical_reason()).unwrap_or("Error")
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "statusCode": self.status });
        if let Some(code) = self.code {
            body["code"] = json!(code);
        }
        body["error"] = json!(reason(self.status));
        body["message"] = json!(self.message);
        (StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), axum::Json(body)).into_response()
    }
}

type ApiResult = Result<Response, Response>;

fn rate_limit(state: &ServerState, route: &str, key: &str, max: u32, window: Duration) -> Result<HeaderMap, Response> {
    let r = state.limiter.check(&format!("{route}|{key}"), max, window);
    let mut headers = HeaderMap::new();
    headers.insert("x-ratelimit-limit", HeaderValue::from(r.limit));
    headers.insert("x-ratelimit-remaining", HeaderValue::from(r.remaining));
    headers.insert("x-ratelimit-reset", HeaderValue::from(r.reset.as_secs().max(if r.reset.is_zero() { 0 } else { 1 })));
    if !r.allowed {
        headers.insert("retry-after", HeaderValue::from(r.reset.as_secs().max(1)));
        let body = json!({ "statusCode": 429, "error": "Too Many Requests", "message": format!("Rate limit exceeded, retry in {}", humanize(r.reset)) });
        return Err((StatusCode::TOO_MANY_REQUESTS, headers, axum::Json(body)).into_response());
    }
    Ok(headers)
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": "UNAUTHORIZED" }))).into_response()
}

async fn authenticate(state: &ServerState, headers: &HeaderMap) -> Result<String, Response> {
    let Some(token) = headers.get("authorization").and_then(|v| v.to_str().ok()) else { return Err(unauthorized()) };
    match auth::is_auth_valid(&state.main, &state.safe, token).await {
        (true, Some(id)) => Ok(id),
        _ => Err(unauthorized()),
    }
}

fn parse_body(body: &Bytes) -> Result<Value, Response> {
    serde_json::from_slice::<Value>(body).map_err(|_| {
        ApiError::new(400, "FST_ERR_CTP_INVALID_JSON_BODY", "Body is not valid JSON but content-type is set to 'application/json'").into_response()
    })
}

fn require_string(body: &Value, key: &str, pattern: &Regex, pattern_src: &str) -> Result<String, ApiError> {
    match body.get(key) {
        None => Err(ApiError::validation(format!("body must have required property '{key}'"))),
        Some(Value::String(s)) => {
            if pattern.is_match(s) {
                Ok(s.clone())
            } else {
                Err(ApiError::validation(format!("body/{key} must match pattern \"{pattern_src}\"")))
            }
        }
        Some(_) => Err(ApiError::validation(format!("body/{key} must be string"))),
    }
}

const APP_ID_PATTERN: &str = r"^[a-z0-9_\-]{2,32}$";
const APP_NAME_PATTERN: &str = r"^[\w\W]{2,48}$";
const APP_VERSION_PATTERN: &str = r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$";
const CODE_PATTERN: &str = r"^[0-9]{4}$";

fn re(p: &str) -> Regex {
    // [\w\W] in JS counts UTF-16 units; char based is close enough
    Regex::new(p).expect("valid regex")
}

pub fn validate_request_code(body: &Value) -> Result<CodeData, ApiError> {
    if !body.is_object() {
        return Err(ApiError::validation("body must be object"));
    }
    Ok(CodeData {
        app_id: require_string(body, "appId", &re(APP_ID_PATTERN), APP_ID_PATTERN)?,
        app_name: require_string(body, "appName", &re(APP_NAME_PATTERN), APP_NAME_PATTERN)?,
        app_version: require_string(body, "appVersion", &re(APP_VERSION_PATTERN), APP_VERSION_PATTERN)?,
    })
}

pub fn validate_request_token(body: &Value) -> Result<(String, String), ApiError> {
    if !body.is_object() {
        return Err(ApiError::validation("body must be object"));
    }
    Ok((require_string(body, "appId", &re(APP_ID_PATTERN), APP_ID_PATTERN)?, require_string(body, "code", &re(CODE_PATTERN), CODE_PATTERN)?))
}

/// Validated command (APIV1CommandRequestBody union)
#[derive(Debug, PartialEq)]
pub enum Command {
    Simple(&'static str),
    SetVolume(f64),
    SeekTo(f64),
    RepeatMode(Value),
    PlayQueueIndex(f64),
    ChangeVideo { video_id: Option<String>, playlist_id: Option<String> },
    QueueAdd { video_id: Option<String>, playlist_id: Option<String>, index: f64 },
    QueueRemove(f64),
    QueueMove { from: f64, to: f64 },
}

pub fn validate_command(body: &Value) -> Result<Command, ApiError> {
    let any_of = || ApiError::validation("body must match a schema in anyOf");
    let command = body.get("command").and_then(|v| v.as_str()).ok_or_else(any_of)?;
    let data = body.get("data");
    let num = |min: f64, max: f64| -> Result<f64, ApiError> {
        match data.and_then(|d| d.as_f64()) {
            Some(v) if v >= min && v <= max => Ok(v),
            _ => Err(any_of()),
        }
    };
    let opt_str = |o: &Value, k: &str| -> Result<Option<String>, ApiError> {
        match o.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(any_of()),
        }
    };
    Ok(match command {
        "playPause" | "play" | "pause" | "volumeUp" | "volumeDown" | "mute" | "unmute" | "next" | "previous" | "shuffle" | "toggleLike" | "toggleDislike" => {
            Command::Simple(match command {
                "playPause" => "playPause",
                "play" => "play",
                "pause" => "pause",
                "volumeUp" => "volumeUp",
                "volumeDown" => "volumeDown",
                "mute" => "mute",
                "unmute" => "unmute",
                "next" => "next",
                "previous" => "previous",
                "shuffle" => "shuffle",
                "toggleLike" => "toggleLike",
                _ => "toggleDislike",
            })
        }
        "setVolume" => Command::SetVolume(num(0.0, 100.0)?),
        "seekTo" => Command::SeekTo(num(0.0, f64::MAX)?),
        "playQueueIndex" => Command::PlayQueueIndex(num(0.0, f64::MAX)?),
        "queueRemove" => Command::QueueRemove(num(0.0, f64::MAX)?),
        "repeatMode" => {
            let v = data.cloned().ok_or_else(any_of)?;
            // Type.Enum(RepeatMode) accepts -1, 0, 1, 2
            match v.as_i64() {
                Some(-1..=2) if v.as_f64().map(|f| f.fract() == 0.0).unwrap_or(false) => Command::RepeatMode(v),
                _ => return Err(any_of()),
            }
        }
        "changeVideo" => {
            let d = data.filter(|d| d.is_object()).ok_or_else(any_of)?;
            Command::ChangeVideo { video_id: opt_str(d, "videoId")?, playlist_id: opt_str(d, "playlistId")? }
        }
        "queueAdd" => {
            let d = data.filter(|d| d.is_object()).ok_or_else(any_of)?;
            let index = d.get("index").and_then(|v| v.as_f64()).filter(|v| *v >= 0.0).ok_or_else(any_of)?;
            Command::QueueAdd { video_id: opt_str(d, "videoId")?, playlist_id: opt_str(d, "playlistId")?, index }
        }
        "queueMove" => {
            let d = data.filter(|d| d.is_object()).ok_or_else(any_of)?;
            let from = d.get("fromIndex").and_then(|v| v.as_f64()).filter(|v| *v >= 0.0).ok_or_else(any_of)?;
            let to = d.get("toIndex").and_then(|v| v.as_f64()).filter(|v| *v >= 0.0).ok_or_else(any_of)?;
            Command::QueueMove { from, to }
        }
        _ => return Err(any_of()),
    })
}

fn n(v: f64) -> Value {
    if v.fract() == 0.0 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

/// sendCommand: checks against the current player state then forwards to RemoteControl
fn send_command(app: &mut crate::app::App, command: Command) -> Result<(), ApiError> {
    let state = app.player_state_store.get_state();
    match command {
        Command::Simple(c) => remote_execute(app, c, vec![]),
        Command::SetVolume(v) => {
            if !(0.0..=100.0).contains(&v) {
                return Err(ApiError::invalid_volume(&n(v)));
            }
            remote_execute(app, "setVolume", vec![n(v)]);
        }
        Command::SeekTo(p) => {
            if let Some(d) = &state.video_details {
                if p < 0.0 || p > d.duration_seconds {
                    return Err(ApiError::invalid_position(&n(p)));
                }
            }
            remote_execute(app, "seekTo", vec![n(p)]);
        }
        Command::ChangeVideo { video_id, playlist_id } => {
            if video_id.is_none() && playlist_id.is_none() {
                return Err(ApiError::invalid_change_request());
            }
            let mut watch = serde_json::Map::new();
            if let Some(v) = video_id {
                watch.insert("videoId".into(), json!(v));
            }
            if let Some(p) = playlist_id {
                watch.insert("playlistId".into(), json!(p));
            }
            remote_execute(app, "navigate", vec![json!({ "watchEndpoint": watch })]);
        }
        Command::RepeatMode(v) => match RepeatMode::from_i64(v.as_i64().unwrap_or(-2)) {
            Some(RepeatMode::None) => remote_execute(app, "repeatMode", vec![json!("NONE")]),
            Some(RepeatMode::All) => remote_execute(app, "repeatMode", vec![json!("ALL")]),
            Some(RepeatMode::One) => remote_execute(app, "repeatMode", vec![json!("ONE")]),
            _ => return Err(ApiError::invalid_repeat_mode(&v)),
        },
        Command::PlayQueueIndex(i) => {
            if let Some(q) = &state.queue {
                if i > (q.items.len() + q.automix_items.len()) as f64 - 1.0 {
                    return Err(ApiError::invalid_queue_index(&n(i)));
                }
            }
            remote_execute(app, "playQueueIndex", vec![n(i)]);
        }
        Command::QueueAdd { video_id, playlist_id, index } => {
            if video_id.is_some() == playlist_id.is_some() {
                return Err(ApiError::invalid_queue_add_request());
            }
            if let Some(q) = &state.queue {
                if index > q.items.len() as f64 {
                    return Err(ApiError::invalid_queue_index(&n(index)));
                }
            }
            remote_execute(app, "queueAdd", vec![json!({ "videoId": video_id, "playlistId": playlist_id, "index": n(index) })]);
        }
        Command::QueueRemove(i) => {
            if let Some(q) = &state.queue {
                if i > q.items.len() as f64 - 1.0 {
                    return Err(ApiError::invalid_queue_index(&n(i)));
                }
            }
            remote_execute(app, "queueRemove", vec![n(i)]);
        }
        Command::QueueMove { from, to } => {
            if let Some(q) = &state.queue {
                if from > q.items.len() as f64 - 1.0 {
                    return Err(ApiError::invalid_queue_index(&n(from)));
                }
                if to > q.items.len() as f64 - 1.0 {
                    return Err(ApiError::invalid_queue_index(&n(to)));
                }
            }
            remote_execute(app, "queueMove", vec![json!({ "fromIndex": n(from), "toIndex": n(to) })]);
        }
    }
    Ok(())
}

fn with_headers(headers: HeaderMap, response: impl IntoResponse) -> Response {
    let mut r = response.into_response();
    r.headers_mut().extend(headers);
    r
}

async fn metadata(State(state): State<ServerState>, ConnectInfo(addr): ConnectInfo<SocketAddr>) -> ApiResult {
    let headers = rate_limit(&state, "global", &addr.ip().to_string(), 100, Duration::from_secs(60))?;
    Ok(with_headers(headers, axum::Json(json!({ "apiVersions": API_VERSIONS }))))
}

async fn not_found(State(state): State<ServerState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, method: axum::http::Method, uri: axum::http::Uri) -> ApiResult {
    let headers = rate_limit(&state, "global", &addr.ip().to_string(), 100, Duration::from_secs(60))?;
    let body = json!({ "message": format!("Route {}:{} not found", method, uri.path()), "error": "Not Found", "statusCode": 404 });
    Ok(with_headers(headers, (StatusCode::NOT_FOUND, axum::Json(body))))
}

async fn request_code(State(state): State<ServerState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, body: Bytes) -> ApiResult {
    let headers = rate_limit(&state, "requestcode", &addr.ip().to_string(), 5, Duration::from_secs(60))?;
    let body = parse_body(&body)?;
    let data = validate_request_code(&body).map_err(IntoResponse::into_response)?;
    let enabled = state.main.call(|app| app.memory_store.get_bool("companionServerAuthWindowEnabled")).await.unwrap_or(false);
    // API Users: The user has companion server authorization disabled, show a feedback error accordingly
    if !enabled {
        return Err(ApiError::authorization_disabled().into_response());
    }
    match state.codes.create(data) {
        Some(code) => Ok(with_headers(headers, axum::Json(json!({ "code": code })))),
        None => Err(ApiError::authorization_time_out().into_response()),
    }
}

struct AuthWindowGuard {
    main: MainHandle,
    window: String,
    counter: Arc<AtomicUsize>,
}

impl Drop for AuthWindowGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
        let window = self.window.clone();
        self.main.run(move |app| {
            if app.window_manager.has_window(&window) {
                if let Some(w) = app.window_manager.get_mut(&window) {
                    w.requested_closure = true;
                }
                crate::services::window_manager::destroy_window(app, &window);
            }
        });
    }
}

async fn request_token(State(state): State<ServerState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, body: Bytes) -> ApiResult {
    let headers = rate_limit(&state, "request", &addr.ip().to_string(), 5, Duration::from_secs(60))?;
    let body = parse_body(&body)?;
    let (app_id, code) = validate_request_token(&body).map_err(IntoResponse::into_response)?;
    let enabled = state.main.call(|app| app.memory_store.get_bool("companionServerAuthWindowEnabled")).await.unwrap_or(false);
    // Too many authorization windows open (prevents malicious use of spamming auth windows)
    if state.authorization_windows.load(Ordering::SeqCst) >= 5 {
        return Err(ApiError::authorization_too_many().into_response());
    }
    if !enabled {
        return Err(ApiError::authorization_disabled().into_response());
    }
    // API Users: Make sure you /requestcode above
    let Some(data) = state.codes.take_if_valid(&app_id, &code) else {
        return Err(ApiError::authorization_invalid().into_response());
    };

    let request_id = uuid::Uuid::new_v4().to_string();
    let window_name = format!("CompanionAuthorization-{request_id}");
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    state.authorization_windows.fetch_add(1, Ordering::SeqCst);
    let guard = AuthWindowGuard { main: state.main.clone(), window: window_name.clone(), counter: state.authorization_windows.clone() };

    let app_name = data.app_name.clone();
    let code_c = code.clone();
    let wn = window_name.clone();
    let created = state
        .main
        .call(move |app| open_authorization_window(app, &wn, &app_name, &code_c, tx))
        .await
        .unwrap_or(false);
    if !created {
        drop(guard);
        return Err(ApiError::internal().into_response());
    }

    let authorized = tokio::time::timeout(Duration::from_secs(30), rx).await.ok().and_then(|r| r.ok()).unwrap_or(false);
    drop(guard);

    if authorized {
        let token = auth::create_auth_token(&state.main, &state.safe, &data).await.map_err(|err| {
            log::error!("companion server: {err}");
            ApiError::internal().into_response()
        })?;
        state.main.run(|app| crate::services::memory_store::set(app, "companionServerAuthWindowEnabled", false));
        Ok(with_headers(headers, axum::Json(json!({ "token": token }))))
    } else {
        Err(ApiError::authorization_denied().into_response())
    }
}

fn open_authorization_window(app: &mut crate::app::App, name: &str, app_name: &str, code: &str, tx: tokio::sync::oneshot::Sender<bool>) -> bool {
    use crate::services::window_manager::{self as wm, AppWindowOptions, NewWindowPolicy};
    let mut opts = AppWindowOptions::new(name, "authorize-companion", "preload/authorize-companion.js", 640.0, 480.0);
    opts.title = "YtmRs - Companion Authorization".into();
    opts.minimizable = false;
    opts.maximizable = false;
    opts.resizable = false;
    opts.new_window = NewWindowPolicy::Deny;
    opts.extra_init.push(format!("window.__ytmdAuthorization = Object.freeze({});", json!({ "appName": app_name, "code": code })));
    if let Err(err) = wm::create_window(app, opts) {
        log::error!("Failed to create authorization window: {err:?}");
        return false;
    }
    let sender = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));
    let w = app.window_manager.get_mut(name).unwrap();
    let s = sender.clone();
    w.ipc.on("companionAuthorization:result", move |_, _, args| {
        if let Some(tx) = s.borrow_mut().take() {
            let _ = tx.send(args.first().and_then(|v| v.as_bool()).unwrap_or(false));
        }
    });
    let s = sender;
    w.events.close.on(move |_, _| {
        if let Some(tx) = s.borrow_mut().take() {
            let _ = tx.send(false);
        }
    });
    let window_name = name.to_string();
    app.window_manager.when_ready(name, move |app| {
        if let Some(w) = app.window_manager.get(&window_name) {
            w.window.set_focus();
            w.window.request_user_attention(Some(tao::window::UserAttentionType::Informational));
        }
    });
    true
}

async fn playlists(State(state): State<ServerState>, headers: HeaderMap) -> ApiResult {
    let id = authenticate(&state, &headers).await?;
    // This endpoint sends a real API request to YTM. API users: Please cache playlists
    let rl = rate_limit(&state, "playlists", &id, 1, Duration::from_secs(30))?;
    let fut = state.main.call(|app| invoke_message(app, "RemoteControl", "getPlaylists", vec![])).await.ok_or_else(|| ApiError::internal().into_response())?;
    match fut.await {
        Ok(result) => Ok(with_headers(rl, axum::Json(result.into_iter().next().unwrap_or(json!([]))))),
        Err(_) => Err(ApiError::youtube_music_time_out().into_response()),
    }
}

async fn player_state(State(state): State<ServerState>, headers: HeaderMap) -> ApiResult {
    let id = authenticate(&state, &headers).await?;
    // API users: Please utilize the realtime websocket to get the state
    let rl = rate_limit(&state, "state", &id, 1, Duration::from_secs(5))?;
    let value = state.main.call(|app| super::current_state_v1(app)).await.unwrap_or(Value::Null);
    Ok(with_headers(rl, axum::Json(value)))
}

async fn command(State(state): State<ServerState>, headers: HeaderMap, body: Bytes) -> ApiResult {
    // Schema validation happens before preHandler (auth) in fastify
    let body = parse_body(&body)?;
    let command = validate_command(&body).map_err(IntoResponse::into_response)?;
    let id = authenticate(&state, &headers).await?;
    let rl = rate_limit(&state, "command", &id, 2, Duration::from_secs(1))?;
    let result = state.main.call(move |app| send_command(app, command)).await.unwrap_or_else(|| Err(ApiError::internal()));
    match result {
        Ok(()) => Ok(with_headers(rl, StatusCode::NO_CONTENT)),
        Err(e) => Err(e.into_response()),
    }
}

#[derive(serde::Deserialize)]
struct HandshakeAuth {
    token: Option<String>,
}

#[derive(Debug)]
struct Unauthenticated;
impl std::fmt::Display for Unauthenticated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Authentication not provided or invalid")
    }
}

pub fn router(state: ServerState, cors_wildcard: bool) -> (Router, SocketIo) {
    let (layer, io) = SocketIo::builder().transports([TransportType::Websocket]).build_layer();

    // Disconnect connections to the default namespace
    io.ns("/", async |socket: SocketRef| {
        let _ = socket.disconnect();
    });
    let auth_state = state.clone();
    let middleware = move |socket: SocketRef, TryData(auth): TryData<HandshakeAuth>| {
        let st = auth_state.clone();
        async move {
            let token = auth.ok().and_then(|a| a.token).unwrap_or_default();
            match auth::is_auth_valid(&st.main, &st.safe, &token).await {
                (true, Some(id)) => {
                    socket.extensions.insert(TokenId(id));
                    Ok(())
                }
                _ => Err(Unauthenticated),
            }
        }
    };
    io.ns(REALTIME_NS, (async |_socket: SocketRef| {}).with(middleware));

    let mut router = Router::new()
        .route("/metadata", get(metadata))
        .route("/api/v1/auth/requestcode", post(request_code))
        .route("/api/v1/auth/request", post(request_token))
        .route("/api/v1/playlists", get(playlists))
        .route("/api/v1/state", get(player_state))
        .route("/api/v1/command", post(command))
        .fallback(not_found)
        .with_state(state)
        .layer(layer);
    if cors_wildcard {
        router = router.layer(tower_http::cors::CorsLayer::new().allow_origin(tower_http::cors::Any).allow_methods(tower_http::cors::Any).allow_headers(tower_http::cors::Any));
    }
    (router, io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_request_code() {
        assert!(validate_request_code(&json!({"appId": "my_app", "appName": "My App", "appVersion": "1.0.0"})).is_ok());
        let e = validate_request_code(&json!({"appId": "My App", "appName": "x1", "appVersion": "1.0.0"})).unwrap_err();
        assert_eq!(e.code, Some("FST_ERR_VALIDATION"));
        assert!(validate_request_code(&json!({"appId": "ok", "appName": "x", "appVersion": "1.0.0"})).is_err());
        assert!(validate_request_code(&json!({"appId": "ok", "appName": "xx", "appVersion": "1.0"})).is_err());
        assert!(validate_request_code(&json!({"appId": "ok", "appName": "xx"})).unwrap_err().message.contains("required property 'appVersion'"));
    }

    #[test]
    fn validates_commands() {
        assert_eq!(validate_command(&json!({"command": "playPause"})).unwrap(), Command::Simple("playPause"));
        assert_eq!(validate_command(&json!({"command": "setVolume", "data": 50})).unwrap(), Command::SetVolume(50.0));
        assert!(validate_command(&json!({"command": "setVolume", "data": 101})).is_err());
        assert!(validate_command(&json!({"command": "repeatMode", "data": 3})).is_err());
        assert!(validate_command(&json!({"command": "repeatMode", "data": 1})).is_ok());
        assert!(validate_command(&json!({"command": "nope"})).is_err());
        assert!(validate_command(&json!({"command": "queueMove", "data": {"fromIndex": 1, "toIndex": 2}})).is_ok());
        assert!(validate_command(&json!({"command": "changeVideo", "data": {"videoId": 5}})).is_err());
    }

    #[test]
    fn error_format() {
        let r = ApiError::invalid_volume(&json!(120)).into_response();
        assert_eq!(r.status(), 400);
    }
}
