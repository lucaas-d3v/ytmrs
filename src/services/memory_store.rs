//! Port of MemoryStore: volatile key/value state observed by the native UI and integrations.

use serde_json::{Map, Value};

use crate::app::App;
use crate::emit;
use crate::events::{Emitter, ListenerId};

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
    let value = value.into();
    if app.memory_store.state.get(key) == Some(&value) {
        return;
    }
    let old = app.memory_store.state.clone();
    app.memory_store.state.insert(key.to_string(), value);
    let change = MemoryChange { new: app.memory_store.state.clone(), old };
    emit!(app, app.memory_store.changed, &change);
    crate::ui::on_memory_changed(app, &change);
}

pub fn on_initialized(_app: &mut App) {
    log::info!("MemoryStore initialized");
}
