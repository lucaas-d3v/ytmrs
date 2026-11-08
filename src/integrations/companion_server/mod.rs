//! Port of the CompanionServer integration: HTTP API (+ Socket.IO realtime), local
//! IPC server and mDNS advertisement.

pub mod api_v1;
pub mod auth;
pub mod ipc_server;
pub mod rate_limit;

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use socketioxide::SocketIo;
use tokio::sync::{broadcast, oneshot};

use crate::app::{App, TimerHandle};
use crate::events::ListenerId;
use crate::services::integration_manager::IntegrationDescriptor;
use crate::services::memory_store;
use crate::shared::player_state::transform_player_state_v1;

pub const API_VERSIONS: [&str; 1] = ["v1"];
pub const LISTEN_PORT: u16 = 9863;
pub const REALTIME_NS: &str = "/api/v1/realtime";

/// Events fanned out to the HTTP realtime namespace and IPC clients
#[derive(Clone, Debug)]
pub enum ServerEvent {
    StateUpdate(Value),
    PlaylistCreated(Value),
    PlaylistDeleted(String),
}

#[derive(Default)]
pub struct CompanionServer {
    shutdown: Option<oneshot::Sender<()>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    io: Option<SocketIo>,
    events: Option<broadcast::Sender<ServerEvent>>,
    listeners: Vec<ListenerId>,
    playlist_listeners: (Option<ListenerId>, Option<ListenerId>),
    store_listener: Option<ListenerId>,
    memory_listener: Option<ListenerId>,
    auth_window_timeout: Option<TimerHandle>,
    mdns: Option<mdns_sd::ServiceDaemon>,
}

pub fn descriptor() -> IntegrationDescriptor {
    IntegrationDescriptor {
        name: "CompanionServer",
        store_enable_property: "integrations.companionServerEnabled",
        dependent_store_properties: &["integrations.companionServerCORSWildcardEnabled"],
        on_setup: |_| {},
        on_enabled,
        on_disabled,
    }
}

fn on_enabled(app: &mut App) -> anyhow::Result<()> {
    if !app.memory_store.get_bool("safeStorageAvailable") {
        log::info!("Refusing to enable Companion Server Integration with reason: safeStorage unavailable");
        return Ok(());
    }

    let (events_tx, _) = broadcast::channel::<ServerEvent>(256);
    app.integrations.companion_server.events = Some(events_tx.clone());

    // IPC server
    let ipc = ipc_server::start(app, events_tx.clone());
    app.integrations.companion_server.tasks.push(ipc);

    // Memory store listener: 5 minute auth window timeout
    let id = app.memory_store.on_state_changed(|app, change| {
        let new = change.new.get("companionServerAuthWindowEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
        let old = change.old.get("companionServerAuthWindowEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
        if new && !old {
            let timer = app.set_timeout(Duration::from_secs(5 * 60), |app| {
                app.integrations.companion_server.auth_window_timeout = None;
                memory_store::set(app, "companionServerAuthWindowEnabled", false);
            });
            if let Some(old) = app.integrations.companion_server.auth_window_timeout.replace(timer) {
                old.cancel();
            }
        } else if !new {
            if let Some(t) = app.integrations.companion_server.auth_window_timeout.take() {
                t.cancel();
            }
        }
    });
    app.integrations.companion_server.memory_listener = Some(id);

    // HTTP server (bind synchronously so failures surface as an integration error)
    let std_listener = std::net::TcpListener::bind(("0.0.0.0", LISTEN_PORT))?;
    std_listener.set_nonblocking(true)?;
    let cors = app.config_store.get_bool("integrations.companionServerCORSWildcardEnabled");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = api_v1::ServerState::new(app.main.clone(), app.safe_storage.clone());
    let (router, io) = api_v1::router(state, cors);
    let server = app.spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(err) => {
                log::error!("companion server: {err}");
                return;
            }
        };
        let serve = axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        if let Err(err) = serve.await {
            log::error!("companion server stopped: {err}");
        }
    });
    app.integrations.companion_server.tasks.push(server);
    app.integrations.companion_server.shutdown = Some(shutdown_tx);

    // Realtime fan out
    let mut rx = events_tx.subscribe();
    let io_clone = io.clone();
    let fanout = app.spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(e) => e,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let Some(ns) = io_clone.of(REALTIME_NS) else { continue };
            let _ = match event {
                ServerEvent::StateUpdate(v) => ns.emit("state-update", &v).await,
                ServerEvent::PlaylistCreated(v) => ns.emit("playlist-created", &v).await,
                ServerEvent::PlaylistDeleted(id) => ns.emit("playlist-deleted", &id).await,
            };
        }
    });
    app.integrations.companion_server.tasks.push(fanout);
    app.integrations.companion_server.io = Some(io);

    let tx = events_tx.clone();
    let id = app.player_state_store.state_changed.on(move |_app, state| {
        let _ = tx.send(ServerEvent::StateUpdate(transform_player_state_v1(state)));
    });
    app.integrations.companion_server.listeners.push(id);
    let tx = events_tx.clone();
    let created = app.player_state_store.playlist_created.on(move |_, p| {
        let _ = tx.send(ServerEvent::PlaylistCreated(p.clone()));
    });
    let tx = events_tx;
    let deleted = app.player_state_store.playlist_deleted.on(move |_, id| {
        let _ = tx.send(ServerEvent::PlaylistDeleted(id.clone()));
    });
    app.integrations.companion_server.playlist_listeners = (Some(created), Some(deleted));

    // Disconnect sockets whose tokens were revoked
    let id = app.config_store.on_did_change("integrations", |app, new, _| {
        let valid: Vec<String> = new
            .get("companionServerAuthTokens")
            .and_then(|v| v.as_str())
            .and_then(|enc| app.safe_storage.decrypt_string(enc).ok())
            .and_then(|json| serde_json::from_str::<Vec<crate::shared::types::AuthToken>>(&json).ok())
            .map(|tokens| tokens.into_iter().map(|t| t.id).collect())
            .unwrap_or_default();
        if let Some(io) = &app.integrations.companion_server.io {
            for ns in io.nsps() {
                for socket in ns.sockets() {
                    let token_id = socket.extensions.get::<api_v1::TokenId>().map(|t| t.0);
                    if !token_id.map(|id| valid.contains(&id)).unwrap_or(false) {
                        let _ = socket.disconnect();
                    }
                }
            }
        }
    });
    app.integrations.companion_server.store_listener = Some(id);

    create_mdns(app);
    Ok(())
}

