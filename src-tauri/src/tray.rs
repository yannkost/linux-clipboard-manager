//! Panel icon and its menu: show, the last five entries, pause, clear,
//! settings, quit. The menu is rebuilt whenever the history changes.

use std::sync::atomic::Ordering;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::db::{Filter, Kind};
use crate::{gnome, AppState};

const TRAY_ID: &str = "main";
const RECENT_IN_MENU: usize = 5;
const LABEL_CHARS: usize = 48;

/// "<Control><Alt>t" → "Ctrl+Alt+T"
pub fn pretty_hotkey(accel: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = accel;
    while let Some(start) = rest.strip_prefix('<') {
        let Some(end) = start.find('>') else { break };
        parts.push(match start[..end].to_ascii_lowercase().as_str() {
            "control" | "ctrl" | "primary" => "Ctrl".to_string(),
            "alt" | "mod1" => "Alt".to_string(),
            "super" | "mod4" => "Super".to_string(),
            "shift" => "Shift".to_string(),
            other => other.to_string(),
        });
        rest = &start[end + 1..];
    }
    if !rest.is_empty() {
        let mut c = rest.chars();
        parts.push(c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default());
    }
    parts.join("+")
}

fn one_line(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > LABEL_CHARS {
        format!("{}…", line.chars().take(LABEL_CHARS - 1).collect::<String>().trim_end())
    } else {
        line
    }
}

fn icon(app: &AppHandle) -> tauri::Result<Image<'static>> {
    let paused = app.state::<AppState>().paused.load(Ordering::Relaxed);
    // Light glyph on a dark panel, dark glyph on a light one.
    let bytes: &[u8] = match (paused, gnome::prefers_dark()) {
        (false, true) => include_bytes!("../icons/tray/recording-light.png"),
        (false, false) => include_bytes!("../icons/tray/recording-dark.png"),
        (true, true) => include_bytes!("../icons/tray/paused-light.png"),
        (true, false) => include_bytes!("../icons/tray/paused-dark.png"),
    };
    Image::from_bytes(bytes)
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let hotkey = pretty_hotkey(&state.config().hotkey);
    let show_label = if hotkey.is_empty() { "Show clipboard".to_string() } else { format!("Show clipboard ({hotkey})") };
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "show", show_label, true, None::<&str>)?)?;

    let recent = state
        .history
        .list(&Filter { pinned: Some(false), ..Filter::default() }, RECENT_IN_MENU)
        .unwrap_or_default();
    if !recent.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        for e in recent {
            let label = match e.kind {
                Kind::Text => one_line(e.text.as_deref().unwrap_or_default()),
                Kind::Image => format!("Image {}×{}", e.width.unwrap_or(0), e.height.unwrap_or(0)),
            };
            menu.append(&MenuItem::with_id(app, format!("entry:{}", e.id), label, true, None::<&str>)?)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let paused = state.paused.load(Ordering::Relaxed);
    menu.append(&CheckMenuItem::with_id(app, "pause", "Pause recording", true, paused, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "clear", "Clear history…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?)?;
    Ok(menu)
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    match id {
        "show" => crate::show_popup(app),
        "pause" => {
            let state = app.state::<AppState>();
            state.paused.fetch_xor(true, Ordering::Relaxed);
            refresh(app);
        }
        "clear" => crate::open_settings(app, Some("history"), true),
        "settings" => crate::open_settings(app, None, false),
        "quit" => app.exit(0),
        _ => {
            if let Some(entry_id) = id.strip_prefix("entry:").and_then(|n| n.parse::<i64>().ok()) {
                let app = app.clone();
                let plain = app.state::<AppState>().config().strip_formatting;
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::commands::use_entry(&app, entry_id, plain).await {
                        eprintln!("tray: {e}");
                    }
                });
            }
        }
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(app)?)
        .tooltip("Clips")
        .menu(&menu(app)?)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;
    tray.set_visible(app.state::<AppState>().config().show_tray)?;
    Ok(())
}

/// Rebuild the menu and icon after history, pause or settings changes.
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let result = (|| -> tauri::Result<()> {
        tray.set_menu(Some(menu(app)?))?;
        tray.set_icon(Some(icon(app)?))?;
        tray.set_visible(app.state::<AppState>().config().show_tray)
    })();
    if let Err(e) = result {
        eprintln!("tray: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_hotkeys_and_labels() {
        assert_eq!(pretty_hotkey("<Control>i"), "Ctrl+I");
        assert_eq!(pretty_hotkey("<Primary><Alt>t"), "Ctrl+Alt+T");
        assert_eq!(pretty_hotkey(""), "");
        assert_eq!(one_line("a\n  b"), "a b");
        assert_eq!(one_line(&"x".repeat(60)).chars().count(), LABEL_CHARS);
    }
}
