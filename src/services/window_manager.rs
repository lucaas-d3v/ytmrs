//! Port of AppWindowManager / AppWindow / AppView on top of tao + wry.
//!
//! Every window owns a full size webview (the BrowserWindow webContents) and may
//! host child views (WebContentsView) positioned with auto-resize rules.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::WindowEvent;
use tao::window::{Window, WindowBuilder, WindowId};
use wry::{Rect, WebContext, WebView, WebViewBuilder};

use crate::app::App;
use crate::emit;
use crate::events::Emitter;
use crate::ipc::{IpcHandler, IpcListener, IpcRegistry, IpcReply, WebviewOwner};
use crate::shared::types::{Rectangle, WindowsEventArguments};

pub struct AppWebview {
    pub id: u64,
    pub webview: WebView,
    _context: WebContext,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavPolicy {
    /// Only app pages, anything else is blocked
    AppOnly,
    /// Block and open in the system browser (changelog)
    OpenExternal,
    /// YouTube Music view rules (see ytm_view_manager)
    Ytm,
}

#[derive(Clone, Copy)]
pub enum NewWindowPolicy {
    Deny,
    External(fn(&str) -> bool),
    Ytm,
}

pub struct WebviewSpec {
    pub url: String,
    pub init_scripts: Vec<String>,
    pub transparent: bool,
    pub autoplay: bool,
    pub data_dir: Option<PathBuf>,
    pub nav: NavPolicy,
    pub new_window: NewWindowPolicy,
    pub resize_edges: &'static str,
}

pub struct AppWindowOptions {
    pub name: String,
    pub title: String,
    pub page: String,
    pub preload: &'static str,
    pub width: f64,
    pub height: f64,
    pub min_size: Option<(f64, f64)>,
    pub position: Option<(f64, f64)>,
    pub resizable: bool,
    pub minimizable: bool,
    pub maximizable: bool,
    pub always_on_top: bool,
    pub parent: Option<String>,
    pub modal: bool,
    pub wait_for_views: bool,
    pub maximized: bool,
    pub nav: NavPolicy,
    pub new_window: NewWindowPolicy,
    pub extra_init: Vec<String>,
}

impl AppWindowOptions {
    pub fn new(name: &str, page: &str, preload: &'static str, width: f64, height: f64) -> Self {
        Self {
            name: name.into(),
            title: "YtmRs".into(),
            page: page.into(),
            preload,
            width,
            height,
            min_size: None,
            position: None,
            resizable: true,
            minimizable: true,
            maximizable: true,
            always_on_top: false,
            parent: None,
            modal: false,
            wait_for_views: true,
            maximized: false,
            nav: NavPolicy::AppOnly,
            new_window: NewWindowPolicy::Deny,
            extra_init: Vec::new(),
        }
    }
}

pub struct CloseEvent {
    prevented: Cell<bool>,
}

impl CloseEvent {
    pub fn prevent_default(&self) {
        self.prevented.set(true);
    }
}

#[derive(Default)]
pub struct WindowEvents {
    pub resize: Emitter<()>,
    pub moved: Emitter<()>,
    pub maximize: Emitter<()>,
    pub unmaximize: Emitter<()>,
    /// Not emitted when the app is quitting
    pub close: Emitter<CloseEvent>,
}

pub struct AppWindow {
    pub name: String,
    pub window: Window,
    #[cfg(target_os = "linux")]
    pub fixed: gtk::Fixed,
    pub webview: Option<AppWebview>,
    pub ready: bool,
    wait_for_views: bool,
    ready_callbacks: Vec<Box<dyn FnOnce(&mut App)>>,
    pub requested_closure: bool,
    pub events: WindowEvents,
    pub ipc: IpcRegistry,
    last_maximized: bool,
    last_minimized: bool,
    pub fullscreen: bool,
    always_on_top: bool,
    last_progress: Option<(u64, bool, bool)>,
    pub modal_parent: Option<String>,
}

pub struct AppView {
    pub name: String,
    pub parent: String,
    pub webview: AppWebview,
    pub offset_top: f64,
    /// Moved off-screen (the loading screen of the parent window is visible instead)
    pub obscured: bool,
    pub ready: bool,
    pub receive_broadcasts: bool,
    pub ipc: IpcRegistry,
    pub ready_event: Emitter<()>,
}

#[derive(Default)]
pub struct AppWindowManager {
    pub windows: Vec<AppWindow>,
    pub views: Vec<AppView>,
    next_webview_id: u64,
    owners: HashMap<u64, WebviewOwner>,
    /// Emitted with the window name after it was destroyed
    pub closed: Emitter<String>,
}

impl AppWindowManager {
    pub fn has_window(&self, name: &str) -> bool {
        self.windows.iter().any(|w| w.name == name)
    }

