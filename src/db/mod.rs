//! SQLite persistence on a dedicated worker thread.

use crate::model::item::{ClipContent, ClipItem, ItemId, Sensitivity};
use crate::msg::{ToDb, ToUi, UiWaker};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

#[derive(Clone, Debug)]
pub struct Row {
    pub id: ItemId,
    pub kind: i64,
    pub created_at: i64,
    pub hash: Vec<u8>,
    pub text: Option<String>,
    pub image_path: Option<String>,
    pub files_json: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub byte_size: Option<i64>,
    pub source_exe: Option<String>,
    pub sensitive: bool,
    pub pinned: bool,
}

impl Row {
    pub fn from_item(item: &ClipItem) -> Row {
        let (text, image_path, files_json, width, height, byte_size) = match &item.content {
            ClipContent::Text { text, .. } => (Some(text.to_string()), None, None, None, None, Some(text.len() as i64)),
            ClipContent::Image { path, width, height, bytes, .. } => (
                None,
                path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                None,
                Some(*width as i64),
                Some(*height as i64),
                Some(*bytes as i64),
            ),
            ClipContent::Files { paths } => {
                let v: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                (None, None, serde_json::to_string(&v).ok(), None, None, Some(paths.len() as i64))
            }
        };
        Row {
            id: item.id,
            kind: item.kind() as i64,
            created_at: item.created_at_ms,
            hash: item.hash.to_vec(),
            text,
            image_path,
            files_json,
            width,
            height,
            byte_size,
            source_exe: item.source_exe.clone(),
            sensitive: item.sensitivity.is_sensitive(),
            pinned: item.pinned,
        }
    }

    pub fn into_item(self) -> Option<ClipItem> {
        use crate::model::item::ItemKind;
        let kind = ItemKind::from_i64(self.kind)?;
        let content = match kind {
            ItemKind::Text => ClipContent::text(self.text?),
            ItemKind::Image => ClipContent::Image {
                path: self.image_path.map(PathBuf::from),
                width: self.width.unwrap_or(0) as u32,
                height: self.height.unwrap_or(0) as u32,
                bytes: self.byte_size.unwrap_or(0) as u64,
                has_alpha: false,
            },
            ItemKind::Files => {
                let v: Vec<String> = serde_json::from_str(self.files_json.as_deref()?).ok()?;
                ClipContent::Files { paths: v.into_iter().map(PathBuf::from).collect() }
            }
        };
        let mut hash = [0u8; 32];
        if self.hash.len() == 32 {
            hash.copy_from_slice(&self.hash);
        }
        let mut item = ClipItem {
            id: self.id,
            content,
            created_at_ms: self.created_at,
            hash,
            source_exe: self.source_exe,
            sensitivity: if self.sensitive { Sensitivity::Manual } else { Sensitivity::None },
            pinned: self.pinned,
            revealed_until: None,
            expires_at: None,
            layout_gen: 0,
        };
        if item.sensitivity.is_sensitive() {
            if let ClipContent::Text { snippet, .. } = &mut item.content {
                snippet.clear();
            }
        }
        Some(item)
    }
}

#[derive(Clone, Debug)]
pub struct ShelfRow {
    pub id: i64,
    pub name: String,
    pub color: i64,
    pub position: i64,
}

#[derive(Clone, Debug)]
pub struct ShelfItemRow {
    pub id: i64,
    pub shelf_id: i64,
    pub kind: i64,
    pub text: Option<String>,
    pub image_path: Option<String>,
    pub files_json: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub position: i64,
    pub created_at: i64,
}

impl ShelfItemRow {
    pub fn from_entry(shelf_id: i64, e: &crate::model::shelf::ShelfEntry, position: i64) -> ShelfItemRow {
        let (text, image_path, files_json, width, height) = match &e.content {
            ClipContent::Text { text, .. } => (Some(text.to_string()), None, None, None, None),
            ClipContent::Image { path, width, height, .. } => (None, path.as_ref().map(|p| p.to_string_lossy().into_owned()), None, Some(*width as i64), Some(*height as i64)),
            ClipContent::Files { paths } => {
                let v: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                (None, None, serde_json::to_string(&v).ok(), None, None)
            }
        };
        ShelfItemRow { id: e.id, shelf_id, kind: e.content.kind() as i64, text, image_path, files_json, width, height, position, created_at: e.created_at_ms }
    }

