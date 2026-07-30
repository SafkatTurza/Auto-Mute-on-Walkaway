//! Platform selection: the native adapter for the host OS.
//!
//! The composition root wires the [`WalkawayController`] with concrete adapter
//! types via static dispatch, so the choice of backend is made here at compile
//! time rather than with runtime `dyn` — keeping the zero-overhead design
//! intact. Each platform picks its native microphone and camera adapter; Linux
//! is the default fallback for any other Unix.
//!
//! Callers use [`PlatformMicrophone`] / [`PlatformCamera`] for the types and
//! [`default_microphone`] / [`default_camera`] to construct them, and never name
//! a concrete OS backend. The concrete adapters remain publicly exported too, so
//! existing code that referenced them directly keeps working unchanged.
//!
//! [`WalkawayController`]: amow_application::WalkawayController

// --- Windows: native WASAPI mic + SetupAPI camera ----------------------------

#[cfg(target_os = "windows")]
mod selection {
    use crate::windows_camera::{SetupApiCameras, WindowsCamera};
    use crate::windows_microphone::{CoreAudioEndpoint, WindowsMicrophone};

    /// Native microphone adapter for this platform.
    pub type PlatformMicrophone = WindowsMicrophone<CoreAudioEndpoint>;
    /// Native camera adapter for this platform.
    pub type PlatformCamera = WindowsCamera<SetupApiCameras>;

    /// Construct the native microphone adapter for this platform.
    pub fn default_microphone() -> PlatformMicrophone {
        WindowsMicrophone::system()
    }

    /// Construct the native camera adapter for this platform.
    pub fn default_camera() -> PlatformCamera {
        WindowsCamera::new()
    }
}

// --- Everything else: Linux PulseAudio mic + UVC camera (secondary) ----------

#[cfg(not(target_os = "windows"))]
mod selection {
    use crate::command::SystemCommandRunner;
    use crate::{LinuxUvcCamera, PulseMicrophone};

    /// Native microphone adapter for this platform.
    pub type PlatformMicrophone = PulseMicrophone<SystemCommandRunner>;
    /// Native camera adapter for this platform.
    pub type PlatformCamera = LinuxUvcCamera;

    /// Construct the native microphone adapter for this platform.
    pub fn default_microphone() -> PlatformMicrophone {
        PulseMicrophone::system()
    }

    /// Construct the native camera adapter for this platform.
    pub fn default_camera() -> PlatformCamera {
        LinuxUvcCamera::new()
    }
}

pub use selection::{default_camera, default_microphone, PlatformCamera, PlatformMicrophone};

#[cfg(test)]
mod tests {
    use super::*;
    use amow_application::{Camera, Microphone};

    // These compile-and-run only on the host that owns this file's build. On
    // Linux (this project's CI target) they exercise the PulseAudio / UVC
    // fallback selection end to end: the factories return usable port objects,
    // and — with no audio server or webcam present — calls degrade to a
    // `PortError` rather than panicking, exactly as the controller relies on.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn factories_build_usable_ports_that_degrade_gracefully() {
        let mic = default_microphone();
        // Result either way; the contract is "never panic".
        let _ = mic.is_muted();

        let camera = default_camera();
        let _ = camera.is_enabled();
    }

    // A type-level check that the selected aliases really implement the ports,
    // valid on every platform even where the bodies cannot be executed.
    #[test]
    fn selected_types_implement_the_ports() {
        fn assert_mic<M: Microphone>() {}
        fn assert_cam<C: Camera>() {}
        assert_mic::<PlatformMicrophone>();
        assert_cam::<PlatformCamera>();
    }
}
