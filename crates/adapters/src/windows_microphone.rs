//! Native Windows microphone adapter.
//!
//! Mutes the system's default capture device through the WASAPI Core Audio
//! endpoint volume interface (`IAudioEndpointVolume`) — the same switch the
//! Windows "Sound" control panel and conferencing apps toggle, and a true
//! system-wide mute rather than a per-app hint. It mutes *both* the general
//! (`eConsole`) and communications (`eCommunications`) default roles, so the mic
//! the user actually uses is silenced even when those roles point at different
//! devices.
//!
//! The COM plumbing is isolated behind the [`EndpointVolume`] seam, exactly as
//! the PulseAudio adapter hides `pactl` behind [`CommandRunner`]. That keeps the
//! only Windows-specific, `windows`-crate code in one small place — compiled
//! solely on Windows — while the port logic (BOOL↔bool mapping, error
//! propagation) is generic and unit-tested on every platform.
//!
//! [`CommandRunner`]: crate::CommandRunner

use amow_application::{Microphone, PortResult};

/// Low-level access to the default capture endpoint's mute flag, expressed as a
/// raw Windows `BOOL` (an `i32`: zero is false, non-zero is true).
///
/// Isolated as a port so the COM/WASAPI calls are the only platform-specific
/// part; the [`WindowsMicrophone`] wrapper over it is testable with a fake.
pub trait EndpointVolume: Send + Sync {
    /// Current mute flag of the default capture endpoint (`BOOL`).
    fn get_mute(&self) -> PortResult<i32>;
    /// Set the mute flag of the default capture endpoint (`BOOL`).
    fn set_mute(&self, muted: i32) -> PortResult<()>;
}

/// Microphone adapter driving the default capture endpoint on Windows.
///
/// Generic over the [`EndpointVolume`] seam so production injects the real
/// Core Audio backend while tests inject a fake — no dynamic dispatch, matching
/// the [`PulseMicrophone`](crate::PulseMicrophone) design.
pub struct WindowsMicrophone<V: EndpointVolume> {
    volume: V,
}

impl<V: EndpointVolume> WindowsMicrophone<V> {
    /// Construct with an explicit endpoint backend — the seam tests use.
    pub fn with_endpoint(volume: V) -> Self {
        Self { volume }
    }
}

#[cfg(target_os = "windows")]
impl WindowsMicrophone<CoreAudioEndpoint> {
    /// Adapter for the system default capture endpoint via real WASAPI calls.
    pub fn system() -> Self {
        Self::with_endpoint(CoreAudioEndpoint::new())
    }
}

impl<V: EndpointVolume> Microphone for WindowsMicrophone<V> {
    fn is_muted(&self) -> PortResult<bool> {
        // Any non-zero BOOL means muted; normalise to a Rust bool.
        Ok(self.volume.get_mute()? != 0)
    }

    fn set_muted(&self, muted: bool) -> PortResult<()> {
        self.volume.set_mute(i32::from(muted))
    }
}

// --- Real Core Audio backend (Windows only) ----------------------------------
//
// Only this module touches the `windows` crate, and it is compiled solely on
// Windows (the dependency is target-gated in Cargo.toml). Everything above is
// portable and unit-tested on Linux, so a Linux `cargo test` needs no COM.

#[cfg(target_os = "windows")]
mod real {
    use super::EndpointVolume;
    use amow_application::{PortError, PortResult};

    use windows::core::GUID;
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, ERole, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    /// Capture-device roles muted on walkaway: the general default (`eConsole`,
    /// what Sound settings shows as "Default Device") and the communications
    /// default (`eCommunications`, what calls use). On most machines these are
    /// the same device; muting both covers the split-device case too.
    const MUTED_ROLES: [ERole; 2] = [eConsole, eCommunications];

    /// Real [`EndpointVolume`] over the default capture device(s).
    pub struct CoreAudioEndpoint;

    impl Default for CoreAudioEndpoint {
        fn default() -> Self {
            Self::new()
        }
    }

    impl CoreAudioEndpoint {
        pub fn new() -> Self {
            Self
        }

        /// Activate `IAudioEndpointVolume` for the default capture endpoint of a
        /// given role.
        ///
        /// Re-acquired per call so the adapter always follows the user's current
        /// default device rather than caching a stale endpoint.
        fn endpoint_for(&self, role: ERole) -> PortResult<IAudioEndpointVolume> {
            unsafe {
                // COM may already be initialised on this thread (e.g. by Tauri);
                // a benign S_FALSE/RPC_E_CHANGED_MODE is not fatal for our use, so
                // the HRESULT is intentionally not treated as an error here.
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                        .map_err(win_err("create device enumerator"))?;
                let device = enumerator
                    .GetDefaultAudioEndpoint(eCapture, role)
                    .map_err(win_err("get default capture endpoint"))?;
                let volume: IAudioEndpointVolume = device
                    .Activate(CLSCTX_ALL, None)
                    .map_err(win_err("activate endpoint volume"))?;
                Ok(volume)
            }
        }
    }

