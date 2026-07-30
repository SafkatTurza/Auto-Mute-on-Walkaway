//! Ports: interfaces the application needs the outside world to provide.
//!
//! Following the Dependency Inversion Principle, the application depends on
//! these traits, and infrastructure adapters (PulseAudio, CoreAudio, the OS
//! notification centre, …) implement them. The application never names a
//! concrete device technology.

/// Error returned by an infrastructure port. Kept as a message plus a
/// recoverable flag; the application logs it and continues rather than crashing
/// — a failed mute must never take the app down mid-call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortError {
    pub message: String,
}

impl PortError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PortError {}

pub type PortResult<T> = Result<T, PortError>;

/// Monotonic time source. Injected so the application layer never reads a
/// global clock directly, keeping it deterministic under test.
pub trait Clock {
    fn now_ms(&self) -> u64;
}

/// Controls the system microphone's mute state.
pub trait Microphone {
    fn is_muted(&self) -> PortResult<bool>;
    fn set_muted(&self, muted: bool) -> PortResult<()>;
}

/// Controls whether the camera is enabled (available to conferencing apps).
pub trait Camera {
    fn is_enabled(&self) -> PortResult<bool>;
    fn set_enabled(&self, enabled: bool) -> PortResult<()>;
}

/// Shows a transient desktop notification.
pub trait Notifier {
    fn notify(&self, title: &str, body: &str);
}
