//! Port of YTMViewManager. YouTube Music is the only web content of the app: a WebKitGTK view
//! with a single injected script (`web/ytm/scripts.js`). Navigation policy, styles, context
//! menu, zoom and crash recovery are handled natively here.

use gtk::prelude::*;
use gtk::{gdk, gio};
use serde_json::{json, Value};
use webkit2gtk::{
    AutoplayPolicy, CacheModel, ContextMenuAction, ContextMenuExt, ContextMenuItem, CookieManagerExt, CookiePersistentStorage,
    HardwareAccelerationPolicy, HitTestResultExt, InputMethodContextExt, LoadEvent, MemoryPressureSettings, PermissionRequestExt, SettingsExt, URIRequestExt,
    UserContentInjectedFrames, UserContentManager, UserContentManagerExt, UserScript, UserScriptInjectionTime, UserStyleLevel,
    UserStyleSheet, WebContext, WebContextExt, WebInspectorExt, WebView, WebViewExt, WebsiteDataManager, WebsiteDataManagerExt,
    WebsitePolicies,
};

use crate::app::{App, MainHandle};
use crate::emit;
use crate::events::Emitter;
use crate::shared::types::{setup_flags, YTMViewStatus};

pub const YTM_URL: &str = "https://music.youtube.com/";

const SUPPORTED_DOMAINS: &[&str] = &[".google.com",".google.ad",".google.ae",".google.com.af",".google.com.ag",".google.al",".google.am",".google.co.ao",".google.com.ar",".google.as",".google.at",".google.com.au",".google.az",".google.ba",".google.com.bd",".google.be",".google.bf",".google.bg",".google.com.bh",".google.bi",".google.bj",".google.com.bn",".google.com.bo",".google.com.br",".google.bs",".google.bt",".google.co.bw",".google.by",".google.com.bz",".google.ca",".google.cd",".google.cf",".google.cg",".google.ch",".google.ci",".google.co.ck",".google.cl",".google.cm",".google.cn",".google.com.co",".google.co.cr",".google.com.cu",".google.cv",".google.com.cy",".google.cz",".google.de",".google.dj",".google.dk",".google.dm",".google.com.do",".google.dz",".google.com.ec",".google.ee",".google.com.eg",".google.es",".google.com.et",".google.fi",".google.com.fj",".google.fm",".google.fr",".google.ga",".google.ge",".google.gg",".google.com.gh",".google.com.gi",".google.gl",".google.gm",".google.gr",".google.com.gt",".google.gy",".google.com.hk",".google.hn",".google.hr",".google.ht",".google.hu",".google.co.id",".google.ie",".google.co.il",".google.im",".google.co.in",".google.iq",".google.is",".google.it",".google.je",".google.com.jm",".google.jo",".google.co.jp",".google.co.ke",".google.com.kh",".google.ki",".google.kg",".google.co.kr",".google.com.kw",".google.kz",".google.la",".google.com.lb",".google.li",".google.lk",".google.co.ls",".google.lt",".google.lu",".google.lv",".google.com.ly",".google.co.ma",".google.md",".google.me",".google.mg",".google.mk",".google.ml",".google.com.mm",".google.mn",".google.com.mt",".google.mu",".google.mv",".google.mw",".google.com.mx",".google.com.my",".google.co.mz",".google.com.na",".google.com.ng",".google.com.ni",".google.ne",".google.nl",".google.no",".google.com.np",".google.nr",".google.nu",".google.co.nz",".google.com.om",".google.com.pa",".google.com.pe",".google.com.pg",".google.com.ph",".google.com.pk",".google.pl",".google.pn",".google.com.pr",".google.ps",".google.pt",".google.com.py",".google.com.qa",".google.ro",".google.ru",".google.rw",".google.com.sa",".google.com.sb",".google.sc",".google.se",".google.com.sg",".google.sh",".google.si",".google.sk",".google.com.sl",".google.sn",".google.so",".google.sm",".google.sr",".google.st",".google.com.sv",".google.td",".google.tg",".google.co.th",".google.com.tj",".google.tl",".google.tm",".google.tn",".google.to",".google.com.tr",".google.tt",".google.com.tw",".google.co.tz",".google.com.ua",".google.co.ug",".google.co.uk",".google.com.uy",".google.co.uz",".google.com.vc",".google.co.ve",".google.co.vi",".google.com.vn",".google.vu",".google.ws",".google.rs",".google.co.za",".google.co.zm",".google.co.zw",".google.cat"];

