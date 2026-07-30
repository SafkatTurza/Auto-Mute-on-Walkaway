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

    /// Whether the adapter can guarantee it is able to *re-enable* the camera
    /// after disabling it.
    ///
    /// When this is false the controller must not disable the camera at all:
    /// switching off a device it cannot switch back on is exactly the failure
    /// that leaves a webcam dark after the app is gone. Windows camera control
    /// needs administrator rights in *both* directions, so the native adapter
    /// reports false when the app is not elevated. The default is true for
    /// adapters with no such asymmetry (or that fail closed on `set_enabled`).
    fn can_restore(&self) -> bool {
        true
    }
}

/// Shows a transient desktop notification.
pub trait Notifier {
    fn notify(&self, title: &str, body: &str);
}
