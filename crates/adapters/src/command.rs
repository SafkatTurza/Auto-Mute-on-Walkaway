use std::process::Command;

use amow_application::{PortError, PortResult};

/// Runs an external command and returns its standard output.
///
/// Extracted as a port so device adapters that shell out (e.g. the PulseAudio
/// microphone via `pactl`) can be unit-tested with a fake runner, without a
/// live audio server or the binary installed.
pub trait CommandRunner: Send + Sync {
    /// Run `program` with `args`. Returns captured stdout on success, or a
    /// [`PortError`] if the process could not be spawned or exited non-zero.
    fn run(&self, program: &str, args: &[&str]) -> PortResult<String>;
}

/// The production [`CommandRunner`], spawning real OS processes.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> PortResult<String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| PortError::new(format!("failed to run `{program}`: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PortError::new(format!(
                "`{program}` exited with {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}