pub fn url_is_google_accounts_domain(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    // JS: hostname.split("accounts")[1] -> the remainder after the first "accounts"
    let split = |needle: &str| host.find(needle).map(|i| &host[i + needle.len()..]);
    [split("accounts"), split("myaccount")].iter().flatten().any(|d| SUPPORTED_DOMAINS.contains(d))
}

pub fn is_prevented_nav_or_redirect(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    let path = url.path();
    host != "consent.youtube.com"
        && host != "accounts.youtube.com"
        && host != "music.youtube.com"
        && !((host == "www.youtube.com" || host == "youtube.com") && matches!(path, "/signin" | "/premium" | "/musicpremium" | "/signin_prompt"))
        && !url_is_google_accounts_domain(url)
}

pub fn is_premium_redirect(url: &url::Url) -> bool {
    matches!(url.host_str(), Some("www.youtube.com") | Some("youtube.com")) && matches!(url.path(), "/premium" | "/musicpremium")
}

pub const PREMIUM_LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?ltmpl=music&service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";

pub fn external_allowed(url_string: &str) -> bool {
    let Ok(url) = url::Url::parse(url_string) else { return false };
    let host = url.host_str().unwrap_or("");
    let mut parts: Vec<&str> = host.split('.').collect();
    parts.reverse();
    if parts.len() < 2 {
        return false;
    }
    let domain = format!("{}.{}", parts[1], parts[0]);
    matches!(domain.as_str(), "google.com" | "youtube.com" | "googleusercontent.com" | "ggpht.com")
}

pub fn open_external_from_ytm_view(url_string: &str) {
    if external_allowed(url_string) {
        crate::util::open_external(url_string);
    } else {
        log::warn!("Attempted to open {url_string} from YTMView but was denied");
    }
}

fn is_ytm_url(url: &str) -> bool {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h == "music.youtube.com")).unwrap_or(false)
}

/// Native replacements of the original in-page style tweaks
const BASE_CSS: &str = "#history-link .history-button { display: inline-block !important; }";
const PERSIST_VOLUME_SLIDER_CSS: &str = "ytmusic-app-layout > ytmusic-player-bar #volume-slider { opacity: 1 !important; pointer-events: initial !important; }";

#[derive(Default)]
pub struct YtmViewManager {
    view: Option<WebView>,
    hooks_ready: bool,
    hook_error: Option<Value>,
    setup_completion_flags: u32,
    initialized: bool,
    status: Option<YTMViewStatus>,
    pub load_error: Option<(i64, String)>,
    ready_callbacks: Vec<Box<dyn FnOnce(&mut App)>>,
    volume_slider_sheet: Option<UserStyleSheet>,
    pub status_changed: Emitter<()>,
    pub view_recreated: Emitter<()>,
    pub unresponsive: Emitter<()>,
    pub responsive: Emitter<()>,
    pub load_errored: Emitter<()>,
    /// Main frame URL changed (history buttons)
    pub navigated: Emitter<()>,
    pub timed_lyrics: Emitter<Value>,
}

impl YtmViewManager {
    pub fn status(&self) -> YTMViewStatus {
        self.status.unwrap_or(YTMViewStatus::Loading)
    }
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    pub fn view(&self) -> Option<&WebView> {
        self.view.as_ref()
    }
    pub fn has_error(&self) -> bool {
        self.hook_error.is_some()
    }
    pub fn error(&self) -> Option<&Value> {
        self.hook_error.as_ref()
    }
    pub fn setup_flags(&self) -> u32 {
        self.setup_completion_flags
    }
}

pub fn on_initialized(app: &mut App) {
    if app.ytm_view_manager.initialized {
        panic!("YTMViewManager is already initialized!");
    }
    app.ytm_view_manager.initialized = true;
    register_ipc(app);
    log::info!("YTMViewManager initialized");
}

