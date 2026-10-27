//! Session bus integration.
//!
//! We own `io.github.yannkost.ClipboardManager` so the GNOME Shell helper
//! extension (or a script) can toggle the popup and push clipboard content.
//! In the other direction we talk to the helper, which lives inside
//! gnome-shell, to set the Wayland clipboard and simulate the paste keystroke.

use tauri::{AppHandle, Manager};
use zbus::{connection, fdo, interface, proxy, proxy::CacheProperties, Connection};

use crate::AppState;

pub const BUS_NAME: &str = "io.github.yannkost.ClipboardManager";
pub const OBJECT_PATH: &str = "/io/github/yannkost/ClipboardManager";

pub struct Service {
    app: AppHandle,
}

#[interface(name = "io.github.yannkost.ClipboardManager")]
impl Service {
    async fn toggle(&self) {
        crate::toggle_popup(&self.app);
    }

    async fn add_text(&self, text: String) -> fdo::Result<()> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || crate::record_text(&app, &text))
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?
            .map_err(fdo::Error::Failed)
    }

    async fn add_image(&self, png: Vec<u8>) -> fdo::Result<()> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || crate::record_image_encoded(&app, &png))
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?
            .map_err(fdo::Error::Failed)
    }

    /// The configured shortcut, for the helper extension to bind.
    #[zbus(property)]
    async fn hotkey(&self) -> String {
        self.app.state::<AppState>().config().hotkey
    }
}

/// The GNOME Shell helper extension (a separate project, step 3).
#[proxy(
    interface = "io.github.yannkost.ClipboardManager.Shell",
    default_service = "org.gnome.Shell",
    default_path = "/io/github/yannkost/ClipboardManager/Shell"
)]
pub trait ShellHelper {
    fn set_text(&self, text: &str) -> zbus::Result<()>;
    fn set_image(&self, png: &[u8]) -> zbus::Result<()>;
    /// Focus the previously active window and send Ctrl+V (Ctrl+Shift+V if `plain`).
    fn paste(&self, plain: bool) -> zbus::Result<()>;
    fn version(&self) -> zbus::Result<u32>;
}

pub async fn start(app: AppHandle) -> zbus::Result<(Connection, ShellHelperProxy<'static>)> {
    let conn = connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Service { app })?
        .build()
        .await?;
    let shell = ShellHelperProxy::builder(&conn).cache_properties(CacheProperties::No).build().await?;
    Ok((conn, shell))
}
