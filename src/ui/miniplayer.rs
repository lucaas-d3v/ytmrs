//! Native miniplayer (port of windows/miniplayer)

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk::prelude::GdkContextExt;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::glib;
use serde_json::Value;

use super::{icon, icon_button};
use crate::app::App;
use crate::services::protected_api::remote_execute;
use crate::services::state_manager;
use crate::shared::player_state::{highest_res_thumbnail_reduce, PlayerState, RepeatMode, VideoState};

pub struct Miniplayer {
    pub window: gtk::Window,
    art: gtk::DrawingArea,
    artwork: Rc<RefCell<Option<Pixbuf>>>,
    artwork_url: Option<String>,
    player: gtk::Box,
    unavailable: gtk::Label,
    title: gtk::Label,
    time: gtk::Label,
    seek: gtk::Scale,
    seeking: Rc<Cell<bool>>,
    play_pause: gtk::Button,
    volume: gtk::Scale,
    volume_dragging: Rc<Cell<bool>>,
    mute: gtk::Button,
    repeat: gtk::Button,
    muted: bool,
}

fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
}

fn remote(main: &crate::app::MainHandle, command: &'static str) -> impl Fn(&gtk::Button) + 'static {
    let main = main.clone();
    move |_| main.run(move |app| remote_execute(app, command, vec![]))
}

pub fn is_open(app: &App) -> bool {
    app.ui.miniplayer.is_some()
}

pub fn open(app: &mut App) {
    if let Some(m) = app.ui.miniplayer.as_ref() {
        m.window.present();
        return;
    }
    let main = app.main.clone();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("YtmRs - Miniplayer");
    window.set_size_request(300, 240);
    window.style_context().add_class("ytmrs-miniplayer");
    super::restore_bounds(&window, &app.config_store.get("state.miniplayerWindowBounds"), (360, 360));
    // On X11 the miniplayer is kept above other windows (the original pins it by default)
    let pinned = !is_wayland();
    window.set_keep_above(pinned);

    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    // No title in the header bar: the track is shown over the artwork
    header.set_custom_title(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let pin = gtk::ToggleButton::new();
    pin.set_image(Some(&icon(&["view-pin-symbolic", "pin-symbolic", "window-pin-symbolic", "go-top-symbolic"])));
    pin.set_active(pinned);
    if is_wayland() {
        pin.set_sensitive(false);
        pin.set_tooltip_text(Some("Use your native window manager to pin the miniplayer"));
    } else {
        pin.set_tooltip_text(Some("Pin/Unpin Miniplayer"));
    }
    let w = window.clone();
    pin.connect_toggled(move |b| w.set_keep_above(b.is_active()));
    header.pack_end(&pin);
    window.set_titlebar(Some(&header));

    //#region Album art background
    let artwork: Rc<RefCell<Option<Pixbuf>>> = Rc::default();
    let art = gtk::DrawingArea::new();
    let a = artwork.clone();
    art.connect_draw(move |area, cr| {
        let (w, h) = (area.allocated_width() as f64, area.allocated_height() as f64);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.paint();
        if let Some(pixbuf) = a.borrow().as_ref() {
            let (iw, ih) = (pixbuf.width() as f64, pixbuf.height() as f64);
            // background-size: cover
            let scale = (w / iw).max(h / ih);
            let _ = cr.save();
            cr.translate((w - iw * scale) / 2.0, (h - ih * scale) / 2.0);
            cr.scale(scale, scale);
            cr.set_source_pixbuf(pixbuf, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();
        }
        glib::Propagation::Proceed
    });

    //#region Controls
    let player = gtk::Box::new(gtk::Orientation::Vertical, 4);
    player.set_valign(gtk::Align::End);
    player.style_context().add_class("controls");
    player.set_margin_start(0);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 2);
    inner.set_margin_start(12);
    inner.set_margin_end(12);
    inner.set_margin_top(24);
    inner.set_margin_bottom(8);
    let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(None);
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.style_context().add_class("title");
    title_row.add(&title);
    let time = gtk::Label::new(None);
    time.style_context().add_class("message");
    title_row.add(&time);
    inner.add(&title_row);

    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
    seek.set_draw_value(false);
    let seeking = Rc::new(Cell::new(false));
    let s = seeking.clone();
    seek.connect_button_press_event(move |_, _| {
        s.set(true);
        glib::Propagation::Proceed
    });
    let (s, m) = (seeking.clone(), main.clone());
    seek.connect_button_release_event(move |scale, _| {
        s.set(false);
        let value = scale.value();
        m.run(move |app| remote_execute(app, "seekTo", vec![Value::from(value)]));
        glib::Propagation::Proceed
    });
    inner.add(&seek);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let repeat = icon_button(&["media-playlist-repeat-symbolic"], "Repeat");
    repeat.set_relief(gtk::ReliefStyle::None);
    repeat.connect_clicked(remote(&main, "cycleRepeatMode"));
    row.add(&repeat);
    let left_pad = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    left_pad.set_hexpand(true);
    row.add(&left_pad);
    let previous = icon_button(&["media-skip-backward-symbolic"], "Previous");
    previous.connect_clicked(remote(&main, "previous"));
    previous.set_relief(gtk::ReliefStyle::None);
    let play_pause = icon_button(&["media-playback-start-symbolic"], "Play/Pause");
    play_pause.connect_clicked(remote(&main, "playPause"));
    play_pause.set_relief(gtk::ReliefStyle::None);
    let next = icon_button(&["media-skip-forward-symbolic"], "Next");
    next.connect_clicked(remote(&main, "next"));
    next.set_relief(gtk::ReliefStyle::None);
    row.add(&previous);
    row.add(&play_pause);
    row.add(&next);
    let right_pad = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    right_pad.set_hexpand(true);
    row.add(&right_pad);

    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    volume.set_draw_value(false);
    volume.set_size_request(70, -1);
    let volume_dragging = Rc::new(Cell::new(false));
    let d = volume_dragging.clone();
    volume.connect_button_press_event(move |_, _| {
        d.set(true);
        glib::Propagation::Proceed
    });
    let d = volume_dragging.clone();
    volume.connect_button_release_event(move |_, _| {
        d.set(false);
        glib::Propagation::Proceed
    });
    let m = main.clone();
    volume.connect_change_value(move |_, _, value| {
        let v = value.clamp(0.0, 100.0).round();
        m.run(move |app| remote_execute(app, "setVolume", vec![Value::from(v as i64)]));
        glib::Propagation::Proceed
    });
    let mute = icon_button(&["audio-volume-high-symbolic"], "Mute");
    mute.set_relief(gtk::ReliefStyle::None);
    let m = main.clone();
    mute.connect_clicked(move |_| {
        m.run(|app| {
            let muted = app.ui.miniplayer.as_ref().map(|p| p.muted).unwrap_or(false);
            remote_execute(app, if muted { "unmute" } else { "mute" }, vec![]);
        })
    });
    row.add(&mute);
    row.add(&volume);
    inner.add(&row);
    player.add(&inner);

    let unavailable = gtk::Label::new(Some("Video is still loading or no video loaded"));
    unavailable.style_context().add_class("message");
    unavailable.set_line_wrap(true);
    unavailable.set_justify(gtk::Justification::Center);

    let overlay = gtk::Overlay::new();
    overlay.add(&art);
    overlay.add_overlay(&player);
    overlay.add_overlay(&unavailable);
    // Dragging the artwork moves the window
    let drag = gtk::EventBox::new();
    drag.set_above_child(false);
    drag.add(&overlay);
    drag.connect_button_press_event(|widget, event| {
        if event.button() == 1 {
            if let Some(window) = widget.toplevel().and_then(|t| t.downcast::<gtk::Window>().ok()) {
                let (x, y) = event.root();
                window.begin_move_drag(1, x as i32, y as i32, event.time());
            }
        }
        glib::Propagation::Proceed
    });
    window.add(&drag);

    let m = main.clone();
    window.connect_configure_event(move |_, _| {
        m.run(save_bounds);
        false
    });
    let m = main.clone();
    window.connect_delete_event(move |_, _| {
        m.run(close);
        glib::Propagation::Stop
    });

    window.show_all();
    app.ui.miniplayer = Some(Miniplayer {
        window,
        art,
        artwork,
        artwork_url: None,
        player,
        unavailable,
        title,
        time,
        seek,
        seeking,
        play_pause,
        volume,
        volume_dragging,
        mute,
        repeat,
        muted: false,
    });
    let state = app.player_state_store.get_state();
    update(app, &state);
    super::main_window::hide(app);
}

/// Closing the miniplayer shows the main window again
pub fn close(app: &mut App) {
    save_bounds(app);
    if let Some(m) = app.ui.miniplayer.take() {
        unsafe { m.window.destroy() };
    }
    super::main_window::show_and_focus(app);
}

fn save_bounds(app: &mut App) {
    let Some(m) = app.ui.miniplayer.as_ref() else { return };
    if !m.window.is_visible() {
        return;
    }
    let bounds = super::window_bounds(&m.window);
    state_manager::update_state(app, vec![("miniplayerWindowBounds", bounds)]);
}

pub fn update(app: &mut App, state: &PlayerState) {
    let Some(m) = app.ui.miniplayer.as_mut() else { return };
    let Some(details) = state.video_details.as_ref() else {
        m.player.hide();
        m.unavailable.show();
        return;
    };
    m.unavailable.hide();
    m.player.show();
    m.window.set_title(&format!("{} - {} | YtmRs - Miniplayer", details.title(), details.author()));
    let text = format!("{} • {}", details.title(), details.author());
    if m.title.text() != text {
        m.title.set_text(&text);
        m.title.set_tooltip_text(Some(&text));
    }
    let duration = details.duration_seconds.max(1.0);
    m.seek.set_range(0.0, duration);
    if !m.seeking.get() {
        m.seek.set_value(state.video_progress);
    }
    m.time.set_text(&format!("{} / {}", super::format_time(state.video_progress), super::format_time(details.duration_seconds)));
    let playing = state.track_state == VideoState::Playing || state.track_state == VideoState::Buffering;
    m.play_pause.set_image(Some(&icon(&[if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" }])));
    m.play_pause.set_tooltip_text(Some(if playing { "Pause" } else { "Play" }));
    if !m.volume_dragging.get() {
        m.volume.set_value(state.volume);
    }
    m.muted = state.muted;
    let volume_icon = if state.muted {
        "audio-volume-muted-symbolic"
    } else if state.volume > 50.0 {
        "audio-volume-high-symbolic"
    } else {
        "audio-volume-medium-symbolic"
    };
    m.mute.set_image(Some(&icon(&[volume_icon])));
    let repeat = state.queue.as_ref().map(|q| q.repeat_mode).unwrap_or(RepeatMode::Unknown);
    m.repeat.set_image(Some(&icon(&[if repeat == RepeatMode::One { "media-playlist-repeat-song-symbolic" } else { "media-playlist-repeat-symbolic" }])));
    m.repeat.set_opacity(if matches!(repeat, RepeatMode::All | RepeatMode::One) { 1.0 } else { 0.5 });

    let url = highest_res_thumbnail_reduce(&details.thumbnails);
    if url != m.artwork_url {
        m.artwork_url = url.clone();
        *m.artwork.borrow_mut() = None;
        m.art.queue_draw();
        if let Some(url) = url {
            load_artwork(app, url);
        }
    }
}

fn load_artwork(app: &App, url: String) {
    let (client, main) = (app.http.clone(), app.main.clone());
    app.spawn(async move {
        let bytes = match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
            Ok(res) => res.bytes().await.ok(),
            Err(err) => {
                log::debug!("miniplayer: artwork download failed {err}");
                None
            }
        };
        let Some(bytes) = bytes else { return };
        main.run(move |app| {
            let Some(m) = app.ui.miniplayer.as_ref() else { return };
            // A newer track may have been selected meanwhile
            if m.artwork_url.as_deref() != Some(url.as_str()) {
                return;
            }
            *m.artwork.borrow_mut() = crate::assets::pixbuf_from_bytes(&bytes);
            m.art.queue_draw();
        });
    });
}

