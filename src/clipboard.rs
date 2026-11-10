//! Electron `clipboard.writeText`

/// Main thread clipboard write (content stays available while the app runs)
pub fn write_text(text: &str) {
    if !gtk::is_initialized_main_thread() {
        return;
    }
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.set_text(text);
    clipboard.store();
}
