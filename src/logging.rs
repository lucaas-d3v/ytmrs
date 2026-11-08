//! electron-log equivalent: console + file (`logs/main.log`, 1MB rotation) with
//! URL path redaction on the file transport.

use std::sync::LazyLock;

use regex::Regex;

static REDACT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"((?:https?://)?[^\s/]{1,64}(?:\.[^\s/]{1,64})?\.[^\s/]{1,64}/)\S+").unwrap());

pub fn sanitize(message: &str) -> Option<String> {
    if message.contains("Third-party cookie will be blocked.") {
        return None;
    }
    Some(REDACT.replace_all(message, "${1}[REDACTED]").into_owned())
}

pub fn init() {
    let paths = crate::paths::get();
    let _ = std::fs::create_dir_all(&paths.logs);
    let log_path = paths.logs.join("main.log");
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() > 1024 * 1024 {
            let _ = std::fs::rename(&log_path, paths.logs.join("main.old.log"));
        }
    }

    let level = if std::env::var("YTMD_LOG_TRACE").is_ok() { log::LevelFilter::Trace } else { log::LevelFilter::Debug };
    let console = fern::Dispatch::new()
        .format(|out, message, record| out.finish(format_args!("[main][{}]{}", record.level().as_str().to_lowercase(), message)))
        .chain(std::io::stdout());

    let mut dispatch = fern::Dispatch::new()
        .level(log::LevelFilter::Warn)
        .level_for("ytmrs", level)
        .chain(console);

    if let Ok(file) = fern::log_file(&log_path) {
        let file_dispatch = fern::Dispatch::new()
            .filter(|_| true)
            .format(|out, message, record| {
                let text = message.to_string();
                let Some(text) = sanitize(&text) else { return };
                out.finish(format_args!(
                    "[{}][main][{}]{}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                    record.level().as_str().to_lowercase(),
                    text
                ))
            })
            .chain(file);
        dispatch = dispatch.chain(file_dispatch);
    }
    let _ = dispatch.apply();
}

#[cfg(test)]
mod tests {
    #[test]
    fn redacts_url_paths() {
        assert_eq!(super::sanitize("Navigating https://music.youtube.com/watch?v=abc").unwrap(), "Navigating https://music.youtube.com/[REDACTED]");
        assert!(super::sanitize("Third-party cookie will be blocked.").is_none());
        assert_eq!(super::sanitize("plain text").unwrap(), "plain text");
    }
}
