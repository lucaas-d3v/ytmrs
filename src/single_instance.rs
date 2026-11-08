//! Electron `app.requestSingleInstanceLock()` + "second-instance" via a local socket

use tao::event_loop::EventLoopProxy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::UserEvent;

#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/ytmrs-instance.sock", crate::paths::runtime_prefix()))
}

#[cfg(windows)]
fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    format!(r"\\.\pipe\ytmrs-instance-{user}")
}

/// Returns false when another instance is running (argv was forwarded to it)
pub fn acquire(rt: &tokio::runtime::Runtime, proxy: EventLoopProxy<UserEvent>) -> bool {
    let args: Vec<String> = std::env::args().collect();
    let payload = serde_json::to_vec(&args).unwrap_or_default();
    // A relaunched process waits for the previous instance to go away
    let attempts = if std::env::var("YTMD_RELAUNCH").is_ok() { 50 } else { 1 };
    for attempt in 0..attempts {
        if rt.block_on(try_forward(&payload)) {
            if attempt + 1 == attempts {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }
        break;
    }
    rt.block_on(async move { start_server(proxy) });
    true
}

#[cfg(unix)]
async fn try_forward(payload: &[u8]) -> bool {
    match tokio::net::UnixStream::connect(socket_path()).await {
        Ok(mut stream) => {
            let _ = stream.write_all(payload).await;
            let _ = stream.shutdown().await;
            true
        }
        Err(_) => false,
    }
}

#[cfg(windows)]
async fn try_forward(payload: &[u8]) -> bool {
    match tokio::net::windows::named_pipe::ClientOptions::new().open(pipe_name()) {
        Ok(mut client) => {
            let _ = client.write_all(payload).await;
            let _ = client.shutdown().await;
            true
        }
        Err(_) => false,
    }
}

fn forward(proxy: &EventLoopProxy<UserEvent>, data: &[u8]) {
    if let Ok(args) = serde_json::from_slice::<Vec<String>>(data) {
        let _ = proxy.send_event(UserEvent::SecondInstance(args));
    }
}

#[cfg(unix)]
fn start_server(proxy: EventLoopProxy<UserEvent>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match tokio::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(err) => {
            log::warn!("Single instance lock could not be created: {err}");
            return;
        }
    };
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { continue };
            let proxy = proxy.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                if stream.read_to_end(&mut buf).await.is_ok() {
                    forward(&proxy, &buf);
                }
            });
        }
    });
}

#[cfg(windows)]
fn start_server(proxy: EventLoopProxy<UserEvent>) {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = pipe_name();
    let Ok(mut server) = ServerOptions::new().first_pipe_instance(true).create(&name) else {
        log::warn!("Single instance lock could not be created");
        return;
    };
    tokio::spawn(async move {
        loop {
            if server.connect().await.is_err() {
                continue;
            }
            let mut connected = server;
            server = match ServerOptions::new().create(&name) {
                Ok(s) => s,
                Err(_) => return,
            };
            let proxy = proxy.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                if connected.read_to_end(&mut buf).await.is_ok() {
                    forward(&proxy, &buf);
                }
            });
        }
    });
}
