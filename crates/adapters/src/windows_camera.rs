//! Native Windows camera adapter.
//!
//! Disables and re-enables the webcam at the OS level by toggling its device
//! node through SetupAPI / Configuration Manager — the same operation as Device
//! Manager's "Disable device". A disabled camera is unavailable to *every*
//! application until re-enabled, which is the genuine walkaway guarantee (not a
//! per-app hint), and it is fully reversible.
//!
//! This mirrors the Linux [`LinuxUvcCamera`](crate::LinuxUvcCamera) design: the
//! adapter remembers exactly which camera devices it disabled and re-enables
//! only those, so it never re-enables a camera the user had turned off
//! themselves. The SetupAPI calls are isolated behind the [`CameraDevices`]
//! seam, so the disable/restore bookkeeping is unit-tested against a fake while
//! the only `windows`-crate code stays in one Windows-only module.
//!
//! Toggling a device's state requires elevated privileges. Where the app lacks
//! them the calls fail and this degrades to the same safe behaviour as
//! [`UnsupportedCamera`](crate::UnsupportedCamera): the controller logs the
//! error and leaves the camera alone while still muting the mic.

use std::sync::Mutex;

use amow_application::{Camera, PortError, PortResult};

/// Enumeration and enable/disable of the machine's camera devices.
///
/// A port so the SetupAPI/CfgMgr plumbing is the only platform-specific part;
/// [`WindowsCamera`]'s logic is exercised through an in-memory fake.
pub trait CameraDevices: Send + Sync {
    /// Instance ids of camera devices that are currently *enabled and present*.
    fn enabled(&self) -> PortResult<Vec<String>>;

    /// Enable or disable the camera device with the given instance id.
    fn set_enabled(&self, id: &str, enabled: bool) -> PortResult<()>;
}

/// Real Windows camera controller: disables/enables webcams via their device
/// nodes. Generic over [`CameraDevices`] for the same static-dispatch,
/// fully-testable shape as the other adapters.
pub struct WindowsCamera<D: CameraDevices> {
    devices: D,
    /// Instance ids this adapter disabled and must re-enable to restore.
    disabled: Mutex<Vec<String>>,
}

#[cfg(target_os = "windows")]
impl Default for WindowsCamera<SetupApiCameras> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl WindowsCamera<SetupApiCameras> {
    /// Controller for the system's cameras using real SetupAPI access.
    pub fn new() -> Self {
        Self::with_devices(SetupApiCameras::new())
    }
}

impl<D: CameraDevices> WindowsCamera<D> {
    /// Construct with an explicit device backend — the seam tests use.
    pub fn with_devices(devices: D) -> Self {
        Self {
            devices,
            disabled: Mutex::new(Vec::new()),
        }
    }

    fn disable(&self) -> PortResult<()> {
        let ids = self.devices.enabled()?;
        if ids.is_empty() {
            // Nothing enabled: the camera is already off. Not an error.
            return Ok(());
        }

        let mut newly_disabled = Vec::new();
        let mut last_err = None;
        for id in ids {
            match self.devices.set_enabled(&id, false) {
                Ok(()) => newly_disabled.push(id),
                Err(e) => last_err = Some(e),
            }
        }

        if newly_disabled.is_empty() {
            // Nothing could be disabled — surface the failure (e.g. no privilege)
            // so the controller records no protection and does not try to restore.
            return Err(last_err.unwrap_or_else(|| PortError::new("no camera disabled")));
        }
        // At least one camera went down; treat the camera as disabled so it is
        // restored later, even if another device failed to toggle.
        self.disabled
            .lock()
            .expect("camera mutex poisoned")
            .extend(newly_disabled);
        Ok(())
    }