pub fn on_post_initialized(app: &mut App) {
    app.config_store.on_did_change("appearance", |app, new, _old| {
        if let (Some(zoom), Some(view)) = (new.get("zoom").and_then(|v| v.as_f64()), app.ytm_view_manager.view()) {
            view.set_zoom_level(zoom / 100.0);
        }
        let persist = new.get("alwaysShowVolumeSlider").and_then(|v| v.as_bool()).unwrap_or(false);
        set_persist_volume_slider(app, persist);
    });
}

/// Wait until the YTM view hooks are ready (`ytmViewManager.ready()`)
pub fn when_ready(app: &mut App, f: impl FnOnce(&mut App) + 'static) {
    if app.ytm_view_manager.view.is_some() && app.ytm_view_manager.hooks_ready && app.ytm_view_manager.status() == YTMViewStatus::Ready {
        f(app);
    } else {
        app.ytm_view_manager.ready_callbacks.push(Box::new(f));
    }
}

fn set_status(app: &mut App, status: YTMViewStatus) {
    log::debug!("YTMView status {status:?}");
    app.ytm_view_manager.status = Some(status);
    if status == YTMViewStatus::Ready && app.ytm_view_manager.hooks_ready {
        let callbacks = std::mem::take(&mut app.ytm_view_manager.ready_callbacks);
        for cb in callbacks {
            cb(app);
        }
    }
    emit!(app, app.ytm_view_manager.status_changed, &());
}

/// Evaluate JavaScript in the YouTube Music page
pub fn eval_js(app: &App, js: &str) {
    if let Some(view) = app.ytm_view_manager.view() {
        view.run_javascript(js, None::<&gio::Cancellable>, |result| {
            if let Err(err) = result {
                log::trace!("ytmview: script error {err}");
            }
        });
    }
}

/// Evaluate a function expression in the YTM view (`ytmView:executeScript`)
pub fn execute_script(app: &App, script: &str) {
    eval_js(app, &format!("(\n{script}\n)()"));
}

pub fn load_default(app: &App) {
    if let Some(view) = app.ytm_view_manager.view() {
        view.load_uri(YTM_URL);
    }
}

/// "Refresh YouTube Music": reload the page, or load YouTube Music if nothing was committed
pub fn retry(app: &App) {
    let Some(view) = app.ytm_view_manager.view() else { return };
    match view.uri().filter(|u| u.starts_with("https://")) {
        Some(_) => view.reload(),
        None => view.load_uri(YTM_URL),
    }
}

pub fn reload(app: &App) {
    if let Some(view) = app.ytm_view_manager.view() {
        view.reload();
    }
}

pub fn go_back(app: &App) {
    if let Some(view) = app.ytm_view_manager.view() {
        view.go_back();
    }
}

pub fn go_forward(app: &App) {
    if let Some(view) = app.ytm_view_manager.view() {
        view.go_forward();
    }
}

/// User level stylesheet in the YTM page (Electron `insertCSS(css, { cssOrigin: "user" })`)
pub fn insert_css(app: &App, css: &str) -> Option<UserStyleSheet> {
    let manager = app.ytm_view_manager.view()?.user_content_manager()?;
    let sheet = UserStyleSheet::new(css, UserContentInjectedFrames::TopFrame, UserStyleLevel::User, &[], &[]);
    manager.add_style_sheet(&sheet);
    Some(sheet)
}

pub fn remove_css(app: &App, sheet: &UserStyleSheet) {
    if let Some(manager) = app.ytm_view_manager.view().and_then(|v| v.user_content_manager()) {
        manager.remove_style_sheet(sheet);
    }
}

fn set_persist_volume_slider(app: &mut App, persist: bool) {
    match (persist, app.ytm_view_manager.volume_slider_sheet.is_some()) {
        (true, false) => app.ytm_view_manager.volume_slider_sheet = insert_css(app, PERSIST_VOLUME_SLIDER_CSS),
        (false, true) => {
            if let Some(sheet) = app.ytm_view_manager.volume_slider_sheet.take() {
                remove_css(app, &sheet);
            }
        }
        _ => {}
    }
}

fn devtools_enabled(app: &App) -> bool {
    !crate::paths::is_packaged() || app.config_store.get_bool("developer.enableDevTools")
}

