//! Translating the presence sidecar's reports into controller face samples.
//!
//! Presence detection runs in a separate process (the Python webcam + MediaPipe
//! sidecar) and speaks a narrow, language-neutral contract: one compact JSON
//! object per line on its stdout, e.g.
//!
//! ```text
//! {"type":"presence","state":"away","at_ms":12345}
//! ```
//!
//! This module is the *host side* of that contract. It parses a line into a
//! [`PresenceReport`] and maps it to the single boolean the
//! [`WalkawayController`](crate::WalkawayController) consumes via
//! `on_face_sample`. It holds no policy of its own — it neither debounces nor
//! decides to mute anything; it only interprets what the detector reported.
//!
//! ## Why this does not duplicate the debounce
//!
//! The sidecar emits four phases. Two are *raw edges* and two are *debounced
//! confirmations*:
//!
//! | phase       | meaning                              | face visible? |
//! | ----------- | ------------------------------------ | ------------- |
//! | `present`   | confirmed at the desk                | yes           |
//! | `leaving`   | face just disappeared (grace running)| **no**        |
//! | `away`      | confirmed absent                     | no            |
//! | `returning` | face just reappeared (grace running) | **yes**       |
//!
//! Because we map on *face visibility*, `leaving`/`returning` — the moment the
//! face is lost or regained — already flip the sample, and `away`/`present`
//! merely reaffirm it. The sidecar's own grace windows therefore do not gate
//! this signal: the raw face edge passes straight through to the domain
//! [`PresenceTracker`](amow_domain::PresenceTracker), which applies the one and
//! only configurable delay (`presence.away_grace_ms` / `return_grace_ms`). The
//! debounce lives in exactly one place — the tested core — not in two.

use serde::Deserialize;

/// The phase vocabulary the presence detector reports. Mirrors the sidecar's
/// `state` field; deserialised leniently so an unrecognised phase is rejected
/// (the line is dropped) rather than crashing the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectorPhase {
    Present,
    Leaving,
    Away,
    Returning,
}

impl DetectorPhase {
    /// Whether the user's face is currently visible in this phase.
    ///
    /// `leaving` and `returning` are the raw edges (face just lost / regained),
    /// so they flip immediately; `present`/`away` reaffirm the same value.
    pub fn face_present(self) -> bool {
        matches!(self, DetectorPhase::Present | DetectorPhase::Returning)
    }
}

/// One decoded presence report from the sidecar.
///
/// `at_ms` is the detector's own monotonic timestamp; it is retained for
/// diagnostics but plays no part in the host's timing — the controller stamps
/// samples with its injected clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceReport {
    pub phase: DetectorPhase,
    pub at_ms: i64,
}

impl PresenceReport {
    /// The face-present sample to feed the controller for this report.
    pub fn face_present(self) -> bool {
        self.phase.face_present()
    }
}

/// The exact on-the-wire shape of a presence line. Unknown extra fields are
/// ignored (no `deny_unknown_fields`) so the contract can grow without breaking
/// older hosts.
#[derive(Deserialize)]
struct Wire {
    #[serde(rename = "type")]
    kind: String,
    state: DetectorPhase,
    at_ms: i64,
}

/// Parse one line of the sidecar's stdout stream.
///
/// Returns `None` — never an error — for anything that is not a well-formed
/// presence report: blank lines, non-JSON diagnostics that slipped onto stdout,
/// objects of a different `type`, or an unrecognised `state`. The bridge simply
/// skips those, so a single malformed line can never take down the detector
/// pipeline.
pub fn parse_line(line: &str) -> Option<PresenceReport> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let wire: Wire = serde_json::from_str(trimmed).ok()?;
    if wire.kind != "presence" {
        return None;
    }
    Some(PresenceReport {
        phase: wire.state,
        at_ms: wire.at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(state: &str) -> String {
        format!(r#"{{"type":"presence","state":"{state}","at_ms":42}}"#)
    }

    #[test]
    fn maps_each_phase_to_face_visibility() {
        assert!(parse_line(&line("present")).unwrap().face_present());
        assert!(parse_line(&line("returning")).unwrap().face_present());
        assert!(!parse_line(&line("leaving")).unwrap().face_present());
        assert!(!parse_line(&line("away")).unwrap().face_present());
    }

    #[test]
    fn decodes_phase_and_timestamp() {
        let report = parse_line(&line("away")).unwrap();
        assert_eq!(report.phase, DetectorPhase::Away);
        assert_eq!(report.at_ms, 42);
    }

    #[test]
    fn ignores_unknown_extra_fields() {
        // The contract may grow; extra keys must not break decoding.
        let l = r#"{"type":"presence","state":"present","at_ms":1,"conf":0.9}"#;
        assert_eq!(parse_line(l).unwrap().phase, DetectorPhase::Present);
    }

    #[test]
    fn rejects_a_different_message_type() {
        let l = r#"{"type":"meeting","state":"present","at_ms":1}"#;
        assert!(parse_line(l).is_none());
    }

    #[test]
    fn rejects_an_unknown_phase() {
        assert!(parse_line(&line("napping")).is_none());
    }

    #[test]
    fn skips_blank_and_whitespace_lines() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   \t").is_none());
    }

    #[test]
    fn skips_non_json_diagnostics() {
        assert!(parse_line("detector started at 15.0 FPS").is_none());
    }

    #[test]
    fn skips_partial_or_malformed_json() {
        assert!(parse_line(r#"{"type":"presence","state":"away""#).is_none());
        assert!(parse_line(r#"{"type":"presence"}"#).is_none());
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        let l = format!("  {}\n", line("returning"));
        assert!(parse_line(&l).unwrap().face_present());
    }
}