    pub fn into_entry(self) -> Option<crate::model::shelf::ShelfEntry> {
        use crate::model::item::ItemKind;
        let content = match ItemKind::from_i64(self.kind)? {
            ItemKind::Text => ClipContent::text(self.text?),
            ItemKind::Image => ClipContent::Image {
                path: self.image_path.map(PathBuf::from),
                width: self.width.unwrap_or(0) as u32,
                height: self.height.unwrap_or(0) as u32,
                bytes: 0,
                has_alpha: false,
            },
            ItemKind::Files => {
                let v: Vec<String> = serde_json::from_str(self.files_json.as_deref()?).ok()?;
                ClipContent::Files { paths: v.into_iter().map(PathBuf::from).collect() }
            }
        };
        Some(crate::model::shelf::ShelfEntry { id: self.id, content, created_at_ms: self.created_at })
    }
}

pub fn spawn(path: PathBuf, rx: Receiver<ToDb>, tx: Sender<ToUi>, waker: UiWaker) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("clipcywin-db".into())
        .spawn(move || {
            let conn = match open(&path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(ToUi::DbError(format!("open: {e}")));
                    waker.wake();
                    // Drain messages so senders never block; nothing persisted.
                    while let Ok(m) = rx.recv() {
                        if matches!(m, ToDb::Shutdown) {
                            break;
                        }
                    }
                    return;
                }
            };
            while let Ok(m) = rx.recv() {
                if let Err(e) = handle(&conn, m, &tx, &waker) {
                    log::warn!("db: {e}");
                }
            }
        })
        .expect("spawn db thread")
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY;
         CREATE TABLE IF NOT EXISTS items (
            id INTEGER PRIMARY KEY, kind INTEGER NOT NULL, created_at INTEGER NOT NULL, hash BLOB NOT NULL,
            text TEXT, image_path TEXT, files_json TEXT, width INTEGER, height INTEGER, byte_size INTEGER,
            source_exe TEXT, sensitive INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX IF NOT EXISTS ix_items_created ON items(created_at DESC);
         CREATE INDEX IF NOT EXISTS ix_items_hash ON items(hash);
         CREATE TABLE IF NOT EXISTS shelves (id INTEGER PRIMARY KEY, name TEXT NOT NULL, color INTEGER NOT NULL, position INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS shelf_items (
            id INTEGER PRIMARY KEY, shelf_id INTEGER NOT NULL, kind INTEGER NOT NULL, text TEXT, image_path TEXT, files_json TEXT,
            width INTEGER, height INTEGER, position INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS ix_shelf_items_shelf ON shelf_items(shelf_id, position);
         PRAGMA user_version=2;",
    )?;
    Ok(conn)
}

enum Flow {
    Continue,
    Stop,
}

fn handle(conn: &Connection, m: ToDb, tx: &Sender<ToUi>, waker: &UiWaker) -> rusqlite::Result<Flow> {
    match m {
        ToDb::Insert(r) => {
            conn.execute(
                "INSERT OR REPLACE INTO items (id,kind,created_at,hash,text,image_path,files_json,width,height,byte_size,source_exe,sensitive,pinned)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![r.id, r.kind, r.created_at, r.hash, r.text, r.image_path, r.files_json, r.width, r.height, r.byte_size, r.source_exe, r.sensitive as i64, r.pinned as i64],
            )?;
        }
        ToDb::SetPinned { id, pinned } => {
            conn.execute("UPDATE items SET pinned=?2 WHERE id=?1", params![id, pinned as i64])?;
        }
        ToDb::Touch { id, created_at } => {
            conn.execute("UPDATE items SET created_at=?2 WHERE id=?1", params![id, created_at])?;
        }
        ToDb::Delete(id) => {
            conn.execute("DELETE FROM items WHERE id=?1", params![id])?;
        }
        ToDb::Clear { keep_pinned } => {
            if keep_pinned {
                conn.execute("DELETE FROM items WHERE pinned=0", [])?;
            } else {
                conn.execute("DELETE FROM items", [])?;
            }
            conn.execute_batch("VACUUM;")?;
        }
        ToDb::LoadAll { limit } => {
            let mut stmt = conn.prepare(
                "SELECT id,kind,created_at,hash,text,image_path,files_json,width,height,byte_size,source_exe,sensitive,pinned
                 FROM items ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], |r| {
                    Ok(Row {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        created_at: r.get(2)?,
                        hash: r.get(3)?,
                        text: r.get(4)?,
                        image_path: r.get(5)?,
                        files_json: r.get(6)?,
                        width: r.get(7)?,
                        height: r.get(8)?,
                        byte_size: r.get(9)?,
                        source_exe: r.get(10)?,
                        sensitive: r.get::<_, i64>(11)? != 0,
                        pinned: r.get::<_, i64>(12)? != 0,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            let _ = tx.send(ToUi::Loaded(rows));
            waker.wake();
        }
        ToDb::Trim { max } => {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0)).optional()?.unwrap_or(0);
            if count as usize > max {
                let excess = count as usize - max;
                conn.execute(
                    "DELETE FROM items WHERE id IN (SELECT id FROM items WHERE pinned=0 ORDER BY created_at ASC LIMIT ?1)",
                    params![excess as i64],
                )?;
            }
        }
        ToDb::ShelfUpsert(r) => {
            conn.execute("INSERT OR REPLACE INTO shelves (id,name,color,position) VALUES (?1,?2,?3,?4)", params![r.id, r.name, r.color, r.position])?;
        }
        ToDb::ShelfDelete(id) => {
            conn.execute("DELETE FROM shelf_items WHERE shelf_id=?1", params![id])?;
            conn.execute("DELETE FROM shelves WHERE id=?1", params![id])?;
        }
        ToDb::ShelfItemInsert(r) => {
            conn.execute(
                "INSERT OR REPLACE INTO shelf_items (id,shelf_id,kind,text,image_path,files_json,width,height,position,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![r.id, r.shelf_id, r.kind, r.text, r.image_path, r.files_json, r.width, r.height, r.position, r.created_at],
            )?;
        }
        ToDb::ShelfItemDelete(id) => {
            conn.execute("DELETE FROM shelf_items WHERE id=?1", params![id])?;
        }
        ToDb::ShelfClear(shelf_id) => {
            conn.execute("DELETE FROM shelf_items WHERE shelf_id=?1", params![shelf_id])?;
        }
        ToDb::LoadShelves => {
            let mut st = conn.prepare("SELECT id,name,color,position FROM shelves ORDER BY position, id")?;
            let shelves = st
                .query_map([], |r| Ok(ShelfRow { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, position: r.get(3)? }))?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            let mut st = conn.prepare("SELECT id,shelf_id,kind,text,image_path,files_json,width,height,position,created_at FROM shelf_items ORDER BY shelf_id, position, id")?;
            let items = st
                .query_map([], |r| {
                    Ok(ShelfItemRow {
                        id: r.get(0)?,
                        shelf_id: r.get(1)?,
                        kind: r.get(2)?,
                        text: r.get(3)?,
                        image_path: r.get(4)?,
                        files_json: r.get(5)?,
                        width: r.get(6)?,
                        height: r.get(7)?,
                        position: r.get(8)?,
                        created_at: r.get(9)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            let _ = tx.send(ToUi::ShelvesLoaded { shelves, items });
            waker.wake();
        }
        ToDb::Shutdown => {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            return Ok(Flow::Stop);
        }
    }
    Ok(Flow::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::hash::hash_text;

    #[test]
    fn row_roundtrip() {
        let item = ClipItem {
            id: 7,
            content: ClipContent::text("hello".into()),
            created_at_ms: 123,
            hash: hash_text("hello"),
            source_exe: Some("x.exe".into()),
            sensitivity: Sensitivity::None,
            pinned: true,
            revealed_until: None,
            expires_at: None,
            layout_gen: 0,
        };
        let row = Row::from_item(&item);
        let back = row.into_item().unwrap();
        assert_eq!(back.id, 7);
        assert!(back.pinned);
        assert_eq!(back.hash, item.hash);
        match back.content {
            ClipContent::Text { text, .. } => assert_eq!(text.as_str(), "hello"),
            _ => panic!(),
        }
    }

    #[test]
    fn schema_and_insert() {
        let conn = open(Path::new(":memory:")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let waker = UiWaker::none();
        let item = ClipItem {
            id: 1,
            content: ClipContent::text("a".into()),
            created_at_ms: 1,
            hash: hash_text("a"),
            source_exe: None,
            sensitivity: Sensitivity::None,
            pinned: false,
            revealed_until: None,
            expires_at: None,
            layout_gen: 0,
        };
        handle(&conn, ToDb::Insert(Row::from_item(&item)), &tx, &waker).unwrap();
        handle(&conn, ToDb::LoadAll { limit: 10 }, &tx, &waker).unwrap();
        match rx.recv().unwrap() {
            ToUi::Loaded(rows) => assert_eq!(rows.len(), 1),
            _ => panic!(),
        }
    }
}
