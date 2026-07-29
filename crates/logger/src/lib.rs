//! Minimal leveled logger for Auto-Mute-on-Walkaway.
//!
//! Design goals: zero external dependencies, level filtering, and a pluggable
//! [`LogSink`] so output can be a local file, stderr, or an in-memory buffer in
//! tests. Timestamps are injected through a `TimeSource` so log formatting is
//! deterministic under test.
//!
//! Privacy: this logger records only the short text messages callers pass. By
//! contract callers MUST NOT pass camera frames, audio, or personal data — the
//! app processes those in memory and never persists them.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub use amow_config::LogLevel;

/// Rank levels so filtering is a simple comparison. Higher = more verbose.
fn rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}

fn label(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
    }
}

/// Destination for formatted log lines.
///
/// Implementations must be cheap to call and tolerate concurrent use; the
/// [`Logger`] serialises writes, so a sink only needs interior consistency for
/// its own resource.
pub trait LogSink: Send + Sync {
    fn write_line(&self, line: &str);
}

/// Produces a timestamp string for each record.
pub type TimeSource = Box<dyn Fn() -> u128 + Send + Sync>;

/// A leveled logger that formats records and forwards them to a sink.
pub struct Logger {
    min_level: LogLevel,
    sink: Box<dyn LogSink>,
    now_ms: TimeSource,
}

impl Logger {
    /// Create a logger emitting records at or above `min_level` to `sink`,
    /// timestamped with wall-clock milliseconds since the Unix epoch.
    pub fn new(min_level: LogLevel, sink: Box<dyn LogSink>) -> Self {
        Self::with_time_source(min_level, sink, Box::new(default_now_ms))
    }

    /// Create a logger with an injected time source (used by tests).
    pub fn with_time_source(
        min_level: LogLevel,
        sink: Box<dyn LogSink>,
        now_ms: TimeSource,
    ) -> Self {
        Self {
            min_level,
            sink,
            now_ms,
        }
    }

    /// Whether a record at `level` would be emitted under the current filter.
    pub fn enabled(&self, level: LogLevel) -> bool {
        rank(level) <= rank(self.min_level)
    }

    /// Log a message at `level`. Filtered records cost only a comparison.
    pub fn log(&self, level: LogLevel, message: &str) {
        if !self.enabled(level) {
            return;
        }
        let line = format!("{} {} {}", (self.now_ms)(), label(level), message);
        self.sink.write_line(&line);
    }

    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message);
    }
    pub fn warn(&self, message: &str) {
        self.log(LogLevel::Warn, message);
    }
    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message);
    }
    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, message);
    }
}

fn default_now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Sink that appends lines to a local file, creating it and parent dirs.
///
/// Writes are guarded by a mutex so records never interleave. Errors are
/// swallowed deliberately: logging must never take down the app, and there is
/// nowhere safe to report a logging failure to.
pub struct FileSink {
    file: Mutex<std::fs::File>,
}

impl FileSink {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl LogSink for FileSink {
    fn write_line(&self, line: &str) {
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Sink that writes to standard error.
pub struct StderrSink;

impl LogSink for StderrSink {
    fn write_line(&self, line: &str) {
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Test sink capturing lines in memory.
    #[derive(Clone, Default)]
    struct MemSink(Arc<Mutex<Vec<String>>>);
    impl LogSink for MemSink {
        fn write_line(&self, line: &str) {
            self.0.lock().unwrap().push(line.to_string());
        }
    }

    fn logger_at(level: LogLevel) -> (Logger, MemSink) {
        let sink = MemSink::default();
        let captured = sink.clone();
        let logger = Logger::with_time_source(level, Box::new(sink), Box::new(|| 42));
        (logger, captured)
    }

    #[test]
    fn filters_below_min_level() {
        let (log, out) = logger_at(LogLevel::Warn);
        log.debug("d");
        log.info("i");
        log.warn("w");
        log.error("e");
        let lines = out.0.lock().unwrap().clone();
        assert_eq!(lines, vec!["42 WARN w", "42 ERROR e"]);
    }

    #[test]
    fn formats_timestamp_level_message() {
        let (log, out) = logger_at(LogLevel::Debug);
        log.info("hello");
        assert_eq!(out.0.lock().unwrap()[0], "42 INFO hello");
    }

    #[test]
    fn enabled_reflects_filter() {
        let (log, _) = logger_at(LogLevel::Info);
        assert!(log.enabled(LogLevel::Error));
        assert!(log.enabled(LogLevel::Info));
        assert!(!log.enabled(LogLevel::Debug));
    }

    #[test]
    fn file_sink_appends() {
        let path = std::env::temp_dir().join(format!("amow-log-{}.log", std::process::id()));
        let _ = fs::remove_file(&path);
        {
            let sink = FileSink::new(&path).unwrap();
            sink.write_line("one");
            sink.write_line("two");
        }
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body, "one\ntwo\n");
        let _ = fs::remove_file(&path);
    }
}
