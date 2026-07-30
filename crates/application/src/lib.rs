//! Application layer: the walkaway orchestration use case.
//!
//! [`WalkawayController`] is the brain of the MVP. It consumes presence and
//! meeting samples, runs them through the domain state machines, and — when the
//! user walks away *during a meeting* — protects the configured devices (mute
//! mic, disable camera), restoring them when the user returns or the meeting
//! ends. It emits [`DomainEvent`]s onto the event bus for the rest of the app.

mod ports;

pub use ports::{Camera, Clock, Microphone, Notifier, PortError, PortResult};

use amow_config::BehaviorConfig;
use amow_domain::{
    DeviceKind, DomainEvent, MeetingState, MeetingTracker, PresenceConfig, PresenceState,
    PresenceTracker,
};
use amow_eventbus::EventBus;

/// Records what the controller changed during one protection episode, so that
/// restore reverts only the app's own actions and never a user's manual choice.
#[derive(Debug, Default, Clone, Copy)]
struct Protection {
    /// The mic's prior `muted` state, set only if the controller muted a live mic.
    prev_mic_muted: Option<bool>,
    /// The camera's prior `enabled` state, set only if the controller disabled it.
    prev_cam_enabled: Option<bool>,
}

/// Orchestrates auto-mute / auto-camera-off / auto-restore.
///
/// Generic over its ports so tests can inject fakes and production can inject
/// concrete OS adapters with no dynamic dispatch.
pub struct WalkawayController<CK, MIC, CAM, NOT>
where
    CK: Clock,
    MIC: Microphone,
    CAM: Camera,
    NOT: Notifier,
{
    behavior: BehaviorConfig,
    presence: PresenceTracker,
    meeting: MeetingTracker,
    clock: CK,
    mic: MIC,
    camera: CAM,
    notifier: NOT,
    bus: EventBus,
    protection: Option<Protection>,
}

impl<CK, MIC, CAM, NOT> WalkawayController<CK, MIC, CAM, NOT>
where
    CK: Clock,
    MIC: Microphone,
    CAM: Camera,
    NOT: Notifier,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        behavior: BehaviorConfig,
        presence_config: PresenceConfig,
        clock: CK,
        mic: MIC,
        camera: CAM,
        notifier: NOT,
        bus: EventBus,
    ) -> Self {
        Self {
            behavior,
            presence: PresenceTracker::new(presence_config),
            meeting: MeetingTracker::new(),
            clock,
            mic,
            camera,
            notifier,
            bus,
            protection: None,
        }
    }

    pub fn presence_state(&self) -> PresenceState {
        self.presence.state()
    }

    pub fn meeting_state(&self) -> MeetingState {
        self.meeting.state()
    }

    pub fn is_protecting(&self) -> bool {
        self.protection.is_some()
    }

    /// Apply new behaviour policy at runtime (e.g. the user changed a toggle in
    /// Settings). Takes effect on the next reconcile; an in-progress protection
    /// episode is left intact so a live mute is not disturbed mid-change.
    pub fn set_behavior(&mut self, behavior: BehaviorConfig) {
        self.behavior = behavior;
    }

    /// Replace the presence-debounce tuning. This resets presence tracking to
    /// `Present` (the safe default), so it should be paired with a fresh sample
    /// stream; it is intended for applying changed grace periods from Settings.
    pub fn set_presence_config(&mut self, presence_config: PresenceConfig) {
        self.presence = PresenceTracker::new(presence_config);
    }

    /// Feed one raw face-detection sample. Times it with the injected clock,
    /// updates presence, and reconciles device protection.
    pub fn on_face_sample(&mut self, face_present: bool) {
        let now = self.clock.now_ms();
        if let Some(state) = self.presence.observe(face_present, now) {
            self.bus
                .publish(&DomainEvent::PresenceChanged { state, at: now });
            self.reconcile(now);
        }
    }

    /// Feed one raw "meeting active" sample.
    pub fn on_meeting_sample(&mut self, active: bool) {
        let now = self.clock.now_ms();
        if let Some(state) = self.meeting.observe(active, now) {
            self.bus
                .publish(&DomainEvent::MeetingChanged { state, at: now });
            self.reconcile(now);
        }
    }

    /// Decide whether devices should currently be protected and act on any
    /// change. Protection engages only while a meeting is active *and* the user
    /// is away; any other combination triggers restore.
    fn reconcile(&mut self, now: u64) {
        let should_protect = self.meeting.state() == MeetingState::Active
            && self.presence.state() == PresenceState::Away;

        match (should_protect, self.protection.is_some()) {
            (true, false) => self.engage_protection(now),
            (false, true) => self.release_protection(now),
            _ => {}
        }
    }

    fn engage_protection(&mut self, now: u64) {
        let mut protection = Protection::default();

        if self.behavior.auto_mute {
            match self.mic.is_muted() {
                Ok(false) => {
                    // Mic was live; mute it and remember to restore.
                    if self.mic.set_muted(true).is_ok() {
                        protection.prev_mic_muted = Some(false);
                        self.bus.publish(&DomainEvent::DeviceProtected {
                            device: DeviceKind::Microphone,
                            was_active: true,
                            at: now,
                        });
                        self.maybe_notify("Microphone muted", "You stepped away — mic muted.");
                    }
                }
                Ok(true) => { /* already muted by the user; leave it untouched */ }
                Err(_) => { /* port failure: skip mic, keep app running */ }
            }
        }

        if self.behavior.auto_camera_off {
            match self.camera.is_enabled() {
                Ok(true) => {
                    if self.camera.set_enabled(false).is_ok() {
                        protection.prev_cam_enabled = Some(true);
                        self.bus.publish(&DomainEvent::DeviceProtected {
                            device: DeviceKind::Camera,
                            was_active: true,
                            at: now,
                        });
                        self.maybe_notify("Camera disabled", "You stepped away — camera off.");
                    }
                }
                Ok(false) => {}
                Err(_) => {}
            }
        }

        // Record the episode even if nothing was changed, so we don't re-engage
        // on every subsequent sample while still away.
        self.protection = Some(protection);
    }

    fn release_protection(&mut self, now: u64) {
        let Some(protection) = self.protection.take() else {
            return;
        };

        // Only restore when configured to; otherwise leave devices as-is but
        // still end the episode so a later walkaway can re-engage.
        if !self.behavior.auto_restore {
            return;
        }

        if let Some(prev) = protection.prev_mic_muted {
            if self.mic.set_muted(prev).is_ok() {
                self.bus.publish(&DomainEvent::DeviceRestored {
                    device: DeviceKind::Microphone,
                    restored_to_active: !prev,
                    at: now,
                });
                self.maybe_notify("Microphone restored", "Welcome back — mic unmuted.");
            }
        }

        if let Some(prev) = protection.prev_cam_enabled {
            if self.camera.set_enabled(prev).is_ok() {
                self.bus.publish(&DomainEvent::DeviceRestored {
                    device: DeviceKind::Camera,
                    restored_to_active: prev,
                    at: now,
                });
                self.maybe_notify("Camera restored", "Welcome back — camera on.");
            }
        }
    }

    fn maybe_notify(&self, title: &str, body: &str) {
        if self.behavior.notify_on_action {
            self.notifier.notify(title, body);
        }
    }
}

#[cfg(test)]
mod tests;
