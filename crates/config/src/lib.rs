//! Application configuration for Auto-Mute-on-Walkaway.
//!
//! Configuration is a plain data structure that serialises to/from JSON. This
//! crate owns the schema, sensible defaults, validation, and file persistence.
//! It deliberately performs its own small file I/O (read/write a JSON file)
//! because that is the entirety of its responsibility; higher layers pass a
//! path in and receive a validated [`AppConfig`] out.
//!
//! Forward compatibility: unknown fields in an existing file are ignored and
//! missing fields fall back to their defaults, so newer settings can be added
//! without breaking older config files.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use amow_domain::PresenceConfig;

/// Top-level configuration, matching the on-disk JSON document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Schema version, reserved for future migrations.
    pub version: u32,
    /// What the app is allowed to do automatically.
    pub behavior: BehaviorConfig,
    /// Presence-detection debouncing (mirrors the domain tuning).
    pub presence: PresenceConfig,
    /// Diagnostics.
    pub logging: LoggingConfig,
}

/// Automatic-action policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BehaviorConfig {
    /// Mute the microphone when the user walks away (while protection is on).
    pub auto_mute: bool,
    /// Disable the camera when the user walks away (while protection is on).
    ///
    /// Off by default: disabling the camera toggles the OS device node (needs
    /// administrator rights on Windows) and is the only action that could, in a
    /// crash, briefly outlive the app. Microphone mute is the safe always-on
    /// default; users opt into camera control deliberately.
    pub auto_camera_off: bool,
    /// Restore mic/camera to their prior state when the user returns.
    pub auto_restore: bool,
    /// Show a desktop notification when an automatic action is taken.
    pub notify_on_action: bool,
    /// How often to sample presence, in milliseconds. Bounds the idle CPU cost.
    pub sample_interval_ms: u64,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            auto_mute: true,
            auto_camera_off: false,
            auto_restore: true,
            notify_on_action: true,
            sample_interval_ms: 500,
        }
    }
}

/// Log level, ordered from least to most verbose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: LogLevel,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            behavior: BehaviorConfig::default(),
            presence: PresenceConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

/// Errors that can occur while loading or saving configuration.
#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Parse(serde_json::Error),
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "config i/o error: {e}"),
            ConfigError::Parse(e) => write!(f, "config parse error: {e}"),
            ConfigError::Invalid(m) => write!(f, "invalid config: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(e: io::Error) -> Self {
        ConfigError::Io(e)
    }
}
impl From<serde_json::Error> for ConfigError {
    fn from(e: serde_json::Error) -> Self {
        ConfigError::Parse(e)
    }
}

impl AppConfig {
    /// Validate cross-field invariants. Kept separate from `serde` so the same
    /// checks apply whether config comes from a file or is built in code.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version == 0 {
            return Err(ConfigError::Invalid("version must be >= 1".into()));
        }
        if self.behavior.sample_interval_ms == 0 {
            return Err(ConfigError::Invalid(
                "behavior.sample_interval_ms must be > 0".into(),
            ));
        }
        if self.presence.away_grace_ms == 0 || self.presence.return_grace_ms == 0 {
            return Err(ConfigError::Invalid(
                "presence grace periods must be > 0".into(),
            ));
        }
        Ok(())
    }

    /// Parse and validate configuration from a JSON string.
    pub fn from_json(s: &str) -> Result<Self, ConfigError> {
        let cfg: AppConfig = serde_json::from_str(s)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialise to pretty JSON suitable for a human-editable config file.
    pub fn to_json(&self) -> Result<String, ConfigError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Load configuration from `path`. If the file does not exist, the default
    /// configuration is written to `path` and returned — a first-run bootstrap
    /// so the app always has a materialised, editable config.
    pub fn load_or_init(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(contents) => Self::from_json(&contents),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let cfg = AppConfig::default();
                cfg.save(path)?;
                Ok(cfg)
            }
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    /// Persist configuration to `path`, creating parent directories as needed.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        self.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.to_json()?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        assert!(AppConfig::default().validate().is_ok());
    }

    #[test]
    fn roundtrips_through_json() {
        let cfg = AppConfig::default();
        let json = cfg.to_json().unwrap();
        assert_eq!(AppConfig::from_json(&json).unwrap(), cfg);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Only one nested field supplied; everything else must default.
        let cfg = AppConfig::from_json(r#"{"behavior":{"auto_mute":false}}"#).unwrap();
        assert!(!cfg.behavior.auto_mute);
        assert!(!cfg.behavior.auto_camera_off); // defaulted (off by default)
        assert!(cfg.behavior.auto_restore); // defaulted (on)
        assert_eq!(cfg.version, 1); // defaulted
    }

    #[test]
    fn rejects_zero_sample_interval() {
        let err = AppConfig::from_json(r#"{"behavior":{"sample_interval_ms":0}}"#).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = AppConfig::from_json(r#"{"nope":1}"#).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn load_or_init_creates_then_reads_file() {
        let dir = std::env::temp_dir().join(format!("amow-cfg-{}", std::process::id()));
        let path = dir.join("config.json");
        let _ = fs::remove_dir_all(&dir);

        let created = AppConfig::load_or_init(&path).unwrap();
        assert_eq!(created, AppConfig::default());
        assert!(path.exists());

        // Second load reads the persisted file rather than re-defaulting.
        let reloaded = AppConfig::load_or_init(&path).unwrap();
        assert_eq!(reloaded, created);

        let _ = fs::remove_dir_all(&dir);
    }
}
