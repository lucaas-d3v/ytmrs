//! Port of IntegrationManager / Integration

use serde_json::Value;

use crate::app::App;
use crate::services::config_store::get_path;

pub struct IntegrationDescriptor {
    pub name: &'static str,
    /// The store property which enables and disables this integration
    pub store_enable_property: &'static str,
    /// Store properties which if changed restart this integration
    pub dependent_store_properties: &'static [&'static str],
    /// Run before the app is ready
    pub on_setup: fn(&mut App),
    pub on_enabled: fn(&mut App) -> anyhow::Result<()>,
    pub on_disabled: fn(&mut App),
}

#[derive(Default)]
pub struct IntegrationManager {
    integrations: Vec<IntegrationDescriptor>,
    enabled: Vec<&'static str>,
}

impl IntegrationManager {
    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains(&name)
    }

    pub fn create_integrations(&mut self, integrations: Vec<IntegrationDescriptor>) {
        self.integrations.extend(integrations);
    }
}

pub fn on_initialized(_app: &mut App) {
    log::info!("IntegrationManager initialized");
}

pub fn on_post_initialized(app: &mut App) {
    app.config_store.on_did_any_change(|app, change| {
        let (new, old) = (change.new.clone(), change.old.clone());
        reconcile_integration_enable_state(app, Some((&new, &old)));
    });
}

/// IntegrationManagerHook.AppBeforeReady
pub fn run_integration_setups(app: &mut App) {
    let setups: Vec<fn(&mut App)> = app.integration_manager.integrations.iter().map(|i| i.on_setup).collect();
    for s in setups {
        s(app);
    }
}

/// IntegrationManagerHook.AppReady
pub fn app_ready(app: &mut App) {
    reconcile_integration_enable_state(app, None);
}

fn enable(app: &mut App, index: usize) {
    let (name, on_enabled) = {
        let i = &app.integration_manager.integrations[index];
        (i.name, i.on_enabled)
    };
    log::info!("Enabling integration: {name}");
    app.integration_manager.enabled.push(name);
    if let Err(err) = on_enabled(app) {
        log::error!("Integration '{name}' failed to enable: {err:?}");
        app.integration_manager.enabled.retain(|n| *n != name);
        enable_error(app, name, &err);
        return;
    }
    log::info!("Enabled integration: {name}");
}

fn disable(app: &mut App, index: usize) {
    let (name, on_disabled) = {
        let i = &app.integration_manager.integrations[index];
        (i.name, i.on_disabled)
    };
    log::info!("Disabling integration: {name}");
    app.integration_manager.enabled.retain(|n| *n != name);
    on_disabled(app);
    log::info!("Disabled integration: {name}");
}

/// "enable-error" -> Integration Error dialog
pub fn enable_error(app: &mut App, name: &str, err: &anyhow::Error) {
    let detail = format!("The '{name}' integration failed to be enabled and will be unavailable.\n\n{err:?}");
    crate::dialogs::show_message_box(
        app,
        None,
        crate::dialogs::MessageBox::new(crate::dialogs::Kind::Warning, "Integration Error", "An integration could not be enabled", &detail, &["Okay"]),
        |_, _| {},
    );
}

fn reconcile_integration_enable_state(app: &mut App, states: Option<(&Value, &Value)>) {
    for index in 0..app.integration_manager.integrations.len() {
        let (name, prop, deps) = {
            let i = &app.integration_manager.integrations[index];
            (i.name, i.store_enable_property, i.dependent_store_properties)
        };
        let should_be_enabled = match states {
            Some((new, _)) => get_path(new, prop).and_then(|v| v.as_bool()).unwrap_or(false),
            None => app.config_store.get_bool(prop),
        };
        let is_enabled = app.integration_manager.is_enabled(name);
        if !is_enabled {
            if should_be_enabled {
                enable(app, index);
            }
        } else if !should_be_enabled {
            disable(app, index);
        } else if let Some((new, old)) = states {
            for dep in deps {
                if get_path(new, dep) != get_path(old, dep) {
                    log::info!("Restarting enabled integration: {name} (dependent properties changed)");
                    disable(app, index);
                    enable(app, index);
                    log::info!("Restarted enabled integration: {name} (dependent properties changed)");
                    break;
                }
            }
        }
    }
}
