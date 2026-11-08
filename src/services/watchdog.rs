//! Port of WatchDog: catches unhandled errors (panics), runs crash callbacks
//! (stops state writes) and shows the crash dialog.

use std::sync::atomic::{AtomicBool, Ordering};

static CRASHED: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);

pub fn crashed() -> bool {
    CRASHED.load(Ordering::SeqCst)
}

pub fn set_app_ready() {
    READY.store(true, Ordering::SeqCst);
}

pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        if CRASHED.swap(true, Ordering::SeqCst) {
            return;
        }
        log::info!("Unhandled error caught in application!");
        log::info!("Running crash callbacks for application cleanup");

        let message = match info.payload().downcast_ref::<&str>() {
            Some(s) => s.to_string(),
            None => info.payload().downcast_ref::<String>().cloned().unwrap_or_else(|| "Unknown".into()),
        };
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!("{message} at {location}\n{backtrace}");

        let dialog_message = format!(
            "Environment Details:\n    {}\n    {}\n\nName: panic\nMessage: {}\nCause: {}\n\n{}",
            crate::APP_VERSION,
            std::env::consts::OS,
            message,
            location,
            backtrace
        );

        if !READY.load(Ordering::SeqCst) {
            crate::dialogs::show_error_box_sync("YtmRs Crashed", &format!("Application crashed before ready\n\n{dialog_message}"));
            std::process::exit(1);
        }

        let mut buttons = vec!["Copy to Clipboard and Exit".to_string(), "Exit".to_string()];
        if !crate::paths::is_packaged() {
            buttons.push("Continue".to_string());
        }
        let result = crate::dialogs::show_message_box_sync("Error", "YtmRs Crashed", &dialog_message, crate::dialogs::Kind::Error, &buttons);
        if result == 0 {
            crate::clipboard::write_text_blocking(&format!("YtmRs Crashed\n\n{dialog_message}"));
        }
        if result == 0 || result == 1 {
            std::process::exit(1);
        }
    }));
}
