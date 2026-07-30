//! Application layer: the walkaway orchestration use case.
//!
//! [`WalkawayController`] is the brain of the MVP. It consumes presence samples,
//! runs them through the domain presence tracker, and — while protection is
//! enabled and the user walks away — protects the configured devices (mute mic,
//! disable camera), restoring them when the user returns or protection is
//! switched off. It emits [`DomainEvent`]s onto the event bus for the rest of
//! the app.

mod ports;
pub mod presence_source;

pub use ports::{Camera, Clock, Microphone, Notifier, PortError, PortResult};
pub use presence_source::{parse_line, DetectorPhase, PresenceReport};

use amow_config::BehaviorConfig;
use amow_domain::{DeviceKind, DomainEvent, PresenceConfig, PresenceState, PresenceTracker};
use amow_eventbus::EventBus;

/// Records what the controller changed during one protection episode, so that
/// restore reverts only the app's own actions and never a user's manual choice.
#[derive(Debug, Default, Clone, Copy)]
struct Protection {
    /// The mic's prior `muted` state, set only if the controller muted a live mic.
    prev_mic_muted: Option<bool>,
    /// The camera's prior `enabled` state, set only if the controller disabled it.
    prev_cam_enabled: Option<bool>,
    /// True when a live camera was found but disabling it failed — almost always
    /// missing privileges (the app is not running elevated). Surfaced to the UI
    /// so the user is told *why* the camera stayed on, rather than guessing.
    cam_blocked: bool,
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
    /// The master switch: protection only ever engages while this is on. It is
    /// toggled by the user (and, later, could be driven by any policy source).
    enabled: bool,
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
            // Starts off: the user explicitly enables protection (App Starts →
            // User enables Protection), so nothing is ever touched until asked.
            enabled: false,
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

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_protecting(&self) -> bool {
        self.protection.is_some()
    }

    /// Whether the app currently has the microphone muted as part of an active
    /// protection episode. Drives the UI's live "mic muted" indicator.
    pub fn mic_muted_by_app(&self) -> bool {
        self.protection.is_some_and(|p| p.prev_mic_muted.is_some())
    }

    /// Whether the app currently has the camera disabled as part of an active
    /// protection episode. Drives the UI's live "camera off" indicator.
    pub fn camera_disabled_by_app(&self) -> bool {
        self.protection
            .is_some_and(|p| p.prev_cam_enabled.is_some())
    }

    /// Whether the app tried to disable a live camera during the current episode
    /// but could not (typically because it is not running elevated). Lets the UI
    /// tell the user to run as administrator instead of silently doing nothing.
    pub fn camera_blocked(&self) -> bool {
        self.protection.is_some_and(|p| p.cam_blocked)
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

    /// Turn walkaway protection on or off (the user's master switch).
    ///
    /// Reconciles immediately so flipping it takes effect at once: enabling
    /// while the user is already away engages protection now, and disabling
    /// while protecting restores the devices. A no-op when unchanged, so it is
    /// cheap to call every sample cycle.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        let now = self.clock.now_ms();
        self.reconcile(now);
    }

    /// Decide whether devices should currently be protected and act on any
    /// change. Protection engages only while it is enabled *and* the user is
    /// away; any other combination triggers restore.
    fn reconcile(&mut self, now: u64) {
        let should_protect = self.enabled && self.presence.state() == PresenceState::Away;

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
                    } else {
                        // A live camera we could not turn off — almost always a
                        // privilege problem. Remember it so the UI can prompt the
                        // user to run elevated; the mic is still protected.
                        protection.cam_blocked = true;
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