    pub fn get(&self, name: &str) -> Option<&AppWindow> {
        self.windows.iter().find(|w| w.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut AppWindow> {
        self.windows.iter_mut().find(|w| w.name == name)
    }

    pub fn view(&self, name: &str) -> Option<&AppView> {
        self.views.iter().find(|v| v.name == name)
    }

    pub fn view_mut(&mut self, name: &str) -> Option<&mut AppView> {
        self.views.iter_mut().find(|v| v.name == name)
    }

    pub fn window_names(&self) -> Vec<String> {
        self.windows.iter().map(|w| w.name.clone()).collect()
    }

    pub fn force_window_closures(&mut self) {
        for w in &mut self.windows {
            w.requested_closure = true;
        }
    }

    pub fn webview_owner(&self, id: u64) -> Option<WebviewOwner> {
        self.owners.get(&id).cloned()
    }

    fn webview_by_id(&self, id: u64) -> Option<&WebView> {
        for w in &self.windows {
            if let Some(wv) = &w.webview {
                if wv.id == id {
                    return Some(&wv.webview);
                }
            }
        }
        self.views.iter().find(|v| v.webview.id == id).map(|v| &v.webview.webview)
    }

    pub fn eval_webview(&self, id: u64, js: &str) {
        if let Some(wv) = self.webview_by_id(id) {
            let _ = wv.evaluate_script(js);
        }
    }

    pub fn scoped_listeners(&self, owner: &WebviewOwner, channel: &str) -> Vec<IpcListener> {
        match owner {
            WebviewOwner::Window(n) => self.get(n).map(|w| w.ipc.listeners(channel)).unwrap_or_default(),
            WebviewOwner::View(n) => self.view(n).map(|v| v.ipc.listeners(channel)).unwrap_or_default(),
        }
    }

    pub fn scoped_handler(&self, owner: &WebviewOwner, channel: &str) -> Option<IpcHandler> {
        match owner {
            WebviewOwner::Window(n) => self.get(n).and_then(|w| w.ipc.handler(channel)),
            WebviewOwner::View(n) => self.view(n).and_then(|v| v.ipc.handler(channel)),
        }
    }

    /// `webContents.send` to a window's own webview
    pub fn send(&self, window: &str, channel: &str, args: &[Value]) {
        if let Some(wv) = self.get(window).and_then(|w| w.webview.as_ref()) {
            let _ = wv.webview.evaluate_script(&crate::ipc::emit_js(channel, args));
        }
    }

    pub fn send_view(&self, view: &str, channel: &str, args: &[Value]) {
        if let Some(v) = self.view(view) {
            let _ = v.webview.webview.evaluate_script(&crate::ipc::emit_js(channel, args));
        }
    }

    pub fn eval_view(&self, view: &str, js: &str) {
        if let Some(v) = self.view(view) {
            let _ = v.webview.webview.evaluate_script(js);
        }
    }

    /// AppWindow.ipcBroadcast for every window (window webview + its views)
    pub fn broadcast_all(&self, channel: &str, args: &[Value]) {
        let js = crate::ipc::emit_js(channel, args);
        for w in &self.windows {
            if let Some(wv) = &w.webview {
                let _ = wv.webview.evaluate_script(&js);
            }
        }
        for v in &self.views {
            if v.receive_broadcasts {
                let _ = v.webview.webview.evaluate_script(&js);
            }
        }
    }

    pub fn broadcast_window(&self, window: &str, channel: &str, args: &[Value]) {
        let js = crate::ipc::emit_js(channel, args);
        if let Some(wv) = self.get(window).and_then(|w| w.webview.as_ref()) {
            let _ = wv.webview.evaluate_script(&js);
        }
        for v in self.views.iter().filter(|v| v.parent == window && v.receive_broadcasts) {
            let _ = v.webview.webview.evaluate_script(&js);
        }
    }

    #[cfg(target_os = "linux")]
    pub fn gtk_window(&self, name: &str) -> Option<gtk::Window> {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        self.get(name).map(|w| w.window.gtk_window().clone().upcast::<gtk::Window>())
    }

    pub fn is_visible(&self, name: &str) -> bool {
        self.get(name).map(|w| w.window.is_visible()).unwrap_or(false)
    }

    pub fn show_and_focus(&self, name: &str) {
        if let Some(w) = self.get(name) {
            if w.window.is_minimized() {
                w.window.set_minimized(false);
            }
            w.window.set_visible(true);
            w.window.set_focus();
        }
    }

    pub fn hide(&self, name: &str) {
        if let Some(w) = self.get(name) {
            w.window.set_visible(false);
        }
    }

    pub fn set_title(&self, name: &str, title: &str) {
        if let Some(w) = self.get(name) {
            if w.window.title() != title {
                w.window.set_title(title);
            }
        }
    }

    /// Electron getBounds() (DIP)
    pub fn bounds(&self, name: &str) -> Option<Rectangle> {
        let w = self.get(name)?;
        let sf = w.window.scale_factor();
        let pos = w.window.outer_position().map(|p| p.to_logical::<f64>(sf)).unwrap_or(LogicalPosition::new(0.0, 0.0));
        let size = w.window.inner_size().to_logical::<f64>(sf);
        Some(Rectangle { x: pos.x.round() as i32, y: pos.y.round() as i32, width: size.width.round() as i32, height: size.height.round() as i32 })
    }

    pub fn is_maximized(&self, name: &str) -> bool {
        self.get(name).map(|w| w.window.is_maximized()).unwrap_or(false)
    }

    pub fn set_progress_bar(&mut self, name: &str, progress: f64, paused: bool) {
        let Some(w) = self.get_mut(name) else { return };
        let visible = progress >= 0.0;
        let value = (progress.clamp(0.0, 1.0) * 100.0).round() as u64;
        let key = (value, paused, visible);
        if w.last_progress == Some(key) {
            return;
        }
        w.last_progress = Some(key);
        let state = if !visible {
            tao::window::ProgressState::None
        } else if paused {
            tao::window::ProgressState::Paused
        } else {
            tao::window::ProgressState::Normal
        };
        w.window.set_progress_bar(tao::window::ProgressBarState {
            state: Some(state),
            progress: if visible { Some(value) } else { None },
            desktop_filename: Some(format!("{}.desktop", crate::APP_ID)),
        });
    }

    /// Run `f` once the window is ready (AppWindow.ready())
    pub fn when_ready(&mut self, name: &str, f: impl FnOnce(&mut App) + 'static) -> bool {
        match self.get_mut(name) {
            Some(w) if !w.ready => {
                w.ready_callbacks.push(Box::new(f));
                true
            }
            _ => false,
        }
    }
}

pub fn window_state(w: &AppWindow) -> WindowsEventArguments {
    WindowsEventArguments {
        minimized: w.window.is_minimized(),
        maximized: w.window.is_maximized(),
        fullscreen: w.fullscreen || w.window.fullscreen().is_some(),
        always_on_top: w.always_on_top,
    }
}

pub fn send_window_controls_state(app: &App, name: &str) {
    if let Some(w) = app.window_manager.get(name) {
        let state = serde_json::to_value(window_state(w)).unwrap_or(Value::Null);
        app.window_manager.send(name, "windowControls:stateChanged", &[state]);
    }
}

fn window_icon() -> Option<tao::window::Icon> {
    let (rgba, w, h) = crate::assets::decode_png(crate::assets::icon_bytes("ytmrs.png")?)?;
    tao::window::Icon::from_rgba(rgba, w, h).ok()
}

fn devtools_enabled(app: &App) -> bool {
    !crate::paths::is_packaged() || app.config_store.get_bool("developer.enableDevTools")
}

fn base_init_scripts(app: &App, resize_edges: &str) -> Vec<String> {
    let is_wayland = cfg!(target_os = "linux") && (std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false));
    let platform = serde_json::json!({
        "os": std::env::consts::OS,
        "isWayland": is_wayland,
        "wcoVisible": cfg!(target_os = "macos"),
        "resizeEdges": if cfg!(target_os = "linux") { resize_edges } else { "" },
        "version": crate::APP_VERSION,
        "devTools": devtools_enabled(app),
    });
    vec![format!("window.__ytmdPlatform = Object.freeze({platform});"), crate::assets::web_text("shared/bridge.js")]
}

pub(crate) fn build_webview(app: &mut App, parent: &AppWindow, spec: WebviewSpec, bounds: Rect, owner: WebviewOwner) -> anyhow::Result<AppWebview> {
    app.window_manager.next_webview_id += 1;
    let id = app.window_manager.next_webview_id;
    let mut context = WebContext::new(spec.data_dir.clone());
    let main = app.main.clone();

    let mut builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(&spec.url)
        .with_bounds(bounds)
        .with_transparent(spec.transparent)
        .with_background_color((0, 0, 0, 255))
        .with_autoplay(spec.autoplay)
        .with_devtools(devtools_enabled(app))
        .with_clipboard(true)
        .with_hotkeys_zoom(false);

    for script in base_init_scripts(app, spec.resize_edges) {
        builder = builder.with_initialization_script(script);
    }
    for script in &spec.init_scripts {
        builder = builder.with_initialization_script(script.clone());
    }

    let ipc_main = main.clone();
    builder = builder.with_ipc_handler(move |req: http::Request<String>| {
        ipc_main.send(crate::app::UserEvent::Ipc { webview_id: id, body: req.into_body() });
    });
    builder = builder.with_custom_protocol(crate::assets::PROTOCOL.into(), |_id, request| crate::assets::serve_protocol(&request));

    let nav_main = main.clone();
    match spec.nav {
        NavPolicy::AppOnly | NavPolicy::OpenExternal => {
            let external = spec.nav == NavPolicy::OpenExternal;
            builder = builder.with_navigation_handler(move |url: String| {
                if crate::assets::is_app_url(&url) || url.starts_with("about:") || url.starts_with("data:") {
                    return true;
                }
                if external {
                    let _ = open::that_detached(&url);
                }
                log::debug!("Blocked navigation of app window to {url}");
                false
            });
        }
        NavPolicy::Ytm => {
            // On Linux the YTM policy is applied through WebKit load events (main frame only)
            #[cfg(not(target_os = "linux"))]
            {
                builder = builder.with_navigation_handler(move |url: String| crate::services::ytm_view_manager::navigation_allowed(&nav_main, &url));
            }
        }
    }
    let _ = &nav_main;

    let new_window_main = main.clone();
    let new_window = spec.new_window;
    builder = builder.with_new_window_req_handler(move |url, _features| {
        match new_window {
            NewWindowPolicy::Deny => {}
            NewWindowPolicy::External(allowed) => {
                if allowed(&url) {
                    let _ = open::that_detached(&url);
                }
            }
            NewWindowPolicy::Ytm => {
                new_window_main.run(move |_app| crate::services::ytm_view_manager::open_external_from_ytm_view(&url));
            }
        }
        wry::NewWindowResponse::Deny
    });

    #[cfg(target_os = "linux")]
    let webview = {
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(&parent.fixed)?
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build_as_child(&parent.window)?;

    #[cfg(target_os = "linux")]
    crate::platform::linux::configure_webview(&webview, app.config_store.get_bool("general.disableHardwareAcceleration"));

    if std::env::var("YTMD_OPEN_DEVTOOLS").is_ok() {
        webview.open_devtools();
    }

    app.window_manager.owners.insert(id, owner);
    Ok(AppWebview { id, webview, _context: context })
}

fn logical_size(window: &Window) -> (f64, f64) {
    let s = window.inner_size().to_logical::<f64>(window.scale_factor());
    (s.width, s.height)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { position: LogicalPosition::new(x, y).into(), size: LogicalSize::new(w.max(1.0), h.max(1.0)).into() }
}

fn apply_bounds(_window: &AppWindow, webview: &WebView, r: Rect) {
    let _ = webview.set_bounds(r);
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        use wry::WebViewExtUnix;
        let widget = webview.webview();
        let pos: LogicalPosition<f64> = r.position.to_logical(1.0);
        let size: LogicalSize<f64> = r.size.to_logical(1.0);
        _window.fixed.move_(&widget, pos.x.round() as i32, pos.y.round() as i32);
        widget.set_size_request(size.width.round() as i32, size.height.round() as i32);
    }
}

/// Recompute bounds of a window's webview and views (AppView.parentWindowResize)
pub fn layout(app: &App, name: &str) {
    let Some(w) = app.window_manager.get(name) else { return };
    let (width, height) = logical_size(&w.window);
    if let Some(wv) = &w.webview {
        apply_bounds(w, &wv.webview, rect(0.0, 0.0, width, height));
    }
    for v in app.window_manager.views.iter().filter(|v| v.parent == name) {
        let offset = if w.fullscreen { 0.0 } else { v.offset_top };
        let x = if v.obscured { width + 16.0 } else { 0.0 };
        apply_bounds(w, &v.webview.webview, rect(x, offset, width, height - offset));
    }
}

pub fn create_window(app: &mut App, opts: AppWindowOptions) -> anyhow::Result<()> {
    if app.window_manager.has_window(&opts.name) {
        anyhow::bail!("Window with the name '{}' already exists", opts.name);
    }
    log::debug!("AppWindow '{}' created", opts.name);
    let target = app.target();
    let mut builder = WindowBuilder::new()
        .with_title(&opts.title)
        .with_decorations(false)
        .with_visible(false)
        .with_inner_size(LogicalSize::new(opts.width, opts.height))
        .with_resizable(opts.resizable)
        .with_minimizable(opts.minimizable)
        .with_maximizable(opts.maximizable)
        .with_always_on_top(opts.always_on_top)
        .with_window_icon(window_icon());
    if let Some((w, h)) = opts.min_size {
        builder = builder.with_min_inner_size(LogicalSize::new(w, h));
    }
    if let Some((x, y)) = opts.position {
        builder = builder.with_position(LogicalPosition::new(x, y));
    }
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder = builder.with_decorations(true).with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true);
    }
    #[cfg(target_os = "linux")]
    if let Some(parent) = opts.parent.as_deref().and_then(|p| app.window_manager.gtk_window(p)) {
        use tao::platform::unix::WindowBuilderExtUnix;
        builder = builder.with_transient_for(&parent);
    }