/// WebKit memory tuning for the web and network processes
fn memory_pressure_settings() -> MemoryPressureSettings {
    let mut settings = MemoryPressureSettings::new();
    settings.set_memory_limit(640);
    // strict first: WebKit asserts conservative < strict on every set
    settings.set_strict_threshold(0.8);
    settings.set_conservative_threshold(0.5);
    // never kill the web process because of memory usage
    settings.set_kill_threshold(0.0);
    settings.set_poll_interval(10.0);
    settings
}

/// Must run before any WebKit process is started
pub fn configure_network_process() {
    WebsiteDataManager::set_memory_pressure_settings(&mut memory_pressure_settings());
}

/// Creates the YouTube Music view; the main window places it in its content area
pub fn create_view(app: &mut App) -> WebView {
    set_status(app, YTMViewStatus::Loading);

    let mut url = YTM_URL.to_string();
    if app.config_store.get_bool("playback.continueWhereYouLeftOff") {
        if let Some(last) = app.config_store.get_str("state.lastUrl") {
            if last.starts_with(YTM_URL) {
                url = last;
            }
        }
    }

    // Same storage layout as previous versions (keeps the user logged in)
    let partition = if crate::paths::is_packaged() { "ytmview" } else { "ytmview-dev" };
    let data_dir = crate::paths::get().user_data.join("Partitions").join(partition);
    let _ = std::fs::create_dir_all(&data_dir);
    // Registrations left by older versions (the page can no longer register one, see scripts.js)
    let _ = std::fs::remove_dir_all(data_dir.join("serviceworkers"));
    let data_manager = WebsiteDataManager::builder()
        .base_cache_directory(data_dir.to_string_lossy().to_string())
        .base_data_directory(data_dir.to_string_lossy().to_string())
        .build();
    if let Some(cookies) = data_manager.cookie_manager() {
        cookies.set_persistent_storage(&data_dir.join("cookies").to_string_lossy(), CookiePersistentStorage::Text);
    }
    let context = WebContext::builder().website_data_manager(&data_manager).memory_pressure_settings(&memory_pressure_settings()).build();
    // A single page app: small in-memory caches, no back/forward page cache
    context.set_cache_model(CacheModel::DocumentBrowser);
    context.set_use_system_appearance_for_scrollbars(false);

    let content = UserContentManager::new();
    let mut scripts = Vec::new();
    if app.config_store.get_bool("integrations.enhancedMediaServiceEnabled") {
        // The enhanced media service owns the system media session (MPRIS)
        scripts.push(crate::assets::ytm_script("disable-media-session.js"));
    }
    scripts.push(crate::assets::ytm_script("scripts.js"));
    for source in scripts {
        content.add_script(&UserScript::new(&source, UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::Start, &["https://music.youtube.com/*"], &[]));
    }
    content.add_style_sheet(&UserStyleSheet::new(BASE_CSS, UserContentInjectedFrames::TopFrame, UserStyleLevel::User, &["https://music.youtube.com/*"], &[]));
    let main = app.main.clone();
    content.connect_script_message_received(Some("ytmd"), move |_, message| {
        let Some(body) = message.js_value().map(|v| v.to_string()) else { return };
        main.run(move |app| crate::ipc::dispatch(app, &body));
    });
    content.register_script_message_handler("ytmd");

    let autoplay = !(app.config_store.get_bool("playback.continueWhereYouLeftOff") && app.config_store.get_bool("playback.continueWhereYouLeftOffPaused"));
    let policies = WebsitePolicies::builder().autoplay(if autoplay { AutoplayPolicy::Allow } else { AutoplayPolicy::AllowWithoutSound }).build();

    let view = WebView::builder().web_context(&context).user_content_manager(&content).website_policies(&policies).build();
    if let Some(settings) = WebViewExt::settings(&view) {
        settings.set_hardware_acceleration_policy(if app.config_store.get_bool("general.disableHardwareAcceleration") {
            HardwareAccelerationPolicy::Never
        } else {
            HardwareAccelerationPolicy::OnDemand
        });
        settings.set_enable_page_cache(false);
        settings.set_javascript_can_access_clipboard(true);
        settings.set_enable_developer_extras(devtools_enabled(app));
    }
    // fcitx and other input methods anchor at the caret without preedit
    if let Some(im) = view.input_method_context() {
        im.set_enable_preedit(false);
    }
    view.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 1.0));
    view.set_zoom_level(app.config_store.get_f64("appearance.zoom").unwrap_or(100.0) / 100.0);

    attach_signals(&view, app.main.clone());
    app.ytm_view_manager.view = Some(view.clone());
    if app.config_store.get_bool("appearance.alwaysShowVolumeSlider") {
        set_persist_volume_slider(app, true);
    }
    if std::env::var("YTMD_OPEN_DEVTOOLS").is_ok() {
        if let Some(inspector) = view.inspector() {
            inspector.show();
        }
    }
    view.load_uri(&url);
    view
}

