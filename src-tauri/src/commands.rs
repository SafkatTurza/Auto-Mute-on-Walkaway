//! Tauri command handlers — the thin bridge from the UI to the application.
//!
//! These contain no business logic: they read/write config, forward inputs to
//! the supervisor, and return the status snapshot. All decision-making lives in
//! the tested `amow_application` layer.

use amow_config::AppConfig;
use tauri::State;

use crate::status::Status;
use crate::AppState;

/// Return the current configuration.
#[tauri::command]
pub fn get_config(state: State<AppState>) -> AppConfig {
    state.config.lock().expect("config mutex poisoned").clone()
}

/// Validate, persist, and apply a new configuration at runtime.
#[tauri::command]
pub fn save_config(new_config: AppConfig, state: State<AppState>) -> Result<(), String> {
    new_config.validate().map_err(|e| e.to_string())?;
    new_config
        .save(&state.config_path)
        .map_err(|e| e.to_string())?;
    *state.config.lock().expect("config mutex poisoned") = new_config.clone();
    state.supervisor.update_config(new_config);
    Ok(())
}

/// Latest presence / meeting / protection snapshot for the UI.
#[tauri::command]
pub fn get_status(state: State<AppState>) -> Status {
    state.status.snapshot()
}

/// Report whether a meeting is active (manual toggle today; detector later).
#[tauri::command]
pub fn set_meeting_active(active: bool, state: State<AppState>) {
    state.supervisor.set_meeting(active);
}

/// Report whether the user is present (manual toggle today; detector later).
#[tauri::command]
pub fn set_present(present: bool, state: State<AppState>) {
    state.supervisor.set_face(present);
}
