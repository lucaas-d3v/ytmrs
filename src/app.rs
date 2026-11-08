//! Application root: owns every service (the ServiceHost of the original app) and
//! is only ever touched from the main (GUI) thread. Worker threads talk to it by
//! posting closures through [`MainHandle`].

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tao::event::WindowEvent;
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::window::WindowId;

use crate::integrations::Integrations;
use crate::ipc::IpcRegistry;
use crate::services::{
    auto_updater::AutoUpdater, config_store::ConfigStore, integration_manager::IntegrationManager,
    memory_store::MemoryStore, player_state_store::PlayerStateStore, protected_api::ProtectedApiManager,
    protocol_manager::ProtocolManager, safe_storage::SafeStorage, service_host::ServiceHost,
    shortcut_manager::ShortcutManager, state_manager::StateManager, taskbar_manager::TaskbarManager,
    window_manager::AppWindowManager, ytm_view_manager::YtmViewManager,
};

pub type Target = EventLoopWindowTarget<UserEvent>;
pub type Task = Box<dyn FnOnce(&mut App) + Send>;

pub enum UserEvent {
    Task(Task),
    Ipc { webview_id: u64, body: String },
    Menu(muda::MenuEvent),
    Tray(tray_icon::TrayIconEvent),
    HotKey(global_hotkey::GlobalHotKeyEvent),
    SecondInstance(Vec<String>),
}

#[derive(Clone)]
pub struct MainHandle {
    proxy: EventLoopProxy<UserEvent>,
}

impl MainHandle {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self { proxy }
    }

    pub fn send(&self, event: UserEvent) {
        let _ = self.proxy.send_event(event);
    }

    /// Run a closure on the main thread with access to the App
    pub fn run(&self, f: impl FnOnce(&mut App) + Send + 'static) {
        self.send(UserEvent::Task(Box::new(f)));
    }

    /// Run a closure on the main thread and await its result
    pub async fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut App) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.run(move |app| {
            let _ = tx.send(f(app));
        });
        rx.await.ok()
    }
}

/// setTimeout equivalent handle
#[derive(Clone)]
pub struct TimerHandle {
    cancelled: Arc<AtomicBool>,
    abort: tokio::task::AbortHandle,
}

impl TimerHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.abort.abort();
    }
}

pub struct App {
    target: *const Target,
    pub main: MainHandle,
    pub rt: tokio::runtime::Handle,
    pub http: reqwest::Client,
    pub exit_requested: bool,
    pub quitting: bool,

    pub host: ServiceHost,
    pub config_store: ConfigStore,
    pub memory_store: MemoryStore,
    pub window_manager: AppWindowManager,
    pub auto_updater: AutoUpdater,
    pub state_manager: StateManager,
    pub protected_api: ProtectedApiManager,
    pub player_state_store: PlayerStateStore,
    pub ytm_view_manager: YtmViewManager,
    pub shortcut_manager: ShortcutManager,
    pub taskbar_manager: TaskbarManager,
    pub protocol_manager: ProtocolManager,
    pub integration_manager: IntegrationManager,
    pub integrations: Integrations,
    pub safe_storage: Arc<SafeStorage>,
    pub ipc: IpcRegistry,
    pub ui: crate::lifecycle::UiState,

