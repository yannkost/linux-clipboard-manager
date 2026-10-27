mod clipboard;
mod config;
mod db;
mod dbus;
mod history;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use clipboard::{Clipboard, Content, Snapshot};
use config::Config;
use db::{Entry, Kind};
use history::History;

const APP_DIR: &str = "linux-clipboard-manager";
const POPUP: &str = "popup";
const LIST_LIMIT: usize = 200;

pub struct AppState {
    pub config: Config,
    pub history: History,
    pub clipboard: Clipboard,
    pub paused: AtomicBool,
}

fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config")).join(APP_DIR).join("config.toml")
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".local/share")).join(APP_DIR)
}

// ---- popup window ----

pub fn show_popup(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(POPUP) {
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

// ---- recording ----

fn history_changed(app: &AppHandle) {
    let _ = app.emit_to(POPUP, "history-changed", ());
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
            if state.clipboard.helper_active() {
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
            last = Some(hash);
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

// ---- commands for the UI ----

#[derive(Serialize)]
struct EntryView {
    #[serde(flatten)]
    entry: Entry,
    /// data: URL of the PNG thumbnail for images.
    thumbnail: Option<String>,
}

#[tauri::command]
fn list_entries(state: tauri::State<'_, AppState>, query: Option<String>) -> Result<Vec<EntryView>, String> {
    let entries = state.history.list(query.as_deref().unwrap_or(""), LIST_LIMIT)?;
    Ok(entries
        .into_iter()
        .map(|mut entry| {
            let thumbnail = entry.thumbnail.take().map(|t| {
                format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(t))
            });
            EntryView { entry, thumbnail }
        })
        .collect())
}

#[tauri::command]
async fn paste_entry(app: AppHandle, id: i64, plain: Option<bool>) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let entry = state.history.get(id)?.ok_or("entry not found")?;
    let content = match entry.kind {
        Kind::Text => Content::Text(entry.text.unwrap_or_default()),
        Kind::Image => {
            let path = entry.image_path.ok_or("image file missing")?;
            Content::Image(std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?)
        }
    };
    hide_popup_window(&app);
    state.history.touch(id)?;
    let paste = state.config.paste_after_select.then_some(plain.unwrap_or(false));
    let pasted = state.clipboard.set(&content, paste).await?;
    history_changed(&app);
    Ok(pasted)
}

#[tauri::command]
fn toggle_pin(app: AppHandle, state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    let pinned = state.history.toggle_pin(id)?.ok_or("entry not found")?;
    history_changed(&app);
    Ok(pinned)
}

#[tauri::command]
fn delete_entry(app: AppHandle, state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    state.history.delete(id)?;
    history_changed(&app);
    Ok(())
}

#[tauri::command]
fn hide_popup(app: AppHandle) {
    hide_popup_window(&app);
}

// ---- tray ----

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show clipboard", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause recording", true, false, None::<&str>)?;
    let clear = MenuItem::with_id(app, "clear", "Clear history", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &clear,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Clipboard history")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_popup(app),
            "pause" => {
                let paused = pause.is_checked().unwrap_or(false);
                app.state::<AppState>().paused.store(paused, Ordering::Relaxed);
            }
            "clear" => {
                if let Err(e) = app.state::<AppState>().history.clear_unpinned() {
                    eprintln!("tray: clear failed: {e}");
                }
                history_changed(app);
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
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
        // from a keyboard shortcut) toggles the popup of the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| toggle_popup(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .manage(AppState {
            config,
            history,
            clipboard: Clipboard::new(),
            paused: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            list_entries,
            paste_entry,
            toggle_pin,
            delete_entry,
            hide_popup
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = handle.state::<AppState>();

            let autolaunch = handle.autolaunch();
            let result =
                if state.config.autostart { autolaunch.enable() } else { autolaunch.disable() };
            if let Err(e) = result {
                eprintln!("autostart: {e}");
            }

            build_tray(&handle)?;
            start_dbus(handle.clone());
            if state.config.poll_fallback {
                spawn_poller(handle.clone());
            }
            if !start_hidden {
                show_popup(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(false) => {
                let _ = window.hide();
            }
            _ => {}
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
