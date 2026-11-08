//! Port of NowPlayingNotifications

use std::time::Duration;

use crate::app::App;
use crate::events::ListenerId;
use crate::services::integration_manager::IntegrationDescriptor;
use crate::shared::player_state::{PlayerState, Thumbnail, VideoState};

#[derive(Default)]
pub struct NowPlayingNotifications {
    last_id: Option<String>,
    listener: Option<ListenerId>,
}

pub fn descriptor() -> IntegrationDescriptor {
    IntegrationDescriptor {
        name: "NowPlayingNotifications",
        store_enable_property: "general.showNotificationOnSongChange",
        dependent_store_properties: &[],
        on_setup: |_| {},
        on_enabled: |app| {
            let id = app.player_state_store.state_changed.on(update_video_details);
            app.integrations.notifications.listener = Some(id);
            Ok(())
        },
        on_disabled: |app| {
            if let Some(id) = app.integrations.notifications.listener.take() {
                app.player_state_store.state_changed.off(id);
            }
        },
    }
}

/// Smallest thumbnail above 100x100
pub fn lowest_res_thumbnail(thumbnails: &[Thumbnail]) -> Option<String> {
    let (mut cw, mut ch) = (1024.0, 1024.0);
    let mut url = None;
    for t in thumbnails {
        let (w, h) = (t.w(), t.h());
        if w < cw && h < ch && w > 100.0 && h > 100.0 {
            cw = w;
            ch = h;
            url = t.url.clone();
        }
    }
    url
}

fn update_video_details(app: &mut App, state: &PlayerState) {
    if !app.integration_manager.is_enabled("NowPlayingNotifications") {
        return;
    }
    let Some(details) = &state.video_details else { return };
    if state.track_state != VideoState::Playing {
        return;
    }
    if app.integrations.notifications.last_id.as_deref() == Some(details.id()) && app.integrations.notifications.last_id.is_some() {
        return;
    }
    app.integrations.notifications.last_id = Some(details.id().to_string());
    let title = details.title().to_string();
    let body = details.author().to_string();
    let thumbnail = lowest_res_thumbnail(&details.thumbnails);
    let client = app.http.clone();
    app.spawn(async move {
        let mut image = None;
        if let Some(url) = thumbnail {
            if let Ok(res) = client.get(&url).send().await {
                if let Ok(bytes) = res.bytes().await {
                    let path = crate::paths::get().cache.join("notification-thumbnail.jpg");
                    let _ = tokio::fs::create_dir_all(&crate::paths::get().cache).await;
                    if tokio::fs::write(&path, &bytes).await.is_ok() {
                        image = Some(path);
                    }
                }
            }
        }
        let _ = tokio::task::spawn_blocking(move || display_notification(&title, &body, image)).await;
    });
}

fn display_notification(title: &str, body: &str, image: Option<std::path::PathBuf>) {
    let mut n = notify_rust::Notification::new();
    n.summary(title).body(body).appname("YtmRs").timeout(notify_rust::Timeout::Milliseconds(5000));
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        n.urgency(notify_rust::Urgency::Low);
        n.hint(notify_rust::Hint::SuppressSound(true));
    }
    if let Some(image) = &image {
        n.image_path(&image.to_string_lossy());
    }
    match n.show() {
        Ok(_handle) => {
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                std::thread::sleep(Duration::from_secs(5));
                _handle.close();
            }
        }
        Err(err) => log::warn!("Failed to show notification: {err}"),
    }
    let _ = Duration::ZERO;
}
