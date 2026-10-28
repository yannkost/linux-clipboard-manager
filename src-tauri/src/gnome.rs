//! GNOME desktop integration through `gsettings`: the custom keyboard
//! shortcut that opens the popup, shortcut conflicts, and the colour scheme.

use std::path::PathBuf;
use std::process::Command;

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const OUR_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/linux-clipboard-manager/";
/// UUID of the GNOME Shell helper extension.
pub const EXTENSION_UUID: &str = "clips-helper@yannkost.github.io";

fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = Command::new("gsettings").args(args).output().map_err(|e| format!("gsettings: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Parse a GVariant string ('text') or string array (['a', 'b']) as printed by gsettings.
fn parse_strings(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\'' {
            let mut s = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(n) = chars.next() {
                            s.push(n);
                        }
                    }
                    '\'' => break,
                    c => s.push(c),
                }
            }
            out.push(s);
        }
    }
    out
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn custom_paths() -> Vec<String> {
    gsettings(&["get", MEDIA_KEYS, "custom-keybindings"]).map(|v| parse_strings(&v)).unwrap_or_default()
}

fn custom_get(path: &str, key: &str) -> Option<String> {
    let schema = format!("{CUSTOM_SCHEMA}:{path}");
    gsettings(&["get", &schema, key]).ok().and_then(|v| parse_strings(&v).into_iter().next())
}

fn is_ours(command: &str) -> bool {
    command.contains("linux-clipboard-manager") && command.contains("--toggle")
}

/// Our custom shortcut, if one exists: (dconf path, accelerator).
pub fn find_binding() -> Option<(String, String)> {
    custom_paths().into_iter().find_map(|path| {
        let command = custom_get(&path, "command")?;
        is_ours(&command).then(|| (path.clone(), custom_get(&path, "binding").unwrap_or_default()))
    })
}

fn toggle_command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("{} --toggle", exe.display()))
}

/// Make our shortcut run this binary, e.g. after moving from a dev build to
/// an installed one.
pub fn refresh_binding_command() -> Result<(), String> {
    let Some((path, _)) = find_binding() else { return Ok(()) };
    let command = toggle_command()?;
    if custom_get(&path, "command").as_deref() != Some(command.as_str()) {
        gsettings(&["set", &format!("{CUSTOM_SCHEMA}:{path}"), "command", &quote(&command)])?;
    }
    Ok(())
}

/// Point our custom shortcut at `accel` (empty disables it), creating it if needed.
pub fn set_binding(accel: &str) -> Result<(), String> {
    let command = toggle_command()?;
    let path = match find_binding() {
        Some((path, _)) => path,
        None => {
            let mut paths = custom_paths();
            paths.push(OUR_PATH.to_string());
            let list = format!("[{}]", paths.iter().map(|p| quote(p)).collect::<Vec<_>>().join(", "));
            gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &list])?;
            gsettings(&["set", &format!("{CUSTOM_SCHEMA}:{OUR_PATH}"), "name", &quote("Clips")])?;
            OUR_PATH.to_string()
        }
    };
    let schema = format!("{CUSTOM_SCHEMA}:{path}");
    gsettings(&["set", &schema, "command", &quote(&command)])?;
    gsettings(&["set", &schema, "binding", &quote(accel)])?;
    Ok(())
}

/// Normalise a GTK accelerator to (sorted modifiers, lowercase key) for comparison.
fn normalize(accel: &str) -> Option<(Vec<String>, String)> {
    let mut mods = Vec::new();
    let mut rest = accel.trim();
    while let Some(start) = rest.strip_prefix('<') {
        let end = start.find('>')?;
        let m = start[..end].to_ascii_lowercase();
        let m = match m.as_str() {
            "ctrl" | "control" | "primary" => "control".to_string(),
            "mod1" | "alt" => "alt".to_string(),
            "mod4" | "super" => "super".to_string(),
            other => other.to_string(),
        };
        mods.push(m);
        rest = &start[end + 1..];
    }
    if rest.is_empty() {
        return None;
    }
    mods.sort();
    mods.dedup();
    Some((mods, rest.to_ascii_lowercase()))
}

