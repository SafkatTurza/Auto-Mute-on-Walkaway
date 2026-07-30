use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use amow_config::BehaviorConfig;
use amow_domain::{DomainEvent, PresenceConfig};
use amow_eventbus::EventBus;

use super::*;

// --- Fakes --------------------------------------------------------------------
//
// The controller owns its ports, so the test mirrors device state through
// atomics behind `Arc` handles it can inspect afterwards. Atomics give the
// `Send + Sync` the port bounds require without any `unsafe`.

const REL: Ordering = Ordering::SeqCst;

struct DeviceCell {
    /// mic: `muted`?  camera: `enabled`?
    value: AtomicBool,
    writes: AtomicU32,
    fail: AtomicBool,
    /// Reads succeed but writes fail — models a camera that is visible/queryable
    /// but cannot be toggled without elevated privileges.
    fail_write: AtomicBool,
    /// The adapter reports it cannot guarantee a re-enable — models the Windows
    /// camera when the app is not elevated (`can_restore() == false`).
    no_restore: AtomicBool,
}
impl DeviceCell {
    fn new(v: bool) -> Self {
        Self {
            value: AtomicBool::new(v),
            writes: AtomicU32::new(0),
            fail: AtomicBool::new(false),
            fail_write: AtomicBool::new(false),
            no_restore: AtomicBool::new(false),
        }
    }
    fn value(&self) -> bool {
        self.value.load(REL)
    }
    fn writes(&self) -> u32 {
        self.writes.load(REL)
    }
    fn set_fail(&self) {
        self.fail.store(true, REL);
    }
    fn set_fail_write(&self) {
        self.fail_write.store(true, REL);
    }
    fn set_no_restore(&self) {
        self.no_restore.store(true, REL);
    }
}

struct FakeClock(Arc<AtomicU64>);
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(REL)
    }
}

struct FakeMic(Arc<DeviceCell>);
impl Microphone for FakeMic {
    fn is_muted(&self) -> PortResult<bool> {
        if self.0.fail.load(REL) {
            return Err(PortError::new("mic fail"));
        }
        Ok(self.0.value.load(REL))
    }
    fn set_muted(&self, muted: bool) -> PortResult<()> {
        if self.0.fail.load(REL) || self.0.fail_write.load(REL) {
            return Err(PortError::new("mic fail"));
        }
        self.0.value.store(muted, REL);
        self.0.writes.fetch_add(1, REL);
        Ok(())
    }
}

struct FakeCam(Arc<DeviceCell>);
impl Camera for FakeCam {
    fn is_enabled(&self) -> PortResult<bool> {
        if self.0.fail.load(REL) {
            return Err(PortError::new("cam fail"));
        }
        Ok(self.0.value.load(REL))
    }
    fn set_enabled(&self, enabled: bool) -> PortResult<()> {
        if self.0.fail.load(REL) || self.0.fail_write.load(REL) {
            return Err(PortError::new("cam fail"));
        }
        self.0.value.store(enabled, REL);
        self.0.writes.fetch_add(1, REL);
        Ok(())
    }
    fn can_restore(&self) -> bool {
        !self.0.no_restore.load(REL)
    }
}

#[derive(Clone, Default)]
struct FakeNotifier(Arc<Mutex<Vec<String>>>);
impl Notifier for FakeNotifier {
    fn notify(&self, title: &str, _body: &str) {
        self.0.lock().unwrap().push(title.to_string());
    }
}

/// Inspectable handles to the fakes owned by the controller under test.
struct Handles {
    clock: Arc<AtomicU64>,
    mic: Arc<DeviceCell>,
    cam: Arc<DeviceCell>,
    notifier: FakeNotifier,
    events: Arc<Mutex<Vec<DomainEvent>>>,
}
impl Handles {
    fn at(&self, t: u64) {
        self.clock.store(t, REL);
    }
    fn events(&self) -> Vec<DomainEvent> {
        self.events.lock().unwrap().clone()
    }
    fn notifications(&self) -> usize {
        self.notifier.0.lock().unwrap().len()
    }
}