/// Events coming from WebKit signals
pub enum ViewSignal {
    LoadStarted,
    LoadCommitted(String),
    LoadFailed(i64, String),
    Crashed,
    Unresponsive,
    Responsive,
    EnterFullscreen,
    LeaveFullscreen,
    Navigated(String),
}

fn attach_signals(view: &WebView, main: MainHandle) {
    let m = main.clone();
    view.connect_load_changed(move |view, event| {
        let uri = view.uri().map(|u| u.to_string()).unwrap_or_default();
        log::trace!("ytmview: load {event:?} {uri}");
        match event {
            LoadEvent::Started => {
                m.run(|app| handle_signal(app, ViewSignal::LoadStarted));
                if let Ok(url) = url::Url::parse(&uri) {
                    if (url.scheme() == "http" || url.scheme() == "https") && is_prevented_nav_or_redirect(&url) {
                        log::info!("Blocking YTM View navigation to {uri}");
                        view.stop_loading();
                        open_external_from_ytm_view(&uri);
                    }
                }
            }
            LoadEvent::Redirected => {
                if let Ok(url) = url::Url::parse(&uri) {
                    if is_prevented_nav_or_redirect(&url) {
                        log::info!("Blocking YTM View redirect to {uri}");
                        view.stop_loading();
                    }
                    if is_premium_redirect(&url) {
                        // This users region requires a premium subscription to use YTM
                        let view = view.clone();
                        gtk::glib::idle_add_local_once(move || view.load_uri(PREMIUM_LOGIN_URL));
                    }
                }
            }
            LoadEvent::Committed => m.run(move |app| handle_signal(app, ViewSignal::LoadCommitted(uri))),
            _ => {}
        }
    });

    let m = main.clone();
    view.connect_load_failed(move |_, _event, uri, error| {
        // Cancellations / policy interruptions are not load failures
        if error.matches(webkit2gtk::NetworkError::Cancelled) || error.matches(webkit2gtk::PolicyError::FrameLoadInterruptedByPolicyChange) {
            return false;
        }
        log::debug!("ytmview: load failed {uri}: {error}");
        let code: i64 = if error.kind::<webkit2gtk::NetworkError>().is_some() {
            -2
        } else if error.kind::<webkit2gtk::PolicyError>().is_some() {
            -3
        } else {
            -1
        };
        let message = error.message().to_string();
        m.run(move |app| handle_signal(app, ViewSignal::LoadFailed(code, message)));
        // The native loading overlay shows the error instead of the WebKit error page
        true
    });

    let m = main.clone();
    view.connect_web_process_terminated(move |_, reason| {
        log::warn!("ytmview: web process terminated ({reason:?})");
        m.run(|app| handle_signal(app, ViewSignal::Crashed));
    });

    let m = main.clone();
    view.connect_is_web_process_responsive_notify(move |view| {
        let signal = if view.is_web_process_responsive() { ViewSignal::Responsive } else { ViewSignal::Unresponsive };
        m.run(move |app| handle_signal(app, signal));
    });

    let m = main.clone();
    view.connect_enter_fullscreen(move |_| {
        m.run(|app| handle_signal(app, ViewSignal::EnterFullscreen));
        false
    });
    let m = main.clone();
    view.connect_leave_fullscreen(move |_| {
        m.run(|app| handle_signal(app, ViewSignal::LeaveFullscreen));
        false
    });

    // did-navigate / did-navigate-in-page (also fires for history.pushState)
    let m = main.clone();
    view.connect_uri_notify(move |view| {
        if let Some(uri) = view.uri() {
            let uri = uri.to_string();
            m.run(move |app| handle_signal(app, ViewSignal::Navigated(uri)));
        }
    });

    // window.open / target=_blank: never create windows, maybe open in the browser
    view.connect_create(|_, action| {
        if let Some(uri) = action.request().and_then(|r| r.uri()) {
            open_external_from_ytm_view(&uri);
        }
        None
    });

    // Notifications, geolocation, media devices...: denied like the original
    view.connect_permission_request(|_, request| {
        request.deny();
        true
    });

    // Mouse back / forward buttons
    view.connect_button_press_event(|view, event| {
        match event.button() {
            8 => view.go_back(),
            9 => view.go_forward(),
            _ => return gtk::glib::Propagation::Proceed,
        }
        gtk::glib::Propagation::Stop
    });

    let m = main;
    view.connect_context_menu(move |view, menu, _event, hit| {
        build_context_menu(view, menu, hit, &m);
        false
    });
}

