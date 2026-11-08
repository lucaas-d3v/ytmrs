//! IPC between webviews and the main process.
//!
//! JS side (`web/shared/bridge.js`) sends `{"k":"send"|"invoke","c":channel,"a":[args],"i":id}`
//! through `window.ipc.postMessage`. This mirrors `ipcRenderer.send/invoke` and the
//! main side registries mirror `ipcMain.on/handle` and `webContents.ipc.on/handle`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::Value;

use crate::app::App;

#[derive(Deserialize)]
struct IpcMessage {
    k: String,
    c: String,
    #[serde(default)]
    a: Vec<Value>,
    #[serde(default)]
    i: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebviewOwner {
    Window(String),
    View(String),
}

#[derive(Clone, Debug)]
pub struct IpcEvent {
    pub webview_id: u64,
    pub owner: WebviewOwner,
}

pub type AsyncReply = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;

pub enum IpcReply {
    Ok(Value),
    Err(String),
    Async(AsyncReply),
}

pub type IpcListener = Rc<dyn Fn(&mut App, &IpcEvent, &[Value])>;
pub type IpcHandler = Rc<dyn Fn(&mut App, &IpcEvent, &[Value]) -> IpcReply>;

#[derive(Default)]
pub struct IpcRegistry {
    on: HashMap<String, Vec<(u64, IpcListener)>>,
    handlers: HashMap<String, IpcHandler>,
    next: u64,
}

impl IpcRegistry {
    pub fn on(&mut self, channel: &str, f: impl Fn(&mut App, &IpcEvent, &[Value]) + 'static) -> u64 {
        self.next += 1;
        let id = self.next;
        self.on.entry(channel.to_string()).or_default().push((id, Rc::new(f)));
        id
    }

    pub fn off(&mut self, channel: &str, id: u64) {
        if let Some(v) = self.on.get_mut(channel) {
            v.retain(|(i, _)| *i != id);
        }
    }

    pub fn handle(&mut self, channel: &str, f: impl Fn(&mut App, &IpcEvent, &[Value]) -> IpcReply + 'static) {
        self.handlers.insert(channel.to_string(), Rc::new(f));
    }

    pub fn remove_handler(&mut self, channel: &str) {
        self.handlers.remove(channel);
    }

    pub fn listeners(&self, channel: &str) -> Vec<IpcListener> {
        self.on.get(channel).map(|v| v.iter().map(|(_, l)| l.clone()).collect()).unwrap_or_default()
    }

    pub fn handler(&self, channel: &str) -> Option<IpcHandler> {
        self.handlers.get(channel).cloned()
    }
}

const YTM_GLOBAL_CHANNELS: &[&str] = &["__ytmd:domReady", "protectedApi:bindPort", "protectedApi:providePort", "protectedApi:message", "protectedApi:invokeResponse"];

pub fn dispatch(app: &mut App, webview_id: u64, body: &str) {
    let Some(owner) = app.window_manager.webview_owner(webview_id) else {
        return;
    };
    let msg: IpcMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(err) => {
            log::warn!("ipc: invalid message from webview {webview_id}: {err}");
            return;
        }
    };
    log::trace!("ipc: {} '{}' from {:?}", msg.k, msg.c, owner);
    let event = IpcEvent { webview_id, owner: owner.clone() };
    // The YouTube Music page is untrusted: only its scoped channels and the protected API are reachable
    let global_allowed = owner != WebviewOwner::View(crate::services::ytm_view_manager::VIEW.to_string()) || YTM_GLOBAL_CHANNELS.contains(&msg.c.as_str());
    match msg.k.as_str() {
        "send" => {
            let mut listeners = app.window_manager.scoped_listeners(&owner, &msg.c);
            if global_allowed {
                listeners.extend(app.ipc.listeners(&msg.c));
            }
            for l in listeners {
                l(app, &event, &msg.a);
            }
        }
        "invoke" => {
            let id = msg.i.unwrap_or(0);
            let handler = app.window_manager.scoped_handler(&owner, &msg.c).or_else(|| if global_allowed { app.ipc.handler(&msg.c) } else { None });
            match handler {
                None => reply(app, webview_id, id, Err(format!("No handler registered for '{}'", msg.c))),
                Some(h) => match h(app, &event, &msg.a) {
                    IpcReply::Ok(v) => reply(app, webview_id, id, Ok(v)),
                    IpcReply::Err(e) => reply(app, webview_id, id, Err(e)),
                    IpcReply::Async(fut) => {
                        let main = app.main.clone();
                        app.rt.spawn(async move {
                            let result = fut.await;
                            main.run(move |app| reply(app, webview_id, id, result));
                        });
                    }
                },
            }
        }
        _ => {}
    }
}

pub fn reply(app: &mut App, webview_id: u64, id: u64, result: Result<Value, String>) {
    let js = match result {
        Ok(v) => format!("window.__ytmdIpc&&window.__ytmdIpc._resolve({id},true,{v})"),
        Err(e) => format!("window.__ytmdIpc&&window.__ytmdIpc._resolve({id},false,{})", Value::String(e)),
    };
    app.window_manager.eval_webview(webview_id, &js);
}

/// JS snippet delivering `channel(...args)` to the page (`ipcRenderer.on`).
pub fn emit_js(channel: &str, args: &[Value]) -> String {
    format!(
        "window.__ytmdIpc&&window.__ytmdIpc._emit({},{})",
        Value::String(channel.to_string()),
        Value::Array(args.to_vec())
    )
}

pub fn arg_str(args: &[Value], i: usize) -> Option<String> {
    args.get(i).and_then(|v| v.as_str()).map(|s| s.to_string())
}
