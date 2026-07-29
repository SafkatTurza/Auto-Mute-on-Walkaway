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
}
impl DeviceCell {
    fn new(v: bool) -> Self {
        Self {
            value: AtomicBool::new(v),
            writes: AtomicU32::new(0),
            fail: AtomicBool::new(false),
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
        if self.0.fail.load(REL) {
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
        if self.0.fail.load(REL) {
            return Err(PortError::new("cam fail"));
        }
        self.0.value.store(enabled, REL);
        self.0.writes.fetch_add(1, REL);
        Ok(())
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

/// Drive a controller from "in meeting, present" to "away" past the grace.
fn walk_away(c: &mut TestController, h: &Handles) {
    h.at(0);
    c.on_meeting_sample(true);
    c.on_face_sample(false);
    h.at(1_000);
    c.on_face_sample(false); // presence -> Away, triggers protection
}

// --- Tests --------------------------------------------------------------------

#[test]
fn mutes_and_disables_on_walkaway_during_meeting() {
    let (mut c, h) = build(BehaviorConfig::default(), false, true);
    walk_away(&mut c, &h);
    assert!(h.mic.value(), "mic should be muted");
    assert!(!h.cam.value(), "camera should be disabled");
    assert!(c.is_protecting());
}

#[test]
fn does_nothing_when_no_meeting() {
    let (mut c, h) = build(BehaviorConfig::default(), false, true);
    h.at(0);
    c.on_face_sample(false);
    h.at(1_000);
    c.on_face_sample(false);
    assert!(!c.is_protecting());
    assert!(!h.mic.value());
    assert!(h.cam.value());
}

#[test]
fn restores_previous_state_on_return() {
    let (mut c, h) = build(BehaviorConfig::default(), false, true);
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
    let (mut c, h) = build(BehaviorConfig::default(), true, true);
    walk_away(&mut c, &h);
    h.at(1_000);
    c.on_face_sample(true);
    h.at(1_500);
    c.on_face_sample(true);
    assert!(h.mic.value(), "user's manual mute must be preserved");
    assert_eq!(h.mic.writes(), 0, "controller must not write the mic");
}

#[test]
fn restores_when_meeting_ends_while_away() {
    let (mut c, h) = build(BehaviorConfig::default(), false, true);
    walk_away(&mut c, &h);
    h.at(1_000);
    c.on_meeting_sample(false);
    assert!(!c.is_protecting());
    assert!(!h.mic.value(), "mic restored on meeting end");
    assert!(h.cam.value(), "camera restored on meeting end");
}

#[test]
fn respects_disabled_auto_restore() {
    let behavior = BehaviorConfig {
        auto_restore: false,
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
    let (mut c, h) = build(BehaviorConfig::default(), false, true);
    h.mic.set_fail();
    walk_away(&mut c, &h);
    assert!(!h.cam.value(), "camera should still be disabled");
    assert!(c.is_protecting());
}

#[test]
fn emits_expected_event_sequence() {
    let (mut c, h) = build(BehaviorConfig::default(), false, true);
    walk_away(&mut c, &h);
    let kinds: Vec<&str> = h
        .events()
        .iter()
        .map(|e| match e {
            DomainEvent::MeetingChanged { .. } => "meeting",
            DomainEvent::PresenceChanged { .. } => "presence",
            DomainEvent::DeviceProtected { .. } => "protected",
            DomainEvent::DeviceRestored { .. } => "restored",
        })
        .collect();
    assert_eq!(kinds, vec!["meeting", "presence", "protected", "protected"]);
}

#[test]
fn notifies_on_action_when_enabled() {
    let (mut c, h) = build(BehaviorConfig::default(), false, true);
    walk_away(&mut c, &h);
    assert_eq!(
        h.notifications(),
        2,
        "one notification per protected device"
    );
}
