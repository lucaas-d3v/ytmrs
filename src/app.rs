//! Application root: owns every service (the ServiceHost of the original app) and is only
//! touched from the GTK main thread. Worker threads (tokio) and GTK signal handlers talk to
//! it by posting closures through [`MainHandle`]; they are run by the main loop, never
//! re-entrantly from inside another App call.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::integrations::Integrations;
use crate::ipc::IpcRegistry;
use crate::services::{
    config_store::ConfigStore, integration_manager::IntegrationManager,
    memory_store::MemoryStore, player_state_store::PlayerStateStore, protected_api::ProtectedApiManager,
    protocol_manager::ProtocolManager, safe_storage::SafeStorage, service_host::ServiceHost,
    shortcut_manager::ShortcutManager, state_manager::StateManager, taskbar_manager::TaskbarManager,
    ytm_view_manager::YtmViewManager,
};

pub type Task = Box<dyn FnOnce(&mut App) + Send>;

pub enum UserEvent {
    Task(Task),
    Menu(tray_icon::menu::MenuEvent),
    Tray(tray_icon::TrayIconEvent),
    HotKey(global_hotkey::GlobalHotKeyEvent),
    SecondInstance(Vec<String>),
}

#[derive(Clone)]
pub struct MainHandle {
    tx: UnboundedSender<UserEvent>,
}

impl MainHandle {
    pub fn new(tx: UnboundedSender<UserEvent>) -> Self {
        Self { tx }
    }

    pub fn send(&self, event: UserEvent) {
        let _ = self.tx.send(event);
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
    pub main: MainHandle,
    pub rt: tokio::runtime::Handle,
    pub http: reqwest::Client,
    pub exit_requested: bool,
    pub quitting: bool,

    pub host: ServiceHost,
    pub config_store: ConfigStore,
    pub memory_store: MemoryStore,
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
    pub ui: crate::ui::Ui,

    pending: HashMap<u64, Box<dyn FnOnce(&mut App, Value)>>,
    next_pending: u64,
    pub menu_actions: HashMap<String, Box<dyn Fn(&mut App)>>,
}

impl App {
    pub fn new(main: MainHandle, rt: tokio::runtime::Handle) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("YtmRs/{} (https://github.com/lucaas-d3v/ytmrs)", crate::APP_VERSION))
            .build()
            .expect("failed to build http client");
        Self {
            main,
            rt,
            http,
            exit_requested: false,
            quitting: false,
            host: ServiceHost::default(),
            config_store: ConfigStore::default(),
            memory_store: MemoryStore::default(),
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
            ui: crate::ui::Ui::default(),
            pending: HashMap::new(),
            next_pending: 1,
            menu_actions: HashMap::new(),
        }
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
            UserEvent::Menu(event) => {
                let id = event.id.0.clone();
                if let Some(action) = self.menu_actions.remove(&id) {
                    action(self);
                    self.menu_actions.insert(id, action);
                }
            }
            UserEvent::Tray(event) => crate::tray::handle_tray_event(self, event),
            UserEvent::HotKey(event) => crate::services::shortcut_manager::handle_hotkey(self, event),
            UserEvent::SecondInstance(args) => crate::lifecycle::second_instance(self, args),
        }
    }

    /// Electron `app.quit()`
    pub fn quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        log::debug!("Application going to quit");
        crate::lifecycle::on_quit(self);
        crate::ui::destroy_all(self);
        self.exit_requested = true;
    }

    /// Electron `app.relaunch()` + quit
    pub fn relaunch(&mut self) {
        if let Ok(exe) = std::env::current_exe() {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let mut command = std::process::Command::new(exe);
            command.args(args).env("YTMD_RELAUNCH", "1");
            // The new instance decides its own engine options from the saved settings
            let _ = crate::engine::scrub(&mut command).spawn();
        }
        self.quit();
    }
}