    let window = builder.build(target)?;

    #[cfg(target_os = "linux")]
    let fixed = {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        let fixed = gtk::Fixed::new();
        let vbox = window.default_vbox().ok_or_else(|| anyhow::anyhow!("window has no default vbox"))?;
        vbox.pack_start(&fixed, true, true, 0);
        fixed.show_all();
        if opts.modal && opts.parent.is_some() {
            window.gtk_window().set_modal(true);
        }
        fixed
    };

    if opts.maximized {
        window.set_maximized(true);
    }

    let mut app_window = AppWindow {
        name: opts.name.clone(),
        window,
        #[cfg(target_os = "linux")]
        fixed,
        webview: None,
        ready: false,
        wait_for_views: opts.wait_for_views,
        ready_callbacks: Vec::new(),
        requested_closure: false,
        events: WindowEvents::default(),
        ipc: IpcRegistry::default(),
        last_maximized: opts.maximized,
        last_minimized: false,
        fullscreen: false,
        always_on_top: opts.always_on_top,
        last_progress: None,
        modal_parent: if opts.modal { opts.parent.clone() } else { None },
    };

    let mut init = vec![crate::assets::web_text(opts.preload)];
    init.extend(opts.extra_init);
    let edges = if opts.resizable { "nsew" } else { "" };
    let spec = WebviewSpec {
        url: crate::assets::app_url(&format!("windows/{}/index.html", opts.page)),
        init_scripts: init,
        transparent: false,
        autoplay: true,
        data_dir: None,
        nav: opts.nav,
        new_window: opts.new_window,
        resize_edges: edges,
    };
    let (w, h) = logical_size(&app_window.window);
    let webview = build_webview(app, &app_window, spec, rect(0.0, 0.0, w, h), WebviewOwner::Window(opts.name.clone()))?;
    app_window.webview = Some(webview);
    register_window_controls(&mut app_window);
    app.window_manager.windows.push(app_window);
    layout(app, &opts.name);
    Ok(())
}

