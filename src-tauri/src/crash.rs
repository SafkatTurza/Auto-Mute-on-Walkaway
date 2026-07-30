//! Crash logging: capture panics to a file.
//!
//! Release builds run as a windowed app (`windows_subsystem = "windows"`), so
//! there is no console and a panic would otherwise vanish silently. This module
//! installs a panic hook that appends a timestamped record — thread, location,
//! message, and a backtrace — to `crash.log` beside the normal app log, and
//! mirrors a one-line summary into the app logger. It chains to the previous
//! hook so the default behaviour (a stderr message in debug) is preserved.
//!
//! The record is short diagnostic text only: a panic message and code location,
//! never camera frames, audio, or personal data — consistent with the app's
//! privacy contract.

use std::backtrace::Backtrace;
use std::fs::OpenOptions;
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use amow_logger::Logger;

/// Install the crash-logging panic hook, writing to `crash_log` and mirroring a
/// summary to `logger`. Safe to call once during startup.
pub fn install(crash_log: PathBuf, logger: Arc<Logger>) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());

        // The panic payload is usually a &str or String.
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "non-string panic payload".to_string()
        };

        let backtrace = Backtrace::force_capture();
        let record = format!(
            "\n===== PANIC {now} =====\n\
             thread:   {thread}\n\
             location: {location}\n\
             message:  {message}\n\
             backtrace:\n{backtrace}\n"
        );

        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_log)
        {
            let _ = f.write_all(record.as_bytes());
        }
        logger.error(&format!(
            "PANIC on thread '{thread}' at {location}: {message} (details in {})",
            crash_log.display()
        ));

        // Preserve prior hook behaviour (e.g. the default stderr print).
        previous(info);
    }));
}
