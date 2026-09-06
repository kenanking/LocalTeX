use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};

pub(super) const SCHEMA_VERSION: i32 = 3;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    migrate_inner(&tx)?;
    tx.commit()?;
    Ok(())
}

fn migrate_inner(conn: &Connection) -> Result<()> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(anyhow!(
            "snips.db schema version {version} is newer than this app ({SCHEMA_VERSION})"
        ));
    }
    if !snips_table_exists(conn)? {
        conn.execute_batch(SNIPS_TABLE)?;
        ensure_aux(conn)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        return Ok(());
    }

    let names = snips_column_names(conn)?;
    let missing: Vec<_> = SNIPS_COLUMNS
        .iter()
        .copied()
        .filter(|column| !names.iter().any(|name| name == column))
        .collect();
    if !missing.is_empty() {
        return Err(anyhow!(
            "snips schema is missing required columns: {}",
            missing.join(", ")
        ));
    }
    if version < 3 {
        for (column, definition) in [
            ("raw_text", "TEXT"),
            ("processing_status", "TEXT NOT NULL DEFAULT 'ready'"),
            ("processing_error", "TEXT"),
        ] {
            if !names.iter().any(|name| name == column) {
                conn.execute_batch(&format!(
                    "ALTER TABLE snips ADD COLUMN {column} {definition}"
                ))?;
            }
        }
    }
    let names = snips_column_names(conn)?;
    for column in ["raw_text", "processing_status", "processing_error"] {
        if !names.iter().any(|name| name == column) {
            return Err(anyhow!("snips schema is missing required column {column}"));
        }
    }
    ensure_aux(conn)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

const SNIPS_COLUMNS: [&str; 9] = [
    "id",
    "created_at",
    "first_line",
    "blocks_json",
    "search_text",
    "thumb_jpeg",
    "ocr_s",
    "confidence",
    "ocr_blocks_json",
];

const SNIPS_TABLE: &str = "
        CREATE TABLE snips (
          id TEXT PRIMARY KEY NOT NULL,
          created_at INTEGER NOT NULL,
          first_line TEXT NOT NULL,
          blocks_json TEXT NOT NULL,
          search_text TEXT NOT NULL,
          thumb_jpeg BLOB NOT NULL,
          ocr_s REAL,
          confidence REAL,
          ocr_blocks_json TEXT NOT NULL,
          raw_text TEXT,
          processing_status TEXT NOT NULL DEFAULT 'ready',
          processing_error TEXT
        );
        ";

fn snips_column_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("PRAGMA table_info(snips)")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(names)
}

fn snips_table_exists(conn: &Connection) -> Result<bool> {
    master_table_exists(conn, "snips")
}

fn master_table_exists(conn: &Connection, name: &str) -> Result<bool> {
    master_object_exists(conn, "table", name)
}