    fn enable(&self) -> PortResult<()> {
        let ids = {
            let mut guard = self.disabled.lock().expect("camera mutex poisoned");
            std::mem::take(&mut *guard)
        };
        if ids.is_empty() {
            // We never disabled anything: nothing to restore.
            return Ok(());
        }

        let mut failed = Vec::new();
        let mut last_err = None;
        for id in ids {
            if let Err(e) = self.devices.set_enabled(&id, true) {
                last_err = Some(e);
                failed.push(id);
            }
        }

        if failed.is_empty() {
            return Ok(());
        }
        // Keep the ids we could not re-enable so a later cycle can retry.
        self.disabled
            .lock()
            .expect("camera mutex poisoned")
            .extend(failed);
        Err(last_err.unwrap_or_else(|| PortError::new("no camera re-enabled")))
    }
}

impl<D: CameraDevices> Camera for WindowsCamera<D> {
    fn is_enabled(&self) -> PortResult<bool> {
        Ok(!self.devices.enabled()?.is_empty())
    }

    fn set_enabled(&self, enabled: bool) -> PortResult<()> {
        if enabled {
            self.enable()
        } else {
            self.disable()
        }
    }
}

// --- Real SetupAPI backend (Windows only) ------------------------------------
//
// Only this module touches the `windows` crate; it is compiled solely on Windows
// (the dependency is target-gated in Cargo.toml). The logic above is portable
// and unit-tested on Linux against a fake device backend.