/// "switch-to-workspace-1" → "Switch to workspace 1"
fn humanize(key: &str) -> String {
    let words = key.replace(['-', '_'], " ");
    let mut c = words.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Name of the GNOME action already using `accel`, if any (ignoring our own shortcut).
pub fn find_conflict(accel: &str) -> Option<String> {
    let wanted = normalize(accel)?;
    let schemas = [
        "org.gnome.desktop.wm.keybindings",
        "org.gnome.shell.keybindings",
        "org.gnome.mutter.keybindings",
        "org.gnome.mutter.wayland.keybindings",
        MEDIA_KEYS,
    ];
    for schema in schemas {
        let Ok(listing) = gsettings(&["list-recursively", schema]) else { continue };
        for line in listing.lines() {
            let mut parts = line.splitn(3, ' ');
            let (Some(_), Some(key), Some(value)) = (parts.next(), parts.next(), parts.next()) else { continue };
            if parse_strings(value).iter().any(|a| normalize(a).as_ref() == Some(&wanted)) {
                return Some(humanize(key));
            }
        }
    }
    for path in custom_paths() {
        if custom_get(&path, "command").is_some_and(|c| is_ours(&c)) {
            continue;
        }
        let binding = custom_get(&path, "binding").unwrap_or_default();
        if normalize(&binding).as_ref() == Some(&wanted) {
            return Some(custom_get(&path, "name").unwrap_or_else(|| "a custom shortcut".into()));
        }
    }
    None
}

/// Whether the desktop prefers a dark colour scheme.
pub fn prefers_dark() -> bool {
    gsettings(&["get", "org.gnome.desktop.interface", "color-scheme"]).is_ok_and(|v| v.contains("dark"))
}

const EXTENSION_FILES: [(&str, &str); 2] = [
    ("metadata.json", include_str!("../../gnome-extension/clips-helper@yannkost.github.io/metadata.json")),
    ("extension.js", include_str!("../../gnome-extension/clips-helper@yannkost.github.io/extension.js")),
];

fn extension_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("gnome-shell/extensions").join(EXTENSION_UUID))
}

pub fn extension_installed() -> bool {
    extension_dir().is_some_and(|d| d.join("extension.js").exists())
}

/// Write the bundled extension into ~/.local/share/gnome-shell/extensions.
/// Returns whether anything changed on disk.
fn write_extension_files() -> Result<bool, String> {
    let dir = extension_dir().ok_or("no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut changed = false;
    for (name, body) in EXTENSION_FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(body) {
            std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
            changed = true;
        }
    }
    Ok(changed)
}

/// Keep an installed extension in step with this build of Clips. GNOME Shell
/// loads the new code at the next login.
pub fn update_installed_extension() {
    if extension_installed() {
        if let Err(e) = write_extension_files() {
            eprintln!("extension: could not update: {e}");
        }
    }
}

/// Install the helper extension and mark it enabled. Returns true if GNOME
/// Shell started it right away; on Wayland a newly installed extension only
/// loads after logging out and back in.
pub fn install_extension() -> Result<bool, String> {
    write_extension_files()?;
    let mut enabled = gsettings(&["get", "org.gnome.shell", "enabled-extensions"]).map(|v| parse_strings(&v))?;
    if !enabled.iter().any(|u| u == EXTENSION_UUID) {
        enabled.push(EXTENSION_UUID.to_string());
        let list = format!("[{}]", enabled.iter().map(|u| quote(u)).collect::<Vec<_>>().join(", "));
        gsettings(&["set", "org.gnome.shell", "enabled-extensions", &list])?;
    }
    let started = Command::new("gnome-extensions")
        .args(["enable", EXTENSION_UUID])
        .output()
        .is_ok_and(|o| o.status.success());
    Ok(started)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gvariant_strings() {
        assert_eq!(parse_strings("['<Super>t', '<Alt>F2']"), vec!["<Super>t", "<Alt>F2"]);
        assert_eq!(parse_strings("'it\\'s'"), vec!["it's"]);
        assert!(parse_strings("@as []").is_empty());
    }

    #[test]
    fn normalizes_accelerators() {
        assert_eq!(normalize("<Primary><Alt>T"), normalize("<Alt><Control>t"));
        assert_ne!(normalize("<Control>i"), normalize("<Control><Shift>i"));
        assert_eq!(normalize("<Control>"), None);
        assert_eq!(humanize("switch-to-workspace-1"), "Switch to workspace 1");
    }
}
