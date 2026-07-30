//! User-input idle time — the OS signal behind input-activity presence.
//!
//! When the app disables the camera on walkaway, the webcam sidecar is blinded
//! (a disabled device yields no frames) and can no longer see the user return.
//! To break that deadlock the app watches for keyboard/mouse **activity** as a
//! camera-free "the user is back" signal. This module exposes only *how idle the
//! machine is* (seconds since the last input) — never what was typed — keeping
//! the privacy promise intact.
//!
//! Only the Windows implementation touches the OS (`GetLastInputInfo`), kept
//! behind the [`IdleTime`] port so the decision policy is pure and unit-tested.

/// Milliseconds since the last user input (keyboard or mouse), system-wide.
///
/// A port so the platform call is swappable: production uses the real OS query,
/// tests inject a fake, and non-Windows hosts get a stub that reports "infinitely
/// idle" so input-based return simply never fires there.
pub trait IdleTime: Send + Sync {
    fn idle_ms(&self) -> u64;
}

/// Whether the activity monitor should emit a "present" pulse this tick.
///
/// Pure policy, so the whole rule is unit-tested with no OS or threads. Input
/// activity is treated as a return signal **only** while the app currently has
/// the camera disabled — the one situation where the webcam cannot see the user
/// come back. When the camera is on, the webcam stays the sole authority, so
/// touching the keyboard while merely leaning out of frame never overrides it.
/// Absence of input is never treated as *away*: that remains the webcam's job.
pub fn should_signal_return(camera_disabled: bool, idle_ms: u64, threshold_ms: u64) -> bool {
    camera_disabled && idle_ms < threshold_ms
}

/// Idle-time source for platforms without a supported query. Reports the maximum
/// idle time, so [`should_signal_return`] never fires — input-based return is
/// simply disabled rather than misbehaving.
pub struct UnsupportedIdleTime;

impl IdleTime for UnsupportedIdleTime {
    fn idle_ms(&self) -> u64 {
        u64::MAX
    }
}

#[cfg(target_os = "windows")]
mod real {
    use super::IdleTime;

    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    /// Real Windows idle-time source via `GetLastInputInfo`, which reports the
    /// tick count of the most recent input event across the whole session.
    pub struct WindowsIdleTime;

    impl Default for WindowsIdleTime {
        fn default() -> Self {
            Self
        }
    }

    impl WindowsIdleTime {
        pub fn new() -> Self {
            Self
        }
    }

    impl IdleTime for WindowsIdleTime {
        fn idle_ms(&self) -> u64 {
            let mut info = LASTINPUTINFO {
                cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            // On the rare query failure, report "maximally idle" so a return is
            // never signalled spuriously (fail safe: never un-mute on its own).
            if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
                return u64::MAX;
            }
            // GetTickCount wraps roughly every 49.7 days; wrapping_sub yields the
            // correct elapsed span across a wrap.
            let now = unsafe { GetTickCount() };
            u64::from(now.wrapping_sub(info.dwTime))
        }
    }
}

#[cfg(target_os = "windows")]
pub use real::WindowsIdleTime;

/// Idle-time source selected for the host platform.
#[cfg(target_os = "windows")]
pub type PlatformIdleTime = WindowsIdleTime;
#[cfg(not(target_os = "windows"))]
pub type PlatformIdleTime = UnsupportedIdleTime;

/// The host's idle-time source: the real query on Windows, the inert stub
/// elsewhere.
#[cfg(target_os = "windows")]
pub fn default_idle_time() -> PlatformIdleTime {
    WindowsIdleTime::new()
}

#[cfg(not(target_os = "windows"))]
pub fn default_idle_time() -> PlatformIdleTime {
    UnsupportedIdleTime
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_return_only_when_camera_disabled_and_recently_active() {
        // Camera off + recent activity: this is the deadlock-breaking case.
        assert!(should_signal_return(true, 200, 1_500));
    }

    #[test]
    fn no_signal_when_the_camera_is_on() {
        // Camera on: the webcam is the authority, input must not override it.
        assert!(!should_signal_return(false, 0, 1_500));
    }

    #[test]
    fn no_signal_when_idle_beyond_the_threshold() {
        // Camera off but no recent input: the user has not come back.
        assert!(!should_signal_return(true, 5_000, 1_500));
        // Exactly at the threshold is treated as idle (strictly-less-than).
        assert!(!should_signal_return(true, 1_500, 1_500));
    }

    #[test]
    fn unsupported_source_is_always_idle_so_return_never_fires() {
        let idle = UnsupportedIdleTime;
        assert_eq!(idle.idle_ms(), u64::MAX);
        assert!(!should_signal_return(true, idle.idle_ms(), 1_500));
    }
}
