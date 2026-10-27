//! Recording clipboard content: hashing, image files, thumbnails and pruning
//! on top of [`Db`].

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use image::{ImageFormat, RgbaImage};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::db::{Db, Entry, Filter, NewImage};

pub const THUMB_WIDTH: u32 = 256;
pub const THUMB_HEIGHT: u32 = 128;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Content hash of decoded pixels, so the same picture copied from two apps
/// (which may encode the PNG differently) is stored once.
pub fn image_hash(width: u32, height: u32, rgba: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(width.to_le_bytes());
    h.update(height.to_le_bytes());
    h.update(rgba);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// PNG that fits inside THUMB_WIDTH x THUMB_HEIGHT, keeping the aspect ratio.
pub fn thumbnail_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    if img.width() <= THUMB_WIDTH && img.height() <= THUMB_HEIGHT {
        return encode_png(img);
    }
    let thumb = image::DynamicImage::ImageRgba8(img.clone()).thumbnail(THUMB_WIDTH, THUMB_HEIGHT);
    encode_png(&thumb.to_rgba8())
}

pub struct History {
    db: Mutex<Db>,
    images_dir: PathBuf,
    max_items: AtomicUsize,
    max_image_bytes: AtomicUsize,
}

impl History {
    pub fn open(data_dir: &Path, config: &Config) -> Result<History, String> {
        let images_dir = data_dir.join("images");
        std::fs::create_dir_all(&images_dir).map_err(|e| e.to_string())?;
        let db = Db::open(&data_dir.join("history.db")).map_err(|e| e.to_string())?;
        Ok(History {
            db: Mutex::new(db),
            images_dir,
            max_items: AtomicUsize::new(config.max_items),
            max_image_bytes: AtomicUsize::new(config.max_image_bytes),
        })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Db> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Apply changed settings, dropping entries beyond a lowered limit right away.
    pub fn set_limits(&self, config: &Config) {
        self.max_items.store(config.max_items, Ordering::Relaxed);
        self.max_image_bytes.store(config.max_image_bytes, Ordering::Relaxed);
        self.prune(&self.db());
    }

    fn max_image_bytes(&self) -> usize {
        self.max_image_bytes.load(Ordering::Relaxed)
    }

    fn prune(&self, db: &Db) {
        match db.prune(self.max_items.load(Ordering::Relaxed)) {
            Ok(paths) => remove_files(&paths),
            Err(e) => eprintln!("history: prune failed: {e}"),
        }
    }

    /// Record text. Returns true if it was not already in history.
    pub fn add_text(&self, text: &str) -> Result<bool, String> {
        if text.is_empty() {
            return Ok(false);
        }
        let db = self.db();
        let ins = db.insert_text(text, now_ms()).map_err(|e| e.to_string())?;
        if ins.is_new {
            self.prune(&db);
        }
        Ok(ins.is_new)
    }

    /// Record an image given as PNG (or any format the `image` crate reads).
    pub fn add_image_encoded(&self, data: &[u8]) -> Result<bool, String> {
        if data.len() > self.max_image_bytes() {
            return Err(format!("image is {} bytes, limit is {}", data.len(), self.max_image_bytes()));
        }
        let img = image::load_from_memory(data).map_err(|e| e.to_string())?.to_rgba8();
        let is_png = data.starts_with(b"\x89PNG");
        self.store_image(img, if is_png { Some(data) } else { None })
    }

    /// Record raw RGBA pixels (what arboard hands us).
    pub fn add_image_rgba(&self, width: u32, height: u32, rgba: Vec<u8>) -> Result<bool, String> {
        let img = RgbaImage::from_raw(width, height, rgba).ok_or("pixel buffer does not match size")?;
        self.store_image(img, None)
    }

    fn store_image(&self, img: RgbaImage, png: Option<&[u8]>) -> Result<bool, String> {
        let hash = image_hash(img.width(), img.height(), img.as_raw());
        if self.db().touch_existing(&hash, now_ms()).map_err(|e| e.to_string())?.is_some() {
            return Ok(false);
        }
        let encoded;
        let png = match png {
            Some(p) => p,
            None => {
                encoded = encode_png(&img)?;
                &encoded
            }
        };
        if png.len() > self.max_image_bytes() {
            return Err(format!("image is {} bytes, limit is {}", png.len(), self.max_image_bytes()));
        }
        let path = self.images_dir.join(format!("{hash}.png"));
        std::fs::write(&path, png).map_err(|e| e.to_string())?;
        let thumbnail = thumbnail_png(&img)?;
        let db = self.db();
        let ins = db
            .insert_image(
                &NewImage {
                    hash: &hash,
                    path: &path,
                    thumbnail: &thumbnail,
                    width: img.width(),
                    height: img.height(),
                    size: png.len() as i64,
                },
                now_ms(),
            )
            .map_err(|e| e.to_string())?;
        if ins.is_new {
            self.prune(&db);
        }
        Ok(ins.is_new)
    }

    pub fn list(&self, filter: &Filter, limit: usize) -> Result<Vec<Entry>, String> {
        self.db().list(filter, limit).map_err(|e| e.to_string())
    }

    pub fn count(&self, filter: &Filter) -> Result<i64, String> {
        self.db().count_matching(filter).map_err(|e| e.to_string())
    }

    /// Drop unpinned entries not used for `days` days (0 keeps everything).
    pub fn expire(&self, days: u32) -> Result<(), String> {
        if days == 0 {
            return Ok(());
        }
        let cutoff = now_ms() - i64::from(days) * 24 * 60 * 60 * 1000;
        let paths = self.db().delete_unused_since(cutoff).map_err(|e| e.to_string())?;
        remove_files(&paths);
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Option<Entry>, String> {
        self.db().get(id).map_err(|e| e.to_string())
    }

    pub fn touch(&self, id: i64) -> Result<(), String> {
        self.db().touch(id, now_ms()).map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn toggle_pin(&self, id: i64) -> Result<Option<bool>, String> {
        self.db().toggle_pin(id).map_err(|e| e.to_string())
    }

    pub fn delete(&self, id: i64) -> Result<(), String> {
        let path = self.db().delete(id).map_err(|e| e.to_string())?;
        remove_files(path.as_slice());
        Ok(())
    }

    pub fn clear_unpinned(&self) -> Result<(), String> {
        let paths = self.db().clear_unpinned().map_err(|e| e.to_string())?;
        remove_files(&paths);
        Ok(())
    }
}

fn remove_files(paths: &[PathBuf]) {
    for p in paths {
        if let Err(e) = std::fs::remove_file(p) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("history: could not remove {}: {e}", p.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(dir: &Path, max_items: usize) -> History {
        let config = Config { max_items, ..Config::default() };
        History::open(dir, &config).unwrap()
    }

    #[test]
    fn thumbnail_fits_box_and_keeps_aspect() {
        let img = RgbaImage::new(1024, 256);
        let thumb = image::load_from_memory(&thumbnail_png(&img).unwrap()).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (256, 64));
        let small = RgbaImage::new(20, 10);
        let thumb = image::load_from_memory(&thumbnail_png(&small).unwrap()).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (20, 10));
    }

    #[test]
    fn images_are_written_deduped_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let h = history(dir.path(), 1);
        let red = RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        assert!(h.add_image_rgba(4, 4, red.as_raw().clone()).unwrap());
        // Same pixels arriving as PNG are recognised as the same image.
        assert!(!h.add_image_encoded(&encode_png(&red).unwrap()).unwrap());

        let entries = h.list(&Filter::default(), 10).unwrap();
        let path = PathBuf::from(entries[0].image_path.clone().unwrap());
        assert!(path.exists());
        assert!(entries[0].thumbnail.is_some());

        // max_items = 1, so new text pushes the image (and its file) out.
        assert!(h.add_text("hi").unwrap());
        assert_eq!(h.list(&Filter::default(), 10).unwrap().len(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn oversized_and_invalid_images_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config { max_image_bytes: 10, ..Config::default() };
        let h = History::open(dir.path(), &config).unwrap();
        assert!(h.add_image_encoded(&[0u8; 11]).is_err());
        assert!(h.add_image_encoded(b"nope").is_err());
        assert!(h.add_image_rgba(2, 2, vec![0; 3]).is_err());
        assert!(!h.add_text("").unwrap());
    }

    #[test]
    fn lowering_max_items_prunes_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let h = history(dir.path(), 10);
        for t in ["a", "b", "c"] {
            h.add_text(t).unwrap();
        }
        h.set_limits(&Config { max_items: 1, ..Config::default() });
        assert_eq!(h.list(&Filter::default(), 10).unwrap().len(), 1);
    }
}
