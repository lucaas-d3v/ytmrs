//! Port of StateManager: debounced persistence of the `state` config section.

use std::time::Duration;

use serde_json::{Map, Value};

use crate::app::{App, TimerHandle};
use crate::services::watchdog;

#[derive(Default)]
pub struct StateManager {
    current_state: Option<Map<String, Value>>,
    disk_write_timer: Option<TimerHandle>,
    disk_stale: bool,
    state_updates: u32,
    initialized: bool,
}

pub fn on_initialized(app: &mut App) {
    if app.state_manager.initialized {
        panic!("StateManager is already initialized!");
    }
    app.state_manager.initialized = true;
    log::info!("StateManager initialized");
}

pub fn on_post_initialized(app: &mut App) {
    app.state_manager.current_state = app.config_store.get("state").as_object().cloned();
    // WatchDog crash callback -> panic(): handled through the global crash flag
}

pub fn on_terminated(app: &mut App) {
    force_write(app);
}

/// Merge a partial state update (shallow, like `{...current, ...partial}`)
pub fn update_state(app: &mut App, partial: Vec<(&str, Value)>) {
    if watchdog::crashed() || !app.state_manager.initialized {
        return;
    }
    let Some(current) = app.state_manager.current_state.as_ref() else { return };
    let mut new_state = current.clone();
    for (k, v) in partial {
        new_state.insert(k.to_string(), v);
    }
    if &new_state == current {
        return;
    }
    let sm = &mut app.state_manager;
    sm.current_state = Some(new_state);
    sm.disk_stale = true;
    sm.state_updates += 1;
    if let Some(t) = sm.disk_write_timer.take() {
        t.cancel();
    }
    if sm.state_updates >= 512 {
        write(app);
    } else {
        let timer = app.set_timeout(Duration::from_secs(30), write);
        app.state_manager.disk_write_timer = Some(timer);
    }
}

pub fn force_write(app: &mut App) {
    if watchdog::crashed() || !app.state_manager.initialized {
        return;
    }
    if let Some(t) = app.state_manager.disk_write_timer.take() {
        t.cancel();
    }
    write(app);
}

fn write(app: &mut App) {
    if watchdog::crashed() {
        return;
    }
    if app.state_manager.disk_stale {
        if let Some(state) = app.state_manager.current_state.clone() {
            crate::services::config_store::set(app, "state", Value::Object(state));
        }
        app.state_manager.disk_stale = false;
        app.state_manager.state_updates = 0;
    }
}
