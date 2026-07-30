//! Infrastructure adapters: concrete implementations of the application ports.
//!
//! These are the OS-facing edge of the Clean Architecture. Each type implements
//! one of the `amow_application` port traits (`Clock`, `Microphone`, `Camera`)
//! against a real system facility, while depending only on the port abstraction
//! — never the other way around.
//!
//! Multiple backends per port coexist behind the same trait: Windows ships
//! [`WindowsMicrophone`] (WASAPI) and [`WindowsCamera`] (SetupAPI); Linux keeps
//! [`PulseMicrophone`] (`pactl`) and [`LinuxUvcCamera`] (`uvcvideo` sysfs) as
//! secondary adapters. The composition root selects the host's native pair via
//! [`PlatformMicrophone`] / [`PlatformCamera`] and the [`default_microphone`] /
//! [`default_camera`] factories, all with static dispatch.
//!
//! The crate carries no GUI dependency, so it builds and unit-tests without a
//! windowing system. Every OS interaction is funnelled through a small seam
//! port — [`CommandRunner`], [`Sysfs`], [`EndpointVolume`], [`CameraDevices`] —
//! so each adapter's logic is tested with a fake, and the actual `windows`-crate
//! syscalls compile only on Windows (a target-gated dependency).

mod camera;
mod clock;
mod command;
mod journal;
mod microphone;
mod platform;
mod sysfs;
mod windows_camera;
mod windows_microphone;

pub use camera::{LinuxUvcCamera, UnsupportedCamera};
pub use clock::SystemClock;
pub use command::{CommandRunner, SystemCommandRunner};
pub use journal::{CameraJournal, FileCameraJournal, NullCameraJournal};
pub use microphone::PulseMicrophone;
pub use platform::{default_camera, default_microphone, PlatformCamera, PlatformMicrophone};
pub use sysfs::{RealSysfs, Sysfs};
pub use windows_camera::{CameraDevices, WindowsCamera};
pub use windows_microphone::{EndpointVolume, WindowsMicrophone};

#[cfg(target_os = "windows")]
pub use windows_camera::SetupApiCameras;
#[cfg(target_os = "windows")]
pub use windows_microphone::CoreAudioEndpoint;