/// Same entries as the original Electron context menu
fn build_context_menu(view: &WebView, menu: &webkit2gtk::ContextMenu, hit: &webkit2gtk::HitTestResult, main: &MainHandle) {
    menu.remove_all();
    let action = |name: &str, f: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| f());
        action
    };
    let separator = || ContextMenuItem::new_separator();

    if hit.context_is_link() {
        if let Some(link) = hit.link_uri().map(|s| s.to_string()) {
            let a = action("open-link", Box::new(move || open_external_from_ytm_view(&link)));
            menu.append(&ContextMenuItem::from_gaction(&a, "Open Link in Browser", None));
            menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::CopyLinkToClipboard));
        }
    }
    if hit.context_is_image() {
        if let Some(src) = hit.image_uri().map(|s| s.to_string()) {
            if menu.n_items() > 0 {
                menu.append(&separator());
            }
            let a = action("open-image", Box::new(move || open_external_from_ytm_view(&src)));
            menu.append(&ContextMenuItem::from_gaction(&a, "Open Image in Browser", None));
        }
    }
    if hit.context_is_editable() {
        if menu.n_items() > 0 {
            menu.append(&separator());
        }
        let v = view.clone();
        let undo = action("undo", Box::new(move || v.execute_editing_command("Undo")));
        let v = view.clone();
        let redo = action("redo", Box::new(move || v.execute_editing_command("Redo")));
        menu.append(&ContextMenuItem::from_gaction(&undo, "Undo", None));
        menu.append(&ContextMenuItem::from_gaction(&redo, "Redo", None));
        menu.append(&separator());
        menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::Cut));
        menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::Copy));
        menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::Paste));
        menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::SelectAll));
    } else {
        if hit.context_is_selection() {
            menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::Copy));
        }
        menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::SelectAll));
    }
    let devtools = WebViewExt::settings(view).map(|s| s.enables_developer_extras()).unwrap_or(false);
    if devtools {
        menu.append(&separator());
        menu.append(&ContextMenuItem::from_stock_action(ContextMenuAction::InspectElement));
    }
    let _ = main;
}

