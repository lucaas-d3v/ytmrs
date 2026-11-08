//! Port of MemoryStore: volatile key/value state shared with renderer windows.

use serde_json::{Map, Value};

use crate::app::App;
use crate::emit;
use crate::events::{Emitter, ListenerId};
use crate::ipc::IpcReply;

pub struct MemoryChange {
    pub new: Map<String, Value>,
    pub old: Map<String, Value>,
}

#[derive(Default)]
pub struct MemoryStore {
    state: Map<String, Value>,
    pub changed: Emitter<MemoryChange>,
}

impl MemoryStore {
    pub fn get(&self, key: &str) -> Value {
        self.state.get(key).cloned().unwrap_or(Value::Null)
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.get(key).as_bool().unwrap_or(false)
    }

    pub fn on_state_changed(&mut self, f: impl Fn(&mut App, &MemoryChange) + 'static) -> ListenerId {
        self.changed.on(f)
    }

    pub fn remove_on_state_changed(&mut self, id: ListenerId) {
        self.changed.off(id);
    }
}

pub fn set(app: &mut App, key: &str, value: impl Into<Value>) {
    let old = app.memory_store.state.clone();
    app.memory_store.state.insert(key.to_string(), value.into());
    let change = MemoryChange { new: app.memory_store.state.clone(), old };
    emit!(app, app.memory_store.changed, &change);
    let args = [Value::Object(change.new), Value::Object(change.old)];
    app.window_manager.broadcast_all("memoryStore:stateChanged", &args);
}

pub fn on_initialized(app: &mut App) {
    app.ipc.handle("memoryStore:get", |app, _e, args| {
        let key = args.first().and_then(|v| v.as_str()).unwrap_or("");
        IpcReply::Ok(app.memory_store.get(key))
    });
    app.ipc.on("memoryStore:set", |app, _e, args| {
        if let Some(key) = args.first().and_then(|v| v.as_str()) {
            let key = key.to_string();
            set(app, &key, args.get(1).cloned().unwrap_or(Value::Null));
        }
    });
    log::info!("MemoryStore initialized");
}