type TestController = WalkawayController<FakeClock, FakeMic, FakeCam, FakeNotifier>;

fn presence_cfg() -> PresenceConfig {
    PresenceConfig {
        away_grace_ms: 1_000,
        return_grace_ms: 500,
    }
}

/// Behavior with every automatic action on, including `auto_camera_off`.
///
/// The shipped default now leaves `auto_camera_off` *off* (camera control is
/// opt-in), so tests that exercise camera protection ask for it explicitly.
fn all_on() -> BehaviorConfig {
    BehaviorConfig {
        auto_camera_off: true,
        ..BehaviorConfig::default()
    }
}

fn build(
    behavior: BehaviorConfig,
    mic_muted: bool,
    cam_enabled: bool,
) -> (TestController, Handles) {
    let clock = Arc::new(AtomicU64::new(0));
    let mic = Arc::new(DeviceCell::new(mic_muted));
    let cam = Arc::new(DeviceCell::new(cam_enabled));
    let notifier = FakeNotifier::default();
    let bus = EventBus::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    bus.subscribe(Box::new(move |e| sink.lock().unwrap().push(*e)));

    let controller = WalkawayController::new(
        behavior,
        presence_cfg(),
        FakeClock(clock.clone()),
        FakeMic(mic.clone()),
        FakeCam(cam.clone()),
        notifier.clone(),
        bus,
    );
    (
        controller,
        Handles {
            clock,
            mic,
            cam,
            notifier,
            events,
        },
    )
}

/// Drive a controller from "protection enabled, present" to "away" past grace.
fn walk_away(c: &mut TestController, h: &Handles) {
    h.at(0);
    c.set_enabled(true);
    c.on_face_sample(false);
    h.at(1_000);
    c.on_face_sample(false); // presence -> Away, triggers protection
}

// --- Tests --------------------------------------------------------------------

#[test]
fn mutes_and_disables_on_walkaway_when_enabled() {
    let (mut c, h) = build(all_on(), false, true);
    walk_away(&mut c, &h);
    assert!(h.mic.value(), "mic should be muted");
    assert!(!h.cam.value(), "camera should be disabled");
    assert!(c.is_protecting());
}

#[test]
fn does_nothing_when_protection_disabled() {
    let (mut c, h) = build(all_on(), false, true);
    // Never enabled: walking away must not touch any device.
    h.at(0);
    c.on_face_sample(false);
    h.at(1_000);
    c.on_face_sample(false);
    assert!(!c.is_protecting());
    assert!(!h.mic.value());
    assert!(h.cam.value());
}

#[test]
fn enabling_protection_while_already_away_engages_immediately() {
    let (mut c, h) = build(all_on(), false, true);
    // User is away first (grace elapsed), protection still off: nothing happens.
    h.at(0);
    c.on_face_sample(false);
    h.at(1_000);
    c.on_face_sample(false);
    assert!(!c.is_protecting(), "away but disabled: no protection");
    // Now enable while already away — it must engage at once.
    c.set_enabled(true);
    assert!(c.is_protecting());
    assert!(h.mic.value(), "mic muted the moment protection is enabled");
    assert!(!h.cam.value(), "camera disabled the moment it is enabled");
}

#[test]
fn restores_previous_state_on_return() {
    let (mut c, h) = build(all_on(), false, true);
    walk_away(&mut c, &h);
    h.at(1_000);
    c.on_face_sample(true);
    h.at(1_500);
    c.on_face_sample(true); // presence -> Present, restore
    assert!(!h.mic.value(), "mic should be unmuted");
    assert!(h.cam.value(), "camera should be re-enabled");
    assert!(!c.is_protecting());
}

#[test]
fn does_not_touch_a_mic_the_user_already_muted() {
    let (mut c, h) = build(all_on(), true, true);
    walk_away(&mut c, &h);
    h.at(1_000);
    c.on_face_sample(true);
    h.at(1_500);
    c.on_face_sample(true);
    assert!(h.mic.value(), "user's manual mute must be preserved");
    assert_eq!(h.mic.writes(), 0, "controller must not write the mic");
}

