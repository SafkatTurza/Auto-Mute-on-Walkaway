//! Presence bridge: connects the webcam sidecar to the controller.
//!
//! Presence detection runs in a separate process — the Python OpenCV +
//! MediaPipe sidecar (see `presence-detector/`) — because those libraries have
//! no production-grade Rust binding. This module is the host end of that link:
//! it spawns the sidecar, reads its newline-delimited JSON events from stdout,
//! translates each into a face-presence sample with the tested
//! [`amow_application::parse_line`] contract, and pushes it at the supervisor.
//!
//! It carries **no policy**: it neither debounces nor decides to mute anything.
//! Every decision stays in the controller, driven off the event bus. The bridge
//! only reports what the detector saw — keeping presence detection fully
//! decoupled from device control, as the architecture requires.
//!
//! Failure is non-fatal by design. If the sidecar cannot start (no Python, no
//! OpenCV/MediaPipe, no camera), the app logs it and keeps running on the manual
//! presence input; a crashed sidecar simply stops producing samples. The webcam
//! feed never leaves the machine — only presence phases cross the process line.

use std::env;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use amow_application::parse_line;
use amow_logger::Logger;

use crate::supervisor::FaceSink;

/// Interpreter used to launch the sidecar (default `python3`).
const ENV_PYTHON: &str = "AMOW_PRESENCE_PYTHON";
/// Directory containing the `amow_presence` package (default: alongside the exe,
/// then `presence-detector` in the working directory).
const ENV_DIR: &str = "AMOW_PRESENCE_DIR";
/// Set to `1`/`true` to skip the sidecar entirely and rely on manual input.
const ENV_DISABLE: &str = "AMOW_PRESENCE_DISABLE";

/// Owns the running sidecar process and the threads draining its output.
/// Dropping it terminates the sidecar and releases the camera.
pub struct PresenceBridge {
    child: Option<Child>,
    threads: Vec<JoinHandle<()>>,
}

impl PresenceBridge {
    /// Start the presence sidecar, feeding samples into `sink`.
    ///
    /// Always returns a handle — an inert one if the sidecar is disabled or
    /// fails to launch — so the caller never has to special-case the degraded
    /// path. Diagnostics go to `logger`.
    pub fn spawn(config_path: &Path, sink: FaceSink, logger: Arc<Logger>) -> Self {
        let mut bridge = Self {
            child: None,
            threads: Vec::new(),
        };

        if disabled() {
            logger.info("presence sidecar disabled via env; using manual input only");
            return bridge;
        }

        match bridge.try_spawn(config_path, sink, &logger) {
            Ok(()) => logger.info("presence sidecar started"),
            Err(e) => logger.warn(&format!(
                "presence sidecar unavailable ({e}); presence falls back to manual input"
            )),
        }
        bridge
    }

    fn try_spawn(
        &mut self,
        config_path: &Path,
        sink: FaceSink,
        logger: &Arc<Logger>,
    ) -> std::io::Result<()> {
        let python = env::var(ENV_PYTHON).unwrap_or_else(|_| "python3".to_string());
        let dir = sidecar_dir();

        let mut child = Command::new(&python)
            .args(["-m", "amow_presence", "--log-level", "warning", "--config"])
            .arg(config_path)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // stdout carries the event stream: parse each line and, if it is a valid
        // presence report, push the mapped face sample. Malformed lines are
        // dropped by the parser, so one bad line never breaks the pipeline.
        if let Some(stdout) = child.stdout.take() {
            self.threads.push(thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if let Some(report) = parse_line(&line) {
                        sink.set(report.face_present());
                    }
                }
            }));
        }

        // stderr carries only the sidecar's own status text (never media); mirror
        // it into the app log so a headless failure is diagnosable.
        if let Some(stderr) = child.stderr.take() {
            let logger = logger.clone();
            self.threads.push(thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if !line.trim().is_empty() {
                        logger.info(&format!("presence: {line}"));
                    }
                }
            }));
        }

        self.child = Some(child);
        Ok(())
    }
}

fn disabled() -> bool {
    env::var(ENV_DISABLE)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Resolve the directory to run the sidecar from — the one holding the
/// `amow_presence` package so `python -m amow_presence` imports it.
fn sidecar_dir() -> PathBuf {
    if let Ok(dir) = env::var(ENV_DIR) {
        return PathBuf::from(dir);
    }
    // Prefer a `presence-detector` folder shipped beside the executable; fall
    // back to the repo layout for `tauri dev` runs from the project root.
    if let Ok(exe) = env::current_exe() {
        if let Some(candidate) = exe.parent().map(|p| p.join("presence-detector")) {
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    PathBuf::from("presence-detector")
}

impl Drop for PresenceBridge {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // The kill closes the pipes, ending the reader loops so the threads join.
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}
