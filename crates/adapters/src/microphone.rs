use amow_application::{Microphone, PortError, PortResult};

use crate::command::{CommandRunner, SystemCommandRunner};

/// PulseAudio / PipeWire source token for "whatever is the current default
/// input". Using the token rather than a fixed index means the adapter follows
/// the user's default-device changes automatically.
const DEFAULT_SOURCE: &str = "@DEFAULT_SOURCE@";

/// Microphone adapter driving the default input source through `pactl`.
///
/// Works with both PulseAudio and PipeWire (via `pipewire-pulse`), which is the
/// common denominator on modern Linux desktops. The external command is behind
/// a [`CommandRunner`] so the mute/parse logic is fully unit-tested.
pub struct PulseMicrophone<R: CommandRunner = SystemCommandRunner> {
    runner: R,
    source: String,
}

impl PulseMicrophone<SystemCommandRunner> {
    /// Adapter for the system default source using real `pactl` invocations.
    pub fn system() -> Self {
        Self::with_runner(SystemCommandRunner, DEFAULT_SOURCE)
    }
}

impl<R: CommandRunner> PulseMicrophone<R> {
    /// Construct with an explicit runner and source token — the seam tests use.
    pub fn with_runner(runner: R, source: impl Into<String>) -> Self {
        Self {
            runner,
            source: source.into(),
        }
    }
}

impl<R: CommandRunner> Microphone for PulseMicrophone<R> {
    fn is_muted(&self) -> PortResult<bool> {
        let out = self
            .runner
            .run("pactl", &["get-source-mute", &self.source])?;
        parse_mute(&out)
    }

    fn set_muted(&self, muted: bool) -> PortResult<()> {
        let flag = if muted { "1" } else { "0" };
        self.runner
            .run("pactl", &["set-source-mute", &self.source, flag])
            .map(|_| ())
    }
}

/// Parse the output of `pactl get-source-mute`, e.g. `"Mute: yes\n"`.
fn parse_mute(out: &str) -> PortResult<bool> {
    match out
        .split_once("Mute:")
        .map(|(_, rest)| rest.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("yes") => Ok(true),
        Some("no") => Ok(false),
        _ => Err(PortError::new(format!(
            "unexpected pactl mute output: {:?}",
            out.trim()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records each invocation and returns a scripted result.
    struct FakeRunner {
        calls: Mutex<Vec<Vec<String>>>,
        reply: PortResult<String>,
    }
    impl FakeRunner {
        fn ok(stdout: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                reply: Ok(stdout.to_string()),
            }
        }
        fn err(msg: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                reply: Err(PortError::new(msg)),
            }
        }
        fn last_call(&self) -> Vec<String> {
            self.calls.lock().unwrap().last().cloned().unwrap()
        }
    }
    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> PortResult<String> {
            let mut call = vec![program.to_string()];
            call.extend(args.iter().map(|a| a.to_string()));
            self.calls.lock().unwrap().push(call);
            self.reply.clone()
        }
    }

    #[test]
    fn parses_muted() {
        assert!(parse_mute("Mute: yes\n").unwrap());
    }

    #[test]
    fn parses_unmuted() {
        assert!(!parse_mute("Mute: no\n").unwrap());
    }

    #[test]
    fn rejects_garbage_output() {
        assert!(parse_mute("nonsense").is_err());
    }

    #[test]
    fn is_muted_queries_default_source() {
        let mic = PulseMicrophone::with_runner(FakeRunner::ok("Mute: yes\n"), DEFAULT_SOURCE);
        assert!(mic.is_muted().unwrap());
        assert_eq!(
            mic.runner.last_call(),
            vec!["pactl", "get-source-mute", DEFAULT_SOURCE]
        );
    }

    #[test]
    fn set_muted_true_sends_flag_1() {
        let mic = PulseMicrophone::with_runner(FakeRunner::ok(""), DEFAULT_SOURCE);
        mic.set_muted(true).unwrap();
        assert_eq!(
            mic.runner.last_call(),
            vec!["pactl", "set-source-mute", DEFAULT_SOURCE, "1"]
        );
    }

    #[test]
    fn set_muted_false_sends_flag_0() {
        let mic = PulseMicrophone::with_runner(FakeRunner::ok(""), DEFAULT_SOURCE);
        mic.set_muted(false).unwrap();
        assert_eq!(
            mic.runner.last_call(),
            vec!["pactl", "set-source-mute", DEFAULT_SOURCE, "0"]
        );
    }

    #[test]
    fn propagates_runner_failure() {
        let mic = PulseMicrophone::with_runner(FakeRunner::err("pactl missing"), DEFAULT_SOURCE);
        assert!(mic.is_muted().is_err());
        assert!(mic.set_muted(true).is_err());
    }
}
