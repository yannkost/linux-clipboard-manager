//! GNOME desktop integration through `gsettings`: the custom keyboard
//! shortcut that opens the popup, shortcut conflicts, and the colour scheme.

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

/// Point our custom shortcut at `accel` (empty disables it), creating it if needed.
pub fn set_binding(accel: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let command = format!("{} --toggle", exe.display());
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

/// Ask GNOME Shell to enable the helper extension.
pub fn enable_extension() -> Result<(), String> {
    let out = Command::new("gnome-extensions")
        .args(["enable", EXTENSION_UUID])
        .output()
        .map_err(|e| format!("gnome-extensions: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err("The Clips GNOME extension is not installed yet.".into())
    }
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