pub struct ViewOptions {
    pub name: String,
    pub url: String,
    pub preload: Option<&'static str>,
    pub receive_broadcasts: bool,
    pub init_scripts: Vec<String>,
    pub data_dir: Option<PathBuf>,
    pub autoplay: bool,
    pub offset_top: f64,
    pub nav: NavPolicy,
    pub new_window: NewWindowPolicy,
    pub obscured: bool,
}

pub fn create_view(app: &mut App, parent: &str, opts: ViewOptions) -> anyhow::Result<()> {
    let Some(index) = app.window_manager.windows.iter().position(|w| w.name == parent) else {
        anyhow::bail!("Parent window '{parent}' does not exist");
    };
    log::debug!("AppView '{}' created", opts.name);
    // Temporarily take the window out to satisfy the borrow checker
    let window = app.window_manager.windows.remove(index);
    let mut init_scripts = opts.preload.map(|p| vec![crate::assets::web_text(p)]).unwrap_or_default();
    init_scripts.extend(opts.init_scripts);
    let spec = WebviewSpec {
        url: opts.url,
        init_scripts,
        transparent: false,
        autoplay: opts.autoplay,
        data_dir: opts.data_dir,
        nav: opts.nav,
        new_window: opts.new_window,
        resize_edges: "ewS",
    };
    let (w, h) = logical_size(&window.window);
    let result = build_webview(app, &window, spec, rect(0.0, opts.offset_top, w, h - opts.offset_top), WebviewOwner::View(opts.name.clone()));
    app.window_manager.windows.insert(index, window);
    let webview = result?;
    let mut view = AppView {
        name: opts.name.clone(),
        parent: parent.to_string(),
        webview,
        offset_top: opts.offset_top,
        obscured: opts.obscured,
        ready: false,
        receive_broadcasts: opts.receive_broadcasts,
        ipc: IpcRegistry::default(),
        ready_event: Emitter::default(),
    };
    let parent_name = parent.to_string();
    let p = parent_name.clone();
    view.ipc.on("windowControls:startResize", move |app, _e, args| start_resize(app, &p, args));
    app.window_manager.views.push(view);
    log::debug!("AppView '{}' attached to parent {}", opts.name, parent_name);
    layout(app, &parent_name);
    Ok(())
}

