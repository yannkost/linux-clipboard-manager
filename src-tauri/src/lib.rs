mod clipboard;
mod commands;
mod config;
mod db;
mod dbus;
mod gnome;
mod history;
mod tray;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, LogicalSize, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use clipboard::{Clipboard, Snapshot};
use config::Config;
use history::History;

const APP_DIR: &str = "linux-clipboard-manager";
/// Set by build.rs from the ui/ contents; reading it makes UI edits rebuild the crate.
const _ASSETS_HASH: &str = env!("CLIPS_ASSETS_HASH");
pub const POPUP: &str = "popup";
pub const SETTINGS: &str = "settings";
pub const WELCOME: &str = "welcome";
/// Transparent space around each window's card so its CSS shadow can fade
/// out (body padding in ui/tokens.css: 20 top, 32 sides, 48 bottom).
const SHADOW_MARGIN_X: f64 = 64.0;
const SHADOW_MARGIN_Y: f64 = 68.0;
const POPUP_WIDTH: f64 = 560.0;
const POPUP_PREVIEW_WIDTH: f64 = 900.0;
const POPUP_HEIGHT: f64 = 640.0;

pub struct AppState {
    config: RwLock<Config>,
    pub history: History,
    pub clipboard: Clipboard,
    pub paused: AtomicBool,
    /// A file dialog is open over the popup, so losing focus must not hide it.
    pub dialog_open: AtomicBool,
    /// Hash of content we just put on the clipboard ourselves, so the poller
    /// does not record it again (which would bump it to the top).
    pub own_clipboard: Mutex<Option<String>>,
}

impl AppState {
    pub fn config(&self) -> Config {
        self.config.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_config(&self, config: Config) {
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = config;
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config")).join(APP_DIR).join("config.toml")
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".local/share")).join(APP_DIR)
}

// ---- windows ----

pub fn show_popup(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(POPUP) {
        set_popup_wide(app, false);
        let _ = w.show();
        let _ = w.set_focus();
        let _ = w.emit("popup-shown", ());
    }
}

pub fn hide_popup_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(POPUP) {
        let _ = w.hide();
    }
}

pub fn toggle_popup(app: &AppHandle) {
    match app.get_webview_window(POPUP) {
        Some(w) if w.is_visible().unwrap_or(false) => hide_popup_window(app),
        Some(_) => show_popup(app),
        None => {}
    }
}

/// Widen the popup for the preview panel, or back to the list only.
pub fn set_popup_wide(app: &AppHandle, wide: bool) {
    if let Some(w) = app.get_webview_window(POPUP) {
        let width = if wide { POPUP_PREVIEW_WIDTH } else { POPUP_WIDTH };
        let _ = w.set_size(LogicalSize::new(width + SHADOW_MARGIN_X, POPUP_HEIGHT + SHADOW_MARGIN_Y));
    }
}

fn build_window(app: &AppHandle, label: &str, url: String, title: &str, width: f64, height: f64) -> tauri::Result<()> {
    let size = LogicalSize::new(width + SHADOW_MARGIN_X, height + SHADOW_MARGIN_Y);
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(size.width, size.height)
        .decorations(false)
        .transparent(true)
        .resizable(false)
        .center()
        .build()?;
    // GTK can map the window larger than requested; insist on the exact size.
    window.set_size(size)?;
    Ok(())
}

/// Open (or focus) the settings window on `page`, optionally asking to clear history.
pub fn open_settings(app: &AppHandle, page: Option<&str>, confirm_clear: bool) {
    hide_popup_window(app);
    let page = page.unwrap_or("general");
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.show();
        let _ = w.set_focus();
        let _ = w.emit("open-page", serde_json::json!({ "page": page, "clear": confirm_clear }));
        return;
    }
    let url = format!("settings.html?page={page}&clear={confirm_clear}");
    if let Err(e) = build_window(app, SETTINGS, url, "Clips Settings", 720.0, 560.0) {
        eprintln!("settings window: {e}");
    }
}

fn open_welcome(app: &AppHandle) {
    if let Err(e) = build_window(app, WELCOME, "welcome.html".into(), "Welcome to Clips", 480.0, 520.0) {
        eprintln!("welcome window: {e}");
    }
}

// ---- recording ----

pub fn history_changed(app: &AppHandle) {
    let _ = app.emit_to(POPUP, "history-changed", ());
}

/// Tell every window the settings changed (theme, shortcut, …).
pub fn settings_changed(app: &AppHandle) {
    let _ = app.emit("settings-changed", app.state::<AppState>().config());
    tray::refresh(app);
}

pub fn record_text(app: &AppHandle, text: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.paused.load(Ordering::Relaxed) {
        return Ok(());
    }
    state.history.add_text(text)?;
    history_changed(app);
    Ok(())
}

pub fn record_image_encoded(app: &AppHandle, data: &[u8]) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.paused.load(Ordering::Relaxed) {
        return Ok(());
    }
    state.history.add_image_encoded(data)?;
    history_changed(app);
    Ok(())
}

