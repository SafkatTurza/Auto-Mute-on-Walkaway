//! Input-activity monitor: a camera-free "the user is back" signal.
//!
//! When the app disables the camera on walkaway, the webcam sidecar goes blind —
//! a disabled device yields no frames — so it can no longer see the user return,
//! and the mic/camera would stay off indefinitely. This monitor breaks that
//! deadlock: while the camera is disabled, any recent keyboard/mouse activity is
//! reported as presence, which drives the controller to restore the devices (and
//! the webcam then resumes as usual).
//!
//! It reads only *idle time* (seconds since the last input) via the adapter
//! port — never keystrokes — and it only ever reports **present**, never away.
//! Absence of input is not proof of absence; that judgement stays with the
//! webcam. So this can only help the user come back, never mute them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use amow_adapters::{default_idle_time, should_signal_return, IdleTime};
use amow_logger::Logger;

use crate::status::SharedStatus;
use crate::supervisor::FaceSink;

/// How often to check for input activity.
const POLL: Duration = Duration::from_millis(750);
/// Activity within this window counts as "the user is here".
const ACTIVITY_THRESHOLD_MS: u64 = 1_500;

/// Owns the monitor thread; dropping it stops and joins the thread.
pub struct ActivityMonitor {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ActivityMonitor {
    /// Start watching input activity, pushing presence pulses into `sink` while
    /// `status` reports the camera as disabled.
    pub fn spawn(sink: FaceSink, status: SharedStatus, logger: Arc<Logger>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let handle = thread::Builder::new()
            .name("amow-activity".into())
            .spawn(move || {
                let idle = default_idle_time();
                logger.info(
                    "activity monitor started (keyboard/mouse marks return while camera is off)",
                );
                while !stop_thread.load(Ordering::SeqCst) {
                    let camera_disabled = status.snapshot().camera_off;
                    if should_signal_return(camera_disabled, idle.idle_ms(), ACTIVITY_THRESHOLD_MS)
                    {
                        // Camera is disabled (webcam blind) but the user is active
                        // at the keyboard/mouse: report present so the controller
                        // restores the devices and the webcam takes over again.
                        sink.set(true);
                    }
                    thread::sleep(POLL);
                }
            })
            .expect("failed to spawn activity monitor thread");
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for ActivityMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