#[test]
fn restores_when_protection_disabled_while_away() {
    let (mut c, h) = build(all_on(), false, true);
    walk_away(&mut c, &h);
    h.at(1_000);
    c.set_enabled(false);
    assert!(!c.is_protecting());
    assert!(!h.mic.value(), "mic restored when protection turned off");
    assert!(h.cam.value(), "camera restored when protection turned off");
}

#[test]
fn respects_disabled_auto_restore() {
    let behavior = BehaviorConfig {
        auto_restore: false,
        auto_camera_off: true,
        ..BehaviorConfig::default()
    };
    let (mut c, h) = build(behavior, false, true);
    walk_away(&mut c, &h);
    h.at(1_000);
    c.on_face_sample(true);
    h.at(1_500);
    c.on_face_sample(true);
    assert!(h.mic.value(), "mic stays muted when restore disabled");
    assert!(!h.cam.value(), "camera stays off when restore disabled");
    assert!(!c.is_protecting());
}

#[test]
fn respects_selective_behavior_flags() {
    let behavior = BehaviorConfig {
        auto_camera_off: false,
        ..BehaviorConfig::default()
    };
    let (mut c, h) = build(behavior, false, true);
    walk_away(&mut c, &h);
    assert!(h.mic.value(), "mic muted");
    assert!(
        h.cam.value(),
        "camera untouched when auto_camera_off is false"
    );
}

#[test]
fn mic_port_failure_does_not_crash_and_still_handles_camera() {
    let (mut c, h) = build(all_on(), false, true);
    h.mic.set_fail();
    walk_away(&mut c, &h);
    assert!(!h.cam.value(), "camera should still be disabled");
    assert!(c.is_protecting());
}

#[test]
fn exposes_live_device_state_for_the_ui() {
    let (mut c, h) = build(all_on(), false, true);
    // Idle: nothing engaged.
    assert!(!c.mic_muted_by_app());
    assert!(!c.camera_disabled_by_app());
    assert!(!c.camera_blocked());

    walk_away(&mut c, &h);
    assert!(c.mic_muted_by_app(), "UI should show the mic as muted");
    assert!(c.camera_disabled_by_app(), "UI should show the camera off");
    assert!(!c.camera_blocked());

    // Return restores devices and clears the live indicators.
    h.at(1_000);
    c.on_face_sample(true);
    h.at(1_500);
    c.on_face_sample(true);
    assert!(!c.mic_muted_by_app());
    assert!(!c.camera_disabled_by_app());
}

#[test]
fn camera_blocked_is_reported_when_disable_is_denied() {
    // A live camera that cannot be toggled (no privilege): the mic is still
    // muted, and the UI is told the camera was blocked rather than silently off.
    let (mut c, h) = build(all_on(), false, true);
    h.cam.set_fail_write();
    walk_away(&mut c, &h);

    assert!(c.is_protecting());
    assert!(c.mic_muted_by_app(), "mic still protected");
    assert!(h.mic.value(), "mic really muted");
    assert!(!c.camera_disabled_by_app(), "camera was not disabled");
    assert!(c.camera_blocked(), "UI told the camera disable was blocked");
    assert!(h.cam.value(), "camera left on because disable failed");
}

#[test]
fn never_disables_a_camera_it_cannot_restore() {
    // The safety invariant: if the adapter cannot guarantee it could re-enable
    // the camera (e.g. the app is not running elevated on Windows, where both
    // directions need admin), the controller must not switch it off at all —
    // never touching it is the only way to guarantee it is never left dark.
    let (mut c, h) = build(all_on(), false, true);
    h.cam.set_no_restore();
    walk_away(&mut c, &h);

    assert!(c.is_protecting());
    assert!(h.mic.value(), "mic is still protected");
    assert!(h.cam.value(), "camera left ON — never disabled");
    assert_eq!(h.cam.writes(), 0, "the camera was never written to");
    assert!(!c.camera_disabled_by_app());
    assert!(
        c.camera_blocked(),
        "UI is told camera control is blocked (run as administrator)"
    );
}

