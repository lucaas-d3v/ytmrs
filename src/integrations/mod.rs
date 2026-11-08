pub mod companion_server;
pub mod custom_css;
pub mod discord_presence;
pub mod last_fm;
pub mod media_service;
pub mod notifications;
pub mod volume_ratio;

use crate::services::integration_manager::IntegrationDescriptor;

/// Per integration runtime state
#[derive(Default)]
pub struct Integrations {
    pub companion_server: companion_server::CompanionServer,
    pub discord_presence: discord_presence::DiscordPresence,
    pub last_fm: last_fm::LastFm,
    pub notifications: notifications::NowPlayingNotifications,
    pub volume_ratio: volume_ratio::VolumeRatio,
    pub custom_css: custom_css::CustomCss,
    pub media_service: media_service::EnhancedMediaService,
}

/// `integrationManager.createIntegrations([...])` in the original order
pub fn all() -> Vec<IntegrationDescriptor> {
    vec![
        companion_server::descriptor(),
        discord_presence::descriptor(),
        last_fm::descriptor(),
        notifications::descriptor(),
        volume_ratio::descriptor(),
        custom_css::descriptor(),
        media_service::descriptor(),
    ]
}
