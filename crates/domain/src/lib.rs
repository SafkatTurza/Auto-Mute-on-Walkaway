//! Core domain layer for Auto-Mute-on-Walkaway.
//!
//! This crate is the innermost ring of the Clean Architecture. It contains
//! pure business rules only — value objects, domain events, and the presence
//! and meeting state machines. It has no knowledge of Tauri, the OS, the
//! camera, or any I/O. Everything here is deterministic and unit-testable.
//!
//! Time is injected as a monotonic millisecond count (`now_ms`) rather than
//! read from a system clock, so state transitions can be tested exactly.

mod event;
mod meeting;
mod presence;

pub use event::{DeviceKind, DomainEvent};
pub use meeting::{MeetingState, MeetingTracker};
pub use presence::{PresenceConfig, PresenceState, PresenceTracker};

/// Monotonic timestamp in milliseconds since an arbitrary but fixed origin.
///
/// The domain never constructs "now" itself; callers pass a value obtained
/// from a `Clock` port. Using a plain `u64` keeps the domain free of any
/// platform time type and makes tests fully deterministic.
pub type Millis = u64;
