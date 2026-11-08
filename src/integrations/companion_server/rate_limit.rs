//! Fixed window rate limiter (@fastify/rate-limit semantics)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

#[derive(Clone, Default)]
pub struct RateLimiter {
    windows: Arc<Mutex<HashMap<String, (Instant, u32)>>>,
}

pub struct RateLimitResult {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub reset: Duration,
}

impl RateLimiter {
    pub fn check(&self, key: &str, max: u32, window: Duration) -> RateLimitResult {
        let now = Instant::now();
        let mut map = self.windows.lock();
        if map.len() > 10_000 {
            map.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(120));
        }
        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        let reset = window.saturating_sub(now.duration_since(entry.0));
        RateLimitResult { allowed: entry.1 <= max, limit: max, remaining: max.saturating_sub(entry.1), reset }
    }
}

/// `ms(ttl, { long: true })`
pub fn humanize(d: Duration) -> String {
    let ms = d.as_millis() as f64;
    let plural = |v: f64, unit: &str| {
        let r = v.round();
        if r == 1.0 {
            format!("1 {unit}")
        } else {
            format!("{r} {unit}s")
        }
    };
    if ms >= 3_600_000.0 {
        plural(ms / 3_600_000.0, "hour")
    } else if ms >= 60_000.0 {
        plural(ms / 60_000.0, "minute")
    } else if ms >= 1000.0 {
        plural(ms / 1000.0, "second")
    } else {
        format!("{} ms", ms.round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_within_window() {
        let l = RateLimiter::default();
        for _ in 0..2 {
            assert!(l.check("k", 2, Duration::from_secs(60)).allowed);
        }
        let r = l.check("k", 2, Duration::from_secs(60));
        assert!(!r.allowed);
        assert_eq!(r.remaining, 0);
        assert!(l.check("other", 2, Duration::from_secs(60)).allowed);
        assert_eq!(humanize(Duration::from_secs(60)), "1 minute");
        assert_eq!(humanize(Duration::from_secs(5)), "5 seconds");
    }
}
