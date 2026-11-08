//! Port of YTMViewManager

use serde_json::{json, Value};

use crate::app::{App, MainHandle};
use crate::emit;
use crate::events::Emitter;
use crate::services::window_manager::{self, NavPolicy, NewWindowPolicy, ViewOptions};
use crate::shared::types::{setup_flags, YTMViewStatus};

pub const VIEW: &str = "YTM";
pub const TITLEBAR_HEIGHT: f64 = 36.0;

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
        let _ = open::that_detached(url_string);
    } else {
        log::warn!("Attempted to open {url_string} from YTMView but was denied");
    }
}

/// Navigation handler for platforms where wry only reports main frame navigations
#[allow(dead_code)]
pub fn navigation_allowed(main: &MainHandle, url_string: &str) -> bool {
    let Ok(url) = url::Url::parse(url_string) else { return true };
    if url.scheme() != "http" && url.scheme() != "https" {
        return true;
    }
    if is_premium_redirect(&url) {
        return true;
    }
    if is_prevented_nav_or_redirect(&url) {
        log::info!("Blocking YTM View navigation to {url_string}");
        let u = url_string.to_string();
        main.run(move |_| open_external_from_ytm_view(&u));
        return false;
    }
    true
}

#[derive(Default)]
pub struct YtmViewManager {
    created: bool,
    hooks_ready: bool,
    hook_error: Option<Value>,
    setup_completion_flags: u32,
    initialized: bool,
    status: Option<YTMViewStatus>,
    pub load_error: Option<(i64, String)>,
    ready_callbacks: Vec<Box<dyn FnOnce(&mut App)>>,
    pub status_changed: Emitter<()>,
    pub view_recreated: Emitter<()>,
    pub unresponsive: Emitter<()>,
    pub responsive: Emitter<()>,
    pub load_errored: Emitter<()>,
}

