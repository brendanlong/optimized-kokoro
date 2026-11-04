//! Logging to stderr, one line per event, filtered by level. The server logs a handful of lines,
//! so this replaces `tracing` and `tracing-subscriber` (and their dependencies).
//!
//! `RUST_LOG` sets the level: `error`, `warn`, `info` (the default), `debug`, `trace` or `off`.
//! As in `tracing-subscriber`, it may also be a comma-separated list of `target=level` directives;
//! only bare levels and `kokoro_server…` targets count here, since nothing else logs.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 1,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => " WARN",
            Level::Info => " INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }
}

/// The most verbose level logged; 0 is off.
static MAX: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Parses a level name; `Some(0)` is `off`.
fn parse_level(s: &str) -> Option<u8> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "off" => 0,
        "error" => Level::Error as u8,
        "warn" => Level::Warn as u8,
        "info" => Level::Info as u8,
        "debug" => Level::Debug as u8,
        "trace" => Level::Trace as u8,
        _ => return None,
    })
}

/// The level `RUST_LOG`'s value sets for this crate, or `None` if it sets none.
pub fn level_from(filter: &str) -> Option<u8> {
    let mut level = None;
    for directive in filter.split(',').map(str::trim).filter(|d| !d.is_empty()) {
        let found = match directive.split_once('=') {
            None => parse_level(directive),
            Some((target, l)) if target.trim().starts_with("kokoro_server") => parse_level(l),
            Some(_) => None,
        };
        if let Some(found) = found {
            // A directive for this crate wins over a bare level, as the more specific one.
            if level.is_none() || directive.contains('=') {
                level = Some(found);
            }
        }
    }
    level
}

/// Sets the level from `RUST_LOG`, keeping `info` when it's unset or names no level.
pub fn init_from_env() {
    if let Some(level) = std::env::var("RUST_LOG").ok().as_deref().and_then(level_from) {
        MAX.store(level, Ordering::Relaxed);
    }
}

pub fn enabled(level: Level) -> bool {
    level as u8 <= MAX.load(Ordering::Relaxed)
}

/// `YYYY-MM-DDTHH:MM:SS.ffffffZ` for a time since the Unix epoch.
fn timestamp(since_epoch: std::time::Duration) -> String {
    let secs = since_epoch.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days, for days since 1970-01-01.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        since_epoch.subsec_micros()
    )
}

#[doc(hidden)]
pub fn write(level: Level, target: &str, message: std::fmt::Arguments) {
    #[cfg(test)]
    if CAPTURED
        .with_borrow_mut(|c| c.as_mut().map(|c| c.push(format!("{} {target}: {message}", level.name()))))
        .is_some()
    {
        return;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    eprintln!("{} {} {target}: {message}", timestamp(now), level.name());
}

#[cfg(test)]
thread_local! {
    static CAPTURED: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f`, returning the lines it logged on this thread instead of writing them.
#[cfg(test)]
pub(crate) fn capture(f: impl FnOnce()) -> Vec<String> {
    CAPTURED.with_borrow_mut(|c| *c = Some(Vec::new()));
    f();
    CAPTURED.with_borrow_mut(Option::take).unwrap_or_default()
}

/// Lets one event through per `period`, counting the ones it holds back.
#[derive(Debug)]
pub struct RateLimit {
    period: std::time::Duration,
    last: Option<std::time::Instant>,
    suppressed: u64,
}

impl RateLimit {
    pub fn new(period: std::time::Duration) -> Self {
        Self { period, last: None, suppressed: 0 }
    }

    /// `Some(events suppressed since the last one allowed)` if this one may be logged.
    pub fn allow(&mut self) -> Option<u64> {
        self.allow_at(std::time::Instant::now())
    }

    fn allow_at(&mut self, now: std::time::Instant) -> Option<u64> {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < self.period => {
                self.suppressed += 1;
                None
            }
            _ => {
                self.last = Some(now);
                Some(std::mem::take(&mut self.suppressed))
            }
        }
    }
}

#[macro_export]
macro_rules! log_at {
    ($level:expr, $($arg:tt)+) => {
        if $crate::log::enabled($level) {
            $crate::log::write($level, module_path!(), format_args!($($arg)+));
        }
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::log_at!($crate::log::Level::Warn, $($arg)+) };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::log_at!($crate::log::Level::Info, $($arg)+) };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::log_at!($crate::log::Level::Debug, $($arg)+) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn levels_from_rust_log() {
        assert_eq!(level_from("info"), Some(Level::Info as u8));
        assert_eq!(level_from("DEBUG"), Some(Level::Debug as u8));
        assert_eq!(level_from("off"), Some(0));
        assert_eq!(level_from("warn,kokoro_server=trace"), Some(Level::Trace as u8));
        assert_eq!(level_from("kokoro_server::worker=debug,error"), Some(Level::Debug as u8));
        assert_eq!(level_from("hyper=trace"), None);
        assert_eq!(level_from("verbose"), None);
        assert_eq!(level_from(""), None);
    }

    #[test]
    fn rate_limit_counts_what_it_holds_back() {
        let mut limit = RateLimit::new(Duration::from_secs(60));
        let t = std::time::Instant::now();
        assert_eq!(limit.allow_at(t), Some(0));
        for i in 1..=500 {
            assert_eq!(limit.allow_at(t + Duration::from_millis(100 * i)), None);
        }
        assert_eq!(limit.allow_at(t + Duration::from_secs(60)), Some(500));
        assert_eq!(limit.allow_at(t + Duration::from_secs(61)), None);
    }

    #[test]
    fn timestamps_are_utc_dates() {
        assert_eq!(timestamp(Duration::ZERO), "1970-01-01T00:00:00.000000Z");
        assert_eq!(timestamp(Duration::from_micros(951_782_400_000_001)), "2000-02-29T00:00:00.000001Z");
        assert_eq!(timestamp(Duration::from_secs(1_793_673_296)), "2026-11-03T02:34:56.000000Z");
    }
}
