//! IPC from the YouTube Music page to the main process.
//!
//! The page is untrusted: it can only reach the channels registered here (protected API
//! transport, hook status and lyrics data). Messages are `{"c":channel,"a":[args]}` posted by
//! `web/ytm/scripts.js` through the `ytmd` WebKit script message handler.

use std::collections::HashMap;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::Value;

use crate::app::App;

#[derive(Deserialize)]
struct IpcMessage {
    c: String,
    #[serde(default)]
    a: Vec<Value>,
}

pub type IpcListener = Rc<dyn Fn(&mut App, &[Value])>;

#[derive(Default)]
pub struct IpcRegistry {
    on: HashMap<String, Vec<IpcListener>>,
}

impl IpcRegistry {
    pub fn on(&mut self, channel: &str, f: impl Fn(&mut App, &[Value]) + 'static) {
        self.on.entry(channel.to_string()).or_default().push(Rc::new(f));
    }

    fn listeners(&self, channel: &str) -> Vec<IpcListener> {
        self.on.get(channel).cloned().unwrap_or_default()
    }
}

pub fn dispatch(app: &mut App, body: &str) {
    let msg: IpcMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(err) => {
            log::warn!("ipc: invalid message from the YTM view: {err}");
            return;
        }
    };
    log::trace!("ipc: '{}' from the YTM view", msg.c);
    for listener in app.ipc.listeners(&msg.c) {
        listener(app, &msg.a);
    }
}
