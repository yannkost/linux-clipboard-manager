//! SQLite-backed clipboard history.
//!
//! Text lives directly in the database. Images live as PNG files on disk
//! (the database stores their path plus a small PNG thumbnail). Every entry
//! is keyed by the sha256 of its content, so copying the same thing twice
//! only bumps `last_used_at` instead of creating a duplicate.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

/// Longest text preview returned by [`Db::list`]; the full text is fetched on paste.
pub const PREVIEW_CHARS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Image,
}

impl Kind {
    fn parse(s: &str) -> Kind {
        if s == "image" {
            Kind::Image
        } else {
            Kind::Text
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    pub kind: Kind,
    /// Full text for text entries (a preview in [`Db::list`] results).
    pub text: Option<String>,
    pub image_path: Option<String>,
    #[serde(skip)]
    pub thumbnail: Option<Vec<u8>>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub hash: String,
    /// Content size in bytes (UTF-8 text length or PNG file size).
    pub size: i64,
    pub created_at: i64,
    pub last_used_at: i64,
    pub pinned: bool,
}

/// A new image to record; the PNG file must already be written to `path`.
pub struct NewImage<'a> {
    pub hash: &'a str,
    pub path: &'a Path,
    pub thumbnail: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub size: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inserted {
    pub id: i64,
    /// False when the content was already in history and only got bumped.
    pub is_new: bool,
}

pub struct Db {
    conn: Connection,
}

const COLUMNS: &str =
    "id, kind, text, image_path, thumbnail, width, height, hash, size, created_at, last_used_at, pinned";

fn row_to_entry(row: &Row) -> rusqlite::Result<Entry> {
    let kind: String = row.get(1)?;
    Ok(Entry {
        id: row.get(0)?,
        kind: Kind::parse(&kind),
        text: row.get(2)?,
        image_path: row.get(3)?,
        thumbnail: row.get(4)?,
        width: row.get(5)?,
        height: row.get(6)?,
        hash: row.get(7)?,
        size: row.get(8)?,
        created_at: row.get(9)?,
        last_used_at: row.get(10)?,
        pinned: row.get::<_, i64>(11)? != 0,
    })
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Escape `%`, `_` and `\` for a `LIKE ... ESCAPE '\'` pattern.
fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Db> {
        Db::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Db> {
        Db::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Db> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS entries (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 kind         TEXT    NOT NULL,
                 text         TEXT,
                 image_path   TEXT,
                 thumbnail    BLOB,
                 width        INTEGER,
                 height       INTEGER,
                 hash         TEXT    NOT NULL UNIQUE,
                 size         INTEGER NOT NULL,
                 created_at   INTEGER NOT NULL,
                 last_used_at INTEGER NOT NULL,
                 pinned       INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS entries_last_used ON entries(last_used_at);",
        )?;
        Ok(Db { conn })
    }

    /// Bump an existing entry with this hash, returning its id.
    fn touch_hash(&self, hash: &str, now: i64) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "UPDATE entries SET last_used_at = ?2 WHERE hash = ?1 RETURNING id",
                params![hash, now],
                |r| r.get(0),
            )
            .optional()
    }

    pub fn insert_text(&self, text: &str, now: i64) -> rusqlite::Result<Inserted> {
        let hash = sha256_hex(text.as_bytes());
        if let Some(id) = self.touch_hash(&hash, now)? {
            return Ok(Inserted { id, is_new: false });
        }
        self.conn.execute(
            "INSERT INTO entries (kind, text, hash, size, created_at, last_used_at)
             VALUES ('text', ?1, ?2, ?3, ?4, ?4)",
            params![text, hash, text.len() as i64, now],
        )?;
        Ok(Inserted { id: self.conn.last_insert_rowid(), is_new: true })
    }

    /// Whether an entry with this content hash exists (bumping it if so).
    /// Lets callers skip writing an image file that is already stored.
    pub fn touch_existing(&self, hash: &str, now: i64) -> rusqlite::Result<Option<i64>> {
        self.touch_hash(hash, now)
    }

    pub fn insert_image(&self, img: &NewImage, now: i64) -> rusqlite::Result<Inserted> {
        if let Some(id) = self.touch_hash(img.hash, now)? {
            return Ok(Inserted { id, is_new: false });
        }
        self.conn.execute(
            "INSERT INTO entries (kind, image_path, thumbnail, width, height, hash, size, created_at, last_used_at)
             VALUES ('image', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                img.path.to_string_lossy(),
                img.thumbnail,
                img.width,
                img.height,
                img.hash,
                img.size,
                now
            ],
        )?;
        Ok(Inserted { id: self.conn.last_insert_rowid(), is_new: true })
    }

    pub fn get(&self, id: i64) -> rusqlite::Result<Option<Entry>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM entries WHERE id = ?1"),
                [id],
                row_to_entry,
            )
            .optional()
    }

    /// Mark an entry as just used so it moves to the top of the history.
    pub fn touch(&self, id: i64, now: i64) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .execute("UPDATE entries SET last_used_at = ?2 WHERE id = ?1", params![id, now])?
            > 0)
    }

    /// Pinned entries first, then most recently used. Text is truncated to a preview.
    pub fn list(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<Entry>> {
        let query = query.trim();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM entries
             WHERE ?1 = '' OR (kind = 'text' AND text LIKE ?2 ESCAPE '\\')
             ORDER BY pinned DESC, last_used_at DESC, id DESC
             LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![query, like_pattern(query), limit as i64], row_to_entry)?;
        rows.map(|r| {
            r.map(|mut e| {
                if let Some(t) = &e.text {
                    if t.chars().count() > PREVIEW_CHARS {
                        e.text = Some(t.chars().take(PREVIEW_CHARS).collect());
                    }
                }
                e
            })
        })
        .collect()
    }

    pub fn toggle_pin(&self, id: i64) -> rusqlite::Result<Option<bool>> {
        self.conn
            .query_row(
                "UPDATE entries SET pinned = 1 - pinned WHERE id = ?1 RETURNING pinned",
                [id],
                |r| r.get::<_, i64>(0).map(|p| p != 0),
            )
            .optional()
    }

    /// Delete one entry. Returns the image file the caller should remove, if any.
    pub fn delete(&self, id: i64) -> rusqlite::Result<Option<PathBuf>> {
        let path: Option<Option<String>> = self
            .conn
            .query_row("DELETE FROM entries WHERE id = ?1 RETURNING image_path", [id], |r| r.get(0))
            .optional()?;
        Ok(path.flatten().map(PathBuf::from))
    }

    fn delete_where(&self, sql: &str, params: impl rusqlite::Params) -> rusqlite::Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, |r| r.get::<_, Option<String>>(0))?;
        let mut paths = Vec::new();
        for p in rows {
            if let Some(p) = p? {
                paths.push(PathBuf::from(p));
            }
        }
        Ok(paths)
    }

    /// Delete every unpinned entry. Returns image files to remove.
    pub fn clear_unpinned(&self) -> rusqlite::Result<Vec<PathBuf>> {
        self.delete_where("DELETE FROM entries WHERE pinned = 0 RETURNING image_path", [])
    }

    /// Keep only the `max_items` most recently used unpinned entries.
    /// Pinned entries never count toward the limit. Returns image files to remove.
    pub fn prune(&self, max_items: usize) -> rusqlite::Result<Vec<PathBuf>> {
        self.delete_where(
            "DELETE FROM entries WHERE id IN (
                 SELECT id FROM entries WHERE pinned = 0
                 ORDER BY last_used_at DESC, id DESC
                 LIMIT -1 OFFSET ?1
             ) RETURNING image_path",
            [max_items as i64],
        )
    }

    #[cfg(test)]
    pub fn count(&self) -> rusqlite::Result<i64> {
        self.conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(db: &Db, hash: &str, now: i64) -> Inserted {
        let path = PathBuf::from(format!("/tmp/{hash}.png"));
        db.insert_image(
            &NewImage { hash, path: &path, thumbnail: b"thumb", width: 10, height: 5, size: 42 },
            now,
        )
        .unwrap()
    }

    #[test]
    fn insert_and_get_text() {
        let db = Db::open_in_memory().unwrap();
        let ins = db.insert_text("hello", 100).unwrap();
        assert!(ins.is_new);
        let e = db.get(ins.id).unwrap().unwrap();
        assert_eq!(e.kind, Kind::Text);
        assert_eq!(e.text.as_deref(), Some("hello"));
        assert_eq!(e.size, 5);
        assert_eq!(e.created_at, 100);
        assert!(!e.pinned);
        assert_eq!(e.hash, sha256_hex(b"hello"));
    }

    #[test]
    fn duplicate_text_bumps_last_used() {
        let db = Db::open_in_memory().unwrap();
        let a = db.insert_text("same", 100).unwrap();
        db.insert_text("other", 150).unwrap();
        let b = db.insert_text("same", 200).unwrap();
        assert_eq!(a.id, b.id);
        assert!(!b.is_new);
        assert_eq!(db.count().unwrap(), 2);
        let e = db.get(a.id).unwrap().unwrap();
        assert_eq!(e.created_at, 100);
        assert_eq!(e.last_used_at, 200);
        assert_eq!(db.list("", 10).unwrap()[0].id, a.id);
    }

    #[test]
    fn images_dedupe_by_hash() {
        let db = Db::open_in_memory().unwrap();
        let a = image(&db, "abc", 1);
        let b = image(&db, "abc", 2);
        assert!(a.is_new && !b.is_new);
        assert_eq!(a.id, b.id);
        let e = db.get(a.id).unwrap().unwrap();
        assert_eq!(e.kind, Kind::Image);
        assert_eq!(e.thumbnail.as_deref(), Some(&b"thumb"[..]));
        assert_eq!((e.width, e.height), (Some(10), Some(5)));
        assert_eq!(db.touch_existing("abc", 3).unwrap(), Some(a.id));
        assert_eq!(db.touch_existing("nope", 3).unwrap(), None);
    }

    #[test]
    fn list_orders_pinned_first_then_recent() {
        let db = Db::open_in_memory().unwrap();
        let old = db.insert_text("old", 1).unwrap().id;
        let mid = db.insert_text("mid", 2).unwrap().id;
        let new = db.insert_text("new", 3).unwrap().id;
        db.toggle_pin(old).unwrap();
        let ids: Vec<i64> = db.list("", 10).unwrap().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![old, new, mid]);
        assert_eq!(db.list("", 2).unwrap().len(), 2);
    }

    #[test]
    fn search_is_case_insensitive_and_escapes_wildcards() {
        let db = Db::open_in_memory().unwrap();
        db.insert_text("Hello World", 1).unwrap();
        db.insert_text("100% sure", 2).unwrap();
        db.insert_text("snake_case", 3).unwrap();
        image(&db, "img", 4);
        let texts = |q: &str| -> Vec<String> {
            db.list(q, 10).unwrap().into_iter().filter_map(|e| e.text).collect()
        };
        assert_eq!(texts("hello"), vec!["Hello World"]);
        assert_eq!(texts("%"), vec!["100% sure"]);
        assert_eq!(texts("_"), vec!["snake_case"]);
        assert!(texts("zzz").is_empty());
        assert_eq!(db.list("  ", 10).unwrap().len(), 4);
    }

    #[test]
    fn list_truncates_long_text_but_get_does_not() {
        let db = Db::open_in_memory().unwrap();
        let long = "é".repeat(PREVIEW_CHARS + 50);
        let id = db.insert_text(&long, 1).unwrap().id;
        let listed = db.list("", 10).unwrap();
        assert_eq!(listed[0].text.as_ref().unwrap().chars().count(), PREVIEW_CHARS);
        assert_eq!(db.get(id).unwrap().unwrap().text.unwrap(), long);
    }

    #[test]
    fn toggle_pin_flips_and_reports_missing() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_text("x", 1).unwrap().id;
        assert_eq!(db.toggle_pin(id).unwrap(), Some(true));
        assert_eq!(db.toggle_pin(id).unwrap(), Some(false));
        assert_eq!(db.toggle_pin(9999).unwrap(), None);
    }

    #[test]
    fn delete_returns_image_path() {
        let db = Db::open_in_memory().unwrap();
        let t = db.insert_text("t", 1).unwrap().id;
        let i = image(&db, "pic", 2).id;
        assert_eq!(db.delete(t).unwrap(), None);
        assert_eq!(db.delete(i).unwrap(), Some(PathBuf::from("/tmp/pic.png")));
        assert_eq!(db.count().unwrap(), 0);
        assert_eq!(db.delete(i).unwrap(), None);
    }

    #[test]
    fn prune_keeps_newest_unpinned_and_all_pinned() {
        let db = Db::open_in_memory().unwrap();
        let pinned = db.insert_text("pinned", 0).unwrap().id;
        db.toggle_pin(pinned).unwrap();
        image(&db, "oldimg", 1);
        for i in 2..10 {
            db.insert_text(&format!("t{i}"), i).unwrap();
        }
        let removed = db.prune(3).unwrap();
        assert_eq!(removed, vec![PathBuf::from("/tmp/oldimg.png")]);
        let texts: Vec<String> = db.list("", 100).unwrap().into_iter().filter_map(|e| e.text).collect();
        assert_eq!(texts, vec!["pinned", "t9", "t8", "t7"]);
        assert!(db.prune(3).unwrap().is_empty());
    }

    #[test]
    fn clear_keeps_pinned() {
        let db = Db::open_in_memory().unwrap();
        let keep = db.insert_text("keep", 1).unwrap().id;
        db.toggle_pin(keep).unwrap();
        db.insert_text("drop", 2).unwrap();
        image(&db, "gone", 3);
        assert_eq!(db.clear_unpinned().unwrap(), vec![PathBuf::from("/tmp/gone.png")]);
        assert_eq!(db.count().unwrap(), 1);
        assert!(db.get(keep).unwrap().is_some());
    }

    #[test]
    fn touch_moves_entry_to_top() {
        let db = Db::open_in_memory().unwrap();
        let a = db.insert_text("a", 1).unwrap().id;
        db.insert_text("b", 2).unwrap();
        assert!(db.touch(a, 3).unwrap());
        assert!(!db.touch(999, 3).unwrap());
        assert_eq!(db.list("", 10).unwrap()[0].id, a);
    }

    #[test]
    fn persists_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        {
            let db = Db::open(&path).unwrap();
            db.insert_text("survives", 1).unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list("surv", 10).unwrap().len(), 1);
    }
}
