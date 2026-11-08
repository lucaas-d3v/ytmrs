//! Embedded web UI + icons and the `ytmd-app` custom protocol (ProtocolManager.handleYTMDAppProtocol)

use std::borrow::Cow;

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/"]
struct Web;

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

pub const PROTOCOL: &str = "ytmd-app";

/// Base origin of app pages (differs on Windows, see wry custom protocol docs)
pub fn app_origin() -> &'static str {
    if cfg!(windows) {
        "http://ytmd-app.localhost"
    } else {
        "ytmd-app://localhost"
    }
}

pub fn app_url(path: &str) -> String {
    format!("{}/{}", app_origin(), path.trim_start_matches('/'))
}

pub fn is_app_url(url: &str) -> bool {
    url.starts_with(app_origin())
}

pub fn web_text(path: &str) -> String {
    Web::get(path).map(|f| String::from_utf8_lossy(&f.data).into_owned()).unwrap_or_else(|| {
        log::error!("missing embedded web asset {path}");
        String::new()
    })
}

pub fn icon_bytes(name: &str) -> Option<Cow<'static, [u8]>> {
    Assets::get(&format!("icons/{name}")).map(|f| f.data)
}

pub fn decode_png(bytes: Cow<'static, [u8]>) -> Option<(Vec<u8>, u32, u32)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes.to_vec()));
    let mut decoder = decoder;
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((rgba, info.width, info.height))
}

/// Safe path policy: no traversal outside the root
pub fn is_safe_path(path: &str) -> bool {
    !path.is_empty() && !path.split(['/', '\\']).any(|p| p == "..") && !path.starts_with('/') && !path.contains(':')
}

pub fn resolve(path: &str) -> Option<(Cow<'static, [u8]>, String)> {
    let mut desired = path.trim_start_matches(['/', '\\']).to_string();
    if desired.is_empty() {
        return None;
    }
    if desired.ends_with('/') {
        desired.push_str("index.html");
    }
    if let Some(rest) = desired.strip_prefix("assets/") {
        let f = Assets::get(rest)?;
        return Some((f.data, f.metadata.mimetype().to_string()));
    }
    let f = Web::get(&desired)?;
    Some((f.data, f.metadata.mimetype().to_string()))
}

pub fn serve_protocol(request: &http::Request<Vec<u8>>) -> http::Response<Cow<'static, [u8]>> {
    let path = percent_decode(request.uri().path());
    log::trace!("{} {}://{}", request.method(), PROTOCOL, path);
    let trimmed = path.trim_start_matches(['/', '\\']);
    if !is_safe_path(trimmed) && !trimmed.is_empty() {
        log::warn!("Requested path violated the safe path policy");
        return http::Response::builder()
            .status(403)
            .header("Content-Type", "text/html")
            .body(Cow::Borrowed(&b"Policy Violation"[..]))
            .unwrap();
    }
    match resolve(&path) {
        Some((data, mime)) => http::Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .header("Access-Control-Allow-Origin", "*")
            .body(data)
            .unwrap(),
        None => http::Response::builder().status(404).body(Cow::Borrowed(&b""[..])).unwrap(),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_paths() {
        assert!(is_safe_path("windows/settings/index.html"));
        assert!(!is_safe_path("../etc/passwd"));
        assert!(!is_safe_path("windows/../../x"));
        assert!(!is_safe_path("C:\\x"));
    }

    #[test]
    fn resolves_embedded() {
        assert!(resolve("/shared/bridge.js").is_some());
        assert!(resolve("/assets/icons/ytmrs.png").is_some());
        assert!(resolve("/nope.txt").is_none());
        assert!(decode_png(icon_bytes("ytmrs.png").unwrap()).is_some());
        assert!(decode_png(icon_bytes("controls/play-button.png").unwrap()).is_some());
    }
}
