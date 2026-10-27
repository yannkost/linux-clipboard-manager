//! Tauri commands called by the popup, settings and welcome pages.

use std::process::Command;
use std::sync::atomic::Ordering;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::clipboard::Content;
use crate::config::Config;
use crate::db::{Entry, Filter, Kind};
use crate::{gnome, AppState, POPUP};

const LIST_LIMIT: usize = 300;
const REPO_URL: &str = "https://github.com/yannkost/linux-clipboard-manager";

fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

#[derive(Serialize)]
pub struct EntryView {
    #[serde(flatten)]
    entry: Entry,
    /// data: URL of the PNG thumbnail for images.
    thumbnail: Option<String>,
}

impl From<Entry> for EntryView {
    fn from(mut entry: Entry) -> Self {
        let thumbnail = entry.thumbnail.take().map(|t| data_url(&t));
        EntryView { entry, thumbnail }
    }
}

#[derive(Serialize)]
pub struct ListResult {
    entries: Vec<EntryView>,
    recent_count: i64,
    pinned_count: i64,
}

fn get(state: &AppState, id: i64) -> Result<Entry, String> {
    state.history.get(id)?.ok_or_else(|| "This entry no longer exists.".into())
}

/// `tab` is "recent" or "pinned"; `filter` is "all", "text" or "images".
#[tauri::command]
pub fn list_entries(
    state: tauri::State<'_, AppState>,
    query: String,
    tab: String,
    filter: String,
) -> Result<ListResult, String> {
    let kind = match filter.as_str() {
        "text" => Some(Kind::Text),
        "images" => Some(Kind::Image),
        _ => None,
    };
    let base = Filter { query: &query, kind, pinned: None };
    let entries = state.history.list(&Filter { pinned: Some(tab == "pinned"), ..base.clone() }, LIST_LIMIT)?;
    Ok(ListResult {
        entries: entries.into_iter().map(EntryView::from).collect(),
        recent_count: state.history.count(&Filter { pinned: Some(false), ..base.clone() })?,
        pinned_count: state.history.count(&Filter { pinned: Some(true), ..base })?,
    })
}

/// One entry with its full text, for the preview panel.
#[tauri::command]
pub fn get_entry(state: tauri::State<'_, AppState>, id: i64) -> Result<EntryView, String> {
    get(&state, id).map(EntryView::from)
}

/// The full-size image as a data: URL.
#[tauri::command]
pub fn get_image(state: tauri::State<'_, AppState>, id: i64) -> Result<String, String> {
    let path = get(&state, id)?.image_path.ok_or("Not an image.")?;
    let png = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    Ok(data_url(&png))
}

fn content_of(entry: &Entry) -> Result<Content, String> {
    match entry.kind {
        Kind::Text => Ok(Content::Text(entry.text.clone().unwrap_or_default())),
        Kind::Image => {
            let path = entry.image_path.as_deref().ok_or("image file missing")?;
            Ok(Content::Image(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?))
        }
    }
}

/// Put `content` on the clipboard and, if auto-paste is on, paste it.
/// Returns whether a paste was sent.
async fn deliver(app: &AppHandle, content: Content, hash: String, ids: &[i64], paste: bool, plain: bool) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let config = state.config();
    if config.move_to_top {
        for id in ids {
            state.history.touch(*id)?;
        }
    }
    *state.own_clipboard.lock().unwrap_or_else(|e| e.into_inner()) = Some(hash);
    let paste = (paste && config.paste_after_select).then_some(plain);
    let pasted = state.clipboard.set(&content, paste).await?;
    crate::history_changed(app);
    Ok(pasted)
}

/// Use one entry: copy it, then paste it if auto-paste is on.
pub async fn use_entry(app: &AppHandle, id: i64, plain: bool) -> Result<bool, String> {
    let entry = get(&app.state::<AppState>(), id)?;
    let content = content_of(&entry)?;
    crate::hide_popup_window(app);
    deliver(app, content, entry.hash, &[id], true, plain).await
}

#[tauri::command]
pub async fn paste_entry(app: AppHandle, id: i64, plain: bool) -> Result<bool, String> {
    use_entry(&app, id, plain).await
}

/// Paste several text entries joined by newlines, in the given order.
#[tauri::command]
pub async fn paste_entries(app: AppHandle, ids: Vec<i64>, plain: bool) -> Result<bool, String> {
    if let [id] = ids[..] {
        return use_entry(&app, id, plain).await;
    }
    let texts = {
        let state = app.state::<AppState>();
        let mut texts = Vec::new();
        for id in &ids {
            if let Some(t) = get(&state, *id)?.text {
                texts.push(t);
            }
        }
        texts
    };
    if texts.is_empty() {
        return Err("Images can only be pasted one at a time.".into());
    }
    let joined = texts.join("\n");
    let hash = crate::db::sha256_hex(joined.as_bytes());
    crate::hide_popup_window(&app);
    deliver(&app, Content::Text(joined), hash, &ids, true, plain).await
}

/// Copy without pasting or closing the popup (preview "Copy" button).
#[tauri::command]
pub async fn copy_entry(app: AppHandle, id: i64) -> Result<(), String> {
    let entry = get(&app.state::<AppState>(), id)?;
    let content = content_of(&entry)?;
    deliver(&app, content, entry.hash, &[id], false, false).await.map(|_| ())
}

