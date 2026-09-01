use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};

pub(super) const SCHEMA_VERSION: i32 = 2;

enum SnipsShape {
    Current,
    PreMetrics,
    Unknown,
}

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    if !snips_table_exists(conn)? {
        conn.execute_batch(SNIPS_TABLE)?;
        ensure_aux(conn)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        return Ok(());
    }

    let mut version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(anyhow!(
            "snips.db schema version {version} is newer than this app ({SCHEMA_VERSION})"
        ));
    }

    let names = snips_column_names(conn)?;
    match classify_snips(&names) {
        SnipsShape::Current => {}
        SnipsShape::PreMetrics => upgrade_pre_metrics(conn, &names)?,
        SnipsShape::Unknown => {
            return Err(anyhow!(
                "unrecognized snips schema (columns: {})",
                names.join(", ")
            ));
        }
    }
    ensure_aux(conn)?;

    while version < SCHEMA_VERSION {
        version = migrate_from(conn, version)?;
        conn.pragma_update(None, "user_version", version)?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_from(conn: &Connection, from: i32) -> Result<i32> {
    match from {
        0 => Ok(1),
        1 => {
            let names = snips_column_names(conn)?;
            if !names.iter().any(|n| n == "ocr_blocks_json") {
                conn.execute("ALTER TABLE snips ADD COLUMN ocr_blocks_json TEXT", [])?;
            }
            conn.execute(
                "UPDATE snips SET ocr_blocks_json = blocks_json WHERE ocr_blocks_json IS NULL",
                [],
            )?;
            Ok(2)
        }
        other => Err(anyhow!("no migration from schema version {other}")),
    }
}

fn classify_snips(names: &[String]) -> SnipsShape {
    if has_all(names, &SNIPS_COLUMNS) {
        return SnipsShape::Current;
    }
    if has_all(names, &PRE_METRICS_COLUMNS)
        && names.iter().all(|n| SNIPS_COLUMNS.contains(&n.as_str()))
    {
        return SnipsShape::PreMetrics;
    }
    SnipsShape::Unknown
}

fn has_all(names: &[String], required: &[&str]) -> bool {
    required.iter().all(|col| names.iter().any(|n| n == col))
}

fn upgrade_pre_metrics(conn: &Connection, names: &[String]) -> Result<()> {
    if !names.iter().any(|n| n == "ocr_s") {
        conn.execute("ALTER TABLE snips ADD COLUMN ocr_s REAL", [])?;
    }
    if !names.iter().any(|n| n == "confidence") {
        conn.execute("ALTER TABLE snips ADD COLUMN confidence REAL", [])?;
    }
    Ok(())
}

const SNIPS_COLUMNS: [&str; 9] = [
    "id",
    "created_at",
    "first_line",
    "blocks_json",
    "search_text",
    "thumb_jpeg",
    "image_relpath",
    "ocr_s",
    "confidence",
];

const PRE_METRICS_COLUMNS: [&str; 7] = [
    "id",
    "created_at",
    "first_line",
    "blocks_json",
    "search_text",
    "thumb_jpeg",
    "image_relpath",
];

const SNIPS_TABLE: &str = "
        CREATE TABLE snips (
          id TEXT PRIMARY KEY NOT NULL,
          created_at INTEGER NOT NULL,
          first_line TEXT NOT NULL,
          blocks_json TEXT NOT NULL,
          search_text TEXT NOT NULL,
          thumb_jpeg BLOB NOT NULL,
          image_relpath TEXT NOT NULL,
          ocr_s REAL,
          confidence REAL,
          ocr_blocks_json TEXT
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
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn ensure_aux(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE INDEX IF NOT EXISTS snips_created_at ON snips (created_at DESC)",
        [],
    )?;
    if master_table_exists(conn, "snips_fts")? {
        return Ok(());
    }
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE snips_fts USING fts5(
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
        INSERT INTO snips_fts(snips_fts) VALUES('rebuild');
        ",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SCHEMA_VERSION;
    use crate::doc::{Block, BlockKind, DocStatus, ImageSlot, Rect};
    use crate::store::{DateRange, Store};
    use image::{Rgba, RgbaImage};
    use rusqlite::{params, Connection};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::SystemTime;
    use uuid::Uuid;

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
            persisted: false,
            blocks_loaded: true,
            ocr: None,
            ink: None,
            revision: 0,
            ocr_blocks: blocks,
        }
    }

    fn tmp_store() -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!("localtex-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        (Store::open(root.clone()).unwrap(), root)
    }

    const PRE_METRICS_ID: &str = "00000000-0000-0000-0000-000000000001";

    fn write_pre_metrics_db(root: &Path, user_version: i32) {
        std::fs::create_dir_all(root.join("snips")).unwrap();
        let conn = Connection::open(root.join("snips.db")).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE snips (
              id TEXT PRIMARY KEY NOT NULL,
              created_at INTEGER NOT NULL,
              first_line TEXT NOT NULL,
              blocks_json TEXT NOT NULL,
              search_text TEXT NOT NULL,
              thumb_jpeg BLOB NOT NULL,
              image_relpath TEXT NOT NULL
            );
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snips (id, created_at, first_line, blocks_json, search_text, thumb_jpeg, image_relpath)
             VALUES (?1, 0, 'old', '[]', 'old', x'00', 'snips/none.png')",
            params![PRE_METRICS_ID],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", user_version)
            .unwrap();
        std::fs::write(root.join("snips/none.png"), b"x").unwrap();
    }

    fn snips_user_version(root: &Path) -> i32 {
        let conn = Connection::open(root.join("snips.db")).unwrap();
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn pre_metrics_schema_is_upgraded_in_place() {
        let root = std::env::temp_dir().join(format!("localtex-store-old-{}", Uuid::new_v4()));
        write_pre_metrics_db(&root, 1);
        let png = root.join("snips/none.png");
        let store = Store::open(root.clone()).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id.to_string(), PRE_METRICS_ID);
        assert!(list[0].ocr.is_none());
        assert!(png.exists());
        assert_eq!(snips_user_version(&root), SCHEMA_VERSION);
        store.load_blocks(list[0].id).unwrap();
        assert_eq!(
            store.query_ids("old", DateRange::default()).unwrap().len(),
            1
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
    fn partial_pre_metrics_upgrade_converges() {
        let root = std::env::temp_dir().join(format!("localtex-store-partial-{}", Uuid::new_v4()));
        write_pre_metrics_db(&root, 0);
        {
            let conn = Connection::open(root.join("snips.db")).unwrap();
            conn.execute("ALTER TABLE snips ADD COLUMN ocr_s REAL", [])
                .unwrap();
            conn.execute("ALTER TABLE snips ADD COLUMN confidence REAL", [])
                .unwrap();
        }
        let store = Store::open(root.clone()).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(root.join("snips/none.png").exists());
        assert_eq!(snips_user_version(&root), SCHEMA_VERSION);
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
        assert!(err.to_string().contains("unrecognized"));
        assert!(root.join("snips/keep.png").exists());
    }
}
