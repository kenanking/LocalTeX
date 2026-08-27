use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::doc::{Block, Document, OcrMeta};
use crate::identity::APP_SLUG;
use crate::imgutil;

const SCHEMA_VERSION: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl CivilDate {
    pub fn from_naive(d: NaiveDate) -> Self {
        Self {
            year: d.year(),
            month: d.month() as u8,
            day: d.day() as u8,
        }
    }

    pub fn today_local() -> Self {
        Self::from_naive(Local::now().date_naive())
    }

    pub(crate) fn to_naive(self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(self.year, self.month as u32, self.day as u32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateRange {
    pub start_day: Option<CivilDate>,
    pub end_day: Option<CivilDate>,
}

impl DateRange {
    /// Inclusive local-calendar window of `n` days ending on `today` (`n = 1` is today).
    pub fn last_n_days(today: CivilDate, n: i64) -> Self {
        let start = today.to_naive().and_then(|d| {
            d.checked_sub_signed(ChronoDuration::days((n - 1).max(0)))
                .map(CivilDate::from_naive)
        });
        Self {
            start_day: start,
            end_day: Some(today),
        }
    }
}

#[derive(Clone)]
pub struct SnipListItem {
    pub id: Uuid,
    pub created_at: SystemTime,
    pub first_line: String,
    pub thumb_jpeg: Vec<u8>,
    pub png_missing: bool,
    pub ocr: Option<OcrMeta>,
}

pub struct Store {
    conn: Mutex<Connection>,
    root: PathBuf,
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("snips")).with_context(|| "create snips dir")?;
        let path = root.join("snips.db");
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        conn.busy_timeout(Duration::from_millis(2000))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn, &root)?;
        Ok(Self {
            conn: Mutex::new(conn),
            root,
        })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(crate::identity::data_dir())
    }

    pub fn list(&self) -> Result<Vec<SnipListItem>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        let mut stmt = conn.prepare(
            "SELECT id, created_at, first_line, ocr_s, confidence
             FROM snips ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id_s, ms, first_line, ocr_s, confidence) = row?;
            let id = Uuid::parse_str(&id_s).map_err(|e| anyhow!("uuid: {e}"))?;
            let ocr = match (ocr_s, confidence) {
                (Some(elapsed_s), Some(confidence)) => Some(OcrMeta {
                    elapsed_s: elapsed_s as f32,
                    confidence: confidence as f32,
                }),
                _ => None,
            };
            out.push(SnipListItem {
                id,
                created_at: system_time_from_ms(ms),
                first_line,
                thumb_jpeg: Vec::new(),
                png_missing: false,
                ocr,
            });
        }
        Ok(out)
    }

    pub fn load_thumb(&self, id: Uuid) -> Result<Vec<u8>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        conn.query_row(
            "SELECT thumb_jpeg FROM snips WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .map_err(|e| anyhow!("load thumb: {e}"))
    }

    pub fn png_missing(&self, id: Uuid) -> Result<bool> {
        let rel = {
            let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
            conn.query_row(
                "SELECT image_relpath FROM snips WHERE id = ?1",
                params![id.to_string()],
                |row| row.get::<_, String>(0),
            )?
        };
        Ok(!self.root.join(rel).is_file())
    }

    pub fn load_blocks(&self, id: Uuid) -> Result<Vec<Block>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        let json: String = conn.query_row(
            "SELECT blocks_json FROM snips WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )?;
        serde_json::from_str(&json).context("blocks_json")
    }

    pub fn load_png(&self, id: Uuid) -> Result<image::RgbaImage> {
        if self.png_missing(id)? {
            return Err(anyhow!("png missing"));
        }
        let rel = {
            let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
            conn.query_row(
                "SELECT image_relpath FROM snips WHERE id = ?1",
                params![id.to_string()],
                |row| row.get::<_, String>(0),
            )?
        };
        let path = self.root.join(rel);
        imgutil::decode_png_file(&path)
    }

    pub fn insert_ready(&self, doc: &Document) -> Result<Vec<u8>> {
        let pixels = doc
            .image
            .pixels()
            .ok_or_else(|| anyhow!("insert requires loaded pixels"))?;
        let png = imgutil::encode_png_fast(pixels)?;
        let thumb = imgutil::encode_thumb_jpeg(pixels)?;
        let rel = format!("snips/{}.png", doc.id);
        let tmp = self.root.join(format!("snips/{}.png.tmp", doc.id));
        let dest = self.root.join(&rel);
        std::fs::write(&tmp, &png).with_context(|| "write png tmp")?;
        std::fs::rename(&tmp, &dest).with_context(|| "rename png")?;

        let blocks_json = serde_json::to_string(&doc.blocks)?;
        let search_text = Document::search_text_for_blocks(&doc.blocks);
        let first_line = doc.first_line();
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        let sql = conn.execute(
            "INSERT INTO snips (id, created_at, first_line, blocks_json, search_text, thumb_jpeg, image_relpath, ocr_s, confidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                doc.id.to_string(),
                unix_ms(doc.created_at),
                first_line,
                blocks_json,
                search_text,
                thumb.clone(),
                rel,
                doc.ocr.map(|m| m.elapsed_s as f64),
                doc.ocr.map(|m| m.confidence as f64),
            ],
        );
        if let Err(err) = sql {
            let _ = std::fs::remove_file(&dest);
            return Err(err.into());
        }
        Ok(thumb)
    }

    pub fn update_ocr(&self, doc: &Document) -> Result<()> {
        let blocks_json = serde_json::to_string(&doc.blocks)?;
        let search_text = Document::search_text_for_blocks(&doc.blocks);
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        conn.execute(
            "UPDATE snips SET first_line = ?1, blocks_json = ?2, search_text = ?3, ocr_s = ?4, confidence = ?5 WHERE id = ?6",
            params![
                doc.first_line(),
                blocks_json,
                search_text,
                doc.ocr.map(|m| m.elapsed_s as f64),
                doc.ocr.map(|m| m.confidence as f64),
                doc.id.to_string()
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        let rel: Option<String> = {
            let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
            conn.query_row(
                "SELECT image_relpath FROM snips WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?
        };
        {
            let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
            conn.execute("DELETE FROM snips WHERE id = ?1", params![id.to_string()])?;
        }
        if let Some(rel) = rel {
            let _ = std::fs::remove_file(self.root.join(rel));
        }
        Ok(())
    }

    pub fn query_ids(&self, text: &str, range: DateRange) -> Result<Vec<Uuid>> {
        let (start_ms, end_ms) = range_to_ms(range, Local);
        let q = text.trim();
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        if q.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT id FROM snips
                 WHERE created_at >= ?1 AND created_at < ?2
                 ORDER BY created_at DESC",
            )?;
            return collect_ids(&mut stmt, params![start_ms, end_ms]);
        }
        if q.chars().count() < 3 {
            let like = format!("%{q}%");
            let mut stmt = conn.prepare(
                "SELECT id FROM snips
                 WHERE created_at >= ?1 AND created_at < ?2
                   AND search_text LIKE ?3
                 ORDER BY created_at DESC
                 LIMIT 200",
            )?;
            return collect_ids(&mut stmt, params![start_ms, end_ms, like]);
        }
        let match_q = fts_literal(q);
        let mut stmt = conn.prepare(
            "SELECT snips.id FROM snips
             WHERE snips.created_at >= ?1 AND snips.created_at < ?2
               AND snips.rowid IN (
                 SELECT rowid FROM snips_fts WHERE snips_fts MATCH ?3
               )
             ORDER BY snips.created_at DESC",
        )?;
        match collect_ids(&mut stmt, params![start_ms, end_ms, match_q]) {
            Ok(ids) => Ok(ids),
            Err(err) => {
                eprintln!("{APP_SLUG}: fts query: {err}");
                Ok(Vec::new())
            }
        }
    }
}

