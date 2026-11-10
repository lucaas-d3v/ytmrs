//! Native companion authorization request (port of windows/authorize-companion)

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::app::App;

/// Shows the request; `result` receives true (allow) or false (deny / closed)
pub fn open(app: &mut App, name: &str, app_name: &str, code: &str, result: tokio::sync::oneshot::Sender<bool>) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("YtmRs - Companion Authorization");
    window.set_default_size(560, 360);
    window.set_resizable(false);
    window.set_keep_above(true);
    window.set_position(gtk::WindowPosition::Center);
    window.set_type_hint(gtk::gdk::WindowTypeHint::Dialog);
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    header.set_title(Some("Companion Authorization"));
    window.set_titlebar(Some(&header));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(32);
    content.set_margin_bottom(32);
    content.set_margin_start(32);
    content.set_margin_end(32);
    content.set_valign(gtk::Align::Center);
    let markup = |m: &str| {
        let l = gtk::Label::new(None);
        l.set_markup(m);
        l.set_line_wrap(true);
        l.set_justify(gtk::Justification::Center);
        l
    };
    let app_name = glib::markup_escape_text(app_name);
    content.add(&markup("<span size='x-large' weight='bold'>Companion Authorization Request</span>"));
    content.add(&markup(&format!("<b>{app_name}</b> would like to control YtmRs")));
    content.add(&markup(&format!("Please ensure the code below matches what <b>{app_name}</b> is showing")));
    content.add(&markup(&format!("<span size='xx-large' weight='bold' font_family='monospace'>{}</span>", glib::markup_escape_text(code))));
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    let deny = gtk::Button::with_label("Deny");
    deny.style_context().add_class("destructive-action");
    let allow = gtk::Button::with_label("Allow");
    allow.style_context().add_class("suggested-action");
    buttons.add(&deny);
    buttons.add(&allow);
    content.add(&buttons);
    window.add(&content);

    let sender = Rc::new(RefCell::new(Some(result)));
    let respond = move |w: &gtk::Window, allowed: bool| {
        if let Some(tx) = sender.borrow_mut().take() {
            let _ = tx.send(allowed);
        }
        w.hide();
    };
    let r = respond.clone();
    let w = window.clone();
    allow.connect_clicked(move |_| r(&w, true));
    let r = respond.clone();
    let w = window.clone();
    deny.connect_clicked(move |_| r(&w, false));
    window.connect_delete_event(move |w, _| {
        respond(w, false);
        glib::Propagation::Stop
    });

    window.show_all();
    window.present();
    window.set_urgency_hint(true);
    app.ui.auth_windows.insert(name.to_string(), window);
}

/// The request finished (answered or timed out)
pub fn close(app: &mut App, name: &str) {
    if let Some(window) = app.ui.auth_windows.remove(name) {
        unsafe { window.destroy() };
    }
}
