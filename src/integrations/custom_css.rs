//! Port of CustomCSS: user origin stylesheet injected into the YTM view, reloaded on file change

use std::path::PathBuf;

use serde_json::Value;

use crate::app::App;
use crate::events::ListenerId;
use crate::services::integration_manager::IntegrationDescriptor;
use crate::services::{config_store, ytm_view_manager};

#[derive(Default)]
pub struct CustomCss {
    injected: bool,
    #[cfg(target_os = "linux")]
    sheet: Option<webkit2gtk::UserStyleSheet>,
    store_listener: Option<ListenerId>,
    recreated_listener: Option<ListenerId>,
    watcher: Option<notify::RecommendedWatcher>,
}

pub fn descriptor() -> IntegrationDescriptor {
    IntegrationDescriptor {
        name: "CustomCSS",
        store_enable_property: "appearance.customCSSEnabled",
        dependent_store_properties: &[],
        on_setup: |_| {},
        on_enabled: |app| {
            on_enabled(app);
            Ok(())
        },
        on_disabled,
    }
}

fn on_enabled(app: &mut App) {
    let id = app.ytm_view_manager.view_recreated.on(|app, _| {
        app.integrations.custom_css.injected = false;
        update_css(app);
    });
    app.integrations.custom_css.recreated_listener = Some(id);
    let id = app.config_store.on_did_change("appearance", |app, new, old| {
        let enabled = new.get("customCSSEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
        if new.is_null() || old.is_null() || (enabled && new.get("customCSSPath") != old.get("customCSSPath")) {
            update_css(app);
        }
    });
    app.integrations.custom_css.store_listener = Some(id);
    inject_css(app);
}

fn on_disabled(app: &mut App) {
    if let Some(id) = app.integrations.custom_css.recreated_listener.take() {
        app.ytm_view_manager.view_recreated.off(id);
    }
    app.integrations.custom_css.watcher = None;
    if let Some(id) = app.integrations.custom_css.store_listener.take() {
        app.config_store.off(id);
    }
    remove_css(app);
}

pub fn update_css(app: &mut App) {
    remove_css(app);
    inject_css(app);
}

fn inject_css(app: &mut App) {
    if app.integrations.custom_css.injected {
        return;
    }
    let Some(path) = app.config_store.get_str("appearance.customCSSPath").map(PathBuf::from) else { return };
    let Ok(content) = std::fs::read_to_string(&path) else { return };
    app.integrations.custom_css.injected = true;
    ytm_view_manager::when_ready(app, move |app| {
        if !app.integrations.custom_css.injected {
            return;
        }
        apply(app, &content);
        watch_css_file(app, path);
    });
}

fn apply(app: &mut App, css: &str) {
    let Some(view) = app.window_manager.view(ytm_view_manager::VIEW) else { return };
    #[cfg(target_os = "linux")]
    {
        let sheet = crate::platform::linux::insert_user_css(&view.webview.webview, css);
        app.integrations.custom_css.sheet = sheet;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let js = format!(
            "(function(){{let s=document.getElementById('ytmd-custom-css');if(!s){{s=document.createElement('style');s.id='ytmd-custom-css';document.documentElement.appendChild(s);}}s.textContent={};}})()",
            Value::from(css)
        );
        let _ = view.webview.webview.evaluate_script(&js);
    }
}

fn remove_css(app: &mut App) {
    if !app.integrations.custom_css.injected {
        return;
    }
    app.integrations.custom_css.injected = false;
    let Some(view) = app.window_manager.view(ytm_view_manager::VIEW) else { return };
    #[cfg(target_os = "linux")]
    if let Some(sheet) = app.integrations.custom_css.sheet.take() {
        crate::platform::linux::remove_user_css(&view.webview.webview, &sheet);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = view.webview.webview.evaluate_script("document.getElementById('ytmd-custom-css')?.remove()");
}

fn watch_css_file(app: &mut App, path: PathBuf) {
    use notify::Watcher;
    app.integrations.custom_css.watcher = None;
    let main = app.main.clone();
    let file = path.clone();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        match event.kind {
            notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) | notify::EventKind::Remove(_) => {
                main.run(|app| {
                    config_store::set(app, "appearance.customCSSPath", Value::Null);
                    remove_css(app);
                    app.integrations.custom_css.watcher = None;
                });
            }
            notify::EventKind::Modify(_) => main.run(update_css),
            _ => {}
        }
    });
    match watcher {
        Ok(mut w) => {
            if w.watch(&file, notify::RecursiveMode::NonRecursive).is_ok() {
                app.integrations.custom_css.watcher = Some(w);
            }
        }
        Err(err) => log::warn!("Failed to watch custom css file: {err}"),
    }
}
