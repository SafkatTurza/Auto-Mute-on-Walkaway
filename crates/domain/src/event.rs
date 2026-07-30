use serde::Serialize;

use crate::{Millis, PresenceState};

/// A device the application can automatically control when the user walks away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Microphone,
    Camera,
}

/// Facts that have happened in the domain.
///
/// Domain events are past-tense and immutable. They are the only thing the
/// outer layers observe about the core's behaviour; UI, logging and the tray
/// all react to these rather than reaching into the state machines. Each event
/// carries the monotonic timestamp at which it occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainEvent {
    /// The user's presence at the desk changed.
    PresenceChanged { state: PresenceState, at: Millis },
    /// A device was automatically muted/disabled by the app.
    DeviceProtected {
        device: DeviceKind,
        /// The device state before the app changed it, so it can be restored.
        was_active: bool,
        at: Millis,
    },
    /// A previously protected device was restored to its earlier state.
    DeviceRestored {
        device: DeviceKind,
        restored_to_active: bool,
        at: Millis,
    },
}
