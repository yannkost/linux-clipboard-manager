//! User settings stored at ~/.config/linux-clipboard-manager/config.toml.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Shortcut that opens the popup, in GTK accelerator syntax.
    pub hotkey: String,
    /// Unpinned entries kept in history.
    pub max_items: usize,
    /// Images larger than this (encoded PNG bytes) are not recorded.
    pub max_image_bytes: usize,
    /// Paste into the previously focused window after choosing an entry.
    pub paste_after_select: bool,
    /// Start hidden in the tray when you log in.
    pub autostart: bool,
    /// Poll the clipboard when the GNOME Shell helper extension is missing.
    pub poll_fallback: bool,
    pub show_tray: bool,
    /// "system", "light" or "dark".
    pub theme: String,
    /// Unpinned entries not used for this many days are deleted (0 = forever).
    pub retention_days: u32,
    /// Skip copies marked as passwords (needs the GNOME helper extension).
    pub ignore_passwords: bool,
    /// Apps whose copies are not saved (needs the GNOME helper extension).
    pub excluded_apps: Vec<String>,
    /// Bump a pasted entry to the top of Recent.
    pub move_to_top: bool,
    /// Enter pastes as plain text and Shift+Enter keeps formatting.
    pub strip_formatting: bool,
    /// The welcome dialog has been shown.
    pub first_run_done: bool,
}

/// Where a clipboard change came from, as reported by the GNOME helper.
#[derive(Debug, Clone, Default)]
pub struct Source {
    pub app_name: String,
    /// Desktop file id, e.g. "signal-desktop.desktop".
    pub app_id: String,
    /// A password manager marked the content as secret.
    pub password: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "<Control>i".into(),
            max_items: 500,
            max_image_bytes: 20 * 1024 * 1024,
            paste_after_select: false,
            autostart: true,
            poll_fallback: true,
            show_tray: true,
            theme: "system".into(),
            retention_days: 0,
            ignore_passwords: true,
            excluded_apps: Vec::new(),
            move_to_top: true,
            strip_formatting: false,
            first_run_done: false,
        }
    }
}

impl Config {
    /// Load the config, writing a default file first if there is none.
    /// A broken file falls back to defaults rather than stopping the app.
    pub fn load_or_create(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                eprintln!("config: ignoring invalid {}: {e}", path.display());
                Config::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Config::default();
                if let Err(e) = config.save(path) {
                    eprintln!("config: could not write {}: {e}", path.display());
                }
                config
            }
            Err(e) => {
                eprintln!("config: could not read {}: {e}", path.display());
                Config::default()
            }
        }
    }

    /// Reject values that would break the app.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=100_000).contains(&self.max_items) {
            return Err("Maximum entries must be between 1 and 100000.".into());
        }
        if self.max_image_bytes < 1024 * 1024 {
            return Err("Maximum image size must be at least 1 MB.".into());
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            return Err("Theme must be system, light or dark.".into());
        }
        if !matches!(self.retention_days, 0 | 1 | 7 | 30) {
            return Err("Keep history for must be forever, 1, 7 or 30 days.".into());
        }
        Ok(())
    }

    /// Whether a copy from `source` must not be recorded (password or excluded app).
    pub fn should_skip(&self, source: &Source) -> bool {
        if source.password && self.ignore_passwords {
            return true;
        }
        let name = source.app_name.to_lowercase();
        let id = source.app_id.to_lowercase();
        self.excluded_apps.iter().map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty()).any(|a| {
            name == a || id == a || id.trim_end_matches(".desktop") == a || id.contains(&a) || name.contains(&a)
        })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_passwords_and_excluded_apps() {
        let source = |name: &str, id: &str, password| Source { app_name: name.into(), app_id: id.into(), password };
        let mut c = Config { excluded_apps: vec!["Signal".into()], ..Config::default() };
        assert!(c.should_skip(&source("Signal", "signal-desktop.desktop", false)));
        assert!(c.should_skip(&source("", "signal-desktop.desktop", false)));
        assert!(!c.should_skip(&source("Firefox", "firefox.desktop", false)));
        assert!(c.should_skip(&source("KeePassXC", "", true)));
        c.ignore_passwords = false;
        assert!(!c.should_skip(&source("KeePassXC", "", true)));
    }

    #[test]
    fn creates_default_file_and_reads_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        assert_eq!(Config::load_or_create(&path), Config::default());
        assert!(std::fs::read_to_string(&path).unwrap().contains("hotkey = \"<Control>i\""));

        std::fs::write(&path, "max_items = 10\n").unwrap();
        let c = Config::load_or_create(&path);
        assert_eq!(c.max_items, 10);
        assert_eq!(c.hotkey, "<Control>i");
    }
}
