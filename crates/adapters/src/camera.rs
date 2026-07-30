use amow_application::{Camera, PortError, PortResult};

/// Camera adapter for platforms without a portable capture-mute mechanism.
///
/// Unlike audio — where `pactl` mutes the input source system-wide — there is
/// no standard way on Linux to force the camera off for another application:
/// the capture device is owned by the conferencing app, not the OS. Rather than
/// pretend, this adapter honestly reports the capability as unavailable.
///
/// This is a real, safe behaviour, not a placeholder: [`WalkawayController`]
/// treats a failing camera port as "leave the camera alone", so `auto_camera_off`
/// simply no-ops here while the microphone is still protected. A future
/// platform-specific backend (e.g. a virtual-camera shim, or a macOS/Windows
/// capture API) can replace this adapter without touching the application layer.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_unavailable_rather_than_lying() {
        let cam = UnsupportedCamera;
        assert!(cam.is_enabled().is_err());
        assert!(cam.set_enabled(false).is_err());
    }
}