#[cfg(target_os = "windows")]
mod real {
    use super::{CameraDevices, PortError, PortResult};

    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_DevNode_Status, SetupDiCallClassInstaller, SetupDiDestroyDeviceInfoList,
        SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
        SetupDiSetClassInstallParamsW, CM_DEVNODE_STATUS_FLAGS, CM_PROB, CM_PROB_DISABLED,
        CR_SUCCESS, DICS_DISABLE, DICS_ENABLE, DICS_FLAG_GLOBAL, DIF_PROPERTYCHANGE, DIGCF_PRESENT,
        HDEVINFO, SP_CLASSINSTALL_HEADER, SP_DEVINFO_DATA, SP_PROPCHANGE_PARAMS,
    };
    use windows::Win32::Foundation::HWND;

    /// `GUID_DEVCLASS_CAMERA` — {ca3e7ab9-b4c3-4ae6-8251-579ef933890f}, the modern
    /// device setup class every USB/integrated webcam is registered under.
    const GUID_DEVCLASS_CAMERA: GUID = GUID::from_u128(0xca3e7ab9_b4c3_4ae6_8251_579ef933890f);

    /// Real [`CameraDevices`] over SetupAPI / Configuration Manager.
    pub struct SetupApiCameras;

    impl Default for SetupApiCameras {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SetupApiCameras {
        pub fn new() -> Self {
            Self
        }

        /// Run `f` over each present camera's `(instance_id, SP_DEVINFO_DATA)`.
        ///
        /// Centralises opening/closing the device-info set so both `enabled` and
        /// `set_enabled` share identical, leak-free enumeration.
        fn for_each_camera<T>(
            &self,
            mut f: impl FnMut(&str, &SP_DEVINFO_DATA, HDEVINFO) -> PortResult<Option<T>>,
        ) -> PortResult<Vec<T>> {
            let mut collected = Vec::new();
            unsafe {
                let dev_info = SetupDiGetClassDevsW(
                    Some(&GUID_DEVCLASS_CAMERA),
                    PCWSTR::null(),
                    HWND::default(),
                    DIGCF_PRESENT,
                )
                .map_err(win_err("enumerate cameras"))?;

                let mut index = 0u32;
                loop {
                    let mut data = SP_DEVINFO_DATA {
                        cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                        ..Default::default()
                    };
                    if SetupDiEnumDeviceInfo(dev_info, index, &mut data).is_err() {
                        // No more devices in the set.
                        break;
                    }
                    index += 1;

                    let id = instance_id(dev_info, &mut data)?;
                    match f(&id, &data, dev_info) {
                        Ok(Some(v)) => collected.push(v),
                        Ok(None) => {}
                        Err(e) => {
                            let _ = SetupDiDestroyDeviceInfoList(dev_info);
                            return Err(e);
                        }
                    }
                }

                let _ = SetupDiDestroyDeviceInfoList(dev_info);
            }
            Ok(collected)
        }
    }

    impl CameraDevices for SetupApiCameras {
        fn enabled(&self) -> PortResult<Vec<String>> {
            self.for_each_camera(|id, data, _| {
                Ok(if is_disabled(data)? {
                    None
                } else {
                    Some(id.to_string())
                })
            })
        }

        fn set_enabled(&self, id: &str, enabled: bool) -> PortResult<()> {
            let matched = self.for_each_camera(|found, data, dev_info| {
                if found == id {
                    change_state(dev_info, data, enabled)?;
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            })?;
            if matched.is_empty() {
                return Err(PortError::new(format!("camera not found: {id}")));
            }
            Ok(())
        }
    }

    /// Read a device's instance id (e.g. `USB\VID_046D&PID_0825\...`).
    fn instance_id(dev_info: HDEVINFO, data: &mut SP_DEVINFO_DATA) -> PortResult<String> {
        unsafe {
            let mut needed = 0u32;
            // First call sizes the buffer; a failure only to learn the size is fine.
            let _ = SetupDiGetDeviceInstanceIdW(dev_info, data, None, Some(&mut needed));
            let mut buf = vec![0u16; needed.max(1) as usize];
            SetupDiGetDeviceInstanceIdW(dev_info, data, Some(&mut buf), Some(&mut needed))
                .map_err(win_err("read device instance id"))?;
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Ok(String::from_utf16_lossy(&buf[..end]))
        }
    }

    /// Whether a device node currently carries the "disabled" problem code.
    fn is_disabled(data: &SP_DEVINFO_DATA) -> PortResult<bool> {
        let mut status = CM_DEVNODE_STATUS_FLAGS::default();
        let mut problem = CM_PROB::default();
        // CM_Get_DevNode_Status returns a CONFIGRET, not an HRESULT; CR_SUCCESS
        // (0) is the only success code.
        let ret = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, data.DevInst, 0) };
        if ret != CR_SUCCESS {
            return Err(PortError::new(format!(
                "windows camera: query device status: CONFIGRET {}",
                ret.0
            )));
        }
        Ok(problem == CM_PROB_DISABLED)
    }

    /// Enable or disable a device node via a `DIF_PROPERTYCHANGE` install action.
    fn change_state(dev_info: HDEVINFO, data: &SP_DEVINFO_DATA, enabled: bool) -> PortResult<()> {
        let params = SP_PROPCHANGE_PARAMS {
            ClassInstallHeader: SP_CLASSINSTALL_HEADER {
                cbSize: std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                InstallFunction: DIF_PROPERTYCHANGE,
            },
            StateChange: if enabled { DICS_ENABLE } else { DICS_DISABLE },
            Scope: DICS_FLAG_GLOBAL,
            HwProfile: 0,
        };
        // These calls take the device and params as `*const`; borrows suffice.
        unsafe {
            SetupDiSetClassInstallParamsW(
                dev_info,
                Some(data),
                Some(&params.ClassInstallHeader),
                std::mem::size_of::<SP_PROPCHANGE_PARAMS>() as u32,
            )
            .map_err(win_err("set install params"))?;
            SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, dev_info, Some(data))
                .map_err(win_err("apply device state change"))?;
        }
        Ok(())
    }

    fn win_err(ctx: &'static str) -> impl Fn(windows::core::Error) -> PortError {
        move |e| PortError::new(format!("windows camera: {ctx}: {e}"))
    }
}

