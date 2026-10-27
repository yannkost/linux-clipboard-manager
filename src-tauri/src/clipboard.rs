//! Reading and writing the system clipboard.
//!
//! Preferred path: the GNOME Shell helper extension over D-Bus, which can
//! see clipboard changes and paste into other windows on Wayland. Fallback:
//! arboard (X11 / XWayland, or wlr data-control on compositors that offer it).

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::dbus::ShellHelperProxy;

pub enum Content {
    Text(String),
    /// Encoded PNG.
    Image(Vec<u8>),
}

/// What the fallback poller found on the clipboard.
pub enum Snapshot {
    Text(String),
    Image { width: u32, height: u32, rgba: Vec<u8> },
}

pub struct Clipboard {
    // Kept alive for the whole run: on X11 the content we set disappears
    // as soon as the owning handle is dropped.
    arboard: Mutex<Option<arboard::Clipboard>>,
    shell: OnceLock<ShellHelperProxy<'static>>,
    helper_active: AtomicBool,
}

impl Clipboard {
    pub fn new() -> Clipboard {
        Clipboard {
            arboard: Mutex::new(new_arboard()),
            shell: OnceLock::new(),
            helper_active: AtomicBool::new(false),
        }
    }

    pub fn set_shell(&self, proxy: ShellHelperProxy<'static>) {
        let _ = self.shell.set(proxy);
    }

    pub fn helper_active(&self) -> bool {
        self.helper_active.load(Ordering::Relaxed)
    }

    /// Ask the helper extension for its version; returns whether it answered.
    pub async fn refresh_helper(&self) -> bool {
        let active = match self.shell.get() {
            Some(shell) => shell.version().await.is_ok(),
            None => false,
        };
        let was = self.helper_active.swap(active, Ordering::Relaxed);
        if was != active {
            eprintln!(
                "clipboard: GNOME Shell helper {}",
                if active { "connected" } else { "not available, using fallback" }
            );
        }
        active
    }

    /// Put `content` on the clipboard. With `paste = Some(plain)` also ask the
    /// helper to paste it into the previous window. Returns whether a paste
    /// was actually sent (only possible through the helper).
    pub async fn set(&self, content: &Content, paste: Option<bool>) -> Result<bool, String> {
        if self.helper_active() {
            if let Some(shell) = self.shell.get() {
                match self.set_via_helper(shell, content, paste).await {
                    Ok(pasted) => return Ok(pasted),
                    Err(e) => eprintln!("clipboard: helper failed ({e}), using fallback"),
                }
            }
        }
        self.set_via_arboard(content)?;
        Ok(false)
    }

    async fn set_via_helper(
        &self,
        shell: &ShellHelperProxy<'static>,
        content: &Content,
        paste: Option<bool>,
    ) -> zbus::Result<bool> {
        match content {
            Content::Text(t) => shell.set_text(t).await?,
            Content::Image(png) => shell.set_image(png).await?,
        }
        if let Some(plain) = paste {
            shell.paste(plain).await?;
            return Ok(true);
        }
        Ok(false)
    }

    fn set_via_arboard(&self, content: &Content) -> Result<(), String> {
        let mut guard = self.arboard.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = new_arboard();
        }
        let cb = guard.as_mut().ok_or("no clipboard available")?;
        match content {
            Content::Text(t) => cb.set_text(t.as_str()).map_err(|e| e.to_string()),
            Content::Image(png) => {
                let img = image::load_from_memory(png).map_err(|e| e.to_string())?.to_rgba8();
                cb.set_image(arboard::ImageData {
                    width: img.width() as usize,
                    height: img.height() as usize,
                    bytes: Cow::Owned(img.into_raw()),
                })
                .map_err(|e| e.to_string())
            }
        }
    }

    /// Read the clipboard for the fallback poller. Text wins over images;
    /// images are only fetched when `with_images` (they are costly to read).
    pub fn read(&self, with_images: bool) -> Option<Snapshot> {
        let mut guard = self.arboard.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = new_arboard();
        }
        let cb = guard.as_mut()?;
        if let Ok(text) = cb.get_text() {
            if !text.is_empty() {
                return Some(Snapshot::Text(text));
            }
        }
        if with_images {
            if let Ok(img) = cb.get_image() {
                return Some(Snapshot::Image {
                    width: img.width as u32,
                    height: img.height as u32,
                    rgba: img.bytes.into_owned(),
                });
            }
        }
        None
    }
}

fn new_arboard() -> Option<arboard::Clipboard> {
    arboard::Clipboard::new().map_err(|e| eprintln!("clipboard: arboard unavailable: {e}")).ok()
}