pub fn instant_in_range(t: SystemTime, range: DateRange) -> bool {
    let (start_ms, end_ms) = range_to_ms(range, Local);
    let ms = unix_ms(t);
    ms >= start_ms && ms < end_ms
}

fn collect_ids(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> Result<Vec<Uuid>> {
    let rows = stmt.query_map(params, |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        let s = row?;
        out.push(Uuid::parse_str(&s).map_err(|e| anyhow!("uuid: {e}"))?);
    }
    Ok(out)
}

fn fts_literal(q: &str) -> String {
    let escaped = q.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn unix_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn system_time_from_ms(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64)
}

fn range_to_ms<Tz: TimeZone>(range: DateRange, tz: Tz) -> (i64, i64) {
    let start_ms = range
        .start_day
        .and_then(CivilDate::to_naive)
        .and_then(|d| local_midnight_ms(d, &tz))
        .unwrap_or(0);
    let end_ms = range
        .end_day
        .and_then(CivilDate::to_naive)
        .and_then(|d| d.succ_opt())
        .and_then(|d| local_midnight_ms(d, &tz))
        .unwrap_or(i64::MAX);
    (start_ms, end_ms)
}

fn local_midnight_ms<Tz: TimeZone>(d: NaiveDate, tz: &Tz) -> Option<i64> {
    let naive = d.and_hms_opt(0, 0, 0)?;
    tz.from_local_datetime(&naive)
        .single()
        .map(|t| t.timestamp_millis())
}

fn migrate(conn: &Connection, root: &Path) -> Result<()> {
    if snips_schema_ok(conn)? {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        return Ok(());
    }
    if snips_table_exists(conn)? {
        eprintln!("{APP_SLUG}: snips.db schema mismatch; recreating");
        conn.execute_batch(
            "
            DROP TRIGGER IF EXISTS snips_ai;
            DROP TRIGGER IF EXISTS snips_ad;
            DROP TRIGGER IF EXISTS snips_au;
            DROP TABLE IF EXISTS snips_fts;
            DROP TABLE IF EXISTS snips;
            ",
        )?;
        wipe_snip_pngs(root);
    }
    conn.execute_batch(SNIPS_SCHEMA)?;
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
    "image_relpath",
    "ocr_s",
    "confidence",
];

const SNIPS_SCHEMA: &str = "
        CREATE TABLE snips (
          id TEXT PRIMARY KEY NOT NULL,
          created_at INTEGER NOT NULL,
          first_line TEXT NOT NULL,
          blocks_json TEXT NOT NULL,
          search_text TEXT NOT NULL,
          thumb_jpeg BLOB NOT NULL,
          image_relpath TEXT NOT NULL,
          ocr_s REAL,
          confidence REAL
        );
        CREATE INDEX snips_created_at ON snips (created_at DESC);
        CREATE VIRTUAL TABLE snips_fts USING fts5(
          search_text,
          content='snips',
          content_rowid='rowid',
          tokenize='trigram'
        );
        CREATE TRIGGER snips_ai AFTER INSERT ON snips BEGIN
          INSERT INTO snips_fts(rowid, search_text) VALUES (new.rowid, new.search_text);
        END;
        CREATE TRIGGER snips_ad AFTER DELETE ON snips BEGIN
          INSERT INTO snips_fts(snips_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
        END;
        CREATE TRIGGER snips_au AFTER UPDATE ON snips BEGIN
          INSERT INTO snips_fts(snips_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
          INSERT INTO snips_fts(rowid, search_text) VALUES (new.rowid, new.search_text);
        END;
        ";

fn snips_schema_ok(conn: &Connection) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(snips)")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    if names.len() != SNIPS_COLUMNS.len() {
        return Ok(false);
    }
    Ok(SNIPS_COLUMNS
        .iter()
        .all(|col| names.iter().any(|n| n == col)))
}

fn snips_table_exists(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'snips'",
        [],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn wipe_snip_pngs(root: &Path) {
    let dir = root.join("snips");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "png") {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Block, BlockKind, DocStatus, ImageSlot, OcrMeta, Rect};
    use image::{Rgba, RgbaImage};
    use std::sync::Arc;

    fn sample_doc(text: &str, created: SystemTime) -> Document {
        let mut img = RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 255]));
        img.put_pixel(0, 0, Rgba([1, 2, 3, 255]));
        Document {
            id: Uuid::new_v4(),
            created_at: created,
            image: ImageSlot::Loaded(Arc::new(img)),
            blocks: vec![Block::new(
                BlockKind::Formula,
                Rect {
                    x: 0,
                    y: 0,
                    w: 8,
                    h: 8,
                },
                text,
            )],
            status: DocStatus::Ready,
            first_line: String::new(),
            thumb_jpeg: Vec::new(),
            persisted: false,
            blocks_loaded: true,
            ocr: None,
            revision: 0,
        }
    }

    fn tmp_store() -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!("localtex-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        (Store::open(root.clone()).unwrap(), root)
    }

    #[test]
    fn round_trip_blocks_and_png() {
        let (store, _root) = tmp_store();
        let doc = sample_doc(r"\frac{1}{2}", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        drop(store);
        let store = Store::open(_root.clone()).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].thumb_jpeg.is_empty());
        assert!(!store.load_thumb(id).unwrap().is_empty());
        assert!(!store.png_missing(id).unwrap());
        let blocks = store.load_blocks(id).unwrap();
        assert_eq!(blocks[0].text, r"\frac{1}{2}");
        let png = store.load_png(id).unwrap();
        assert_eq!(png.dimensions(), (8, 8));
    }

    #[test]
    fn missing_png_still_lists_and_loads_blocks() {
        let (store, root) = tmp_store();
        let doc = sample_doc("hello", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        std::fs::remove_file(root.join(format!("snips/{id}.png"))).unwrap();
        let list = store.list().unwrap();
        assert!(store.png_missing(id).unwrap());
        assert!(!list[0].png_missing);
        assert_eq!(store.load_blocks(id).unwrap()[0].text, "hello");
    }

    #[test]
    fn delete_removes_row_and_fts() {
        let (store, root) = tmp_store();
        let doc = sample_doc(r"\frac{a}{b}", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        store.delete(id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(!root.join(format!("snips/{id}.png")).exists());
        assert!(store
            .query_ids("frac", DateRange::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn fts_matches_latex_fragment() {
        let (store, _) = tmp_store();
        let doc = sample_doc(r"\frac{1}{2}", SystemTime::now());
        store.insert_ready(&doc).unwrap();
        let ids = store.query_ids("frac", DateRange::default()).unwrap();
        assert_eq!(ids, vec![doc.id]);
        let ids = store.query_ids(r"\frac", DateRange::default()).unwrap();
        assert_eq!(ids, vec![doc.id]);
    }

    #[test]
    fn short_query_uses_like() {
        let (store, _) = tmp_store();
        let doc = sample_doc("ab formula", SystemTime::now());
        store.insert_ready(&doc).unwrap();
        let ids = store.query_ids("ab", DateRange::default()).unwrap();
        assert_eq!(ids, vec![doc.id]);
    }

    #[test]
    fn date_presets_filter_created_at() {
        let (store, _) = tmp_store();
        let today = CivilDate::today_local();
        let recent = sample_doc("recent", SystemTime::now());
        let old = sample_doc(
            "oldone",
            SystemTime::now() - Duration::from_secs(10 * 24 * 3600),
        );
        store.insert_ready(&recent).unwrap();
        store.insert_ready(&old).unwrap();

        let all = store.query_ids("", DateRange::default()).unwrap();
        assert_eq!(all.len(), 2);

        let t = store
            .query_ids("", DateRange::last_n_days(today, 1))
            .unwrap();
        assert_eq!(t, vec![recent.id]);

        let week = store
            .query_ids("", DateRange::last_n_days(today, 7))
            .unwrap();
        assert_eq!(week, vec![recent.id]);

        let month = store
            .query_ids("", DateRange::last_n_days(today, 30))
            .unwrap();
        assert_eq!(month.len(), 2);
    }

    #[test]
    fn date_and_fts_are_and() {
        let (store, _) = tmp_store();
        let today = CivilDate::today_local();
        let now = SystemTime::now();
        let a = sample_doc(r"\frac{1}{2}", now);
        let b = sample_doc("plain text only", now);
        store.insert_ready(&a).unwrap();
        store.insert_ready(&b).unwrap();
        let ids = store
            .query_ids("frac", DateRange::last_n_days(today, 1))
            .unwrap();
        assert_eq!(ids, vec![a.id]);
    }

    #[test]
    fn retry_does_not_change_created_at() {
        let (store, _) = tmp_store();
        let doc = sample_doc("first", SystemTime::now());
        let id = doc.id;
        let created = doc.created_at;
        store.insert_ready(&doc).unwrap();
        let mut retry = sample_doc("second", created);
        retry.id = id;
        retry.ocr = Some(OcrMeta {
            elapsed_s: 0.5,
            confidence: 0.9,
        });
        store.update_ocr(&retry).unwrap();
        let ms0 = unix_ms(created);
        let listed = store.list().unwrap();
        let ms1 = unix_ms(listed[0].created_at);
        assert_eq!(ms0, ms1);
        assert_eq!(store.load_blocks(id).unwrap()[0].text, "second");
    }

    #[test]
    fn search_text_uses_default_delimiters() {
        let blocks = vec![Block::new(
            BlockKind::Formula,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            "x^2",
        )];
        let blob = Document::search_text_for_blocks(&blocks);
        assert!(blob.contains("x^2"));
        assert!(blob.contains("$"));
    }

    #[test]
    fn round_trips_ocr_metrics() {
        let (store, _) = tmp_store();
        let mut doc = sample_doc("hello", SystemTime::now());
        doc.ocr = Some(OcrMeta {
            elapsed_s: 1.25,
            confidence: 0.8,
        });
        store.insert_ready(&doc).unwrap();
        let item = &store.list().unwrap()[0];
        let meta = item.ocr.expect("ocr meta");
        assert!((meta.elapsed_s - 1.25).abs() < 1e-5);
        assert!((meta.confidence - 0.8).abs() < 1e-5);
        let from = Document::from_list_item(item.clone());
        assert_eq!(from.ocr, item.ocr);
    }

    #[test]
    fn incompatible_schema_is_recreated() {
        let root = std::env::temp_dir().join(format!("localtex-store-old-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("snips")).unwrap();
        {
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
                params!["00000000-0000-0000-0000-000000000001"],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }
        let orphan = root.join("snips/none.png");
        std::fs::write(&orphan, b"x").unwrap();
        let store = Store::open(root).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(!orphan.exists());
        let mut doc = sample_doc("fresh", SystemTime::now());
        doc.ocr = Some(OcrMeta {
            elapsed_s: 0.85,
            confidence: 0.92,
        });
        store.insert_ready(&doc).unwrap();
        let item = &store.list().unwrap()[0];
        assert_eq!(item.id, doc.id);
        let meta = item.ocr.expect("ocr meta");
        assert!((meta.elapsed_s - 0.85).abs() < 1e-5);
        assert!((meta.confidence - 0.92).abs() < 1e-5);
    }
}
