// Typed wrappers over the Tauri command bridge. Argument keys are camelCase on
// this side; Tauri maps them to the Rust commands' snake_case parameters.

import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, Status } from "./types";

export const getConfig = (): Promise<AppConfig> => invoke("get_config");

export const saveConfig = (newConfig: AppConfig): Promise<void> =>
  invoke("save_config", { newConfig });

export const getStatus = (): Promise<Status> => invoke("get_status");

/** Whether the app can control the camera (elevated on Windows). */
export const getCameraControlAvailable = (): Promise<boolean> =>
  invoke("get_camera_control_available");

/** Whether presence is driven automatically by the webcam sidecar (vs manual). */
export const getPresenceAutomatic = (): Promise<boolean> =>
  invoke("get_presence_automatic");

export const setEnabled = (enabled: boolean): Promise<void> =>
  invoke("set_enabled", { enabled });

export const setPresent = (present: boolean): Promise<void> =>
  invoke("set_present", { present });

export const getAutostart = (): Promise<boolean> => invoke("get_autostart");

export const setAutostart = (enabled: boolean): Promise<void> =>
  invoke("set_autostart", { enabled });