    impl EndpointVolume for CoreAudioEndpoint {
        fn get_mute(&self) -> PortResult<i32> {
            // Report the general default device's mute state.
            let volume = self.endpoint_for(eConsole)?;
            let muted: BOOL = unsafe { volume.GetMute() }.map_err(win_err("query mute"))?;
            Ok(muted.0)
        }

        fn set_mute(&self, muted: i32) -> PortResult<()> {
            // Apply to every default role; succeed if at least one endpoint took
            // it, so a machine missing one role still gets muted. Setting the
            // same underlying device twice is harmless.
            let mut any_ok = false;
            let mut last_err = None;
            for role in MUTED_ROLES {
                match self.endpoint_for(role) {
                    // A null event-context GUID: we are not correlating changes.
                    Ok(volume) => {
                        match unsafe { volume.SetMute(BOOL(muted), std::ptr::null::<GUID>()) } {
                            Ok(()) => any_ok = true,
                            Err(e) => last_err = Some(win_err("set mute")(e)),
                        }
                    }
                    Err(e) => last_err = Some(e),
                }
            }
            if any_ok {
                Ok(())
            } else {
                Err(last_err
                    .unwrap_or_else(|| PortError::new("windows audio: no default capture device")))
            }
        }
    }

    /// Wrap a `windows::core::Error` as a [`PortError`] with context, so a
    /// failure degrades gracefully (the controller leaves the mic alone) instead
    /// of panicking.
    fn win_err(ctx: &'static str) -> impl Fn(windows::core::Error) -> PortError {
        move |e| PortError::new(format!("windows audio: {ctx}: {e}"))
    }
}

#[cfg(target_os = "windows")]
pub use real::CoreAudioEndpoint;

#[cfg(test)]
mod tests {
    use super::*;
    use amow_application::PortError;
    use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

    const REL: Ordering = Ordering::SeqCst;

    /// In-memory endpoint mirroring a real device's mute flag, so a full
    /// query→set→query cycle can be driven without any COM.
    struct FakeEndpoint {
        mute: AtomicI32,
        writes: AtomicU32,
        fail: bool,
    }
    impl FakeEndpoint {
        fn new(mute: i32) -> Self {
            Self {
                mute: AtomicI32::new(mute),
                writes: AtomicU32::new(0),
                fail: false,
            }
        }
        fn failing() -> Self {
            Self {
                fail: true,
                ..Self::new(0)
            }
        }
    }
    impl EndpointVolume for FakeEndpoint {
        fn get_mute(&self) -> PortResult<i32> {
            if self.fail {
                return Err(PortError::new("endpoint unavailable"));
            }
            Ok(self.mute.load(REL))
        }
        fn set_mute(&self, muted: i32) -> PortResult<()> {
            if self.fail {
                return Err(PortError::new("endpoint unavailable"));
            }
            self.mute.store(muted, REL);
            self.writes.fetch_add(1, REL);
            Ok(())
        }
    }

    #[test]
    fn reports_muted_for_nonzero_bool() {
        let mic = WindowsMicrophone::with_endpoint(FakeEndpoint::new(1));
        assert!(mic.is_muted().unwrap());
    }

    #[test]
    fn reports_unmuted_for_zero_bool() {
        let mic = WindowsMicrophone::with_endpoint(FakeEndpoint::new(0));
        assert!(!mic.is_muted().unwrap());
    }

    #[test]
    fn treats_any_nonzero_bool_as_muted() {
        // Win32 BOOL is "non-zero is true"; -1 (the canonical TRUE) included.
        let mic = WindowsMicrophone::with_endpoint(FakeEndpoint::new(-1));
        assert!(mic.is_muted().unwrap());
    }

    #[test]
    fn set_muted_true_writes_bool_1() {
        let mic = WindowsMicrophone::with_endpoint(FakeEndpoint::new(0));
        mic.set_muted(true).unwrap();
        assert_eq!(mic.volume.mute.load(REL), 1);
        assert!(mic.is_muted().unwrap());
    }

    #[test]
    fn set_muted_false_writes_bool_0() {
        let mic = WindowsMicrophone::with_endpoint(FakeEndpoint::new(1));
        mic.set_muted(false).unwrap();
        assert_eq!(mic.volume.mute.load(REL), 0);
        assert!(!mic.is_muted().unwrap());
    }

    #[test]
    fn propagates_endpoint_failure() {
        let mic = WindowsMicrophone::with_endpoint(FakeEndpoint::failing());
        assert!(mic.is_muted().is_err());
        assert!(mic.set_muted(true).is_err());
    }
}