/// AppView.show()/hide() (setVisible)
pub fn set_view_visible(app: &mut App, view: &str, visible: bool) {
    if let Some(v) = app.window_manager.view(view) {
        let _ = v.webview.webview.set_visible(visible);
        log::debug!("AppView '{view}' {}", if visible { "shown" } else { "hidden" });
    }
}

pub fn set_view_obscured(app: &mut App, view: &str, obscured: bool) {
    let parent = match app.window_manager.view_mut(view) {
        Some(v) => {
            if v.obscured == obscured {
                return;
            }
            v.obscured = obscured;
            v.parent.clone()
        }
        None => return,
    };
    log::debug!("AppView '{view}' {}", if obscured { "obscured" } else { "revealed" });
    layout(app, &parent);
}

fn start_resize(app: &mut App, window: &str, args: &[Value]) {
    use tao::window::ResizeDirection::*;
    let Some(w) = app.window_manager.get(window) else { return };
    if w.window.is_maximized() || w.fullscreen || !w.window.is_resizable() {
        return;
    }
    let dir = match args.first().and_then(|v| v.as_str()).unwrap_or("") {
        "n" => North,
        "s" => South,
        "e" => East,
        "w" => West,
        "ne" => NorthEast,
        "nw" => NorthWest,
        "se" => SouthEast,
        "sw" => SouthWest,
        _ => return,
    };
    let _ = w.window.drag_resize_window(dir);
}