impl YtmViewManager {
    pub fn status(&self) -> YTMViewStatus {
        self.status.unwrap_or(YTMViewStatus::Loading)
    }
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    pub fn has_view(&self) -> bool {
        self.created
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
    pub fn hooks_ready(&self) -> bool {
        self.hooks_ready
    }
}

pub fn on_initialized(app: &mut App) {
    if app.ytm_view_manager.initialized {
        panic!("YTMViewManager is already initialized!");
    }
    app.ytm_view_manager.initialized = true;
    log::info!("YTMViewManager initialized");
}

pub fn on_post_initialized(app: &mut App) {
    app.config_store.on_did_change("appearance", |app, new, _old| {
        if let Some(zoom) = new.get("zoom").and_then(|v| v.as_f64()) {
            if let Some(v) = app.window_manager.view(VIEW) {
                let _ = v.webview.webview.zoom(zoom / 100.0);
            }
        }
        let persist = new.get("alwaysShowVolumeSlider").and_then(|v| v.as_bool()).unwrap_or(false);
        app.window_manager.send_view(VIEW, "ytmView:alwaysShowVolumeSlider", &[Value::from(persist)]);
    });
    // Permission handling: WebKit/WebView2 handle fullscreen natively, other permissions are denied by default
}

/// Wait until the YTM view hooks are ready (`ytmViewManager.ready()`)
pub fn when_ready(app: &mut App, f: impl FnOnce(&mut App) + 'static) {
    if app.ytm_view_manager.created && app.ytm_view_manager.hooks_ready {
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

fn set_load_error(app: &mut App, code: i64, description: String) {
    app.ytm_view_manager.load_error = Some((code, description));
    emit!(app, app.ytm_view_manager.load_errored, &());
}

/// Evaluate a function expression in the YTM view (`ytmView:executeScript`)
pub fn execute_script(app: &App, script: &str) {
    app.window_manager.eval_view(VIEW, &format!("(\n{script}\n)()"));
}

pub fn create_view(app: &mut App) -> anyhow::Result<()> {
    set_status(app, YTMViewStatus::Loading);

    let mut url = "https://music.youtube.com/".to_string();
    if app.config_store.get_bool("playback.continueWhereYouLeftOff") {
        if let Some(last) = app.config_store.get_str("state.lastUrl") {
            if last.starts_with("https://music.youtube.com/") {
                url = last;
            }
        }
    }

    let partition = if crate::paths::is_packaged() { "ytmview" } else { "ytmview-dev" };
    let data_dir = crate::paths::get().user_data.join("Partitions").join(partition);
    let _ = std::fs::create_dir_all(&data_dir);

    let autoplay = !(app.config_store.get_bool("playback.continueWhereYouLeftOff") && app.config_store.get_bool("playback.continueWhereYouLeftOffPaused"));

    let mut init_scripts = vec![crate::assets::web_text("ytmview/preload.js")];
    if app.config_store.get_bool("integrations.enhancedMediaServiceEnabled") && !cfg!(target_os = "macos") {
        // Equivalent of disabling Chromium's MediaSessionService: avoid a second media session
        init_scripts.insert(0, crate::assets::web_text("ytmview/disable-media-session.js"));
    }
    init_scripts.push(format!(
        "if (window.top === window && location.hostname === 'music.youtube.com') {{\n{}\n}}",
        crate::assets::web_text("ytmview/scripts.js")
    ));

    window_manager::create_view(
        app,
        "Main",
        ViewOptions {
            name: VIEW.into(),
            url,
            preload: None,
            receive_broadcasts: false,
            init_scripts,
            data_dir: Some(data_dir),
            autoplay,
            offset_top: TITLEBAR_HEIGHT,
            nav: NavPolicy::Ytm,
            new_window: NewWindowPolicy::Ytm,
            obscured: false,
        },
    )?;
    app.ytm_view_manager.created = true;

    #[cfg(target_os = "linux")]
    if let Some(v) = app.window_manager.view(VIEW) {
        crate::platform::linux::attach_ytm_view_hooks(&v.webview.webview, app.main.clone());
    }

    // dom-ready
    if let Some(v) = app.window_manager.view_mut(VIEW) {
        v.ready_event.on(|app, _| on_dom_ready(app));
        register_view_ipc(&mut v.ipc);
    }
    Ok(())
}

fn on_dom_ready(app: &mut App) {
    let zoom = app.config_store.get_f64("appearance.zoom").unwrap_or(100.0);
    if let Some(v) = app.window_manager.view(VIEW) {
        let _ = v.webview.webview.zoom(zoom / 100.0);
    }
    let url = app.window_manager.view(VIEW).and_then(|v| v.webview.webview.url().ok()).unwrap_or_default();
    let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(|s| s.to_string())).unwrap_or_default();
    if host == "music.youtube.com" {
        if app.ytm_view_manager.status() != YTMViewStatus::Ready || !app.ytm_view_manager.hooks_ready {
            set_status(app, YTMViewStatus::Hooking);
        }
    } else {
        app.ytm_view_manager.hooks_ready = true;
        app.ytm_view_manager.setup_completion_flags = setup_flags::LOCATION_NOT_APPLICABLE;
        set_status(app, YTMViewStatus::Ready);
    }
}

fn register_view_ipc(ipc: &mut crate::ipc::IpcRegistry) {
    ipc.on("ytmView:ready", |app, _e, args| {
        log::info!("YTMView hooks ready (setup flags {:?})", args.first());
        app.ytm_view_manager.hooks_ready = true;
        app.ytm_view_manager.setup_completion_flags = args.first().and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        set_status(app, YTMViewStatus::Ready);
        // Data the original preload read from the config store
        let config = json!({
            "lastVideoId": app.config_store.get("state.lastVideoId"),
            "lastPlaylistId": app.config_store.get("state.lastPlaylistId"),
            "continueWhereYouLeftOff": app.config_store.get_bool("playback.continueWhereYouLeftOff"),
            "alwaysShowVolumeSlider": app.config_store.get_bool("appearance.alwaysShowVolumeSlider"),
        });
        app.window_manager.send_view(VIEW, "ytmView:config", &[config]);
    });
    ipc.on("ytmView:errored", |app, _e, args| {
        log::warn!("YTMView hook error: {:?}", args.first());
        app.ytm_view_manager.hook_error = Some(args.first().cloned().unwrap_or(Value::Null));
    });
    ipc.on("ytmView:navigated", |app, _e, args| {
        let Some(url) = args.first().and_then(|v| v.as_str()) else { return };
        if url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h == "music.youtube.com")).unwrap_or(false) {
            let url = url.to_string();
            crate::services::state_manager::update_state(app, vec![("lastUrl", Value::from(url))]);
        }
    });
    ipc.on("ytmView:contextMenu", |app, _e, args| {
        if let Some(params) = args.first() {
            let params = params.clone();
            show_context_menu(app, &params);
        }
    });
    ipc.on("ytmView:openExternal", |_app, _e, args| {
        if let Some(url) = args.first().and_then(|v| v.as_str()) {
            open_external_from_ytm_view(url);
        }
    });
}

/// Events coming from platform webview hooks
pub enum ViewSignal {
    LoadStarted,
    LoadFailed(i64, String),
    Crashed,
    Unresponsive,
    Responsive,
    EnterFullscreen,
    LeaveFullscreen,
    Navigated(String),
}

