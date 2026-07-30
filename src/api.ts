// Typed wrappers over the Tauri command bridge. Argument keys are camelCase on
// this side; Tauri maps them to the Rust commands' snake_case parameters.

import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, Status } from "./types";

export const getConfig = (): Promise<AppConfig> => invoke("get_config");

export const saveConfig = (newConfig: AppConfig): Promise<void> =>
  invoke("save_config", { newConfig });

export const getStatus = (): Promise<Status> => invoke("get_status");

export const setMeetingActive = (active: boolean): Promise<void> =>
  invoke("set_meeting_active", { active });

export const setPresent = (present: boolean): Promise<void> =>
  invoke("set_present", { present });
