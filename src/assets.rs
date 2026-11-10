//! Embedded assets: the scripts injected into YouTube Music and the app icons.

use std::borrow::Cow;

use gtk::gdk_pixbuf::{self, Pixbuf};
use gtk::prelude::*;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/ytm/"]
struct YtmScripts;

#[derive(RustEmbed)]
#[folder = "assets/icons/"]
#[include = "ytmrs.png"]
#[include = "ytmrs_white.png"]
#[include = "ytmrs_black.png"]
struct Icons;

pub fn ytm_script(path: &str) -> String {
    YtmScripts::get(path).map(|f| String::from_utf8_lossy(&f.data).into_owned()).unwrap_or_else(|| {
        log::error!("missing embedded script {path}");
        String::new()
    })
}

pub fn icon_bytes(name: &str) -> Option<Cow<'static, [u8]>> {
    Icons::get(name).map(|f| f.data)
}

/// Decode an embedded icon, optionally scaled to `size`x`size`
pub fn icon_pixbuf(name: &str, size: Option<i32>) -> Option<Pixbuf> {
    let pixbuf = pixbuf_from_bytes(&icon_bytes(name)?)?;
    match size {
        Some(s) => pixbuf.scale_simple(s, s, gdk_pixbuf::InterpType::Bilinear),
        None => Some(pixbuf),
    }
}

/// Decode image bytes (png/jpeg/webp, whatever gdk-pixbuf supports)
pub fn pixbuf_from_bytes(bytes: &[u8]) -> Option<Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}

/// Tight RGBA buffer of an embedded icon (tray icon)
pub fn icon_rgba(name: &str, size: i32) -> Option<(Vec<u8>, u32, u32)> {
    let pixbuf = icon_pixbuf(name, Some(size))?.add_alpha(false, 0, 0, 0).ok()?;
    let (w, h, stride) = (pixbuf.width() as usize, pixbuf.height() as usize, pixbuf.rowstride() as usize);
    let bytes = pixbuf.read_pixel_bytes();
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        rgba.extend_from_slice(&bytes[row * stride..row * stride + w * 4]);
    }
    Some((rgba, w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_exist() {
        assert!(!ytm_script("disable-media-session.js").is_empty());
        assert!(!ytm_script("scripts.js").is_empty());
        assert!(!ytm_script("volume-ratio/enable.js").is_empty());
        assert!(icon_bytes("ytmrs.png").is_some());
        assert!(icon_bytes("ytmrs_white.png").is_some());
        // Only the app icons are embedded
        assert!(icon_bytes("tray.ico").is_none());
    }
}
