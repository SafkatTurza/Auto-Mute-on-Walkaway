// Hide the extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Auto-Mute on Walkaway — application composition root.
//!
//! This is the only place the layers are wired together: it loads config, sets
//! up logging and the event bus, starts the supervisor that owns the control
//! logic, installs the tray, and exposes the Tauri commands. It holds no
//! business logic itself — that all lives in the tested crates.

mod bridge;
mod commands;
mod crash;
mod notifier;
mod status;
mod supervisor;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use amow_config::AppConfig;
use amow_domain::DomainEvent;
use amow_eventbus::EventBus;
use amow_logger::{FileSink, LogSink, Logger, StderrSink};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;

use bridge::PresenceBridge;
use notifier::AppNotifier;
use status::SharedStatus;
use supervisor::Supervisor;

/// Shared application state managed by Tauri and reachable from commands.
pub struct AppState {
    pub supervisor: Supervisor,
    pub status: SharedStatus,
    pub config_path: PathBuf,
    pub config: Mutex<AppConfig>,
    /// Kept alive for the app's lifetime; dropping it stops the webcam sidecar.
    _presence_bridge: PresenceBridge,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        // Auto-start on login (opt-in; toggled from Settings). No launch args —
        // it starts like a normal launch, with the window shown and the tray
        // installed.
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle = app.handle();

            // --- Paths: config and logs live in the OS app data dirs ----------
            let config_path = app.path().app_config_dir()?.join("config.json");
            let log_dir = app.path().app_log_dir()?;
            let log_path = log_dir.join("amow.log");
            let crash_path = log_dir.join("crash.log");

            // --- Config: first run bootstraps a default file ------------------
            let config = AppConfig::load_or_init(&config_path).unwrap_or_else(|e| {
                eprintln!("config load failed ({e}); falling back to defaults");
                AppConfig::default()
            });

            // --- Logger: file sink, falling back to stderr --------------------
            let sink: Box<dyn LogSink> = match FileSink::new(&log_path) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!("log file unavailable ({e}); logging to stderr");
                    Box::new(StderrSink)
                }
            };
            let logger = Arc::new(Logger::new(config.logging.level, sink));
            logger.info("Auto-Mute on Walkaway starting");

            // --- Crash logging: capture panics to crash.log -------------------
            // Installed early so any later panic (device I/O on the supervisor
            // thread, sidecar reader threads, command handlers) leaves a trace
            // even in a windowed release build with no console.
            crash::install(crash_path, logger.clone());

            // --- Event bus: log every domain event and mirror it to the UI ----
            let bus = EventBus::new();
            {
                let logger = logger.clone();
                bus.subscribe(Box::new(move |event| {
                    logger.info(&describe_event(event));
                }));
            }
            {
                let handle = handle.clone();
                bus.subscribe(Box::new(move |event| {
                    let _ = handle.emit("amow://event", event);
                }));
            }

            // --- Supervisor: owns the controller on its own thread ------------
            let notifier = AppNotifier::new(handle.clone());
            let shared_status = SharedStatus::default();
            let supervisor = Supervisor::spawn(
                config.clone(),
                bus,
                notifier,
                shared_status.clone(),
                logger.clone(),
            );

            // --- Presence bridge: spawn the webcam sidecar and feed samples ---
            // in through the supervisor. Degrades gracefully to manual input if
            // the sidecar can't start, so the app is never blocked on it.
            let presence_bridge =
                PresenceBridge::spawn(&config_path, supervisor.face_sink(), logger.clone());

            app.manage(AppState {
                supervisor,
                status: shared_status,
                config_path,
                config: Mutex::new(config),
                _presence_bridge: presence_bridge,
            });

            install_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_status,
            commands::set_enabled,
            commands::set_present,
            commands::get_autostart,
            commands::set_autostart,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Auto-Mute on Walkaway");
}

/// Render a domain event as a short log line (never any media or personal data).
fn describe_event(event: &DomainEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|_| "event".to_string())
}

/// Install a system-tray icon with a small menu (show / quit).
fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("Auto-Mute on Walkaway")
        .menu(&menu)
        // Keep the menu on right-click only; a left click restores the window,
        // the behaviour Windows users expect from a tray app.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "show" => show_main_window(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

/// Bring the main window to the foreground (shared by the tray menu and a
/// left click on the tray icon).
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
