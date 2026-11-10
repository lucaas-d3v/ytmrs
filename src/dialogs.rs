//! Native GTK dialogs (Electron `dialog`)

use std::path::PathBuf;

use gtk::prelude::*;

use crate::app::App;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Info,
    Warning,
    Error,
    Question,
}

impl Kind {
    fn message_type(self) -> gtk::MessageType {
        match self {
            Kind::Info => gtk::MessageType::Info,
            Kind::Warning => gtk::MessageType::Warning,
            Kind::Error => gtk::MessageType::Error,
            Kind::Question => gtk::MessageType::Question,
        }
    }
}

pub struct MessageBox {
    pub title: String,
    pub message: String,
    pub detail: String,
    pub kind: Kind,
    pub buttons: Vec<String>,
    pub cancel_id: usize,
}

impl MessageBox {
    pub fn new(kind: Kind, title: &str, message: &str, detail: &str, buttons: &[&str]) -> Self {
        let buttons: Vec<String> = if buttons.is_empty() { vec!["OK".into()] } else { buttons.iter().map(|s| s.to_string()).collect() };
        let cancel_id = buttons.len().saturating_sub(1);
        Self { title: title.into(), message: message.into(), detail: detail.into(), kind, buttons, cancel_id }
    }
}

fn dialog(parent: Option<&gtk::Window>, mb: &MessageBox) -> gtk::MessageDialog {
    let flags = if parent.is_some() { gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT } else { gtk::DialogFlags::empty() };
    let dialog = gtk::MessageDialog::new(parent, flags, mb.kind.message_type(), gtk::ButtonsType::None, &mb.message);
    dialog.set_title(&mb.title);
    if !mb.detail.is_empty() {
        dialog.set_secondary_text(Some(&mb.detail));
    }
    for (i, b) in mb.buttons.iter().enumerate() {
        dialog.add_button(b, gtk::ResponseType::Other(i as u16));
    }
    dialog
}

/// Async message box (`dialog.showMessageBox`), the button index is delivered on the main loop
pub fn show_message_box(app: &mut App, parent: Option<&gtk::Window>, mb: MessageBox, on_result: impl FnOnce(&mut App, usize) + 'static) {
    let pending = app.register_pending(move |app, v| on_result(app, v.as_u64().unwrap_or(0) as usize));
    let main = app.main.clone();
    let dialog = dialog(parent, &mb);
    let cancel = mb.cancel_id;
    dialog.connect_response(move |d, resp| {
        let idx = match resp {
            gtk::ResponseType::Other(i) => i as usize,
            _ => cancel,
        };
        unsafe { d.destroy() };
        main.run(move |app| app.resolve_pending(pending, serde_json::Value::from(idx)));
    });
    dialog.show_all();
}

/// Blocking message box (`dialog.showMessageBoxSync`); returns the button index
pub fn show_message_box_sync(title: &str, message: &str, detail: &str, kind: Kind, buttons: &[String]) -> usize {
    if !gtk::is_initialized_main_thread() {
        eprintln!("{title}: {message}\n{detail}");
        return buttons.len().saturating_sub(1);
    }
    let labels: Vec<&str> = buttons.iter().map(|s| s.as_str()).collect();
    let mb = MessageBox::new(kind, title, message, detail, &labels);
    let dialog = dialog(None, &mb);
    let resp = dialog.run();
    unsafe { dialog.destroy() };
    match resp {
        gtk::ResponseType::Other(i) => i as usize,
        _ => mb.cancel_id,
    }
}

pub fn show_error_box_sync(title: &str, content: &str) {
    show_message_box_sync(title, title, content, Kind::Error, &["OK".to_string()]);
}

/// File picker for the custom CSS setting (portal aware)
pub fn pick_css_file(app: &mut App, parent: Option<&gtk::Window>, on_result: impl FnOnce(&mut App, Option<PathBuf>) + 'static) {
    let pending = app.register_pending(move |app, v| on_result(app, v.as_str().map(PathBuf::from)));
    let main = app.main.clone();
    let dialog = gtk::FileChooserNative::new(Some("Select CSS file"), parent, gtk::FileChooserAction::Open, Some("Open"), Some("Cancel"));
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("CSS"));
    filter.add_pattern("*.css");
    dialog.add_filter(filter);
    // The native dialog must stay referenced until it responds
    let keep = std::rc::Rc::new(std::cell::RefCell::new(Some(dialog.clone())));
    dialog.connect_response(move |d, resp| {
        let path = if resp == gtk::ResponseType::Accept { d.file().and_then(|f| f.path()) } else { None };
        let value = path.map(|p| serde_json::Value::from(p.to_string_lossy().to_string())).unwrap_or(serde_json::Value::Null);
        main.run(move |app| app.resolve_pending(pending, value));
        keep.borrow_mut().take();
    });
    dialog.show();
}
