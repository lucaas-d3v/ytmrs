pub mod config_store;
pub mod integration_manager;
pub mod memory_store;
pub mod player_state_store;
pub mod protected_api;
pub mod protocol_manager;
pub mod safe_storage;
pub mod service_host;
pub mod shortcut_manager;
pub mod state_manager;
pub mod taskbar_manager;
pub mod watchdog;
pub mod ytm_view_manager;

use service_host::{noop, ServiceDescriptor};

/// The service collection of `index.ts` (dependency ordered by the host)
pub fn collection() -> Vec<ServiceDescriptor> {
    vec![
        ServiceDescriptor { name: "WatchDog", dependencies: &[], on_pre_initialized: noop, on_initialized: |_| log::info!("WatchDog initialized"), on_post_initialized: noop, on_terminated: noop },
        ServiceDescriptor { name: "ConfigStore", dependencies: &[], on_pre_initialized: config_store::on_pre_initialized, on_initialized: config_store::on_initialized, on_post_initialized: noop, on_terminated: noop },
        ServiceDescriptor { name: "MemoryStore", dependencies: &[], on_pre_initialized: noop, on_initialized: memory_store::on_initialized, on_post_initialized: noop, on_terminated: noop },
        ServiceDescriptor { name: "StateManager", dependencies: &["ConfigStore", "WatchDog"], on_pre_initialized: noop, on_initialized: state_manager::on_initialized, on_post_initialized: state_manager::on_post_initialized, on_terminated: state_manager::on_terminated },
        ServiceDescriptor { name: "ProtectedAPIManager", dependencies: &[], on_pre_initialized: noop, on_initialized: protected_api::on_initialized, on_post_initialized: noop, on_terminated: noop },
        ServiceDescriptor { name: "PlayerStateStore", dependencies: &["ProtectedAPIManager", "StateManager"], on_pre_initialized: noop, on_initialized: player_state_store::on_initialized, on_post_initialized: noop, on_terminated: noop },
        ServiceDescriptor { name: "YTMViewManager", dependencies: &["ConfigStore", "StateManager", "MemoryStore"], on_pre_initialized: noop, on_initialized: ytm_view_manager::on_initialized, on_post_initialized: ytm_view_manager::on_post_initialized, on_terminated: noop },
        ServiceDescriptor { name: "ShortcutManager", dependencies: &["ConfigStore", "ProtectedAPIManager", "MemoryStore"], on_pre_initialized: noop, on_initialized: shortcut_manager::on_initialized, on_post_initialized: shortcut_manager::on_post_initialized, on_terminated: noop },
        ServiceDescriptor { name: "TaskbarManager", dependencies: &["ConfigStore", "ProtectedAPIManager", "PlayerStateStore"], on_pre_initialized: noop, on_initialized: taskbar_manager::on_initialized, on_post_initialized: taskbar_manager::on_post_initialized, on_terminated: noop },
        ServiceDescriptor { name: "ProtocolManager", dependencies: &["YTMViewManager", "ProtectedAPIManager"], on_pre_initialized: noop, on_initialized: protocol_manager::on_initialized, on_post_initialized: noop, on_terminated: noop },
        ServiceDescriptor { name: "IntegrationManager", dependencies: &["ConfigStore"], on_pre_initialized: noop, on_initialized: integration_manager::on_initialized, on_post_initialized: integration_manager::on_post_initialized, on_terminated: noop },
    ]
}