#[tauri::command]
pub fn toggle_pin(app: AppHandle, state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    let pinned = state.history.toggle_pin(id)?.ok_or("This entry no longer exists.")?;
    crate::history_changed(&app);
    Ok(pinned)
}

#[tauri::command]
pub fn delete_entry(app: AppHandle, state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    state.history.delete(id)?;
    crate::history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_entries(app: AppHandle, state: tauri::State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    for id in ids {
        state.history.delete(id)?;
    }
    crate::history_changed(&app);
    Ok(())
}

fn xdg_open(target: &str) -> Result<(), String> {
    Command::new("xdg-open").arg(target).spawn().map(|_| ()).map_err(|e| format!("xdg-open: {e}"))
}

/// Open a link, file path or image in its default app.
#[tauri::command]
pub fn open_entry(app: AppHandle, state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let entry = get(&state, id)?;
    let target = match entry.kind {
        Kind::Image => entry.image_path.ok_or("image file missing")?,
        Kind::Text => {
            let text = entry.text.unwrap_or_default();
            let text = text.trim();
            if let Some(rest) = text.strip_prefix("~/") {
                dirs::home_dir().ok_or("no home folder")?.join(rest).to_string_lossy().into_owned()
            } else if ["http://", "https://", "file://", "/"].iter().any(|p| text.starts_with(p)) {
                text.to_string()
            } else {
                return Err("Only links and file paths can be opened.".into());
            }
        }
    };
    crate::hide_popup_window(&app);
    xdg_open(&target)
}

/// Ask where to save an image entry. Returns the saved path, or None if cancelled.
#[tauri::command]
pub async fn save_image_as(app: AppHandle, id: i64) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let src = get(&state, id)?.image_path.ok_or("Not an image.")?;
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Save image")
        .set_file_name(format!("clip-{id}.png"))
        .add_filter("PNG image", &["png"]);
    if let Some(dir) = dirs::picture_dir() {
        dialog = dialog.set_directory(dir);
    }
    if let Some(popup) = app.get_webview_window(POPUP) {
        dialog = dialog.set_parent(&popup);
    }
    state.dialog_open.store(true, Ordering::Relaxed);
    let picked = dialog.blocking_save_file();
    state.dialog_open.store(false, Ordering::Relaxed);
    let Some(dest) = picked else { return Ok(None) };
    let dest = dest.into_path().map_err(|e| e.to_string())?;
    std::fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(Some(dest.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn hide_popup(app: AppHandle) {
    crate::hide_popup_window(&app);
}

#[tauri::command]
pub fn set_preview(app: AppHandle, open: bool) {
    crate::set_popup_wide(&app, open);
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Config {
    state.config()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: tauri::State<'_, AppState>, settings: Config) -> Result<Config, String> {
    settings.validate()?;
    let old = state.config();
    if settings.hotkey != old.hotkey {
        gnome::set_binding(&settings.hotkey).map_err(|e| format!("Could not change the GNOME shortcut: {e}"))?;
    }
    settings.save(&crate::config_path()).map_err(|e| format!("Could not save settings: {e}"))?;
    state.set_config(settings.clone());
    state.history.set_limits(&settings);
    if settings.retention_days != old.retention_days {
        state.history.expire(settings.retention_days)?;
    }
    if settings.autostart != old.autostart {
        crate::apply_autostart(&app, settings.autostart);
    }
    crate::settings_changed(&app);
    crate::history_changed(&app);
    Ok(settings)
}

/// The GNOME action already using `accel`, if any.
#[tauri::command]
pub fn check_shortcut(accel: String) -> Option<String> {
    gnome::find_conflict(&accel)
}

#[derive(Serialize)]
pub struct Status {
    helper_active: bool,
    prefers_dark: bool,
    paused: bool,
    version: &'static str,
}

#[tauri::command]
pub fn get_status(state: tauri::State<'_, AppState>) -> Status {
    Status {
        helper_active: state.clipboard.helper_active(),
        prefers_dark: gnome::prefers_dark(),
        paused: state.paused.load(Ordering::Relaxed),
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Delete every unpinned entry.
#[tauri::command]
pub fn clear_history(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.history.clear_unpinned()?;
    crate::history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle, page: Option<String>) {
    crate::open_settings(&app, page.as_deref(), false);
}

#[tauri::command]
pub fn close_window(window: tauri::WebviewWindow) {
    let _ = window.close();
}

/// Close the welcome dialog for good and show the popup.
#[tauri::command]
pub fn finish_welcome(app: AppHandle, state: tauri::State<'_, AppState>, window: tauri::WebviewWindow) -> Result<(), String> {
    let mut config = state.config();
    config.first_run_done = true;
    config.save(&crate::config_path()).map_err(|e| e.to_string())?;
    state.set_config(config);
    let _ = window.close();
    crate::show_popup(&app);
    Ok(())
}

#[tauri::command]
pub fn enable_extension() -> Result<(), String> {
    gnome::enable_extension()
}

/// About page links: "website", "source" or "issues".
#[tauri::command]
pub fn open_link(which: String) -> Result<(), String> {
    let url = match which.as_str() {
        "issues" => format!("{REPO_URL}/issues"),
        _ => REPO_URL.to_string(),
    };
    xdg_open(&url)
}
