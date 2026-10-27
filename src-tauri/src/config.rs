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
        if self.hotkey.trim().is_empty() {
            return Err("The shortcut cannot be empty.".into());
        }
        Ok(())
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
