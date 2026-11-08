//! Port of ConfigStore (backed by `conf`): JSON file at `<userData>/config.json`,
//! dot-path get/set, defaults, semver range migrations, file watching and change events.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::app::App;
use crate::emit;
use crate::events::{Emitter, ListenerId};
use crate::ipc::IpcReply;

const MIGRATION_KEY: &str = "__internal__.migrations.version";

pub struct ConfigChange {
    pub new: Value,
    pub old: Value,
}

#[derive(Default)]
pub struct ConfigStore {
    pub store: Value,
    path: Option<PathBuf>,
    pub any_change: Emitter<ConfigChange>,
    initialized: bool,
    _watcher: Option<notify::RecommendedWatcher>,
}

pub fn get_path<'a>(obj: &'a Value, key: &str) -> Option<&'a Value> {
    let mut cur = obj;
    for part in key.split('.') {
        cur = cur.as_object()?.get(part)?;
    }
    Some(cur)
}

pub fn set_path(obj: &mut Value, key: &str, value: Value) {
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = obj;
    for (i, part) in parts.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let map = cur.as_object_mut().unwrap();
        if i == parts.len() - 1 {
            map.insert(part.to_string(), value);
            return;
        }
        cur = map.entry(part.to_string()).or_insert_with(|| Value::Object(Map::new()));
    }
}

pub fn delete_path(obj: &mut Value, key: &str) {
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = obj;
    for (i, part) in parts.iter().enumerate() {
        let Some(map) = cur.as_object_mut() else { return };
        if i == parts.len() - 1 {
            map.shift_remove(*part);
            return;
        }
        match map.get_mut(*part) {
            Some(v) => cur = v,
            None => return,
        }
    }
}

/// Fill missing keys from defaults (recursively)
pub fn merge_defaults(target: &mut Value, defaults: &Value) {
    if let (Some(t), Some(d)) = (target.as_object_mut(), defaults.as_object()) {
        for (k, dv) in d {
            match t.get_mut(k) {
                None => {
                    t.insert(k.clone(), dv.clone());
                }
                Some(tv) => {
                    if tv.is_object() && dv.is_object() {
                        merge_defaults(tv, dv);
                    }
                }
            }
        }
    }
}

type Migration = fn(&mut Value);

fn migrations() -> Vec<(&'static str, Migration)> {
    fn has(v: &Value, k: &str) -> bool {
        get_path(v, k).is_some()
    }
    vec![
        (">=2.0.0", |s| {
            delete_path(s, "integrations.companionServerAuthWindowEnabled");
            delete_path(s, "state.companionServerAuthWindowEnableTime");
            if !has(s, "appearance.zoom") {
                set_path(s, "appearance.zoom", Value::from(100));
            }
        }),
        (">=2.0.1", |s| {
            if !has(s, "lastfm.scrobblePercent") {
                set_path(s, "lastfm.scrobblePercent", Value::from(50));
            }
        }),
        (">=2.0.7", |s| {
            if !has(s, "appearance.trayIconStyle") {
                set_path(s, "appearance.trayIconStyle", Value::from(0));
            }
        }),
        (">=2.1.0", |s| {
            for (k, v) in [
                ("shortcuts.volumeMute", Value::from("")),
                ("shortcuts.toggleShuffle", Value::from("")),
                ("shortcuts.toggleRepeat", Value::from("")),
                ("integrations.enhancedMediaServiceEnabled", Value::from(true)),
                ("state.miniplayerWindowBounds", Value::Null),
                ("metadata.appVersion", Value::from("2.0.10")),
            ] {
                if !has(s, k) {
                    set_path(s, k, v);
                }
            }
            for k in ["lastfm.api_key", "lastfm.secret", "lastfm.token"] {
                if has(s, k) {
                    delete_path(s, k);
                }
            }
        }),
    ]
}

fn is_range(version: &str) -> bool {
    semver::Version::parse(version.trim_start_matches('v')).is_err()
}

/// Port of conf's `_shouldPerformMigration`
pub fn should_perform_migration(candidate: &str, previous: &str, to: &str) -> bool {
    let to_v = match semver::Version::parse(to) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if is_range(candidate) {
        let Ok(req) = semver::VersionReq::parse(candidate) else { return false };
        if previous != "0.0.0" {
            if let Ok(prev) = semver::Version::parse(previous) {
                if req.matches(&prev) {
                    return false;
                }
            }
        }
        return req.matches(&to_v);
    }
    let (Ok(c), Ok(p)) = (semver::Version::parse(candidate), semver::Version::parse(previous)) else { return false };
    if c <= p {
        return false;
    }
    c <= to_v
}

