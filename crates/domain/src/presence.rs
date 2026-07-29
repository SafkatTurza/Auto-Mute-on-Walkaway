use serde::{Deserialize, Serialize};

use crate::Millis;

/// Whether the user is currently judged to be at the desk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresenceState {
    Present,
    Away,
}

/// Tuning for how quickly presence transitions are believed.
///
/// Raw face-detection samples are noisy: a single dropped frame must not be
/// read as "the user left". Both directions are debounced by requiring the
/// new condition to hold continuously for a grace period before the state
/// flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceConfig {
    /// How long a face must be continuously absent before we declare `Away`.
    pub away_grace_ms: Millis,
    /// How long a face must be continuously present before we declare `Present`.
    pub return_grace_ms: Millis,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        // Conservative defaults: react to leaving after 3s of no face, and
        // confirm a return after 0.8s to avoid flapping on a brief glance.
        Self {
            away_grace_ms: 3_000,
            return_grace_ms: 800,
        }
    }
}

/// Debouncing state machine that converts a stream of raw face-present samples
/// into stable `PresenceState` transitions.
///
/// It is driven by [`PresenceTracker::observe`], which is fed one sample per
/// detection tick together with the monotonic time of that tick.
#[derive(Debug, Clone)]
pub struct PresenceTracker {
    config: PresenceConfig,
    state: PresenceState,
    /// When the *opposite* condition first began holding, if it currently is.
    /// `None` means the latest sample agreed with the committed state.
    pending_since: Option<Millis>,
}

impl PresenceTracker {
    /// Create a tracker that starts in `Present` (the safe default: we assume
    /// the user is there until proven otherwise, so nothing is muted on start).
    pub fn new(config: PresenceConfig) -> Self {
        Self {
            config,
            state: PresenceState::Present,
            pending_since: None,
        }
    }

    /// The currently committed presence state.
    pub fn state(&self) -> PresenceState {
        self.state
    }

    /// Feed one detection sample.
    ///
    /// `face_present` is the raw result of the current detection tick and
    /// `now_ms` is the monotonic time of that tick. Returns `Some(new_state)`
    /// exactly on the tick where the committed state changes, otherwise `None`.
    pub fn observe(&mut self, face_present: bool, now_ms: Millis) -> Option<PresenceState> {
        let agrees = matches!(
            (self.state, face_present),
            (PresenceState::Present, true) | (PresenceState::Away, false)
        );

        if agrees {
            // Sample confirms the current state; cancel any pending flip.
            self.pending_since = None;
            return None;
        }

        // Sample disagrees: start or continue the grace timer for the flip.
        let started = *self.pending_since.get_or_insert(now_ms);
        let grace = match self.state {
            PresenceState::Present => self.config.away_grace_ms,
            PresenceState::Away => self.config.return_grace_ms,
        };

        // `saturating_sub` guards against a non-monotonic `now_ms`.
        if now_ms.saturating_sub(started) >= grace {
            self.state = match self.state {
                PresenceState::Present => PresenceState::Away,
                PresenceState::Away => PresenceState::Present,
            };
            self.pending_since = None;
            Some(self.state)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> PresenceTracker {
        PresenceTracker::new(PresenceConfig {
            away_grace_ms: 1_000,
            return_grace_ms: 500,
        })
    }

    #[test]
    fn starts_present() {
        assert_eq!(tracker().state(), PresenceState::Present);
    }

    #[test]
    fn transient_absence_is_ignored() {
        let mut t = tracker();
        // Face gone briefly then back before the away grace elapses.
        assert_eq!(t.observe(false, 0), None);
        assert_eq!(t.observe(false, 400), None);
        assert_eq!(t.observe(true, 500), None);
        assert_eq!(t.state(), PresenceState::Present);
    }

    #[test]
    fn sustained_absence_goes_away_after_grace() {
        let mut t = tracker();
        assert_eq!(t.observe(false, 0), None);
        assert_eq!(t.observe(false, 999), None);
        assert_eq!(t.observe(false, 1_000), Some(PresenceState::Away));
        assert_eq!(t.state(), PresenceState::Away);
    }

    #[test]
    fn transition_fires_only_once() {
        let mut t = tracker();
        t.observe(false, 0);
        assert_eq!(t.observe(false, 1_000), Some(PresenceState::Away));
        // Further absent samples must not re-emit the transition.
        assert_eq!(t.observe(false, 2_000), None);
    }

    #[test]
    fn return_uses_its_own_grace() {
        let mut t = tracker();
        t.observe(false, 0);
        t.observe(false, 1_000); // -> Away
        assert_eq!(t.observe(true, 1_000), None);
        assert_eq!(t.observe(true, 1_499), None);
        assert_eq!(t.observe(true, 1_500), Some(PresenceState::Present));
    }

    #[test]
    fn flip_pending_is_cancelled_by_disagreeing_sample() {
        let mut t = tracker();
        t.observe(false, 0); // pending away
        t.observe(true, 200); // cancels
                              // Grace timer must restart, not resume, on the next absence.
        assert_eq!(t.observe(false, 300), None);
        assert_eq!(t.observe(false, 1_299), None);
        assert_eq!(t.observe(false, 1_300), Some(PresenceState::Away));
    }

    #[test]
    fn non_monotonic_time_does_not_panic_or_flip_early() {
        let mut t = tracker();
        t.observe(false, 1_000); // pending since 1000
                                 // Clock went backwards; saturating_sub keeps elapsed at 0.
        assert_eq!(t.observe(false, 500), None);
    }
}