#[test]
fn shutdown_restores_devices_it_changed() {
    // The clean-exit contract: quitting while protecting must un-mute the mic and
    // re-enable the camera the app disabled.
    let (mut c, h) = build(all_on(), false, true);
    walk_away(&mut c, &h);
    assert!(h.mic.value() && !h.cam.value(), "protected before shutdown");

    c.shutdown();

    assert!(!h.mic.value(), "mic un-muted on shutdown");
    assert!(h.cam.value(), "camera re-enabled on shutdown");
    assert!(!c.is_protecting(), "the episode is ended");
}

#[test]
fn shutdown_restores_even_when_auto_restore_is_off() {
    // auto_restore=false keeps devices off on a *return*, but shutting the app
    // down must still restore them — a disabled camera must never outlive the app.
    let behavior = BehaviorConfig {
        auto_restore: false,
        auto_camera_off: true,
        ..BehaviorConfig::default()
    };
    let (mut c, h) = build(behavior, false, true);
    walk_away(&mut c, &h);
    assert!(h.mic.value() && !h.cam.value(), "protected before shutdown");

    c.shutdown();

    assert!(
        !h.mic.value(),
        "mic un-muted on shutdown despite auto_restore off"
    );
    assert!(
        h.cam.value(),
        "camera re-enabled on shutdown despite auto_restore off"
    );
    assert!(!c.is_protecting());
}

#[test]
fn shutdown_without_protection_touches_nothing() {
    // Quitting while idle (never walked away) must not write to any device.
    let (mut c, h) = build(all_on(), false, true);
    c.set_enabled(true);
    c.shutdown();
    assert_eq!(
        h.mic.writes(),
        0,
        "no mic writes when nothing was protected"
    );
    assert_eq!(
        h.cam.writes(),
        0,
        "no cam writes when nothing was protected"
    );
    assert!(h.cam.value(), "camera left as the user had it");
}

#[test]
fn shutdown_does_not_touch_devices_the_user_had_set() {
    // The user muted their own mic and disabled their own camera before walking
    // away; the app engaged an (empty) episode. Shutdown must leave both as the
    // user set them — it only reverts the app's own changes.
    let (mut c, h) = build(all_on(), true, false);
    walk_away(&mut c, &h);
    assert!(
        c.is_protecting(),
        "an episode is recorded even with nothing to change"
    );

    c.shutdown();

    assert!(h.mic.value(), "user's mic mute preserved");
    assert!(!h.cam.value(), "user's disabled camera preserved");
    assert_eq!(h.mic.writes(), 0, "app never wrote the mic");
    assert_eq!(h.cam.writes(), 0, "app never wrote the camera");
}

#[test]
fn emits_expected_event_sequence() {
    let (mut c, h) = build(all_on(), false, true);
    walk_away(&mut c, &h);
    let kinds: Vec<&str> = h
        .events()
        .iter()
        .map(|e| match e {
            DomainEvent::PresenceChanged { .. } => "presence",
            DomainEvent::DeviceProtected { .. } => "protected",
            DomainEvent::DeviceRestored { .. } => "restored",
        })
        .collect();
    assert_eq!(kinds, vec!["presence", "protected", "protected"]);
}

#[test]
fn notifies_on_action_when_enabled() {
    let (mut c, h) = build(all_on(), false, true);
    walk_away(&mut c, &h);
    assert_eq!(
        h.notifications(),
        2,
        "one notification per protected device"
    );
}

#[test]
fn set_behavior_applies_to_next_reconcile() {
    let behavior = BehaviorConfig {
        auto_mute: false,
        ..BehaviorConfig::default()
    };
    let (mut c, h) = build(behavior, false, true);
    // Re-enable auto_mute at runtime; the next walkaway must now mute.
    c.set_behavior(BehaviorConfig::default());
    walk_away(&mut c, &h);
    assert!(h.mic.value(), "runtime behavior change should take effect");
}

// --- End-to-end integration through the presence-sidecar contract -------------
//
// These drive the *whole* walkaway pipeline exactly as the running app does:
// raw JSON lines from the presence detector are parsed by the public
// `presence_source` translator, fed to the controller as face samples, and the
// controller drives the device ports and the event bus. Nothing here reaches
// past the public API — it is the same path the Tauri bridge uses, minus the
// OS process. This is the "complete user flow" verification.

