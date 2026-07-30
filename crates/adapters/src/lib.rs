//! Infrastructure adapters: concrete implementations of the application ports.
//!
//! These are the OS-facing edge of the Clean Architecture. Each type implements
//! one of the `amow_application` port traits (`Clock`, `Microphone`, `Camera`)
//! against a real system facility, while depending only on the port abstraction
//! — never the other way around.
//!
//! The crate carries no GUI dependency, so it builds and unit-tests without a
//! windowing system. External-process interaction (e.g. `pactl`) is funnelled
//! through the [`CommandRunner`] port, letting the adapter logic be tested with
//! a fake runner instead of a live audio server.

mod camera;
mod clock;
mod command;
mod microphone;

pub use camera::UnsupportedCamera;
pub use clock::SystemClock;
pub use command::{CommandRunner, SystemCommandRunner};
pub use microphone::PulseMicrophone;