fn register_window_controls(w: &mut AppWindow) {
    let name = w.name.clone();
    let n = name.clone();
    w.ipc.on("windowControls:minimize", move |app, _, _| {
        if let Some(w) = app.window_manager.get(&n) {
            w.window.set_minimized(true);
        }
    });
    let n = name.clone();
    w.ipc.on("windowControls:maximize", move |app, _, _| {
        if let Some(w) = app.window_manager.get(&n) {
            w.window.set_maximized(true);
        }
    });
    let n = name.clone();
    w.ipc.on("windowControls:restore", move |app, _, _| {
        if let Some(w) = app.window_manager.get(&n) {
            if w.window.is_minimized() {
                w.window.set_minimized(false);
            } else {
                w.window.set_maximized(false);
            }
        }
    });
    let n = name.clone();
    w.ipc.on("windowControls:toggleMaximize", move |app, _, _| {
        if let Some(w) = app.window_manager.get(&n) {
            if w.window.is_maximizable() && w.window.is_resizable() {
                w.window.set_maximized(!w.window.is_maximized());
            }
        }
    });
    let n = name.clone();
    w.ipc.on("windowControls:close", move |app, _, _| close_window(app, &n));
    let n = name.clone();
    w.ipc.on("windowControls:setAlwaysOnTop", move |app, _, args| {
        let on_top = args.first().and_then(|v| v.as_bool()).unwrap_or(false);
        if let Some(w) = app.window_manager.get_mut(&n) {
            w.window.set_always_on_top(on_top);
            w.always_on_top = on_top;
        }
        send_window_controls_state(app, &n);
    });
    let n = name.clone();
    w.ipc.on("windowControls:requestState", move |app, _, _| send_window_controls_state(app, &n));
    let n = name.clone();
    w.ipc.on("windowControls:startDrag", move |app, _, _| {
        if let Some(w) = app.window_manager.get(&n) {
            let _ = w.window.drag_window();
        }
    });
    let n = name.clone();
    w.ipc.on("windowControls:startResize", move |app, _, args| start_resize(app, &n, args));
    let n = name;
    w.ipc.handle("windowControls:getState", move |app, _, _| {
        IpcReply::Ok(app.window_manager.get(&n).map(|w| serde_json::to_value(window_state(w)).unwrap_or(Value::Null)).unwrap_or(Value::Null))
    });
}

