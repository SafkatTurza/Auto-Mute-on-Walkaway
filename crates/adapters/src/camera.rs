use std::sync::Mutex;

use amow_application::{Camera, PortError, PortResult};

use crate::sysfs::{RealSysfs, Sysfs};

/// Camera adapter for platforms without a portable capture-mute mechanism.
///
/// A safe, honest fallback used when no real backend is available (non-Linux
/// hosts, or a Linux box with no UVC webcam). [`WalkawayController`] treats a
/// failing camera port as "leave the camera alone", so `auto_camera_off` simply
/// no-ops here while the microphone is still protected.
///
/// [`WalkawayController`]: amow_application::WalkawayController
pub struct UnsupportedCamera;

const REASON: &str = "camera control is not supported on this platform";

impl Camera for UnsupportedCamera {
    fn is_enabled(&self) -> PortResult<bool> {
        Err(PortError::new(REASON))
    }

    fn set_enabled(&self, _enabled: bool) -> PortResult<()> {
        Err(PortError::new(REASON))
    }
}

/// The sysfs directory of the Linux USB Video Class driver. Every USB webcam's
/// interfaces are bound to it; unbinding them is the reversible, OS-level way to
/// switch the camera off for *all* applications at once.
const UVC_DRIVER_DIR: &str = "/sys/bus/usb/drivers/uvcvideo";

/// Real Linux camera controller: enables/disables the webcam by binding and
/// unbinding it from the `uvcvideo` kernel driver via sysfs.
///
/// This is a genuine OS-level control, not a per-application hint: an unbound
/// device disappears from `/dev/video*`, so no conferencing app can capture from
/// it until it is rebound. Unlike `modprobe -r uvcvideo` it works per-device and
/// even while the camera is in use, which is exactly the walkaway case.
///
/// Because "enabled" is not a single writable flag but a set of device
/// bindings, the adapter remembers which interfaces it unbound so it can restore
/// precisely those on re-enable — it never rebinds devices it did not touch.
///
/// Writing to the driver's `bind`/`unbind` files requires elevated privileges.
/// Where the app lacks them the writes fail and this degrades to the same safe
/// behaviour as [`UnsupportedCamera`]: the controller logs the error and leaves
/// the camera alone while still muting the mic.
pub struct LinuxUvcCamera<F: Sysfs = RealSysfs> {
    fs: F,
    driver_dir: String,
    /// Interface ids this adapter unbound and must rebind to restore.
    unbound: Mutex<Vec<String>>,
}

impl Default for LinuxUvcCamera<RealSysfs> {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxUvcCamera<RealSysfs> {
    /// Controller for the system's UVC webcams using real sysfs access.
    pub fn new() -> Self {
        Self::with_sysfs(RealSysfs, UVC_DRIVER_DIR)
    }
}

impl<F: Sysfs> LinuxUvcCamera<F> {
    /// Construct with an explicit sysfs backend and driver directory — the seam
    /// tests use to drive the logic without a real `/sys`.
    pub fn with_sysfs(fs: F, driver_dir: impl Into<String>) -> Self {
        Self {
            fs,
            driver_dir: driver_dir.into(),
            unbound: Mutex::new(Vec::new()),
        }
    }

    fn bind_path(&self) -> String {
        format!("{}/bind", self.driver_dir)
    }

    fn unbind_path(&self) -> String {
        format!("{}/unbind", self.driver_dir)
    }

    /// USB interface ids currently bound to the driver.
    ///
    /// The driver directory holds one symlink per bound interface (named like
    /// `1-1:1.0`) alongside control files (`bind`, `unbind`, `uevent`, …). Only
    /// the interface entries contain a `:`, which cleanly separates them.
    fn bound_ids(&self) -> PortResult<Vec<String>> {
        let mut ids: Vec<String> = self
            .fs
            .list_dir(&self.driver_dir)?
            .into_iter()
            .filter(|name| name.contains(':'))
            .collect();
        ids.sort();
        Ok(ids)
    }

    fn disable(&self) -> PortResult<()> {
        let ids = self.bound_ids()?;
        if ids.is_empty() {
            // Nothing bound: the camera is already off. Not an error.
            return Ok(());
        }

        let unbind_path = self.unbind_path();
        let mut newly_unbound = Vec::new();
        let mut last_err = None;
        for id in ids {
            match self.fs.write(&unbind_path, &id) {
                Ok(()) => newly_unbound.push(id),
                Err(e) => last_err = Some(e),
            }
        }

        if newly_unbound.is_empty() {
            // Nothing could be unbound — surface the failure (e.g. no privilege)
            // so the controller records no protection and does not try to restore.
            return Err(last_err.unwrap_or_else(|| PortError::new("no uvc interface unbound")));
        }
        // At least one interface went down; treat the camera as disabled so it is
        // restored later, even if another interface failed to unbind.
        self.unbound
            .lock()
            .expect("camera mutex poisoned")
            .extend(newly_unbound);
        Ok(())
    }

    fn enable(&self) -> PortResult<()> {
        let ids = {
            let mut guard = self.unbound.lock().expect("camera mutex poisoned");
            std::mem::take(&mut *guard)
        };
        if ids.is_empty() {
            // We never disabled anything: nothing to restore.
            return Ok(());
        }

        let bind_path = self.bind_path();
        let mut failed = Vec::new();
        let mut last_err = None;
        for id in ids {
            if let Err(e) = self.fs.write(&bind_path, &id) {
                last_err = Some(e);
                failed.push(id);
            }
        }

        if failed.is_empty() {
            return Ok(());
        }
        // Keep the ids we could not rebind so a later cycle can retry.
        self.unbound
            .lock()
            .expect("camera mutex poisoned")
            .extend(failed);
        Err(last_err.unwrap_or_else(|| PortError::new("no uvc interface rebound")))
    }
}

impl<F: Sysfs> Camera for LinuxUvcCamera<F> {
    fn is_enabled(&self) -> PortResult<bool> {
        Ok(!self.bound_ids()?.is_empty())
    }