fn master_object_exists(conn: &Connection, kind: &str, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        params![kind, name],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn ensure_aux(conn: &Connection) -> Result<()> {
    let fts_exists = master_table_exists(conn, "snips_fts")?;
    let triggers_complete = ["snips_ai", "snips_ad", "snips_au"]
        .into_iter()
        .map(|name| master_object_exists(conn, "trigger", name))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .all(|exists| exists);
    let needs_rebuild = !fts_exists || !triggers_complete;

    conn.execute_batch("SAVEPOINT auxiliary")?;
    let result = (|| -> Result<()> {
        conn.execute(
            "CREATE INDEX IF NOT EXISTS snips_created_at ON snips (created_at DESC)",
            [],
        )?;
        conn.execute_batch(
            "
        CREATE VIRTUAL TABLE IF NOT EXISTS snips_fts USING fts5(
          search_text,
          content='snips',
          content_rowid='rowid',
          tokenize='trigram'
        );
        CREATE TRIGGER IF NOT EXISTS snips_ai AFTER INSERT ON snips BEGIN
          INSERT INTO snips_fts(rowid, search_text) VALUES (new.rowid, new.search_text);
        END;
        CREATE TRIGGER IF NOT EXISTS snips_ad AFTER DELETE ON snips BEGIN
          INSERT INTO snips_fts(snips_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
        END;
        CREATE TRIGGER IF NOT EXISTS snips_au AFTER UPDATE ON snips BEGIN
          INSERT INTO snips_fts(snips_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
          INSERT INTO snips_fts(rowid, search_text) VALUES (new.rowid, new.search_text);
        END;
        ",
        )?;
        if needs_rebuild {
            conn.execute("INSERT INTO snips_fts(snips_fts) VALUES('rebuild')", [])?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("RELEASE auxiliary").map_err(Into::into),
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK TO auxiliary; RELEASE auxiliary");
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SCHEMA_VERSION;
    use crate::doc::{Block, BlockKind, DocStatus, ImageSlot, Rect};
    use crate::store::{DateRange, Store};
    use image::{Rgba, RgbaImage};
    use rusqlite::{Connection, params};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::SystemTime;
    use uuid::Uuid;

    #[test]
    fn v2_upgrade_is_atomic_and_preserves_ocr_snapshot() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE snips(id TEXT PRIMARY KEY NOT NULL, created_at INTEGER NOT NULL, first_line TEXT NOT NULL, blocks_json TEXT NOT NULL, search_text TEXT NOT NULL, thumb_jpeg BLOB NOT NULL, ocr_s REAL, confidence REAL, ocr_blocks_json TEXT NOT NULL); PRAGMA user_version = 2; INSERT INTO snips VALUES ('id', 1, 'edited', '[]', 'edited', X'', NULL, NULL, '[original]'); CREATE TABLE snips_created_at(conflict TEXT);").unwrap();
        assert!(super::migrate(&conn).is_err());
        assert!(
            !super::snips_column_names(&conn)
                .unwrap()
                .iter()
                .any(|name| name == "raw_text")
        );
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        conn.execute_batch("DROP TABLE snips_created_at").unwrap();
        super::migrate(&conn).unwrap();
        let (raw, status, ocr): (Option<String>, String, String) = conn
            .query_row(
                "SELECT raw_text, processing_status, ocr_blocks_json FROM snips",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(raw, None);
        assert_eq!(status, "ready");
        assert_eq!(ocr, "[original]");
    }

    fn sample_doc(text: &str, created: SystemTime) -> crate::doc::Document {
        let mut img = RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 255]));
        img.put_pixel(0, 0, Rgba([1, 2, 3, 255]));
        let blocks = vec![Block::new(
            BlockKind::Formula,
            Rect {
                x: 0,
                y: 0,
                w: 8,
                h: 8,
            },
            text,
        )];
        crate::doc::Document {
            id: Uuid::new_v4(),
            created_at: created,
            image: ImageSlot::Loaded(Arc::new(img)),
            blocks: blocks.clone(),
            status: DocStatus::Ready,
            first_line: String::new(),
            thumb_jpeg: Vec::new(),
            persist: crate::doc::PersistState::New,
            blocks_loaded: true,
            ocr: None,
            ink: None,
            revision: 0,
            ocr_blocks: blocks,
            raw_text: None,
            source_error: None,
            source_pending: false,
            source_updated_at: None,
        }
    }

    fn tmp_store() -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!("localtex-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        (Store::open(root.clone()).unwrap(), root)
    }

    #[test]
    fn fresh_schema_contains_only_current_columns() {
        let (store, root) = tmp_store();
        drop(store);
        let conn = Connection::open(root.join("snips.db")).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(snips)").unwrap();
        let columns: Vec<(String, bool)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i32>(3)? != 0))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();

        assert!(!columns.iter().any(|(name, _)| name == "image_relpath"));
        assert!(
            columns
                .iter()
                .any(|(name, not_null)| name == "ocr_blocks_json" && *not_null)
        );
    }

    #[test]
    fn newer_schema_version_refuses_open() {
        let (store, root) = tmp_store();
        let doc = sample_doc("keep-me", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        drop(store);
        {
            let conn = Connection::open(root.join("snips.db")).unwrap();
            conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .unwrap();
        }
        let png = root.join(format!("snips/{id}.png"));
        assert!(png.exists());
        let err = match Store::open(root.clone()) {
            Ok(_) => panic!("expected open to fail"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("newer"));
        assert!(png.exists());
        let conn = Connection::open(root.join("snips.db")).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM snips", [], |row| row.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn extra_column_on_current_version_still_opens() {
        let (store, root) = tmp_store();
        let doc = sample_doc("keep-me", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        drop(store);
        {
            let conn = Connection::open(root.join("snips.db")).unwrap();
            conn.execute("ALTER TABLE snips ADD COLUMN extra INTEGER", [])
                .unwrap();
        }
        let store = Store::open(root).unwrap();
        assert_eq!(store.list().unwrap()[0].id, id);
    }

    #[test]
    fn missing_fts_trigger_is_repaired_and_rebuilt() {
        let (store, root) = tmp_store();
        let doc = sample_doc("before repair", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        drop(store);
        {
            let conn = Connection::open(root.join("snips.db")).unwrap();
            conn.execute_batch("DROP TRIGGER snips_au;").unwrap();
            conn.execute(
                "UPDATE snips SET search_text = 'after repair' WHERE id = ?1",
                params![id.to_string()],
            )
            .unwrap();
        }

        let store = Store::open(root).unwrap();
        assert_eq!(
            store
                .query_ids("after repair", DateRange::default())
                .unwrap(),
            vec![id]
        );
    }

    #[test]
    fn unknown_snips_shape_refuses_open() {
        let root = std::env::temp_dir().join(format!("localtex-store-unknown-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("snips")).unwrap();
        {
            let conn = Connection::open(root.join("snips.db")).unwrap();
            conn.execute_batch("CREATE TABLE snips (id TEXT PRIMARY KEY NOT NULL);")
                .unwrap();
            conn.execute("INSERT INTO snips (id) VALUES ('x')", [])
                .unwrap();
            std::fs::write(root.join("snips/keep.png"), b"x").unwrap();
        }
        let err = match Store::open(root.clone()) {
            Ok(_) => panic!("expected open to fail"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("missing required columns"));
        assert!(root.join("snips/keep.png").exists());
    }
}
