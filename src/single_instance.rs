//! Electron `app.requestSingleInstanceLock()` + "second-instance" via a local socket

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::{MainHandle, UserEvent};

fn socket_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/ytmrs-instance.sock", crate::paths::runtime_prefix()))
}

/// Returns false when another instance is running (argv was forwarded to it)
pub fn acquire(rt: &tokio::runtime::Runtime, main: MainHandle) -> bool {
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
    rt.block_on(async move { start_server(main) });
    true
}

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

fn start_server(main: MainHandle) {
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
            let main = main.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                if stream.read_to_end(&mut buf).await.is_ok() {
                    if let Ok(args) = serde_json::from_slice::<Vec<String>>(&buf) {
                        main.send(UserEvent::SecondInstance(args));
                    }
                }
            });
        }
    });
}