fn on_disabled(app: &mut App) {
    memory_store::set(app, "companionServerAuthWindowEnabled", false);
    let cs = &mut app.integrations.companion_server;
    if let Some(id) = cs.memory_listener.take() {
        app.memory_store.remove_on_state_changed(id);
    }
    let cs = &mut app.integrations.companion_server;
    if let Some(tx) = cs.shutdown.take() {
        let _ = tx.send(());
    }
    if let Some(io) = cs.io.take() {
        app.rt.spawn(async move {
            let _ = io.close().await;
        });
    }
    let cs = &mut app.integrations.companion_server;
    for t in cs.tasks.drain(..) {
        t.abort();
    }
    if let Some(id) = cs.store_listener.take() {
        app.config_store.off(id);
    }
    let cs = &mut app.integrations.companion_server;
    for id in cs.listeners.drain(..) {
        app.player_state_store.state_changed.off(id);
    }
    let cs = &mut app.integrations.companion_server;
    if let Some(id) = cs.playlist_listeners.0.take() {
        app.player_state_store.playlist_created.off(id);
    }
    let cs = &mut app.integrations.companion_server;
    if let Some(id) = cs.playlist_listeners.1.take() {
        app.player_state_store.playlist_deleted.off(id);
    }
    let cs = &mut app.integrations.companion_server;
    cs.events = None;
    if let Some(t) = cs.auth_window_timeout.take() {
        t.cancel();
    }
    if let Some(mdns) = cs.mdns.take() {
        let _ = mdns.shutdown();
    }
    ipc_server::cleanup();
}

fn create_mdns(app: &mut App) {
    let hostname = hostname::get().map(|h| h.to_string_lossy().split('.').next().unwrap_or("ytmdesktop").to_string()).unwrap_or_else(|_| "ytmdesktop".into());
    let service_type = "_ytmdesktop._tcp.local.";
    let host = format!("{hostname}.local.");
    match mdns_sd::ServiceDaemon::new() {
        Ok(daemon) => {
            let info = mdns_sd::ServiceInfo::new(service_type, &hostname, &host, "", LISTEN_PORT, None::<std::collections::HashMap<String, String>>).map(|i| i.enable_addr_auto());
            match info {
                Ok(info) => {
                    if let Err(err) = daemon.register(info) {
                        log::warn!("mdns: could not register service: {err}");
                    } else {
                        log::info!("mdns: created mdns server advertising companion server with instance name {hostname}.{service_type}");
                    }
                }
                Err(err) => log::warn!("mdns: invalid service info: {err}"),
            }
            app.integrations.companion_server.mdns = Some(daemon);
        }
        Err(err) => log::warn!("mdns: could not start: {err}"),
    }
}

/// Shared by HTTP API and IPC server
pub fn current_state_v1(app: &App) -> Value {
    transform_player_state_v1(app.player_state_store.state())
}

pub type SharedSafeStorage = Arc<crate::services::safe_storage::SafeStorage>;