/// Feed one raw sidecar stdout line through the real parser into the controller.
/// A line the parser rejects is silently ignored, just as the bridge does.
fn feed_line(c: &mut TestController, raw: &str) {
    if let Some(report) = parse_line(raw) {
        c.on_face_sample(report.face_present());
    }
}

fn presence_line(state: &str, at_ms: i64) -> String {
    format!(r#"{{"type":"presence","state":"{state}","at_ms":{at_ms}}}"#)
}

#[test]
fn full_flow_from_sidecar_lines_mutes_then_restores() {
    let (mut c, h) = build(all_on(), false, true);

    // Protection enabled, user confirmed present: no action yet.
    h.at(0);
    c.set_enabled(true);
    feed_line(&mut c, &presence_line("present", 0));
    assert!(!c.is_protecting(), "present user must not be protected");

    // Face lost: `leaving` is the raw edge, `away` reaffirms it. The domain
    // tracker owns the delay, so protection engages only once its away grace
    // (1000ms) has elapsed — not on the sidecar's phase alone.
    feed_line(&mut c, &presence_line("leaving", 10));
    assert!(
        !c.is_protecting(),
        "must wait out the configured away grace"
    );
    h.at(1_000);
    feed_line(&mut c, &presence_line("away", 1_000));
    assert!(h.mic.value(), "mic muted after walkaway");
    assert!(!h.cam.value(), "camera disabled after walkaway");
    assert!(c.is_protecting());

    // Face regained: `returning` edge starts the return grace (500ms); once it
    // elapses the user is Present again and prior state is restored.
    h.at(1_000);
    feed_line(&mut c, &presence_line("returning", 1_000));
    assert!(c.is_protecting(), "still protected during return grace");
    h.at(1_500);
    feed_line(&mut c, &presence_line("present", 1_500));
    assert!(!h.mic.value(), "mic unmuted on return");
    assert!(h.cam.value(), "camera re-enabled on return");
    assert!(!c.is_protecting());

    // Every action was announced on the event bus, in order.
    let kinds: Vec<&str> = h
        .events()
        .iter()
        .map(|e| match e {
            DomainEvent::PresenceChanged { .. } => "presence",
            DomainEvent::DeviceProtected { .. } => "protected",
            DomainEvent::DeviceRestored { .. } => "restored",
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "presence", // away
            "protected",
            "protected", // mic, camera
            "presence",  // present
            "restored",
            "restored", // mic, camera
        ]
    );
}

#[test]
fn flickering_face_edges_do_not_trip_protection_early() {
    // A dropped frame or two (leaving) that recovers before the away grace must
    // never mute — the debounce in the domain absorbs the noise even though the
    // sidecar reported transitional phases.
    let (mut c, h) = build(all_on(), false, true);
    h.at(0);
    c.set_enabled(true);

    feed_line(&mut c, &presence_line("leaving", 0)); // face blips out
    h.at(400);
    feed_line(&mut c, &presence_line("returning", 400)); // and back
    feed_line(&mut c, &presence_line("present", 400));
    h.at(2_000);
    feed_line(&mut c, &presence_line("present", 2_000));

    assert!(!c.is_protecting(), "transient blips must not protect");
    assert!(!h.mic.value());
    assert!(h.cam.value());
}

#[test]
fn malformed_sidecar_lines_are_ignored_without_affecting_state() {
    let (mut c, h) = build(all_on(), false, true);
    h.at(0);
    c.set_enabled(true);

    // Garbage, a stray diagnostic on stdout, and a wrong-type message: all no-ops.
    feed_line(&mut c, "not json at all");
    feed_line(&mut c, "detector started at 15.0 FPS");
    feed_line(&mut c, r#"{"type":"heartbeat","state":"away","at_ms":1}"#);
    h.at(5_000);
    feed_line(&mut c, "");

    assert!(!c.is_protecting(), "no valid away report ever arrived");
    assert_eq!(h.mic.writes(), 0);
    assert_eq!(h.cam.writes(), 0);
}
