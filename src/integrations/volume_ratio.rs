//! Port of VolumeRatio (based on https://greasyfork.org/en/scripts/397686-youtube-music-fix-volume-ratio
//! by Marco Pfeiffer <git@marco.zone>)

use crate::app::App;
use crate::events::ListenerId;
use crate::services::integration_manager::IntegrationDescriptor;
use crate::services::ytm_view_manager::{self, execute_script};

#[derive(Default)]
pub struct VolumeRatio {
    injected: bool,
    recreated_listener: Option<ListenerId>,
}

pub fn descriptor() -> IntegrationDescriptor {
    IntegrationDescriptor {
        name: "VolumeRatio",
        store_enable_property: "playback.ratioVolume",
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
    if app.integrations.volume_ratio.recreated_listener.is_none() {
        let id = app.ytm_view_manager.view_recreated.on(|app, _| {
            app.integrations.volume_ratio.injected = false;
            on_enabled(app);
        });
        app.integrations.volume_ratio.recreated_listener = Some(id);
    }
    if !app.integrations.volume_ratio.injected {
        app.integrations.volume_ratio.injected = true;
        ytm_view_manager::when_ready(app, |app| {
            if !app.integration_manager.is_enabled("VolumeRatio") {
                return;
            }
            execute_script(app, &crate::assets::ytm_script("volume-ratio/enable.js"));
            execute_script(app, &crate::assets::ytm_script("volume-ratio/forceupdatevolume.js"));
        });
    }
}

fn on_disabled(app: &mut App) {
    if let Some(id) = app.integrations.volume_ratio.recreated_listener.take() {
        app.ytm_view_manager.view_recreated.off(id);
    }
    if app.integrations.volume_ratio.injected {
        app.integrations.volume_ratio.injected = false;
        ytm_view_manager::when_ready(app, |app| {
            execute_script(app, &crate::assets::ytm_script("volume-ratio/disable.js"));
            execute_script(app, &crate::assets::ytm_script("volume-ratio/forceupdatevolume.js"));
        });
    }
}