/// Global IPC used by every page: DOM ready notifications
pub fn on_initialized(app: &mut App) {
    app.ipc.on("__ytmd:domReady", |app, event, _args| match &event.owner {
        WebviewOwner::Window(name) => {
            let name = name.clone();
            mark_window_ready(app, &name);
        }
        WebviewOwner::View(name) => {
            let name = name.clone();
            if let Some(v) = app.window_manager.view_mut(&name) {
                log::debug!("AppView '{name}' webContents dom ready");
                v.ready = true;
                emit!(app, v.ready_event, &());
            }
        }
    });
    log::info!("AppWindowManager initialized");
}

fn mark_window_ready(app: &mut App, name: &str) {
    let callbacks = match app.window_manager.get_mut(name) {
        Some(w) if !w.ready => {
            if w.wait_for_views {
                w.window.set_visible(true);
            }
            w.ready = true;
            std::mem::take(&mut w.ready_callbacks)
        }
        _ => return,
    };
    send_window_controls_state(app, name);
    for cb in callbacks {
        cb(app);
    }
}

/// AppWindow.closeWindow(): emits the cancellable close event
pub fn close_window(app: &mut App, name: &str) {
    let Some(w) = app.window_manager.get(name) else { return };
    let requested = w.requested_closure;
    let event = CloseEvent { prevented: Cell::new(false) };
    if !requested {
        let callbacks = app.window_manager.get_mut(name).map(|w| w.events.close.take_for_emit()).unwrap_or_default();
        crate::events::run_callbacks(app, callbacks, &event);
    }
    if !event.prevented.get() || requested {
        if let Some(w) = app.window_manager.get_mut(name) {
            w.requested_closure = true;
        }
        destroy_window(app, name);
    }
}

