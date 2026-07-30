// TypeScript mirror of the Rust `AppConfig` (amow-config) and status snapshot.
// Kept in one place so the shape stays in sync with the backend contract.

export type LogLevel = "error" | "warn" | "info" | "debug";

export interface BehaviorConfig {
  auto_mute: boolean;
  auto_camera_off: boolean;
  auto_restore: boolean;
  notify_on_action: boolean;
  sample_interval_ms: number;
}

export interface PresenceConfig {
  away_grace_ms: number;
  return_grace_ms: number;
}

export interface LoggingConfig {
  level: LogLevel;
}

export interface AppConfig {
  version: number;
  behavior: BehaviorConfig;
  presence: PresenceConfig;
  logging: LoggingConfig;
}

export interface Status {
  presence: "present" | "away";
  enabled: boolean;
  protecting: boolean;
  mic_muted: boolean;
  camera_off: boolean;
  camera_blocked: boolean;
}
