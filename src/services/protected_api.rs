//! Port of ProtectedAPIManager / ProtectedAPI.
//!
//! The original passes MessagePorts from the YTM main world to the main process.
//! Here the YTM view talks through the IPC bridge with the same op codes:
//! ProvideAPIPort(0), Message(1), Invoke(2), InvokeResponse(3).

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::app::App;
use crate::events::{Emitter, ListenerId};

pub const YTM_VIEW: &str = "YTM";

#[derive(Default)]
pub struct ProtectedApi {
    pub name: String,
    bound: bool,
    listeners: HashMap<String, Emitter<Vec<Value>>>,
    waiting_invokes: HashMap<String, oneshot::Sender<Vec<Value>>>,
}

#[derive(Default)]
pub struct ProtectedApiManager {
    main_bound: bool,
    apis: HashMap<String, ProtectedApi>,
}

impl ProtectedApiManager {
    pub fn create_or_get_api(&mut self, name: &str) -> &mut ProtectedApi {
        if !self.apis.contains_key(name) {
            log::info!("protectedapi: api '{name}' created");
            self.apis.insert(name.to_string(), ProtectedApi { name: name.to_string(), ..Default::default() });
        }
        self.apis.get_mut(name).unwrap()
    }

    pub fn on(&mut self, api: &str, message: &str, f: impl Fn(&mut App, &Vec<Value>) + 'static) -> ListenerId {
        self.create_or_get_api(api).listeners.entry(message.to_string()).or_default().on(f)
    }

    pub fn off(&mut self, api: &str, message: &str, id: ListenerId) {
        if let Some(e) = self.apis.get_mut(api).and_then(|a| a.listeners.get_mut(message)) {
            e.off(id);
        }
    }

    /// Port closed (view navigated / reloaded / crashed)
    pub fn unbind_all(&mut self) {
        self.main_bound = false;
        for (name, api) in self.apis.iter_mut() {
            if api.bound {
                log::info!("protectedapi: api '{name}' port closed unbinding");
            }
            api.bound = false;
        }
    }
}

/// Posts a RPC message to the other end of the ProtectedAPI
pub fn post_message(app: &mut App, api: &str, name: &str, args: Vec<Value>) {
    let bound = app.protected_api.create_or_get_api(api).bound;
    if !bound {
        return;
    }
    let packet = json!({ "op": 1, "api": api, "name": name, "args": args });
    app.window_manager.eval_view(YTM_VIEW, &format!("window.__ytmdProtectedApi&&window.__ytmdProtectedApi._receive({packet})"));
}

/// RemoteControl "execute" shortcut used everywhere
pub fn remote_execute(app: &mut App, command: &str, args: Vec<Value>) {
    let mut all = vec![Value::from(command)];
    all.extend(args);
    post_message(app, "RemoteControl", "execute", all);
}

/// Posts a RPC message and waits (30s) for the result
pub fn invoke_message(app: &mut App, api: &str, name: &str, args: Vec<Value>) -> impl std::future::Future<Output = Result<Vec<Value>, ()>> + Send {
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    let bound = {
        let a = app.protected_api.create_or_get_api(api);
        a.waiting_invokes.insert(id.clone(), tx);
        a.bound
    };
    if bound {
        let packet = json!({ "op": 2, "api": api, "id": id, "name": name, "args": args });
        app.window_manager.eval_view(YTM_VIEW, &format!("window.__ytmdProtectedApi&&window.__ytmdProtectedApi._receive({packet})"));
    }
    let main = app.main.clone();
    let api = api.to_string();
    async move {
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(v)) => Ok(v),
            _ => {
                main.run(move |app| {
                    if let Some(a) = app.protected_api.apis.get_mut(&api) {
                        a.waiting_invokes.remove(&id);
                    }
                });
                Err(())
            }
        }
    }
}

pub fn on_initialized(app: &mut App) {
    app.ipc.on("protectedApi:bindPort", |app, event, _| {
        if !is_ytm_view(event) {
            return;
        }
        // Reset the ports on all APIs as when setting the main port the other ports are no longer valid
        for (name, api) in app.protected_api.apis.iter_mut() {
            log::info!("protectedapi: unbinding api '{name}' port");
            api.bound = false;
        }
        app.protected_api.main_bound = true;
        log::info!("protectedapi: main port bound");
    });
    app.ipc.on("protectedApi:providePort", |app, event, args| {
        if !is_ytm_view(event) || !app.protected_api.main_bound {
            return;
        }
        if let Some(name) = args.first().and_then(|v| v.as_str()) {
            let api = app.protected_api.create_or_get_api(name);
            api.bound = true;
            log::info!("protectedapi: api '{name}' port bound");
        }
    });
    app.ipc.on("protectedApi:message", |app, event, args| {
        if !is_ytm_view(event) {
            return;
        }
        let (Some(api), Some(name)) = (args.first().and_then(|v| v.as_str()), args.get(1).and_then(|v| v.as_str())) else { return };
        let msg_args = args.get(2).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let callbacks = match app.protected_api.apis.get_mut(api) {
            Some(a) if a.bound => a.listeners.get_mut(name).map(|e| e.take_for_emit()).unwrap_or_default(),
            _ => return,
        };
        crate::events::run_callbacks(app, callbacks, &msg_args);
    });
    app.ipc.on("protectedApi:invokeResponse", |app, event, args| {
        if !is_ytm_view(event) {
            return;
        }
        let (Some(api), Some(id)) = (args.first().and_then(|v| v.as_str()), args.get(1).and_then(|v| v.as_str())) else { return };
        let response = args.get(2).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        if let Some(tx) = app.protected_api.apis.get_mut(api).and_then(|a| a.waiting_invokes.remove(id)) {
            let _ = tx.send(response);
        }
    });
    log::info!("ProtectedAPIManager initialized");
}

fn is_ytm_view(event: &crate::ipc::IpcEvent) -> bool {
    event.owner == crate::ipc::WebviewOwner::View(YTM_VIEW.to_string())
}
