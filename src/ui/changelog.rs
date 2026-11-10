//! Native changelog window (port of windows/changelog), shown once after an update.
//! Release notes come from the GitHub release of the running version.

use gtk::prelude::*;
use serde_json::Value;

use crate::app::App;
use crate::services::config_store;

const RELEASES: &str = "https://github.com/lucaas-d3v/ytmrs/releases";

pub fn maybe_show(app: &mut App) {
    if app.config_store.get_str("metadata.appVersion").as_deref() == Some(crate::APP_VERSION) || app.ui.changelog.is_some() {
        return;
    }
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("YtmRs - Changelog");
    window.set_default_size(720, 560);
    window.set_modal(true);
    window.set_destroy_with_parent(true);
    if let Some(parent) = super::main_gtk_window(app) {
        window.set_transient_for(Some(&parent));
        window.set_position(gtk::WindowPosition::CenterOnParent);
    }
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    header.set_title(Some("What's new"));
    header.set_subtitle(Some(&format!("YtmRs {}", crate::APP_VERSION)));
    let github = super::link_button(&format!("{RELEASES}/tag/v{}", crate::APP_VERSION), "View on GitHub");
    header.pack_end(&github);
    window.set_titlebar(Some(&header));

    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(24);
    view.set_right_margin(24);
    view.set_top_margin(24);
    view.set_bottom_margin(24);
    view.buffer().map(|b| b.set_text("Loading release notes..."));
    let scrolled = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scrolled.add(&view);
    window.add(&scrolled);

    let main = app.main.clone();
    window.connect_destroy(move |_| {
        main.run(|app| {
            app.ui.changelog = None;
            config_store::set(app, "metadata.appVersion", Value::from(crate::APP_VERSION));
        })
    });
    window.show_all();
    app.ui.changelog = Some(window);

    let (client, main) = (app.http.clone(), app.main.clone());
    app.spawn(async move {
        let url = format!("https://api.github.com/repos/lucaas-d3v/ytmrs/releases/tags/v{}", crate::APP_VERSION);
        let notes = match client.get(url).header("Accept", "application/vnd.github+json").send().await.and_then(|r| r.error_for_status()) {
            Ok(res) => res.json::<Value>().await.ok().and_then(|v| v.get("body").and_then(|b| b.as_str()).map(|s| s.to_string())),
            Err(err) => {
                log::warn!("Failed to retrieve changelog {err}");
                None
            }
        };
        let notes = notes.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "# Failed to retrieve changelog".to_string());
        main.run(move |app| {
            // window -> scrolled window -> text view
            let view = app.ui.changelog.as_ref().and_then(|w| w.child()).and_then(|s| s.downcast::<gtk::ScrolledWindow>().ok()).and_then(|s| s.child());
            if let Some(buffer) = view.and_then(|v| v.downcast::<gtk::TextView>().ok()).and_then(|v| v.buffer()) {
                render_markdown(&buffer, &notes);
            }
        });
    });
}

/// Minimal markdown (headings, lists, bold, inline code, links) into a text buffer
fn render_markdown(buffer: &gtk::TextBuffer, markdown: &str) {
    buffer.set_text("");
    let table = buffer.tag_table();
    let tag = |name: &str, f: &dyn Fn(&gtk::TextTag)| {
        if table.as_ref().and_then(|t| t.lookup(name)).is_none() {
            let tag = gtk::TextTag::new(Some(name));
            f(&tag);
            table.as_ref().map(|t| t.add(&tag));
        }
    };
    tag("h1", &|t| {
        t.set_weight(700);
        t.set_scale(1.6);
        t.set_pixels_below_lines(8);
    });
    tag("h2", &|t| {
        t.set_weight(700);
        t.set_scale(1.3);
        t.set_pixels_above_lines(12);
        t.set_pixels_below_lines(4);
    });
    tag("h3", &|t| {
        t.set_weight(700);
        t.set_scale(1.1);
        t.set_pixels_above_lines(8);
    });
    tag("bold", &|t| t.set_weight(700));
    tag("code", &|t| t.set_family(Some("monospace")));
    tag("link", &|t| {
        t.set_underline(gtk::pango::Underline::Single);
        t.set_foreground(Some("#62a0ea"));
    });
    tag("item", &|t| t.set_left_margin(36));

    let mut iter = buffer.end_iter();
    for line in markdown.lines() {
        let trimmed = line.trim_end();
        let (block, text) = if let Some(t) = trimmed.strip_prefix("### ") {
            (Some("h3"), t.to_string())
        } else if let Some(t) = trimmed.strip_prefix("## ") {
            (Some("h2"), t.to_string())
        } else if let Some(t) = trimmed.strip_prefix("# ") {
            (Some("h1"), t.to_string())
        } else if let Some(t) = trimmed.trim_start().strip_prefix("- ").or_else(|| trimmed.trim_start().strip_prefix("* ")) {
            (Some("item"), format!("• {t}"))
        } else {
            (None, trimmed.to_string())
        };
        for (segment, style) in inline_segments(&text) {
            let tags: Vec<&str> = block.into_iter().chain(style).collect();
            let start = iter.offset();
            buffer.insert(&mut iter, &segment);
            let start_iter = buffer.iter_at_offset(start);
            for t in tags {
                buffer.apply_tag_by_name(t, &start_iter, &iter);
            }
        }
        buffer.insert(&mut iter, "\n");
    }
}

/// Splits `**bold**`, `` `code` `` and `[text](url)` spans
fn inline_segments(text: &str) -> Vec<(String, Option<&'static str>)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next = [rest.find("**"), rest.find('`'), rest.find('[')].into_iter().flatten().min();
        let Some(i) = next else {
            out.push((rest.to_string(), None));
            break;
        };
        if i > 0 {
            out.push((rest[..i].to_string(), None));
        }
        let span = &rest[i..];
        let parsed = if let Some(inner) = span.strip_prefix("**") {
            inner.find("**").map(|end| ((inner[..end].to_string(), Some("bold")), 4 + end))
        } else if let Some(inner) = span.strip_prefix('`') {
            inner.find('`').map(|end| ((inner[..end].to_string(), Some("code")), 2 + end))
        } else {
            span.find("](").and_then(|mid| span[mid..].find(')').map(|close| ((span[1..mid].to_string(), Some("link")), mid + close + 1)))
        };
        match parsed {
            Some((segment, len)) => {
                out.push(segment);
                rest = &span[len..];
            }
            None => {
                out.push((span[..1].to_string(), None));
                rest = &span[1..];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::inline_segments;

    #[test]
    fn inline_markdown() {
        let s = inline_segments("a **b** `c` [d](https://e) [x");
        let texts: Vec<_> = s.iter().map(|(t, st)| (t.as_str(), *st)).collect();
        assert_eq!(texts, vec![("a ", None), ("b", Some("bold")), (" ", None), ("c", Some("code")), (" ", None), ("d", Some("link")), (" ", None), ("[", None), ("x", None)]);
    }
}
