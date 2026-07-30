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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use amow_application::parse_line;
use amow_logger::Logger;

use crate::supervisor::FaceSink;

/// Interpreter used to launch the sidecar (default `python3`).
const ENV_PYTHON: &str = "AMOW_PRESENCE_PYTHON";
/// Directory containing the `amow_presence` package (default: the copy bundled
/// into the installer's resource dir, then alongside the exe, then
/// `presence-detector` in the working directory).
const ENV_DIR: &str = "AMOW_PRESENCE_DIR";
/// Set to `1`/`true` to skip the sidecar entirely and rely on manual input.
const ENV_DISABLE: &str = "AMOW_PRESENCE_DISABLE";

/// Owns the running sidecar process and the threads draining its output.
/// Dropping it terminates the sidecar and releases the camera.
pub struct PresenceBridge {
    child: Option<Child>,
    threads: Vec<JoinHandle<()>>,
    /// True while the sidecar process is running and streaming samples — i.e.
    /// presence is being driven automatically by the webcam rather than the
    /// manual toggle. Set when the process starts; cleared when its stdout
    /// closes (the process exited, crashed, or was killed).
    active: Arc<AtomicBool>,
}

impl PresenceBridge {
    /// Start the presence sidecar, feeding samples into `sink`.
    ///
    /// Always returns a handle — an inert one if the sidecar is disabled or
    /// fails to launch — so the caller never has to special-case the degraded
    /// path. Diagnostics go to `logger`.
    pub fn spawn(
        config_path: &Path,
        resource_dir: Option<PathBuf>,
        sink: FaceSink,
        logger: Arc<Logger>,
    ) -> Self {
        let mut bridge = Self {
            child: None,
            threads: Vec::new(),
            active: Arc::new(AtomicBool::new(false)),
        };

        if disabled() {
            logger.info("presence sidecar disabled via env; using manual input only");
            return bridge;
        }

        match bridge.try_spawn(config_path, resource_dir.as_deref(), sink, &logger) {
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
        resource_dir: Option<&Path>,
        sink: FaceSink,
        logger: &Arc<Logger>,
    ) -> std::io::Result<()> {
        let python = resolve_python();
        let dir = sidecar_dir(resource_dir);
        logger.info(&format!(
            "presence sidecar: python={python}, dir={}",
            dir.display()
        ));

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
            let active = self.active.clone();
            self.threads.push(thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if let Some(report) = parse_line(&line) {
                        sink.set(report.face_present());
                    }
                }
                // stdout closed: the sidecar exited (or crashed/was killed), so
                // presence is no longer coming from the webcam. Fall back to the
                // manual toggle as the live source.
                active.store(false, Ordering::SeqCst);
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
        // The process is up and its reader is draining stdout: presence is now
        // automatic. The reader thread clears this if the process later dies.
        self.active.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Whether presence is currently being driven automatically by the webcam
    /// sidecar (as opposed to the manual toggle). False when the sidecar is
    /// disabled, failed to start, or has since exited.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

fn disabled() -> bool {
    env::var(ENV_DISABLE)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Resolve the directory to run the sidecar from — the one holding the
/// `amow_presence` package so `python -m amow_presence` imports it.
///
/// Order of preference: an explicit `AMOW_PRESENCE_DIR`; the copy bundled into
/// the installer (under the Tauri resource dir); a `presence-detector` folder
/// beside the executable; finally the repo layout for `tauri dev` runs from the
/// project root.
fn sidecar_dir(resource_dir: Option<&Path>) -> PathBuf {
    if let Ok(dir) = env::var(ENV_DIR) {
        return PathBuf::from(dir);
    }
    if let Some(res) = resource_dir {
        let candidate = res.join("presence-detector");
        if candidate.is_dir() {
            return candidate;
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(candidate) = exe.parent().map(|p| p.join("presence-detector")) {
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    PathBuf::from("presence-detector")
}

/// Choose the Python interpreter to launch the sidecar with.
///
/// An explicit `AMOW_PRESENCE_PYTHON` always wins. Otherwise the app discovers
/// one itself — mirroring `run-with-presence.ps1` so a plain double-click works
/// without setting env vars. It prefers an interpreter that can actually import
/// the detector's dependencies (OpenCV + MediaPipe, which ship wheels only for
/// Python 3.9–3.12), falling back to the first interpreter that merely runs so
/// the sidecar can start and log a clear error rather than silently doing
/// nothing.
fn resolve_python() -> String {
    if let Ok(p) = env::var(ENV_PYTHON) {
        return p;
    }

    // Candidate launchers, best first: the Windows `py` launcher pinned to the
    // MediaPipe-supported versions, then the generic interpreter names.
    let candidates: &[&[&str]] = &[
        &["py", "-3.12"],
        &["py", "-3.11"],
        &["py", "-3.10"],
        &["py", "-3.9"],
        &["python3"],
        &["python"],
        &["py"],
    ];

    let mut first_runnable: Option<String> = None;
    for &cand in candidates {
        if let Some(exe) = python_executable(cand) {
            if first_runnable.is_none() {
                first_runnable = Some(exe.clone());
            }
            if deps_importable(&exe) {
                return exe;
            }
        }
    }
    first_runnable.unwrap_or_else(|| "python3".to_string())
}

/// Resolve a candidate launcher (e.g. `["py", "-3.12"]`) to the concrete
/// interpreter path it runs, so the app can invoke it directly (the `py`
/// launcher itself isn't re-invoked with the sidecar's args). `None` if the
/// candidate can't be run.
fn python_executable(cand: &[&str]) -> Option<String> {
    let out = Command::new(cand[0])
        .args(&cand[1..])
        .args(["-c", "import sys; print(sys.executable)"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Whether the detector's Python dependencies import in the given interpreter.
fn deps_importable(exe: &str) -> bool {
    Command::new(exe)
        .args(["-c", "import cv2, mediapipe"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
