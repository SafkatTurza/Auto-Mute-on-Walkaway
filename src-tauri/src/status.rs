//! A serialisable snapshot of the controller's live state, shared with the UI.

use std::sync::{Arc, Mutex};

use amow_application::{Camera, Clock, Microphone, Notifier, WalkawayController};
use amow_domain::{MeetingState, PresenceState};
use serde::Serialize;

/// What the front end renders: current presence, meeting, and protection state.
#[derive(Clone, Serialize)]
pub struct Status {
    /// `"present"` or `"away"`.
    pub presence: &'static str,
    /// `"active"` or `"idle"`.
    pub meeting: &'static str,
    /// Whether the app is currently holding devices protected.
    pub protecting: bool,
}

impl Default for Status {
    fn default() -> Self {
        // Matches the trackers' initial state: present, not in a meeting.
        Self {
            presence: "present",
            meeting: "idle",
            protecting: false,
        }
    }
}

/// Thread-safe handle to the latest [`Status`], written by the supervisor and
/// read by Tauri commands.
#[derive(Clone, Default)]
pub struct SharedStatus(Arc<Mutex<Status>>);

impl SharedStatus {
    /// Recompute the snapshot from the controller. Called after each sample.
    pub fn update<CK, MIC, CAM, NOT>(&self, controller: &WalkawayController<CK, MIC, CAM, NOT>)
    where
        CK: Clock,
        MIC: Microphone,
        CAM: Camera,
        NOT: Notifier,
    {
        let snapshot = Status {
            presence: match controller.presence_state() {
                PresenceState::Present => "present",
                PresenceState::Away => "away",
            },
            meeting: match controller.meeting_state() {
                MeetingState::Active => "active",
                MeetingState::Idle => "idle",
            },
            protecting: controller.is_protecting(),
        };
        *self.0.lock().expect("status mutex poisoned") = snapshot;
    }

    /// Current snapshot for the UI.
    pub fn snapshot(&self) -> Status {
        self.0.lock().expect("status mutex poisoned").clone()
    }
}
