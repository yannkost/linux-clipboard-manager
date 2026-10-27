//! Panel icon and its menu: show, pause, clear, settings, quit. The menu is
//! rebuilt when the pause state or the shortcut changes.

use std::sync::atomic::Ordering;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::{gnome, AppState};

const TRAY_ID: &str = "main";

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
        _ => {}
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

/// Rebuild the menu and icon after pause or settings changes.
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
    fn formats_hotkeys() {
        assert_eq!(pretty_hotkey("<Control>i"), "Ctrl+I");
        assert_eq!(pretty_hotkey("<Primary><Alt>t"), "Ctrl+Alt+T");
        assert_eq!(pretty_hotkey(""), "");
    }
}
