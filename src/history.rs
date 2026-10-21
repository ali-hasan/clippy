//! Clipboard history stored in a local SQLite database.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

/// After each insert, text beyond this count is deleted, oldest first.
pub const MAX_ITEMS: usize = 1000;
/// Images take far more room than text, so fewer of them are kept.
pub const MAX_IMAGES: usize = 50;

/// Text and images are stored in separate tables, so each kind has its own numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemId {
    Text(i64),
    Image(i64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Milliseconds since the Unix epoch when the item was last copied.
    pub copied_at: i64,
    /// Bundle identifier of the app the item was copied from, such as `com.apple.Safari`.
    pub source_app: Option<String>,
    pub content: Content,
}

impl Item {
    pub fn id(&self) -> ItemId {
        match self.content {
            Content::Text { id, .. } => ItemId::Text(id),
            Content::Image { id, .. } => ItemId::Image(id),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text { id: i64, text: String },
    /// The full image is left in the database until it is copied again; see [`History::image_png`].
    Image { id: i64, width: u32, height: u32, thumbnail: Thumbnail },
}

/// A small version of an image for the list, as RGBA pixels with colors multiplied by alpha.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct NewImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub thumbnail: Thumbnail,
}

pub fn now_millis() -> i64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    since_epoch.as_millis() as i64
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS items (
                 id INTEGER PRIMARY KEY,
                 text TEXT NOT NULL UNIQUE,
                 copied_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS items_by_copied_at ON items (copied_at);",
        )?;
        // Each step upgrades a database written by an older version of Clippy, exactly once.
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < 1 {
            conn.execute_batch("ALTER TABLE items ADD COLUMN source_app TEXT; PRAGMA user_version = 1;")?;
        }
        if version < 2 {
            conn.execute_batch(
                "CREATE TABLE images (
                     id INTEGER PRIMARY KEY,
                     hash TEXT NOT NULL UNIQUE,
                     copied_at INTEGER NOT NULL,
                     source_app TEXT,
                     png BLOB NOT NULL,
                     width INTEGER NOT NULL,
                     height INTEGER NOT NULL,
                     thumbnail BLOB NOT NULL,
                     thumbnail_width INTEGER NOT NULL,
                     thumbnail_height INTEGER NOT NULL
                 );
                 CREATE INDEX images_by_copied_at ON images (copied_at);
                 PRAGMA user_version = 2;",
            )?;
        }
        Ok(Self { conn })
    }

    /// Saves `text` as the newest item. Copying text that is already saved moves it to the top
    /// instead of adding a duplicate; it keeps its earlier app when `source_app` is unknown.
    pub fn add(&self, text: &str, copied_at: i64, source_app: Option<&str>) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO items (text, copied_at, source_app) VALUES (?1, ?2, ?3)
             ON CONFLICT (text) DO UPDATE SET
                 copied_at = excluded.copied_at,
                 source_app = COALESCE(excluded.source_app, source_app)",
            params![text, copied_at, source_app],
        )?;
        self.conn.execute(
            "DELETE FROM items WHERE id NOT IN (
                 SELECT id FROM items ORDER BY copied_at DESC, id DESC LIMIT ?1
             )",
            params![MAX_ITEMS as i64],
        )?;
        Ok(())
    }

    /// Saves an image as the newest item. Copying the same image again moves it to the top, the
    /// same way as for text.
    pub fn add_image(&self, image: &NewImage, copied_at: i64, source_app: Option<&str>) -> rusqlite::Result<()> {
        let thumbnail = &image.thumbnail;
        self.conn.execute(
            "INSERT INTO images
                 (hash, copied_at, source_app, png, width, height, thumbnail, thumbnail_width, thumbnail_height)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (hash) DO UPDATE SET
                 copied_at = excluded.copied_at,
                 source_app = COALESCE(excluded.source_app, source_app)",
            params![
                fingerprint(&image.png),
                copied_at,
                source_app,
                image.png,
                image.width,
                image.height,
                thumbnail.pixels,
                thumbnail.width,
                thumbnail.height
            ],
        )?;
        self.conn.execute(
            "DELETE FROM images WHERE id NOT IN (
                 SELECT id FROM images ORDER BY copied_at DESC, id DESC LIMIT ?1
             )",
            params![MAX_IMAGES as i64],
        )?;
        Ok(())
    }

    /// Returns every saved item, newest first. Images come with their thumbnail only.
    pub fn items(&self) -> rusqlite::Result<Vec<Item>> {
        let mut texts = self.conn.prepare("SELECT id, text, copied_at, source_app FROM items ORDER BY copied_at DESC, id DESC")?;
        let texts = texts.query_map([], |row| {
            Ok(Item {
                content: Content::Text { id: row.get(0)?, text: row.get(1)? },
                copied_at: row.get(2)?,
                source_app: row.get(3)?,
            })
        })?;
        let mut images = self.conn.prepare(
            "SELECT id, copied_at, source_app, width, height, thumbnail, thumbnail_width, thumbnail_height
             FROM images ORDER BY copied_at DESC, id DESC",
        )?;
        let images = images.query_map([], |row| {
            Ok(Item {
                copied_at: row.get(1)?,
                source_app: row.get(2)?,
                content: Content::Image {
                    id: row.get(0)?,
                    width: row.get(3)?,
                    height: row.get(4)?,
                    thumbnail: Thumbnail { pixels: row.get(5)?, width: row.get(6)?, height: row.get(7)? },
                },
            })
        })?;
        let mut items = texts.chain(images).collect::<rusqlite::Result<Vec<_>>>()?;
        // A stable sort, so items copied in the same millisecond keep their order from the query.
        items.sort_by_key(|item| std::cmp::Reverse(item.copied_at));
        Ok(items)
    }

    /// The full image, as PNG bytes.
    pub fn image_png(&self, id: i64) -> rusqlite::Result<Vec<u8>> {
        self.conn.query_row("SELECT png FROM images WHERE id = ?1", params![id], |row| row.get(0))
    }

    pub fn delete(&self, id: ItemId) -> rusqlite::Result<()> {
        match id {
            ItemId::Text(id) => self.conn.execute("DELETE FROM items WHERE id = ?1", params![id])?,
            ItemId::Image(id) => self.conn.execute("DELETE FROM images WHERE id = ?1", params![id])?,
        };
        Ok(())
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        // Without VACUUM the file would keep the space that the deleted images took.
        self.conn.execute_batch("DELETE FROM items; DELETE FROM images; VACUUM;")
    }
}