/// Fallback capture when the GNOME Shell helper is not running: poll every
/// 500 ms (images only every 2 s, reading them is expensive).
fn spawn_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<String> = None;
        let mut tick: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            tick += 1;
            let state = app.state::<AppState>();
            if state.clipboard.helper_active() || !state.config().poll_fallback {
                continue;
            }
            let Some(snapshot) = state.clipboard.read(tick % 4 == 0) else { continue };
            let hash = match &snapshot {
                Snapshot::Text(t) => db::sha256_hex(t.as_bytes()),
                Snapshot::Image { width, height, rgba } => history::image_hash(*width, *height, rgba),
            };
            if last.as_deref() == Some(hash.as_str()) {
                continue;
            }
            let ours = state.own_clipboard.lock().unwrap_or_else(|e| e.into_inner()).take();
            let is_ours = ours.as_deref() == Some(hash.as_str());
            last = Some(hash);
            if is_ours {
                continue;
            }
            // Content copied while paused is remembered as "seen" but not stored.
            if state.paused.load(Ordering::Relaxed) {
                continue;
            }
            let result = match snapshot {
                Snapshot::Text(t) => state.history.add_text(&t),
                Snapshot::Image { width, height, rgba } => state.history.add_image_rgba(width, height, rgba),
            };
            match result {
                Ok(_) => history_changed(&app),
                Err(e) => eprintln!("poll: could not record clipboard: {e}"),
            }
        }
    });
}

/// Hourly cleanup for the "Keep history for" setting.
fn spawn_expiry(app: AppHandle) {
    std::thread::spawn(move || loop {
        let state = app.state::<AppState>();
        match state.history.expire(state.config().retention_days) {
            Ok(()) => history_changed(&app),
            Err(e) => eprintln!("expiry: {e}"),
        }
        std::thread::sleep(Duration::from_secs(60 * 60));
    });
}

/// Keep config.hotkey and the GNOME custom shortcut in step. An existing
/// shortcut (possibly edited in GNOME Settings) wins; otherwise create one.
fn sync_shortcut(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut config = state.config();
    match gnome::find_binding() {
        Some((_, binding)) if binding != config.hotkey => {
            config.hotkey = binding;
            if let Err(e) = config.save(&config_path()) {
                eprintln!("config: {e}");
            }
            state.set_config(config);
        }
        Some(_) => {}
        None if !config.hotkey.is_empty() => {
            if let Err(e) = gnome::set_binding(&config.hotkey) {
                eprintln!("shortcut: could not create GNOME shortcut: {e}");
            }
        }
        None => {}
    }
}

fn start_dbus(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let (conn, shell) = match dbus::start(app.clone()).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("dbus: could not start service: {e}");
                return;
            }
        };
        // Owning the connection here keeps the bus name for the app's lifetime.
        let _conn = conn;
        let state = app.state::<AppState>();
        state.clipboard.set_shell(shell);
        // Keep checking, the extension may be enabled or reloaded at any time.
        loop {
            state.clipboard.refresh_helper().await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

pub fn apply_autostart(app: &AppHandle, enabled: bool) {
    let autolaunch = app.autolaunch();
    let result = if enabled { autolaunch.enable() } else { autolaunch.disable() };
    if let Err(e) = result {
        eprintln!("autostart: {e}");
    }
}

// ---- entry point ----

pub fn run() {
    let config = Config::load_or_create(&config_path());
    let history = match History::open(&data_dir(), &config) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("could not open history in {}: {e}", data_dir().display());
            std::process::exit(1);
        }
    };
    let start_hidden = std::env::args().any(|a| a == "--hidden");

    let app = tauri::Builder::default()
        // Must be first: a second launch (e.g. `linux-clipboard-manager --toggle`
        // from a keyboard shortcut) toggles the popup of the running instance,
        // `--settings` opens the settings window.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--settings") {
                open_settings(app, None, false);
            } else {
                toggle_popup(app);
            }
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            config: RwLock::new(config),
            history,
            clipboard: Clipboard::new(),
            paused: AtomicBool::new(false),
            dialog_open: AtomicBool::new(false),
            own_clipboard: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_entries,
            commands::get_entry,
            commands::get_image,
            commands::paste_entry,
            commands::paste_entries,
            commands::copy_entry,
            commands::toggle_pin,
            commands::delete_entry,
            commands::delete_entries,
            commands::open_entry,
            commands::save_image_as,
            commands::hide_popup,
            commands::set_preview,
            commands::get_settings,
            commands::save_settings,
            commands::check_shortcut,
            commands::get_status,
            commands::clear_history,
            commands::open_settings_window,
            commands::close_window,
            commands::finish_welcome,
            commands::enable_extension,
            commands::open_link
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = handle.state::<AppState>();

            apply_autostart(&handle, state.config().autostart);
            sync_shortcut(&handle);
            tray::build(&handle)?;
            start_dbus(handle.clone());
            // Always running; it checks the poll_fallback setting on every tick.
            spawn_poller(handle.clone());
            spawn_expiry(handle.clone());
            if !state.config().first_run_done {
                open_welcome(&handle);
            } else if !start_hidden {
                show_popup(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Only the popup is kept around; settings and welcome close normally.
            if window.label() != POPUP {
                return;
            }
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                WindowEvent::Focused(false) => {
                    let state = window.state::<AppState>();
                    if !state.dialog_open.load(Ordering::Relaxed) {
                        let _ = window.hide();
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app, event| {
        // Keep running in the tray when the popup is hidden; only Quit exits.
        if let RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
}