pub fn handle_signal(app: &mut App, signal: ViewSignal) {
    match signal {
        ViewSignal::LoadStarted => app.protected_api.unbind_all(),
        ViewSignal::LoadFailed(code, description) => {
            set_load_error(app, code, description);
            set_status(app, YTMViewStatus::LoadFailed);
        }
        ViewSignal::Crashed => {
            log::debug!("AppView '{VIEW}' webContents render process gone");
            // autoRecreate: reload the view in place
            app.protected_api.unbind_all();
            app.ytm_view_manager.hooks_ready = false;
            app.ytm_view_manager.hook_error = None;
            if let Some(v) = app.window_manager.view_mut(VIEW) {
                v.ready = false;
                let _ = v.webview.webview.reload();
            }
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
        ViewSignal::EnterFullscreen => window_manager::set_fullscreen_layout(app, "Main", true),
        ViewSignal::LeaveFullscreen => window_manager::set_fullscreen_layout(app, "Main", false),
        ViewSignal::Navigated(url) => {
            if url::Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h == "music.youtube.com")).unwrap_or(false) {
                crate::services::state_manager::update_state(app, vec![("lastUrl", Value::from(url))]);
            }
        }
    }
}

fn show_context_menu(app: &mut App, params: &Value) {
    use muda::{ContextMenu, Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::new();
    let count = std::cell::Cell::new(0usize);
    let add = |app: &mut App, menu: &Menu, label: &str, enabled: bool, action: Box<dyn Fn(&mut App)>| {
        let id = format!("ctx:{}", uuid::Uuid::new_v4());
        let item = MenuItem::with_id(id.clone(), label, enabled, None);
        let _ = menu.append(&item);
        app.menu_actions.insert(id, action);
        count.set(count.get() + 1);
    };
    let sep = |menu: &Menu| {
        let _ = menu.append(&PredefinedMenuItem::separator());
    };
    let get_bool = |k: &str| params.get(k).and_then(|v| v.as_bool()).unwrap_or(false);

    if let Some(link) = params.get("linkURL").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        let link = link.to_string();
        add(app, &menu, "Open Link in Browser", true, Box::new(move |_| open_external_from_ytm_view(&link)));
    }
    if get_bool("hasImageContents") {
        if let Some(src) = params.get("srcURL").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            if count.get() > 0 {
                sep(&menu);
            }
            let src = src.to_string();
            add(app, &menu, "Open Image in Browser", true, Box::new(move |_| open_external_from_ytm_view(&src)));
        }
    }
    let edit = |cmd: &'static str| -> Box<dyn Fn(&mut App)> { Box::new(move |app: &mut App| editing_command(app, cmd)) };
    if get_bool("isEditable") {
        if count.get() > 0 {
            sep(&menu);
        }
        add(app, &menu, "Undo", get_bool("canUndo"), edit("Undo"));
        add(app, &menu, "Redo", get_bool("canRedo"), edit("Redo"));
        sep(&menu);
        add(app, &menu, "Cut", get_bool("canCut"), edit("Cut"));
        add(app, &menu, "Copy", get_bool("canCopy"), edit("Copy"));
        add(app, &menu, "Paste", true, edit("Paste"));
        add(app, &menu, "Select All", true, edit("SelectAll"));
    } else {
        if params.get("selectionText").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false) {
            add(app, &menu, "Copy", true, edit("Copy"));
        }
        add(app, &menu, "Select All", true, edit("SelectAll"));
    }
    if app.config_store.get_bool("developer.enableDevTools") {
        sep(&menu);
        add(app, &menu, "Inspect", true, Box::new(|app| {
            if let Some(v) = app.window_manager.view(VIEW) {
                v.webview.webview.open_devtools();
            }
        }));
    }

    let x = params.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) + TITLEBAR_HEIGHT;
    let position: Option<dpi::Position> = Some(dpi::LogicalPosition::new(x, y).into());
    #[cfg(target_os = "linux")]
    if let Some(window) = app.window_manager.gtk_window("Main") {
        menu.show_context_menu_for_gtk_window(&window, position);
    }
    #[cfg(windows)]
    if let Some(w) = app.window_manager.get("Main") {
        use tao::platform::windows::WindowExtWindows;
        unsafe { menu.show_context_menu_for_hwnd(w.window.hwnd() as _, position) };
    }
    #[cfg(target_os = "macos")]
    if let Some(w) = app.window_manager.get("Main") {
        use tao::platform::macos::WindowExtMacOS;
        unsafe { menu.show_context_menu_for_nsview(w.window.ns_view() as _, position) };
    }
    // Keep the menu alive until an item is activated
    app.ui.context_menu = Some(menu);
}

fn editing_command(app: &mut App, command: &str) {
    let Some(v) = app.window_manager.view(VIEW) else { return };
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::WebViewExt;
        use wry::WebViewExtUnix;
        v.webview.webview.webview().execute_editing_command(command);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let js_cmd = match command {
            "SelectAll" => "selectAll",
            other => other,
        };
        let _ = v.webview.webview.evaluate_script(&format!("document.execCommand({:?})", js_cmd.to_lowercase()));
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
