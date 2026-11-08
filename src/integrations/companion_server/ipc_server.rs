//! Port of the companion server IPC server (`$XDG_RUNTIME_DIR/ytmdesktop-ipc` /
//! `\\?\pipe\ytmdesktop-ipc`). Frames: i32 LE op; SELECT_VERSION carries a length
//! prefixed version string; EVENT frames are [op][nameLen][dataLen][name][json].

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast;

use super::{ServerEvent, API_VERSIONS};
use crate::app::{App, MainHandle};

pub const OP_HELLO: i32 = 0;
pub const OP_SELECT_VERSION: i32 = 1;
pub const OP_EVENT: i32 = 2;
pub const OP_PING: i32 = 3;
pub const OP_PONG: i32 = 4;

pub fn ipc_path() -> String {
    #[cfg(windows)]
    {
        r"\\?\pipe\ytmdesktop-ipc".to_string()
    }
    #[cfg(unix)]
    {
        format!("{}/ytmdesktop-ipc", crate::paths::runtime_prefix())
    }
}

pub fn encode_event(name: &str, data: &serde_json::Value) -> Vec<u8> {
    let data = data.to_string();
    let mut buf = Vec::with_capacity(12 + name.len() + data.len());
    buf.extend_from_slice(&OP_EVENT.to_le_bytes());
    buf.extend_from_slice(&(name.len() as i32).to_le_bytes());
    buf.extend_from_slice(&(data.len() as i32).to_le_bytes());
    buf.extend_from_slice(name.as_bytes());
    buf.extend_from_slice(data.as_bytes());
    buf
}

pub fn start(app: &App, events: broadcast::Sender<ServerEvent>) -> tokio::task::JoinHandle<()> {
    let main = app.main.clone();
    let path = ipc_path();
    app.spawn(async move {
        #[cfg(unix)]
        {
            // Attempt clean up of the IPC file before starting the IPC server
            match std::fs::remove_file(&path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    log::warn!("ipc: could not remove stale socket {e}");
                }
            }
            let listener = match tokio::net::UnixListener::bind(&path) {
                Ok(l) => l,
                Err(err) => {
                    log::error!("ipc: failed to listen on {path}: {err}");
                    return;
                }
            };
            log::info!("ipc: listening on {path}");
            loop {
                let Ok((stream, _)) = listener.accept().await else { continue };
                tokio::spawn(client(stream, main.clone(), events.subscribe()));
            }
        }
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ServerOptions;
            let Ok(mut server) = ServerOptions::new().first_pipe_instance(true).create(&path) else {
                log::error!("ipc: failed to listen on {path}");
                return;
            };
            log::info!("ipc: listening on {path}");
            loop {
                if server.connect().await.is_err() {
                    continue;
                }
                let connected = server;
                server = match ServerOptions::new().create(&path) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                tokio::spawn(client(connected, main.clone(), events.subscribe()));
            }
        }
    })
}

pub fn cleanup() {
    #[cfg(unix)]
    let _ = std::fs::remove_file(ipc_path());
}

async fn client<S: AsyncRead + AsyncWrite + Unpin + Send>(stream: S, main: MainHandle, mut events: broadcast::Receiver<ServerEvent>) {
    log::info!("ipc: client connected");
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut version: Option<String> = None;
    let mut heartbeated = true;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            op = reader.read_i32_le() => {
                let Ok(op) = op else { break };
                match op {
                    OP_SELECT_VERSION => {
                        if version.is_some() { break; }
                        let Ok(len) = reader.read_i32_le().await else { break };
                        if !(0..=255).contains(&len) { break; }
                        let mut buf = vec![0u8; len as usize];
                        if reader.read_exact(&mut buf).await.is_err() { break; }
                        let selected = String::from_utf8_lossy(&buf).to_string();
                        if !API_VERSIONS.contains(&selected.as_str()) { break; }
                        log::info!("ipc: client selected api version {selected}");
                        version = Some(selected);
                        // Send initial state after version selection
                        let state = main.call(|app| super::current_state_v1(app)).await.unwrap_or_default();
                        if writer.write_all(&encode_event("state-update", &state)).await.is_err() { break; }
                    }
                    OP_PING => {
                        heartbeated = true;
                        if writer.write_all(&OP_PONG.to_le_bytes()).await.is_err() { break; }
                    }
                    OP_HELLO | OP_PONG | OP_EVENT => {}
                    _ => break,
                }
            }
            event = events.recv() => match event {
                Ok(ServerEvent::StateUpdate(state)) => {
                    if version.is_some() && writer.write_all(&encode_event("state-update", &state)).await.is_err() { break; }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            },
            _ = heartbeat.tick() => {
                if !heartbeated {
                    log::info!("ipc: client missed heartbeat - disconnecting");
                    break;
                }
                heartbeated = false;
            }
        }
    }
    log::info!("ipc: client disconnected");
}

#[cfg(test)]
mod tests {
    #[test]
    fn event_frame_layout() {
        let f = super::encode_event("state-update", &serde_json::json!({"a": 1}));
        assert_eq!(i32::from_le_bytes(f[0..4].try_into().unwrap()), super::OP_EVENT);
        assert_eq!(i32::from_le_bytes(f[4..8].try_into().unwrap()), 12);
        assert_eq!(i32::from_le_bytes(f[8..12].try_into().unwrap()), 7);
        assert_eq!(&f[12..24], b"state-update");
    }
}
