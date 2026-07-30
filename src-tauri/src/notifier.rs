//! Desktop-notification adapter implementing the application's `Notifier` port.

use amow_application::Notifier;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

/// Shows transient OS notifications via the Tauri notification plugin.
///
/// Cloneable and `Send` because it holds only an [`AppHandle`], so it can be
/// moved onto the supervisor thread that owns the controller.
#[derive(Clone)]
pub struct AppNotifier {
    app: AppHandle,
}

impl AppNotifier {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Notifier for AppNotifier {
    fn notify(&self, title: &str, body: &str) {
        // A failed notification must never affect device protection, so errors
        // are swallowed — consistent with the port contract.
        let _ = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show();
    }
}
