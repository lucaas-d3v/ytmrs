//! Native dialogs (Electron `dialog`). GTK on Linux (on the GUI thread, as tao owns
//! GTK there), rfd on Windows/macOS.

use std::path::PathBuf;

use crate::app::App;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Info,
    Warning,
    Error,
    Question,
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

/// Async message box (`dialog.showMessageBox`), result delivered on the main thread
pub fn show_message_box(app: &mut App, parent: Option<&str>, mb: MessageBox, on_result: impl FnOnce(&mut App, usize) + 'static) {
    let pending = app.register_pending(move |app, v| on_result(app, v.as_u64().unwrap_or(0) as usize));
    let main = app.main.clone();
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        let parent_window = parent.and_then(|p| app.window_manager.gtk_window(p));
        let msg_type = match mb.kind {
            Kind::Info => gtk::MessageType::Info,
            Kind::Warning => gtk::MessageType::Warning,
            Kind::Error => gtk::MessageType::Error,
            Kind::Question => gtk::MessageType::Question,
        };
        let flags = if parent_window.is_some() { gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT } else { gtk::DialogFlags::empty() };
        let dialog = gtk::MessageDialog::new(parent_window.as_ref(), flags, msg_type, gtk::ButtonsType::None, &mb.message);
        dialog.set_title(&mb.title);
        if !mb.detail.is_empty() {
            dialog.set_secondary_text(Some(&mb.detail));
        }
        for (i, b) in mb.buttons.iter().enumerate() {
            dialog.add_button(b, gtk::ResponseType::Other(i as u16));
        }
        let cancel = mb.cancel_id;
        dialog.connect_response(move |d, resp| {
            let idx = match resp {
                gtk::ResponseType::Other(i) => i as usize,
                _ => cancel,
            };
            d.close();
            main.run(move |app| app.resolve_pending(pending, serde_json::Value::from(idx)));
        });
        dialog.show_all();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = parent;
        app.rt.spawn_blocking(move || {
            let idx = show_message_box_sync(&mb.title, &mb.message, &mb.detail, mb.kind, &mb.buttons);
            main.run(move |app| app.resolve_pending(pending, serde_json::Value::from(idx)));
        });
    }
}

/// Blocking message box (`dialog.showMessageBoxSync`); returns the button index.
pub fn show_message_box_sync(title: &str, message: &str, detail: &str, kind: Kind, buttons: &[String]) -> usize {
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        if !gtk::is_initialized_main_thread() {
            eprintln!("{title}: {message}\n{detail}");
            return buttons.len().saturating_sub(1);
        }
        let msg_type = match kind {
            Kind::Info => gtk::MessageType::Info,
            Kind::Warning => gtk::MessageType::Warning,
            Kind::Error => gtk::MessageType::Error,
            Kind::Question => gtk::MessageType::Question,
        };
        let dialog = gtk::MessageDialog::new(None::<&gtk::Window>, gtk::DialogFlags::MODAL, msg_type, gtk::ButtonsType::None, message);
        dialog.set_title(title);
        dialog.set_secondary_text(Some(detail));
        for (i, b) in buttons.iter().enumerate() {
            dialog.add_button(b, gtk::ResponseType::Other(i as u16));
        }
        let resp = dialog.run();
        dialog.close();
        match resp {
            gtk::ResponseType::Other(i) => i as usize,
            _ => buttons.len().saturating_sub(1),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let level = match kind {
            Kind::Info | Kind::Question => rfd::MessageLevel::Info,
            Kind::Warning => rfd::MessageLevel::Warning,
            Kind::Error => rfd::MessageLevel::Error,
        };
        let (btns, map): (rfd::MessageButtons, Vec<String>) = match buttons.len() {
            0 | 1 => (rfd::MessageButtons::OkCustom(buttons.first().cloned().unwrap_or("OK".into())), buttons.to_vec()),
            2 => (rfd::MessageButtons::OkCancelCustom(buttons[0].clone(), buttons[1].clone()), buttons.to_vec()),
            _ => (rfd::MessageButtons::YesNoCancelCustom(buttons[0].clone(), buttons[1].clone(), buttons[2].clone()), buttons[..3].to_vec()),
        };
        let result = rfd::MessageDialog::new().set_level(level).set_title(title).set_description(format!("{message}\n\n{detail}")).set_buttons(btns).show();
        match result {
            rfd::MessageDialogResult::Custom(label) => map.iter().position(|b| *b == label).unwrap_or(map.len().saturating_sub(1)),
            rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes => 0,
            rfd::MessageDialogResult::No => 1,
            _ => map.len().saturating_sub(1),
        }
    }
}

pub fn show_error_box_sync(title: &str, content: &str) {
    show_message_box_sync(title, title, content, Kind::Error, &["OK".to_string()]);
}

/// File picker for the custom CSS setting
pub fn pick_css_file(app: &mut App, parent: Option<&str>, on_result: impl FnOnce(&mut App, Option<PathBuf>) + 'static) {
    let pending = app.register_pending(move |app, v| on_result(app, v.as_str().map(PathBuf::from)));
    let main = app.main.clone();
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        let parent_window = parent.and_then(|p| app.window_manager.gtk_window(p));
        let dialog = gtk::FileChooserNative::new(Some("Select CSS file"), parent_window.as_ref(), gtk::FileChooserAction::Open, Some("Open"), Some("Cancel"));
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("CSS"));
        filter.add_pattern("*.css");
        dialog.add_filter(filter);
        dialog.connect_response(move |d, resp| {
            let path = if resp == gtk::ResponseType::Accept { d.file().and_then(|f| f.path()) } else { None };
            let value = path.map(|p| serde_json::Value::from(p.to_string_lossy().to_string())).unwrap_or(serde_json::Value::Null);
            main.run(move |app| app.resolve_pending(pending, value));
            d.destroy();
        });
        dialog.show();
        // keep the native dialog alive until it responds
        std::mem::forget(dialog);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = parent;
        app.rt.spawn(async move {
            let file = rfd::AsyncFileDialog::new().add_filter("CSS", &["css"]).pick_file().await;
            let value = file.map(|f| serde_json::Value::from(f.path().to_string_lossy().to_string())).unwrap_or(serde_json::Value::Null);
            main.run(move |app| app.resolve_pending(pending, value));
        });
    }
}
