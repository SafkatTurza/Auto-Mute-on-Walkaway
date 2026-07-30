//! Background supervisor: owns the [`WalkawayController`] on a dedicated thread.
//!
//! The controller uses `&mut self` and is intentionally single-threaded. This
//! module gives it a home: one thread that (a) samples the current inputs at
//! the configured interval and (b) applies control messages from the UI. The
//! rest of the app talks to it only through a channel, keeping all mutation on
//! one thread with no locking around the controller itself.
//!
//! Presence and meeting inputs are held as plain values updated by messages.
//! Today those messages come from the UI's manual toggles; when the webcam
//! presence detector and meeting monitor land, they will push into the very
//! same channel — the sampling model does not change.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use amow_adapters::{LinuxUvcCamera, PulseMicrophone, SystemClock, SystemCommandRunner};
use amow_application::WalkawayController;
use amow_config::AppConfig;
use amow_eventbus::EventBus;
use amow_logger::Logger;

use crate::notifier::AppNotifier;
use crate::status::SharedStatus;

/// Concrete controller type wired to the real OS adapters.
type Controller =
    WalkawayController<SystemClock, PulseMicrophone<SystemCommandRunner>, LinuxUvcCamera, AppNotifier>;

/// Control messages sent to the supervisor thread.
enum Msg {
    SetMeeting(bool),
    SetFace(bool),
    UpdateConfig(Box<AppConfig>),
    Shutdown,
}

/// Handle to the running supervisor thread. Dropping it shuts the thread down.
pub struct Supervisor {
    tx: Sender<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl Supervisor {
    /// Spawn the supervisor thread with an initial configuration.
    pub fn spawn(
        config: AppConfig,
        bus: EventBus,
        notifier: AppNotifier,
        status: SharedStatus,
        logger: Arc<Logger>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("amow-supervisor".into())
            .spawn(move || run(config, bus, notifier, status, logger, rx))
            .expect("failed to spawn supervisor thread");
        Self {
            tx,
            handle: Some(handle),
        }
    }

    /// Report whether a meeting is currently active.
    pub fn set_meeting(&self, active: bool) {
        let _ = self.tx.send(Msg::SetMeeting(active));
    }

    /// Report whether the user's face is currently present.
    pub fn set_face(&self, present: bool) {
        let _ = self.tx.send(Msg::SetFace(present));
    }

    /// A cheap, cloneable sink for face-presence samples.
    ///
    /// Handed to the presence bridge so an external source (the webcam sidecar)
    /// can feed samples into the controller without touching the supervisor's
    /// internal message channel. Sending the raw sample keeps *all* debounce and
    /// policy inside the controller — the sink carries no logic.
    pub fn face_sink(&self) -> FaceSink {
        FaceSink {
            tx: self.tx.clone(),
        }
    }

    /// Apply an updated configuration at runtime.
    pub fn update_config(&self, config: AppConfig) {
        let _ = self.tx.send(Msg::UpdateConfig(Box::new(config)));
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// A write-only handle for pushing face-presence samples at the supervisor.
///
/// Opaque on purpose: it exposes only `set`, keeping the message protocol
/// private while letting the presence bridge live in its own module.
#[derive(Clone)]
pub struct FaceSink {
    tx: Sender<Msg>,
}

impl FaceSink {
    /// Report the latest face-presence sample. A closed channel (app shutting
    /// down) is ignored — there is nothing left to protect.
    pub fn set(&self, present: bool) {
        let _ = self.tx.send(Msg::SetFace(present));
    }
}

fn build_controller(config: &AppConfig, bus: EventBus, notifier: AppNotifier) -> Controller {
    WalkawayController::new(
        config.behavior,
        config.presence,
        SystemClock::new(),
        PulseMicrophone::system(),
        LinuxUvcCamera::new(),
        notifier,
        bus,
    )
}

fn run(
    mut config: AppConfig,
    bus: EventBus,
    notifier: AppNotifier,
    status: SharedStatus,
    logger: Arc<Logger>,
    rx: Receiver<Msg>,
) {
    let mut controller = build_controller(&config, bus, notifier);
    let mut interval = sample_interval(&config);

    // Initial inputs match the trackers' initial state: present, no meeting.
    let mut face_present = true;
    let mut meeting_active = false;

    logger.info("supervisor started");

    loop {
        match rx.recv_timeout(interval) {
            Ok(Msg::SetMeeting(active)) => meeting_active = active,
            Ok(Msg::SetFace(present)) => face_present = present,
            Ok(Msg::UpdateConfig(new_config)) => {
                config = *new_config;
                controller.set_behavior(config.behavior);
                controller.set_presence_config(config.presence);
                interval = sample_interval(&config);
                logger.info("configuration applied");
            }
            Ok(Msg::Shutdown) => {
                logger.info("supervisor stopping");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // Feed the current inputs through the domain each cycle. `observe`
        // reports only edges, so steady state is cheap; the debounce timing is
        // driven by the real monotonic clock inside the controller.
        controller.on_meeting_sample(meeting_active);
        controller.on_face_sample(face_present);

        status.update(&controller);
    }
}

fn sample_interval(config: &AppConfig) -> Duration {
    Duration::from_millis(config.behavior.sample_interval_ms.max(1))
}
