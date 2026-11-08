//! Electron `clipboard.writeText`

/// Main thread clipboard write (content stays available while the app runs)
pub fn write_text(text: &str) {
    #[cfg(target_os = "linux")]
    {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text(text);
        clipboard.store();
    }
    #[cfg(not(target_os = "linux"))]
    {
        if let Ok(mut c) = arboard::Clipboard::new() {
            let _ = c.set_text(text.to_string());
        }
    }
}

/// Used right before the process exits
pub fn write_text_blocking(text: &str) {
    #[cfg(target_os = "linux")]
    {
        if gtk::is_initialized_main_thread() {
            write_text(text);
        }
    }
    #[cfg(not(target_os = "linux"))]
    write_text(text);
}