    fn set_enabled(&self, enabled: bool) -> PortResult<()> {
        if enabled {
            self.enable()
        } else {
            self.disable()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_reports_unavailable_rather_than_lying() {
        let cam = UnsupportedCamera;
        assert!(cam.is_enabled().is_err());
        assert!(cam.set_enabled(false).is_err());
    }

    const DRIVER: &str = "/sys/bus/usb/drivers/uvcvideo";

    /// In-memory sysfs. The driver directory's contents track bind/unbind writes
    /// so a full disable→enable cycle can be exercised end to end.
    struct FakeSysfs {
        dir: Mutex<Vec<String>>,
        writes: Mutex<Vec<(String, String)>>,
        fail_writes: bool,
        missing_dir: bool,
    }
    impl FakeSysfs {
        fn with(entries: &[&str]) -> Self {
            Self {
                dir: Mutex::new(entries.iter().map(|s| s.to_string()).collect()),
                writes: Mutex::new(Vec::new()),
                fail_writes: false,
                missing_dir: false,
            }
        }
        fn failing(entries: &[&str]) -> Self {
            Self {
                fail_writes: true,
                ..Self::with(entries)
            }
        }
        fn missing() -> Self {
            Self {
                missing_dir: true,
                ..Self::with(&[])
            }
        }
        fn writes(&self) -> Vec<(String, String)> {
            self.writes.lock().unwrap().clone()
        }
    }
    impl Sysfs for FakeSysfs {
        fn list_dir(&self, path: &str) -> PortResult<Vec<String>> {
            if self.missing_dir {
                return Err(PortError::new(format!("no such directory: {path}")));
            }
            Ok(self.dir.lock().unwrap().clone())
        }
        fn write(&self, path: &str, value: &str) -> PortResult<()> {
            if self.fail_writes {
                return Err(PortError::new("permission denied"));
            }
            self.writes
                .lock()
                .unwrap()
                .push((path.to_string(), value.to_string()));
            let mut dir = self.dir.lock().unwrap();
            if path.ends_with("/unbind") {
                dir.retain(|e| e != value);
            } else if path.ends_with("/bind") {
                dir.push(value.to_string());
            }
            Ok(())
        }
    }

    fn cam(fs: FakeSysfs) -> LinuxUvcCamera<FakeSysfs> {
        LinuxUvcCamera::with_sysfs(fs, DRIVER)
    }

    #[test]
    fn is_enabled_true_when_interfaces_are_bound() {
        // Control files must be ignored; only `:`-bearing interface names count.
        let c = cam(FakeSysfs::with(&[
            "1-1:1.0", "1-1:1.1", "bind", "unbind", "uevent", "module",
        ]));
        assert!(c.is_enabled().unwrap());
    }

    #[test]
    fn is_enabled_false_when_only_control_files_present() {
        let c = cam(FakeSysfs::with(&["bind", "unbind", "uevent"]));
        assert!(!c.is_enabled().unwrap());
    }

    #[test]
    fn disable_then_enable_round_trips_the_camera_state() {
        let c = cam(FakeSysfs::with(&["1-1:1.0", "1-1:1.1", "bind", "unbind"]));
        assert!(c.is_enabled().unwrap());

        c.set_enabled(false).unwrap();
        assert!(!c.is_enabled().unwrap(), "camera should read disabled");

        c.set_enabled(true).unwrap();
        assert!(c.is_enabled().unwrap(), "camera should read enabled again");
    }

    #[test]
    fn enable_only_rebinds_exactly_what_was_unbound() {
        let c = cam(FakeSysfs::with(&["1-1:1.0", "1-1:1.1", "bind", "unbind"]));
        c.set_enabled(false).unwrap();
        c.set_enabled(true).unwrap();

        let writes = c.fs.writes();
        let unbinds: Vec<&str> = writes
            .iter()
            .filter(|(p, _)| p.ends_with("/unbind"))
            .map(|(_, v)| v.as_str())
            .collect();
        let binds: Vec<&str> = writes
            .iter()
            .filter(|(p, _)| p.ends_with("/bind"))
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(unbinds, vec!["1-1:1.0", "1-1:1.1"]);
        assert_eq!(binds, vec!["1-1:1.0", "1-1:1.1"]);
    }

    #[test]
    fn disable_with_nothing_bound_is_ok_noop() {
        let c = cam(FakeSysfs::with(&["bind", "unbind"]));
        assert!(c.set_enabled(false).is_ok());
    }

    #[test]
    fn enable_without_prior_disable_touches_nothing() {
        let c = cam(FakeSysfs::with(&["1-1:1.0", "bind", "unbind"]));
        // No unbound ids remembered; restoring must not write to the device.
        assert!(c.set_enabled(true).is_ok());
        assert!(c.fs.writes().is_empty());
    }

    #[test]
    fn write_failure_surfaces_so_controller_skips_protection() {
        let c = cam(FakeSysfs::failing(&[
            "1-1:1.0", "1-1:1.1", "bind", "unbind",
        ]));
        assert!(c.set_enabled(false).is_err());
    }

    #[test]
    fn missing_driver_dir_is_reported_as_error() {
        let c = cam(FakeSysfs::missing());
        assert!(c.is_enabled().is_err());
        assert!(c.set_enabled(false).is_err());
    }
}
