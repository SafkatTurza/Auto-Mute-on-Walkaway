use serde::{Deserialize, Serialize};

use crate::Millis;

/// Whether a meeting is currently in progress.
///
/// The domain only tracks the *state*; how a meeting is detected (a known
/// conferencing app running, the camera being in use, a calendar event, …)
/// is an infrastructure concern that feeds boolean samples in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingState {
    Active,
    Idle,
}

/// De-duplicates a stream of "is a meeting active" samples into transitions.
///
/// Unlike presence, meeting detection is not frame-noisy, so no grace period
/// is applied here — the tracker simply reports edges. Keeping it as its own
/// type leaves room to add debouncing later without touching callers.
#[derive(Debug, Clone)]
pub struct MeetingTracker {
    state: MeetingState,
}

impl MeetingTracker {
    /// Start idle: auto-muting only ever engages inside a meeting.
    pub fn new() -> Self {
        Self {
            state: MeetingState::Idle,
        }
    }

    pub fn state(&self) -> MeetingState {
        self.state
    }

    /// Feed the latest "meeting active" sample. Returns `Some(new_state)` only
    /// on the tick where the state changes.
    pub fn observe(&mut self, active: bool, _now_ms: Millis) -> Option<MeetingState> {
        let next = if active {
            MeetingState::Active
        } else {
            MeetingState::Idle
        };
        if next == self.state {
            None
        } else {
            self.state = next;
            Some(next)
        }
    }
}

impl Default for MeetingTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_idle() {
        assert_eq!(MeetingTracker::new().state(), MeetingState::Idle);
    }

    #[test]
    fn reports_edges_only() {
        let mut m = MeetingTracker::new();
        assert_eq!(m.observe(true, 0), Some(MeetingState::Active));
        assert_eq!(m.observe(true, 1), None);
        assert_eq!(m.observe(false, 2), Some(MeetingState::Idle));
        assert_eq!(m.observe(false, 3), None);
    }
}
