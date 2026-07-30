//! Persistence for the camera devices the app has disabled.
//!
//! The camera adapters remember, *in memory*, exactly which device ids they
//! disabled so they can re-enable precisely those. That in-memory set is lost if
//! the process dies unexpectedly (a crash, a kill, power loss) while the camera
//! is off — and the OS keeps the device disabled across the restart, so the
//! webcam would stay off with nothing left that knows to restore it.
//!
//! A [`CameraJournal`] closes that gap: the adapter writes its disabled-id set to
//! stable storage the moment it disables a camera, and clears it once restored.
//! On the next launch the composition root asks the adapter to
//! [`recover`](crate::WindowsCamera::recover) — re-enabling exactly the ids the
//! journal still lists, and never a camera the user disabled themselves.
//!
//! The record is a tiny list of opaque device instance ids — no media, no
//! personal data — consistent with the app's privacy contract.

use std::path::PathBuf;

/// Stable storage for the set of camera device ids currently disabled by the app.
///
/// Kept deliberately small: load the recorded set, or overwrite it. Saving an
/// empty set clears the record entirely (a clean state leaves nothing behind).
pub trait CameraJournal: Send + Sync {
    /// The device ids recorded as "disabled by us and not yet restored".
    fn load(&self) -> Vec<String>;
    /// Replace the recorded set. An empty slice clears the record.
    fn save(&self, ids: &[String]);
}

/// A journal that persists nothing — the default for adapters constructed
/// without recovery (tests, and any backend where crash-recovery is not wired).
pub struct NullCameraJournal;

impl CameraJournal for NullCameraJournal {
    fn load(&self) -> Vec<String> {
        Vec::new()
    }
    fn save(&self, _ids: &[String]) {}
}

/// A file-backed [`CameraJournal`] storing one device id per line.
///
/// Newline-delimited plain text keeps it dependency-free and trivially
/// inspectable; device instance ids never contain newlines. All I/O errors are
/// swallowed on purpose — the journal is a best-effort safety net and must never
/// take down the app or block a device toggle if the disk misbehaves.
pub struct FileCameraJournal {
    path: PathBuf,
}

impl FileCameraJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl CameraJournal for FileCameraJournal {
    fn load(&self) -> Vec<String> {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn save(&self, ids: &[String]) {
        if ids.is_empty() {
            // Nothing to restore: remove the record so a later launch recovers
            // nothing. A missing file is the "clean" state.
            let _ = std::fs::remove_file(&self.path);
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, ids.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_journal_round_trips_ids() {
        let dir = std::env::temp_dir().join(format!("amow-journal-{}", std::process::id()));
        let path = dir.join("camera-recovery.txt");
        let journal = FileCameraJournal::new(&path);

        assert!(journal.load().is_empty(), "no file yet → empty");

        journal.save(&["cam-a".to_string(), "cam-b".to_string()]);
        assert_eq!(
            journal.load(),
            vec!["cam-a".to_string(), "cam-b".to_string()]
        );

        // Saving an empty set clears the record.
        journal.save(&[]);
        assert!(journal.load().is_empty(), "cleared → empty");
        assert!(!path.exists(), "clean state removes the file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn null_journal_never_persists() {
        let journal = NullCameraJournal;
        journal.save(&["cam-a".to_string()]);
        assert!(journal.load().is_empty());
    }
}