/// Identifies an image by its bytes, to notice when the same image is copied again. This is
/// FNV-1a, chosen because its result never changes between Rust versions.
fn fingerprint(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{hash:016x}-{}", bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(history: &History) -> Vec<String> {
        history.items().unwrap().into_iter().filter_map(|item| text_of(&item)).collect()
    }

    fn text_of(item: &Item) -> Option<String> {
        match &item.content {
            Content::Text { text, .. } => Some(text.clone()),
            Content::Image { .. } => None,
        }
    }

    fn image(png: &[u8]) -> NewImage {
        let thumbnail = Thumbnail { width: 1, height: 1, pixels: vec![255; 4] };
        NewImage { png: png.to_vec(), width: 640, height: 480, thumbnail }
    }

    #[test]
    fn lists_items_newest_first() {
        let history = History::open_in_memory().unwrap();
        history.add("first", 1, None).unwrap();
        history.add("second", 2, None).unwrap();
        assert_eq!(texts(&history), ["second", "first"]);
    }

    #[test]
    fn copying_saved_text_again_moves_it_to_the_top_without_a_duplicate() {
        let history = History::open_in_memory().unwrap();
        history.add("first", 1, None).unwrap();
        history.add("second", 2, None).unwrap();
        history.add("first", 3, None).unwrap();
        assert_eq!(texts(&history), ["first", "second"]);
    }

    #[test]
    fn drops_the_oldest_items_once_the_history_is_full() {
        let history = History::open_in_memory().unwrap();
        for i in 0..MAX_ITEMS + 5 {
            history.add(&format!("item {i}"), i as i64, None).unwrap();
        }
        let items = texts(&history);
        assert_eq!(items.len(), MAX_ITEMS);
        assert_eq!(items.first().unwrap(), &format!("item {}", MAX_ITEMS + 4));
        assert_eq!(items.last().unwrap(), "item 5");
    }

    #[test]
    fn deleting_an_item_keeps_the_others() {
        let history = History::open_in_memory().unwrap();
        history.add("keep", 1, None).unwrap();
        history.add("remove", 2, None).unwrap();
        let id = history.items().unwrap()[0].id();
        history.delete(id).unwrap();
        assert_eq!(texts(&history), ["keep"]);
    }

    #[test]
    fn clearing_removes_every_item() {
        let history = History::open_in_memory().unwrap();
        history.add("a", 1, None).unwrap();
        history.add_image(&image(b"png"), 2, None).unwrap();
        history.clear().unwrap();
        assert!(history.items().unwrap().is_empty());
    }

    #[test]
    fn keeps_items_after_the_database_is_reopened() {
        let dir = std::env::temp_dir().join(format!("clippy-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        History::open(&path).unwrap().add("saved", 1, None).unwrap();
        assert_eq!(texts(&History::open(&path).unwrap()), ["saved"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remembers_which_app_the_text_was_copied_from() {
        let history = History::open_in_memory().unwrap();
        history.add("link", 1, Some("com.apple.Safari")).unwrap();
        assert_eq!(history.items().unwrap()[0].source_app.as_deref(), Some("com.apple.Safari"));
    }

    #[test]
    fn copying_again_from_an_unknown_app_keeps_the_earlier_app() {
        let history = History::open_in_memory().unwrap();
        history.add("link", 1, Some("com.apple.Safari")).unwrap();
        history.add("link", 2, None).unwrap();
        assert_eq!(history.items().unwrap()[0].source_app.as_deref(), Some("com.apple.Safari"));
    }

    #[test]
    fn copying_again_from_another_app_shows_the_newer_app() {
        let history = History::open_in_memory().unwrap();
        history.add("link", 1, Some("com.apple.Safari")).unwrap();
        history.add("link", 2, Some("com.apple.Notes")).unwrap();
        assert_eq!(history.items().unwrap()[0].source_app.as_deref(), Some("com.apple.Notes"));
    }

    #[test]
    fn lists_an_image_with_its_size_and_thumbnail() {
        let history = History::open_in_memory().unwrap();
        history.add_image(&image(b"png"), 1, Some("com.apple.Preview")).unwrap();
        let item = &history.items().unwrap()[0];
        assert_eq!(item.source_app.as_deref(), Some("com.apple.Preview"));
        let Content::Image { width, height, thumbnail, .. } = &item.content else { panic!("not an image") };
        assert_eq!((*width, *height, thumbnail), (640, 480, &image(b"png").thumbnail));
    }

    #[test]
    fn gives_back_the_full_image_for_copying_again() {
        let history = History::open_in_memory().unwrap();
        history.add_image(&image(b"the png bytes"), 1, None).unwrap();
        let ItemId::Image(id) = history.items().unwrap()[0].id() else { panic!("not an image") };
        assert_eq!(history.image_png(id).unwrap(), b"the png bytes");
    }

    #[test]
    fn copying_the_same_image_again_moves_it_to_the_top_without_a_duplicate() {
        let history = History::open_in_memory().unwrap();
        history.add_image(&image(b"photo"), 1, None).unwrap();
        history.add("text", 2, None).unwrap();
        history.add_image(&image(b"photo"), 3, None).unwrap();
        let items = history.items().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].content, Content::Image { .. }));
    }

    #[test]
    fn deleting_an_image_keeps_the_other_items() {
        let history = History::open_in_memory().unwrap();
        history.add("keep", 1, None).unwrap();
        history.add_image(&image(b"photo"), 2, None).unwrap();
        history.delete(history.items().unwrap()[0].id()).unwrap();
        assert_eq!(texts(&history), ["keep"]);
        assert_eq!(history.items().unwrap().len(), 1);
    }

    #[test]
    fn different_images_are_kept_apart() {
        let history = History::open_in_memory().unwrap();
        history.add_image(&image(b"photo one"), 1, None).unwrap();
        history.add_image(&image(b"photo two"), 2, None).unwrap();
        assert_eq!(history.items().unwrap().len(), 2);
    }

    #[test]
    fn drops_the_oldest_images_once_enough_are_saved() {
        let history = History::open_in_memory().unwrap();
        history.add("text survives", 0, None).unwrap();
        for i in 0..MAX_IMAGES + 3 {
            history.add_image(&image(format!("photo {i}").as_bytes()), 1 + i as i64, None).unwrap();
        }
        let items = history.items().unwrap();
        assert_eq!(items.iter().filter(|item| matches!(item.content, Content::Image { .. })).count(), MAX_IMAGES);
        assert_eq!(texts(&history), ["text survives"]);
    }

    #[test]
    fn text_and_images_are_listed_together_newest_first() {
        let history = History::open_in_memory().unwrap();
        history.add("old text", 1, None).unwrap();
        history.add_image(&image(b"photo"), 2, None).unwrap();
        history.add("new text", 3, None).unwrap();
        let kinds: Vec<_> = history.items().unwrap().iter().map(|item| text_of(item).unwrap_or("image".into())).collect();
        assert_eq!(kinds, ["new text", "image", "old text"]);
    }

    #[test]
    fn keeps_history_saved_before_apps_were_recorded() {
        let dir = std::env::temp_dir().join(format!("clippy-upgrade-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        // The table as the first version of Clippy created it.
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, text TEXT NOT NULL UNIQUE, copied_at INTEGER NOT NULL);
                 INSERT INTO items (text, copied_at) VALUES ('old copy', 1);",
            )
            .unwrap();

        let history = History::open(&path).unwrap();
        history.add("new copy", 2, Some("com.apple.Notes")).unwrap();
        let items = history.items().unwrap();
        assert_eq!(items[0].source_app.as_deref(), Some("com.apple.Notes"));
        assert_eq!((text_of(&items[1]).unwrap().as_str(), items[1].source_app.as_deref()), ("old copy", None));
        drop(history);
        History::open(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_history_saved_before_images_were_supported() {
        let dir = std::env::temp_dir().join(format!("clippy-images-upgrade-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        // The database as the version that added source apps left it.
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, text TEXT NOT NULL UNIQUE, copied_at INTEGER NOT NULL,
                                     source_app TEXT);
                 INSERT INTO items (text, copied_at, source_app) VALUES ('old copy', 1, 'com.apple.Safari');
                 PRAGMA user_version = 1;",
            )
            .unwrap();

        let history = History::open(&path).unwrap();
        history.add_image(&image(b"photo"), 2, None).unwrap();
        let items = history.items().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!((text_of(&items[1]).unwrap().as_str(), items[1].source_app.as_deref()), ("old copy", Some("com.apple.Safari")));
        drop(history);
        History::open(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