pub fn destroy_window(app: &mut App, name: &str) {
    let Some(index) = app.window_manager.windows.iter().position(|w| w.name == name) else { return };
    let window = app.window_manager.windows.remove(index);
    let mut removed_ids = vec![];
    if let Some(wv) = &window.webview {
        removed_ids.push(wv.id);
    }
    let (views, rest): (Vec<AppView>, Vec<AppView>) = std::mem::take(&mut app.window_manager.views).into_iter().partition(|v| v.parent == name);
    app.window_manager.views = rest;
    for v in &views {
        removed_ids.push(v.webview.id);
        log::debug!("AppView '{}' detached from parent {}", v.name, name);
    }
    for id in removed_ids {
        app.window_manager.owners.remove(&id);
    }
    #[cfg(target_os = "linux")]
    if let Some(parent) = &window.modal_parent {
        let _ = parent;
    }
    drop(views);
    drop(window);
    log::debug!("AppWindow '{name}' closed");
    let n = name.to_string();
    emit!(app, app.window_manager.closed, &n);

    // Default Electron behavior: quit once every window has been closed
    if app.window_manager.windows.is_empty() && !app.quitting {
        app.quit();
    }
}

pub fn handle_window_event(app: &mut App, id: WindowId, event: WindowEvent) {
    let Some(name) = app.window_manager.windows.iter().find(|w| w.window.id() == id).map(|w| w.name.clone()) else { return };
    match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            layout(app, &name);
            let (maximized, minimized, was_max, was_min) = {
                let w = app.window_manager.get(&name).unwrap();
                (w.window.is_maximized(), w.window.is_minimized(), w.last_maximized, w.last_minimized)
            };
            if let Some(w) = app.window_manager.get_mut(&name) {
                w.last_maximized = maximized;
                w.last_minimized = minimized;
            }
            if maximized != was_max {
                if maximized {
                    let cbs = app.window_manager.get_mut(&name).map(|w| w.events.maximize.take_for_emit()).unwrap_or_default();
                    crate::events::run_callbacks(app, cbs, &());
                } else {
                    let cbs = app.window_manager.get_mut(&name).map(|w| w.events.unmaximize.take_for_emit()).unwrap_or_default();
                    crate::events::run_callbacks(app, cbs, &());
                }
            }
            if maximized != was_max || minimized != was_min {
                send_window_controls_state(app, &name);
            }
            let cbs = app.window_manager.get_mut(&name).map(|w| w.events.resize.take_for_emit()).unwrap_or_default();
            crate::events::run_callbacks(app, cbs, &());
        }
        WindowEvent::Moved(_) => {
            let cbs = app.window_manager.get_mut(&name).map(|w| w.events.moved.take_for_emit()).unwrap_or_default();
            crate::events::run_callbacks(app, cbs, &());
        }
        WindowEvent::CloseRequested => close_window(app, &name),
        WindowEvent::ThemeChanged(_) => crate::tray::update_icon(app),
        _ => {}
    }
}

/// Toggle the HTML fullscreen layout of a window (enter/leave-html-full-screen)
pub fn set_fullscreen_layout(app: &mut App, name: &str, fullscreen: bool) {
    if let Some(w) = app.window_manager.get_mut(name) {
        w.fullscreen = fullscreen;
    }
    layout(app, name);
    send_window_controls_state(app, name);
}
