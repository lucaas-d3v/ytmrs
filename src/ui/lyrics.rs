//! Native synced lyrics window (the original rendered timed lyrics inside the YTM lyrics tab;
//! the page now only reports the lyrics data).

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::glib;
use serde_json::Value;

use crate::app::App;
use crate::services::protected_api::remote_execute;
use crate::shared::player_state::PlayerState;

pub struct LyricsWindow {
    pub window: gtk::Window,
    header: gtk::HeaderBar,
    scrolled: gtk::ScrolledWindow,
    list: gtk::Box,
    lines: Vec<(f64, f64, gtk::Label)>,
    active: Option<usize>,
    source: gtk::Label,
    empty: gtk::Label,
    sync_button: gtk::Button,
    auto_scroll: Rc<Cell<bool>>,
}

pub fn toggle(app: &mut App) {
    if let Some(l) = app.ui.lyrics.take() {
        unsafe { l.window.destroy() };
    } else {
        open(app);
    }
    super::main_window::sync_lyrics_toggle(app);
}

pub fn open(app: &mut App) {
    if let Some(l) = app.ui.lyrics.as_ref() {
        l.window.present();
        return;
    }
    let main = app.main.clone();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("YtmRs - Lyrics");
    window.set_default_size(420, 640);
    window.style_context().add_class("ytmrs-lyrics");
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    header.set_title(Some("Lyrics"));
    let sync_button = gtk::Button::with_label("Sync to video time");
    sync_button.set_no_show_all(true);
    header.pack_end(&sync_button);
    window.set_titlebar(Some(&header));

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_margin_top(24);
    list.set_margin_bottom(24);
    let empty = gtk::Label::new(Some("No lyrics available for this song"));
    empty.style_context().add_class("empty");
    empty.set_vexpand(true);
    empty.set_valign(gtk::Align::Center);
    let source = gtk::Label::new(None);
    source.style_context().add_class("source");
    source.set_xalign(0.0);
    source.set_line_wrap(true);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add(&empty);
    content.add(&list);
    content.add(&source);
    let scrolled = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.add(&content);
    window.add(&scrolled);

    // Scrolling by hand pauses the automatic scrolling (original "return to live" behavior)
    let auto_scroll = Rc::new(Cell::new(true));
    let (a, b) = (auto_scroll.clone(), sync_button.clone());
    scrolled.connect_scroll_event(move |_, _| {
        a.set(false);
        b.show();
        glib::Propagation::Proceed
    });
    let (a, m) = (auto_scroll.clone(), main.clone());
    sync_button.connect_clicked(move |b| {
        a.set(true);
        b.hide();
        m.run(|app| {
            if let Some(l) = app.ui.lyrics.as_mut() {
                l.active = None;
            }
            let state = app.player_state_store.get_state();
            update_progress(app, &state);
        });
    });
    window.connect_delete_event(move |_, _| {
        main.run(toggle);
        glib::Propagation::Stop
    });
    window.show_all();
    app.ui.lyrics = Some(LyricsWindow { window, header, scrolled, list, lines: Vec::new(), active: None, source, empty, sync_button, auto_scroll });
    render(app);
}

/// New lyrics data from the YTM page (`ytmView:timedLyrics`)
pub fn set_lyrics(app: &mut App, lyrics: Value) {
    app.ui.timed_lyrics = lyrics;
    render(app);
}

fn render(app: &mut App) {
    let lyrics = app.ui.timed_lyrics.clone();
    let main = app.main.clone();
    let title = app.player_state_store.state().video_details.as_ref().map(|d| format!("{} • {}", d.title(), d.author()));
    let Some(l) = app.ui.lyrics.as_mut() else { return };
    l.header.set_subtitle(title.as_deref());
    for child in l.list.children() {
        l.list.remove(&child);
    }
    l.lines.clear();
    l.active = None;
    let lines = lyrics.get("lines").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for line in lines {
        let text = line.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let start = line.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let end = line.get("end").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_line_wrap(true);
        label.style_context().add_class("line");
        // Clicking a line seeks to it
        let event_box = gtk::EventBox::new();
        event_box.add(&label);
        let (m, a, b) = (main.clone(), l.auto_scroll.clone(), l.sync_button.clone());
        event_box.connect_button_press_event(move |_, _| {
            // YouTube does not always send timings: untimed lines are plain text
            if end <= start {
                return glib::Propagation::Proceed;
            }
            a.set(true);
            b.hide();
            m.run(move |app| remote_execute(app, "seekTo", vec![Value::from(start / 1000.0)]));
            glib::Propagation::Stop
        });
        event_box.show_all();
        l.list.add(&event_box);
        l.lines.push((start, end, label));
    }
    let has_lines = !l.lines.is_empty();
    l.empty.set_visible(!has_lines);
    let source = lyrics.get("source").and_then(|v| v.as_str()).unwrap_or("");
    l.source.set_text(source);
    l.source.set_visible(has_lines && !source.is_empty());
    let state = app.player_state_store.get_state();
    update_progress(app, &state);
}

pub fn update_progress(app: &mut App, state: &PlayerState) {
    let Some(l) = app.ui.lyrics.as_mut() else { return };
    let ms = state.video_progress * 1000.0;
    let active = l.lines.iter().position(|(start, end, _)| ms >= *start && ms < *end);
    if active == l.active {
        return;
    }
    if let Some((_, _, label)) = l.active.and_then(|i| l.lines.get(i)) {
        label.style_context().remove_class("active");
    }
    l.active = active;
    let Some((_, _, label)) = active.and_then(|i| l.lines.get(i)) else { return };
    label.style_context().add_class("active");
    if l.auto_scroll.get() {
        scroll_to_center(&l.scrolled, label);
    }
}

/// Smoothly scrolls so that `widget` is centered (250ms ease out)
fn scroll_to_center(scrolled: &gtk::ScrolledWindow, widget: &gtk::Label) {
    let adjustment = scrolled.vadjustment();
    // label -> event box (row) -> list -> content
    let Some(row) = widget.parent() else { return };
    let Some(content) = row.parent().and_then(|list| list.parent()) else { return };
    let Some((_, y)) = row.translate_coordinates(&content, 0, 0) else { return };
    let height = row.allocated_height() as f64;
    let target = (y as f64 + height / 2.0 - adjustment.page_size() / 2.0).clamp(adjustment.lower(), (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()));
    let start = adjustment.value();
    let started = Rc::new(Cell::new(None::<i64>));
    scrolled.add_tick_callback(move |_, clock| {
        let now = clock.frame_time();
        let t0 = started.get().unwrap_or_else(|| {
            started.set(Some(now));
            now
        });
        let t = ((now - t0) as f64 / 250_000.0).min(1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        adjustment.set_value(start + (target - start) * eased);
        if t < 1.0 {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}