#[cfg(target_os = "windows")]
pub use real::SetupApiCameras;

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory camera set mapping instance id → enabled flag, tracking each
    /// toggle so a full disable→enable cycle can be verified without SetupAPI.
    struct FakeDevices {
        state: Mutex<Vec<(String, bool)>>,
        toggles: Mutex<Vec<(String, bool)>>,
        fail: bool,
    }
    impl FakeDevices {
        fn with(cams: &[(&str, bool)]) -> Self {
            Self {
                state: Mutex::new(cams.iter().map(|(id, on)| (id.to_string(), *on)).collect()),
                toggles: Mutex::new(Vec::new()),
                fail: false,
            }
        }
        fn failing(cams: &[(&str, bool)]) -> Self {
            Self {
                fail: true,
                ..Self::with(cams)
            }
        }
        fn toggles(&self) -> Vec<(String, bool)> {
            self.toggles.lock().unwrap().clone()
        }
    }
    impl CameraDevices for FakeDevices {
        fn enabled(&self) -> PortResult<Vec<String>> {
            if self.fail {
                return Err(PortError::new("device set unavailable"));
            }
            let mut ids: Vec<String> = self
                .state
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, on)| *on)
                .map(|(id, _)| id.clone())
                .collect();
            ids.sort();
            Ok(ids)
        }
        fn set_enabled(&self, id: &str, enabled: bool) -> PortResult<()> {
            if self.fail {
                return Err(PortError::new("toggle denied"));
            }
            self.toggles.lock().unwrap().push((id.to_string(), enabled));
            let mut state = self.state.lock().unwrap();
            for (cam, on) in state.iter_mut() {
                if cam == id {
                    *on = enabled;
                }
            }
            Ok(())
        }
    }

    fn cam(devices: FakeDevices) -> WindowsCamera<FakeDevices> {
        WindowsCamera::with_devices(devices)
    }

    #[test]
    fn is_enabled_true_when_a_camera_is_on() {
        let c = cam(FakeDevices::with(&[("cam-a", true), ("cam-b", false)]));
        assert!(c.is_enabled().unwrap());
    }

    #[test]
    fn is_enabled_false_when_all_cameras_off() {
        let c = cam(FakeDevices::with(&[("cam-a", false)]));
        assert!(!c.is_enabled().unwrap());
    }

    #[test]
    fn disable_then_enable_round_trips_the_camera_state() {
        let c = cam(FakeDevices::with(&[("cam-a", true), ("cam-b", true)]));
        assert!(c.is_enabled().unwrap());

        c.set_enabled(false).unwrap();
        assert!(!c.is_enabled().unwrap(), "camera should read disabled");

        c.set_enabled(true).unwrap();
        assert!(c.is_enabled().unwrap(), "camera should read enabled again");
    }

    #[test]
    fn enable_only_restores_exactly_what_was_disabled() {
        // cam-b starts off (user's own choice) and must never be re-enabled by us.
        let c = cam(FakeDevices::with(&[("cam-a", true), ("cam-b", false)]));
        c.set_enabled(false).unwrap();
        c.set_enabled(true).unwrap();

        assert_eq!(
            c.devices.toggles(),
            vec![("cam-a".to_string(), false), ("cam-a".to_string(), true)],
            "only the camera we disabled is toggled, once each way"
        );
    }

    #[test]
    fn disable_with_nothing_enabled_is_ok_noop() {
        let c = cam(FakeDevices::with(&[("cam-a", false)]));
        assert!(c.set_enabled(false).is_ok());
        assert!(c.devices.toggles().is_empty());
    }

    #[test]
    fn enable_without_prior_disable_touches_nothing() {
        let c = cam(FakeDevices::with(&[("cam-a", true)]));
        assert!(c.set_enabled(true).is_ok());
        assert!(c.devices.toggles().is_empty());
    }

    #[test]
    fn toggle_failure_surfaces_so_controller_skips_protection() {
        let c = cam(FakeDevices::failing(&[("cam-a", true)]));
        assert!(c.set_enabled(false).is_err());
    }

    #[test]
    fn enumeration_failure_is_reported_as_error() {
        let c = cam(FakeDevices::failing(&[("cam-a", true)]));
        assert!(c.is_enabled().is_err());
    }
}