    pending: HashMap<u64, Box<dyn FnOnce(&mut App, Value)>>,
    next_pending: u64,
    pub menu_actions: HashMap<String, Box<dyn Fn(&mut App)>>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, rt: tokio::runtime::Handle) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("YtmRs/{} (https://github.com/ytmdesktop/ytmdesktop)", crate::APP_VERSION))
            .build()
            .expect("failed to build http client");
        Self {
            target: std::ptr::null(),
            main: MainHandle::new(proxy),
            rt,
            http,
            exit_requested: false,
            quitting: false,
            host: ServiceHost::default(),
            config_store: ConfigStore::default(),
            memory_store: MemoryStore::default(),
            window_manager: AppWindowManager::default(),
            auto_updater: AutoUpdater::default(),
            state_manager: StateManager::default(),
            protected_api: ProtectedApiManager::default(),
            player_state_store: PlayerStateStore::default(),
            ytm_view_manager: YtmViewManager::default(),
            shortcut_manager: ShortcutManager::default(),
            taskbar_manager: TaskbarManager::default(),
            protocol_manager: ProtocolManager::default(),
            integration_manager: IntegrationManager::default(),
            integrations: Integrations::default(),
            safe_storage: Arc::new(SafeStorage::default()),
            ipc: IpcRegistry::default(),
            ui: Default::default(),
            pending: HashMap::new(),
            next_pending: 1,
            menu_actions: HashMap::new(),
        }
    }

    pub fn set_target(&mut self, target: &Target) {
        self.target = target as *const Target;
    }

    /// The event loop window target. Only valid while the event loop is running,
    /// which is always the case for code executing on the main thread.
    pub fn target(&self) -> &'static Target {
        assert!(!self.target.is_null(), "event loop target not available yet");
        // SAFETY: the target lives inside the event loop which lives for the remaining
        // lifetime of the process once `run` is called; App is only used on that thread.
        unsafe { &*self.target }
    }

    pub fn spawn<F>(&self, fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(fut)
    }

    pub fn set_timeout(&self, delay: Duration, f: impl FnOnce(&mut App) + Send + 'static) -> TimerHandle {
        let cancelled = Arc::new(AtomicBool::new(false));
        let c = cancelled.clone();
        let main = self.main.clone();
        let handle = self.rt.spawn(async move {
            tokio::time::sleep(delay).await;
            main.run(move |app| {
                if !c.load(Ordering::SeqCst) {
                    f(app)
                }
            });
        });
        TimerHandle { cancelled, abort: handle.abort_handle() }
    }

    /// Register a main thread callback that can be resolved later through [`App::resolve_pending`]
    pub fn register_pending(&mut self, f: impl FnOnce(&mut App, Value) + 'static) -> u64 {
        let id = self.next_pending;
        self.next_pending += 1;
        self.pending.insert(id, Box::new(f));
        id
    }

    pub fn resolve_pending(&mut self, id: u64, value: Value) {
        if let Some(f) = self.pending.remove(&id) {
            f(self, value);
        }
    }

    pub fn handle_user_event(&mut self, event: UserEvent) {
        match event {
            UserEvent::Task(task) => task(self),
            UserEvent::Ipc { webview_id, body } => crate::ipc::dispatch(self, webview_id, &body),
            UserEvent::Menu(event) => {
                let id = event.id.0.clone();
                if let Some(action) = self.menu_actions.remove(&id) {
                    action(self);
                    // Tray actions are persistent, context menu actions are one-shot
                    if id.starts_with("tray:") {
                        self.menu_actions.insert(id, action);
                    }
                }
            }
            UserEvent::Tray(event) => crate::tray::handle_tray_event(self, event),
            UserEvent::HotKey(event) => crate::services::shortcut_manager::handle_hotkey(self, event),
            UserEvent::SecondInstance(args) => crate::lifecycle::second_instance(self, args),
        }
    }

    pub fn handle_window_event(&mut self, id: WindowId, event: WindowEvent) {
        crate::services::window_manager::handle_window_event(self, id, event);
    }

    /// Electron `app.quit()`
    pub fn quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        log::debug!("Application going to quit");
        self.window_manager.force_window_closures();
        let names = self.window_manager.window_names();
        for name in names {
            crate::services::window_manager::destroy_window(self, &name);
        }
        crate::lifecycle::on_quit(self);
        self.exit_requested = true;
    }

    /// Electron `app.relaunch()` + quit
    pub fn relaunch(&mut self) {
        if let Ok(exe) = std::env::current_exe() {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let _ = std::process::Command::new(exe).args(args).env("YTMD_RELAUNCH", "1").spawn();
        }
        self.quit();
    }
}
