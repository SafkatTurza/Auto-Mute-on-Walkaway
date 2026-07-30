//! Background supervisor: owns the [`WalkawayController`] on a dedicated thread.
//!
//! The controller uses `&mut self` and is intentionally single-threaded. This
//! module gives it a home: one thread that (a) samples the current inputs at
//! the configured interval and (b) applies control messages from the UI. The
//! rest of the app talks to it only through a channel, keeping all mutation on
//! one thread with no locking around the controller itself.
//!
//! Presence samples and the enable switch are held as plain values updated by
//! messages. Presence comes from the webcam sidecar (or the UI's manual toggle
//! as a fallback); the enable switch is the user's master on/off. Both push
//! into the very same channel — the sampling model does not change.

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
    SetEnabled(bool),
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

    /// Turn walkaway protection on or off (the user's master switch).
    pub fn set_enabled(&self, enabled: bool) {
        let _ = self.tx.send(Msg::SetEnabled(enabled));
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

    // Initial inputs match the controller's initial state: present, protection
    // off until the user enables it.
    let mut face_present = true;
    let mut enabled = false;

    logger.info("supervisor started");

    loop {
        match rx.recv_timeout(interval) {
            Ok(Msg::SetEnabled(on)) => {
                enabled = on;
                logger.info(if on {
                    "protection enabled"
                } else {
                    "protection disabled"
                });
            }
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

        // Feed the current inputs through the domain each cycle. Both calls are
        // no-ops in steady state (edge-detected), so idle cost stays low; the
        // debounce timing is driven by the real monotonic clock in the controller.
        controller.set_enabled(enabled);
        controller.on_face_sample(face_present);

        status.update(&controller);
    }
}

fn sample_interval(config: &AppConfig) -> Duration {
    Duration::from_millis(config.behavior.sample_interval_ms.max(1))
}