pub fn run_migrations(store: &mut Value, project_version: &str) {
    let mut previous = get_path(store, MIGRATION_KEY).and_then(|v| v.as_str()).unwrap_or("0.0.0").to_string();
    for (version, migration) in migrations() {
        if !should_perform_migration(version, &previous, project_version) {
            continue;
        }
        log::info!("Performing store migration from {previous} to {version}");
        migration(store);
        set_path(store, MIGRATION_KEY, Value::from(version));
        previous = version.to_string();
    }
    if is_range(&previous) || semver::Version::parse(&previous).map(|v| v.to_string() != project_version).unwrap_or(true) {
        set_path(store, MIGRATION_KEY, Value::from(project_version));
    }
}

impl ConfigStore {
    pub fn get(&self, key: &str) -> Value {
        get_path(&self.store, key).cloned().unwrap_or(Value::Null)
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.get(key).as_bool().unwrap_or(false)
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.get(key).as_str().map(|s| s.to_string())
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).as_f64()
    }

    fn write(&self) {
        let Some(path) = &self.path else { return };
        let mut data = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
        let mut ser = serde_json::Serializer::with_formatter(&mut data, formatter);
        if serde::Serialize::serialize(&self.store, &mut ser).is_err() {
            return;
        }
        let tmp = path.with_extension("json.tmp");
        if let Err(err) = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, path)) {
            log::error!("Failed to write config store: {err}");
        }
    }

    pub fn on_did_any_change(&mut self, f: impl Fn(&mut App, &ConfigChange) + 'static) -> ListenerId {
        self.any_change.on(f)
    }

    /// conf `onDidChange(key, cb)` -> cb(newValue, oldValue) only when the key changed
    pub fn on_did_change(&mut self, key: &str, f: impl Fn(&mut App, &Value, &Value) + 'static) -> ListenerId {
        let key = key.to_string();
        self.any_change.on(move |app, change| {
            let new = get_path(&change.new, &key).cloned().unwrap_or(Value::Null);
            let old = get_path(&change.old, &key).cloned().unwrap_or(Value::Null);
            if new != old {
                f(app, &new, &old);
            }
        })
    }

    pub fn off(&mut self, id: ListenerId) {
        self.any_change.off(id);
    }
}

/// Store a value and notify listeners (conf `set`)
pub fn set(app: &mut App, key: &str, value: Value) {
    let old = app.config_store.store.clone();
    set_path(&mut app.config_store.store, key, value);
    commit(app, old);
}

pub fn reset(app: &mut App, key: &str) {
    let defaults = crate::shared::store_schema::defaults(crate::APP_VERSION);
    let old = app.config_store.store.clone();
    let v = get_path(&defaults, key).cloned().unwrap_or(Value::Null);
    set_path(&mut app.config_store.store, key, v);
    commit(app, old);
}

fn commit(app: &mut App, old: Value) {
    if old == app.config_store.store {
        return;
    }
    app.config_store.write();
    let change = ConfigChange { new: app.config_store.store.clone(), old };
    emit!(app, app.config_store.any_change, &change);
}

fn load_file(path: &PathBuf) -> Value {
    match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_else(|err| {
            log::warn!("Config file is invalid, using defaults: {err}");
            Value::Object(Map::new())
        }),
        Err(_) => Value::Object(Map::new()),
    }
}

pub fn on_pre_initialized(app: &mut App) {
    let path = crate::paths::get().user_data.join("config.json");
    let mut store = load_file(&path);
    if !store.is_object() {
        store = Value::Object(Map::new());
    }
    merge_defaults(&mut store, &crate::shared::store_schema::defaults(crate::APP_VERSION));
    run_migrations(&mut store, crate::APP_VERSION);
    app.config_store.store = store;
    app.config_store.path = Some(path.clone());
    app.config_store.write();

    if app.config_store.get_bool("general.disableHardwareAcceleration") {
        log::info!("Hardware acceleration disabled");
    }
    if app.config_store.get_bool("playback.enableSpeakerFill") {
        log::info!("Speaker fill enabled");
    }

    // watch: true -> reload on external modification
    use notify::Watcher;
    let main = app.main.clone();
    let watch_path = path.clone();
    let debounce = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let rt = app.rt.clone();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if !event.paths.iter().any(|p| p.file_name() == watch_path.file_name()) {
            return;
        }
        if !matches!(event.kind, notify::EventKind::Modify(_) | notify::EventKind::Create(_)) {
            return;
        }
        let generation = debounce.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let debounce = debounce.clone();
        let main = main.clone();
        rt.spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if debounce.load(std::sync::atomic::Ordering::SeqCst) == generation {
                main.run(reload_from_disk);
            }
        });
    });
    if let Ok(mut watcher) = watcher {
        if let Some(dir) = path.parent() {
            let _ = watcher.watch(dir, notify::RecursiveMode::NonRecursive);
        }
        app.config_store._watcher = Some(watcher);
    }
}