pub fn handle_signal(app: &mut App, signal: ViewSignal) {
    match signal {
        ViewSignal::LoadStarted => app.protected_api.unbind_all(),
        ViewSignal::LoadCommitted(url) => {
            // Electron "dom-ready" equivalent: the page of the main frame changed
            let status = app.ytm_view_manager.status();
            if is_ytm_url(&url) {
                app.ytm_view_manager.hooks_ready = false;
                app.ytm_view_manager.hook_error = None;
                if status != YTMViewStatus::Hooking {
                    set_status(app, YTMViewStatus::Hooking);
                }
            } else {
                app.ytm_view_manager.hooks_ready = true;
                app.ytm_view_manager.setup_completion_flags = setup_flags::LOCATION_NOT_APPLICABLE;
                set_status(app, YTMViewStatus::Ready);
            }
        }
        ViewSignal::LoadFailed(code, description) => {
            app.ytm_view_manager.load_error = Some((code, description));
            emit!(app, app.ytm_view_manager.load_errored, &());
            set_status(app, YTMViewStatus::LoadFailed);
        }
        ViewSignal::Crashed => {
            log::debug!("YTM view web process gone, reloading");
            // autoRecreate: reload the view in place (WebKit starts a new web process)
            app.protected_api.unbind_all();
            app.ytm_view_manager.hooks_ready = false;
            app.ytm_view_manager.hook_error = None;
            reload(app);
            set_status(app, YTMViewStatus::Loading);
            emit!(app, app.ytm_view_manager.view_recreated, &());
        }
        ViewSignal::Unresponsive => {
            crate::services::memory_store::set(app, "ytmViewUnresponsive", true);
            emit!(app, app.ytm_view_manager.unresponsive, &());
        }
        ViewSignal::Responsive => {
            crate::services::memory_store::set(app, "ytmViewUnresponsive", false);
            emit!(app, app.ytm_view_manager.responsive, &());
        }
        ViewSignal::EnterFullscreen => crate::ui::main_window::set_fullscreen(app, true),
        ViewSignal::LeaveFullscreen => crate::ui::main_window::set_fullscreen(app, false),
        ViewSignal::Navigated(url) => {
            if is_ytm_url(&url) {
                crate::services::state_manager::update_state(app, vec![("lastUrl", Value::from(url))]);
            }
            emit!(app, app.ytm_view_manager.navigated, &());
        }
    }
}

fn register_ipc(app: &mut App) {
    app.ipc.on("ytmView:ready", |app, args| {
        log::info!("YTMView hooks ready (setup flags {:?})", args.first());
        app.ytm_view_manager.hooks_ready = true;
        app.ytm_view_manager.setup_completion_flags = args.first().and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        set_status(app, YTMViewStatus::Ready);
        continue_where_left_off(app);
    });
    app.ipc.on("ytmView:errored", |app, args| {
        log::warn!("YTMView hook error: {:?}", args.first());
        app.ytm_view_manager.hook_error = Some(args.first().cloned().unwrap_or(Value::Null));
    });
    app.ipc.on("ytmView:timedLyrics", |app, args| {
        let lyrics = args.first().cloned().unwrap_or(Value::Null);
        emit!(app, app.ytm_view_manager.timed_lyrics, &lyrics);
    });
}

fn continue_where_left_off(app: &mut App) {
    if !app.config_store.get_bool("playback.continueWhereYouLeftOff") {
        return;
    }
    // The last page the user was on is already a page where it will be playing a song from
    let uri = app.ytm_view_manager.view().and_then(|v| v.uri()).map(|u| u.to_string()).unwrap_or_default();
    if url::Url::parse(&uri).map(|u| u.path().starts_with("/watch")).unwrap_or(false) {
        return;
    }
    let video_id = app.config_store.get_str("state.lastVideoId").filter(|s| !s.is_empty());
    if let Some(video_id) = video_id {
        let playlist_id = app.config_store.get("state.lastPlaylistId");
        let endpoint = json!({ "watchEndpoint": { "videoId": video_id, "playlistId": playlist_id } });
        crate::services::protected_api::remote_execute(app, "navigate", vec![endpoint]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn navigation_policy() {
        assert!(!is_prevented_nav_or_redirect(&u("https://music.youtube.com/watch?v=x")));
        assert!(!is_prevented_nav_or_redirect(&u("https://accounts.google.com/ServiceLogin")));
        assert!(!is_prevented_nav_or_redirect(&u("https://accounts.google.co.uk/x")));
        assert!(!is_prevented_nav_or_redirect(&u("https://www.youtube.com/signin")));
        assert!(is_prevented_nav_or_redirect(&u("https://www.youtube.com/watch?v=x")));
        assert!(is_prevented_nav_or_redirect(&u("https://example.com/")));
        assert!(is_premium_redirect(&u("https://youtube.com/musicpremium")));
    }

    #[test]
    fn external_policy() {
        assert!(external_allowed("https://www.youtube.com/watch?v=1"));
        assert!(external_allowed("https://lh3.googleusercontent.com/a.jpg"));
        assert!(!external_allowed("https://evil.example.com/"));
        assert!(!external_allowed("not a url"));
    }
}
