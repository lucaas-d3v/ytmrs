//! Options of the WebKit web process that have to be decided before it starts.
//!
//! JavaScriptCore reads its options from `JSC_*` environment variables, which the web process
//! inherits from the app. Low memory mode turns the JIT compilers off: YouTube Music then runs
//! in the interpreter with JavaScriptCore's small heap growth ("mini VM"). Measured on the
//! YouTube Music page: about 180 MiB less in the web process (517 -> 335 MiB) for about twice
//! the processor time while the page is busy.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

const JIT_VAR: &str = "JSC_useJIT";

static LOW_MEMORY: AtomicBool = AtomicBool::new(false);

/// Applies `general.lowMemoryMode`. Must run before any thread is started, because it
/// changes the process environment.
pub fn configure() {
    // An explicit choice in the environment wins over the setting
    if std::env::var_os(JIT_VAR).is_some() {
        return;
    }
    let config = std::fs::read_to_string(crate::services::config_store::config_path()).unwrap_or_default();
    if low_memory_mode(&config) {
        std::env::set_var(JIT_VAR, "0");
        LOW_MEMORY.store(true, Ordering::Relaxed);
        log::info!("Low memory mode: the JavaScript JIT is disabled for YouTube Music");
    }
}

/// `general.lowMemoryMode` of the stored config; enabled unless it was turned off
fn low_memory_mode(config: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(config)
        .ok()
        .and_then(|v| v.pointer("/general/lowMemoryMode").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

/// Programs started by the app (browser, relaunched instance) must not inherit the engine
/// options: a JavaScriptCore based browser would silently run without its JIT.
pub fn scrub(command: &mut Command) -> &mut Command {
    if LOW_MEMORY.load(Ordering::Relaxed) {
        command.env_remove(JIT_VAR);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_memory_mode_defaults_to_enabled() {
        assert!(low_memory_mode(""));
        assert!(low_memory_mode("not json"));
        assert!(low_memory_mode(r#"{"general":{}}"#));
        assert!(low_memory_mode(r#"{"general":{"lowMemoryMode":true}}"#));
        assert!(!low_memory_mode(r#"{"general":{"lowMemoryMode":false}}"#));
    }

    #[test]
    fn scrub_removes_the_engine_options() {
        LOW_MEMORY.store(true, Ordering::Relaxed);
        let mut command = Command::new("true");
        scrub(&mut command);
        assert!(command.get_envs().any(|(key, value)| key == JIT_VAR && value.is_none()));
        LOW_MEMORY.store(false, Ordering::Relaxed);
    }
}
