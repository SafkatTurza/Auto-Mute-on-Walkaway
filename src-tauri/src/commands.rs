//! Tauri command handlers — the thin bridge from the UI to the application.
//!
//! These contain no business logic: they read/write config, forward inputs to
//! the supervisor, and return the status snapshot. All decision-making lives in
//! the tested `amow_application` layer.

use amow_config::AppConfig;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

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

/// Latest presence / protection snapshot for the UI.
#[tauri::command]
pub fn get_status(state: State<AppState>) -> Status {
    state.status.snapshot()
}

/// Whether the app can actually control the camera (disable *and* re-enable it)
/// in this process — on Windows, whether it is running elevated. The UI uses
/// this to warn before the user enables camera control it could not honour.
#[tauri::command]
pub fn get_camera_control_available() -> bool {
    amow_adapters::camera_control_available()
}

/// Turn walkaway protection on or off (the user's master switch).
#[tauri::command]
pub fn set_enabled(enabled: bool, state: State<AppState>) {
    state.supervisor.set_enabled(enabled);
}

/// Report whether the user is present (manual toggle today; detector later).
#[tauri::command]
pub fn set_present(present: bool, state: State<AppState>) {
    state.supervisor.set_face(present);
}

/// Whether the app is registered to start automatically on login. The OS (the
/// registry on Windows) is the source of truth, so this preference persists
/// across restarts without any config file of our own.
#[tauri::command]
pub fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Enable or disable starting the app automatically on login.
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| e.to_string())
}