fn reload_from_disk(app: &mut App) {
    let Some(path) = app.config_store.path.clone() else { return };
    let new = load_file(&path);
    if !new.is_object() || new == app.config_store.store {
        return;
    }
    let old = std::mem::replace(&mut app.config_store.store, new);
    let change = ConfigChange { new: app.config_store.store.clone(), old };
    emit!(app, app.config_store.any_change, &change);
}

pub fn on_initialized(app: &mut App) {
    if app.config_store.initialized {
        panic!("ConfigStore is already initialized!");
    }
    app.config_store.initialized = true;

    app.config_store.on_did_any_change(|app, change| {
        let args = [change.new.clone(), change.old.clone()];
        app.window_manager.broadcast_all("configStore:stateChanged", &args);
        reconcile_config(app, &change.new);
    });

    app.ipc.handle("configStore:get", |app, _e, args| {
        let key = args.first().and_then(|v| v.as_str()).unwrap_or("");
        IpcReply::Ok(app.config_store.get(key))
    });
    app.ipc.on("configStore:set", |app, _e, args| {
        if let Some(key) = args.first().and_then(|v| v.as_str()) {
            let key = key.to_string();
            set(app, &key, args.get(1).cloned().unwrap_or(Value::Null));
        }
    });
    app.ipc.on("configStore:reset", |app, _e, args| {
        if let Some(key) = args.first().and_then(|v| v.as_str()) {
            let key = key.to_string();
            reset(app, &key);
        }
    });

    log::info!("ConfigStore initialized");
}

fn reconcile_config(app: &mut App, new_state: &Value) {
    let start_on_boot = get_path(new_state, "general.startOnBoot").and_then(|v| v.as_bool()).unwrap_or(false);
    if app.ui.login_item_enabled != Some(start_on_boot) {
        app.ui.login_item_enabled = Some(start_on_boot);
        crate::platform::set_login_item(start_on_boot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths() {
        let mut v = json!({});
        set_path(&mut v, "a.b.c", json!(1));
        assert_eq!(get_path(&v, "a.b.c"), Some(&json!(1)));
        delete_path(&mut v, "a.b.c");
        assert_eq!(get_path(&v, "a.b.c"), None);
        assert_eq!(get_path(&v, "a.b"), Some(&json!({})));
    }

    #[test]
    fn migrations_run_once() {
        let mut store = json!({"lastfm": {"api_key": "x", "sessionKey": null}, "integrations": {"companionServerAuthWindowEnabled": true}});
        run_migrations(&mut store, "2.1.0");
        assert_eq!(get_path(&store, "appearance.zoom"), Some(&json!(100)));
        assert_eq!(get_path(&store, "lastfm.scrobblePercent"), Some(&json!(50)));
        assert_eq!(get_path(&store, "metadata.appVersion"), Some(&json!("2.0.10")));
        assert!(get_path(&store, "lastfm.api_key").is_none());
        assert!(get_path(&store, "integrations.companionServerAuthWindowEnabled").is_none());
        assert_eq!(get_path(&store, MIGRATION_KEY), Some(&json!("2.1.0")));
        // second run is a no-op
        set_path(&mut store, "appearance.zoom", json!(150));
        delete_path(&mut store, "lastfm.scrobblePercent");
        run_migrations(&mut store, "2.1.0");
        assert_eq!(get_path(&store, "appearance.zoom"), Some(&json!(150)));
        assert!(get_path(&store, "lastfm.scrobblePercent").is_none());
    }

    #[test]
    fn should_migrate_semantics() {
        assert!(should_perform_migration(">=2.0.0", "0.0.0", "2.1.0"));
        assert!(!should_perform_migration(">=2.0.0", "2.0.5", "2.1.0"));
        assert!(should_perform_migration(">=2.1.0", "2.0.10", "2.1.0"));
        assert!(!should_perform_migration("2.0.0", "2.0.0", "2.1.0"));
        assert!(should_perform_migration("2.0.5", "2.0.0", "2.1.0"));
    }

    #[test]
    fn defaults_merge() {
        let mut v = json!({"general": {"hideToTrayOnClose": true}});
        merge_defaults(&mut v, &crate::shared::store_schema::defaults("2.1.0"));
        assert_eq!(get_path(&v, "general.hideToTrayOnClose"), Some(&json!(true)));
        assert_eq!(get_path(&v, "general.startOnBoot"), Some(&json!(false)));
        assert_eq!(get_path(&v, "appearance.zoom"), Some(&json!(100)));
    }
}
