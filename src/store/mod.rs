use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use chrono::{Local, NaiveDate, TimeZone};
use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::doc::{Block, Document, OcrMeta, decode_blocks_json, encode_blocks_json};
use crate::identity::APP_SLUG;
use crate::imgutil;

mod date;
mod migrate;
mod writer;

pub use date::{CivilDate, DateRange};
use migrate::migrate;
pub use writer::{StoreWriter, WriteEvent, WriteKind, WriteResult};

#[derive(Clone)]
pub struct SnipListItem {
    pub id: Uuid,
    pub created_at: SystemTime,
    pub first_line: String,
    pub ocr: Option<OcrMeta>,
    pub status: crate::doc::DocStatus,
}

pub struct Store {
    conn: Mutex<Connection>,
    root: PathBuf,
}

pub(super) struct ContentUpdate {
    id: Uuid,
    blocks: Vec<Block>,
    raw_text: Option<String>,
    first_line: String,
}

impl From<&Document> for ContentUpdate {
    fn from(doc: &Document) -> Self {
        Self {
            id: doc.id,
            blocks: doc.blocks.clone(),
            raw_text: doc.raw_text.clone(),
            first_line: doc.first_line(),
        }
    }
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
        migrate(&conn)?;
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
            "SELECT id, created_at, first_line, ocr_s, confidence, processing_status, processing_error
             FROM snips ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id_s, ms, first_line, ocr_s, confidence, status, error) = row?;
            let id = match Uuid::parse_str(&id_s) {
                Ok(id) => id,
                Err(err) => {
                    eprintln!("{APP_SLUG}: skip invalid snip id {id_s:?}: {err}");
                    continue;
                }
            };
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
                ocr,
                status: match status.as_str() {
                    "ready" => crate::doc::DocStatus::Ready,
                    "pending" => crate::doc::DocStatus::Failed(
                        "Recognition interrupted; retry available".into(),
                    ),
                    _ => crate::doc::DocStatus::Failed(
                        error.unwrap_or_else(|| "Recognition failed".into()),
                    ),
                },
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

    fn image_path(&self, id: Uuid) -> PathBuf {
        self.root.join(format!("snips/{id}.png"))
    }

    #[cfg(test)]
    pub fn png_missing(&self, id: Uuid) -> Result<bool> {
        Ok(!self.image_path(id).is_file())
    }

    pub fn png_path(&self, id: Uuid) -> Result<PathBuf> {
        let path = self.image_path(id);
        if !path.is_file() {
            return Err(anyhow!("png missing"));
        }
        Ok(path)
    }

    #[cfg(test)]
    pub fn load_blocks(&self, id: Uuid) -> Result<Vec<Block>> {
        Ok(self.load_block_pair(id)?.0)
    }

    pub fn load_block_pair(&self, id: Uuid) -> Result<(Vec<Block>, Vec<Block>)> {
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        let (json, ocr_json): (String, String) = conn.query_row(
            "SELECT blocks_json, ocr_blocks_json FROM snips WHERE id = ?1",
            params![id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let blocks = decode_blocks_json(&json).context("blocks_json")?;
        let ocr_blocks = decode_blocks_json(&ocr_json).context("ocr_blocks_json")?;
        Ok((blocks, ocr_blocks))
    }

    pub fn load_png(&self, id: Uuid) -> Result<image::RgbaImage> {
        imgutil::decode_png_file(&self.png_path(id)?)
    }

    pub fn load_source(&self, id: Uuid) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        Ok(conn.query_row(
            "SELECT raw_text FROM snips WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )?)
    }

    pub fn insert_ready(&self, doc: &Document) -> Result<Vec<u8>> {
        let pixels = doc
            .image
            .pixels()
            .ok_or_else(|| anyhow!("insert requires loaded pixels"))?;
        let png = imgutil::encode_png_fast(pixels)?;
        let thumb = imgutil::encode_thumb_jpeg(pixels)?;
        let tmp = self.root.join(format!("snips/{}.png.tmp", doc.id));
        let dest = self.image_path(doc.id);
        std::fs::write(&tmp, &png).with_context(|| "write png tmp")?;
        std::fs::rename(&tmp, &dest).with_context(|| "rename png")?;
        if let Err(err) = match &doc.ink {
            Some(ink) => self.write_ink(doc.id, ink),
            None => Ok(()),
        } {
            let _ = std::fs::remove_file(&dest);
            return Err(err);
        }

        let blocks_json = encode_blocks_json(&doc.blocks)?;
        let ocr_json = encode_blocks_json(if doc.ocr_blocks.is_empty() {
            &doc.blocks
        } else {
            &doc.ocr_blocks
        })?;
        let search_text = doc
            .raw_text
            .clone()
            .unwrap_or_else(|| Document::search_text_for_blocks(&doc.blocks));
        let first_line = doc.first_line();
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        let sql = conn.execute(
            "INSERT INTO snips (id, created_at, first_line, blocks_json, search_text, thumb_jpeg, ocr_s, confidence, ocr_blocks_json, raw_text, processing_status, processing_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                doc.id.to_string(),
                unix_ms(doc.created_at),
                first_line,
                blocks_json,
                search_text,
                thumb.clone(),
                doc.ocr.map(|m| m.elapsed_s as f64),
                doc.ocr.map(|m| m.confidence as f64),
                ocr_json,
                doc.raw_text,
                status_fields(&doc.status).0,
                status_fields(&doc.status).1,
            ],
        );
        if let Err(err) = sql {
            let _ = std::fs::remove_file(&dest);
            let _ = std::fs::remove_file(self.ink_path(doc.id));
            return Err(err.into());
        }
        Ok(thumb)
    }

    pub fn update_ocr(&self, doc: &Document) -> Result<()> {
        let blocks_json = encode_blocks_json(&doc.blocks)?;
        let ocr_json = encode_blocks_json(if doc.ocr_blocks.is_empty() {
            &doc.blocks
        } else {
            &doc.ocr_blocks
        })?;
        let search_text = doc
            .raw_text
            .clone()
            .unwrap_or_else(|| Document::search_text_for_blocks(&doc.blocks));
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        let updated = conn.execute(
            "UPDATE snips SET first_line = ?1, blocks_json = ?2, search_text = ?3, ocr_s = ?4, confidence = ?5, ocr_blocks_json = ?6, raw_text = ?8, processing_status = ?9, processing_error = ?10 WHERE id = ?7",
            params![
                doc.first_line(),
                blocks_json,
                search_text,
                doc.ocr.map(|m| m.elapsed_s as f64),
                doc.ocr.map(|m| m.confidence as f64),
                ocr_json,
                doc.id.to_string(),
                doc.raw_text,
                status_fields(&doc.status).0,
                status_fields(&doc.status).1,
            ],
        )?;
        anyhow::ensure!(updated == 1, "snip {} is missing during OCR update", doc.id);
        Ok(())
    }

    fn update_blocks(&self, doc: &ContentUpdate) -> Result<()> {
        let blocks_json = encode_blocks_json(&doc.blocks)?;
        let search_text = doc
            .raw_text
            .clone()
            .unwrap_or_else(|| Document::search_text_for_blocks(&doc.blocks));
        let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
        let updated = conn.execute(
            "UPDATE snips SET first_line = ?1, blocks_json = ?2, search_text = ?3, raw_text = ?5 WHERE id = ?4",
            params![
                doc.first_line,
                blocks_json,
                search_text,
                doc.id.to_string(),
                doc.raw_text,
            ],
        )?;
        anyhow::ensure!(
            updated == 1,
            "snip {} is missing during content update",
            doc.id
        );
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        {
            let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
            conn.execute("DELETE FROM snips WHERE id = ?1", params![id.to_string()])?;
        }
        remove_if_present(&self.image_path(id)).with_context(|| "remove snip png")?;
        remove_if_present(&self.ink_path(id)).with_context(|| "remove snip ink")?;
        Ok(())
    }

    pub fn wipe(&self) -> Result<()> {
        {
            let conn = self.conn.lock().map_err(|_| anyhow!("store lock"))?;
            conn.execute("DELETE FROM snips", [])?;
        }
        let snips = self.root.join("snips");
        if snips.exists() {
            std::fs::remove_dir_all(&snips).with_context(|| "remove snips dir")?;
        }
        std::fs::create_dir_all(&snips).with_context(|| "create snips dir")?;
        Ok(())
    }

    fn ink_path(&self, id: Uuid) -> PathBuf {
        self.root.join(format!("snips/{id}.ink.json"))
    }

    fn write_ink(&self, id: Uuid, traces: &[Vec<[f32; 3]>]) -> Result<()> {
        let dest = self.ink_path(id);
        let tmp = self.root.join(format!("snips/{id}.ink.json.tmp"));
        let bytes = serde_json::to_vec(traces).context("encode ink")?;
        std::fs::write(&tmp, bytes).with_context(|| "write ink tmp")?;
        std::fs::rename(&tmp, &dest).with_context(|| "rename ink")?;
        Ok(())
    }

    pub fn load_ink(&self, id: Uuid) -> Result<Option<Vec<Vec<[f32; 3]>>>> {
        let path = self.ink_path(id);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let traces = serde_json::from_slice(&bytes).context("decode ink")?;
        Ok(Some(traces))
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
            let like = like_literal(q);
            let mut stmt = conn.prepare(
                "SELECT id FROM snips
                 WHERE created_at >= ?1 AND created_at < ?2
                   AND search_text LIKE ?3 ESCAPE '\\'
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
                let like = like_literal(q);
                let mut fallback = conn.prepare(
                    "SELECT id FROM snips
                     WHERE created_at >= ?1 AND created_at < ?2
                       AND search_text LIKE ?3 ESCAPE '\\'
                     ORDER BY created_at DESC
                     LIMIT 200",
                )?;
                collect_ids(&mut fallback, params![start_ms, end_ms, like])
            }
        }
    }
}

fn status_fields(status: &crate::doc::DocStatus) -> (&str, Option<&str>) {
    match status {
        crate::doc::DocStatus::Ready => ("ready", None),
        crate::doc::DocStatus::Recognizing => ("pending", None),
        crate::doc::DocStatus::Failed(error) => ("failed", Some(error)),
    }
}

fn remove_if_present(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
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

fn like_literal(q: &str) -> String {
    let mut out = String::from("%");
    for c in q.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Block, BlockKind, DocStatus, ImageSlot, OcrMeta, Rect};
    use image::{Rgba, RgbaImage};
    use std::sync::Arc;

    fn sample_doc(text: &str, created: SystemTime) -> Document {
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
        Document {
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
    fn raw_content_and_pending_failures_survive_reopen() {
        let (store, root) = tmp_store();
        let mut doc = sample_doc("original", SystemTime::now());
        doc.status = DocStatus::Recognizing;
        store.insert_ready(&doc).unwrap();
        assert!(matches!(
            store.list().unwrap()[0].status,
            DocStatus::Failed(_)
        ));
        doc.status = DocStatus::Failed("inference failed".into());
        store.update_ocr(&doc).unwrap();
        for text in ["<table>broken", "", " 中文\n  exact formatting\n"] {
            doc.raw_text = Some(text.into());
            store.update_blocks(&ContentUpdate::from(&doc)).unwrap();
            assert_eq!(store.load_source(doc.id).unwrap().as_deref(), Some(text));
        }
        drop(store);
        let store = Store::open(root.clone()).unwrap();
        assert_eq!(store.load_source(doc.id).unwrap(), doc.raw_text);
        assert_eq!(store.load_block_pair(doc.id).unwrap().1[0].text, "original");
        assert_eq!(
            store.query_ids("exact", DateRange::default()).unwrap(),
            vec![doc.id]
        );
        assert!(
            matches!(&store.list().unwrap()[0].status, DocStatus::Failed(error) if error == "inference failed")
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn png_path_round_trips_after_insert_ready() {
        let (store, root) = tmp_store();
        let doc = sample_doc("hello", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        let path = store.png_path(id).unwrap();
        assert_eq!(path, root.join(format!("snips/{id}.png")));
        assert!(path.is_file());
        let (blocks, ocr) = store.load_block_pair(id).unwrap();
        assert_eq!(blocks[0].text, "hello");
        assert_eq!(ocr[0].text, "hello");
        let mut edited = doc;
        edited.blocks[0].text = "edited".into();
        store.update_blocks(&ContentUpdate::from(&edited)).unwrap();
        let (blocks, ocr) = store.load_block_pair(id).unwrap();
        assert_eq!(blocks[0].text, "edited");
        assert_eq!(ocr[0].text, "hello");
    }

    #[test]
    fn png_path_errors_when_file_missing() {
        let (store, root) = tmp_store();
        let doc = sample_doc("hello", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        std::fs::remove_file(root.join(format!("snips/{id}.png"))).unwrap();
        assert!(store.png_path(id).is_err());
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
        assert!(!store.load_thumb(id).unwrap().is_empty());
        assert!(!store.png_missing(id).unwrap());
        let blocks = store.load_blocks(id).unwrap();
        assert_eq!(blocks[0].text, r"\frac{1}{2}");
        let png = store.load_png(id).unwrap();
        assert_eq!(png.dimensions(), (8, 8));
    }

    #[test]
    fn ink_sidecar_round_trips_and_deletes() {
        let (store, root) = tmp_store();
        let mut doc = sample_doc("a+b", SystemTime::now());
        let id = doc.id;
        doc.ink = Some(Arc::new(vec![vec![[1.0, 2.0, 0.0], [3.0, 4.0, 12.0]]]));
        store.insert_ready(&doc).unwrap();
        let loaded = store.load_ink(id).unwrap().expect("ink");
        assert_eq!(loaded, vec![vec![[1.0, 2.0, 0.0], [3.0, 4.0, 12.0]]]);
        store.delete(id).unwrap();
        assert!(!root.join(format!("snips/{id}.ink.json")).exists());
        assert!(store.load_ink(id).unwrap().is_none());
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
        assert_eq!(
            list.len(),
            1,
            "missing PNGs are detected on lazy image load"
        );
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
        assert!(
            store
                .query_ids("frac", DateRange::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn wipe_clears_rows_and_png_and_is_idempotent() {
        let (store, root) = tmp_store();
        let doc = sample_doc("hello", SystemTime::now());
        let id = doc.id;
        store.insert_ready(&doc).unwrap();
        let png = root.join(format!("snips/{id}.png"));
        assert!(png.is_file());
        store.wipe().unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(!png.exists());
        assert!(root.join("snips").is_dir());
        store.wipe().unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(root.join("snips").is_dir());
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
    fn short_query_like_escapes_wildcards() {
        let (store, _) = tmp_store();
        let now = SystemTime::now();
        let plain = sample_doc("xy formula", now);
        let sub = sample_doc("x_1 formula", now + Duration::from_secs(1));
        store.insert_ready(&plain).unwrap();
        store.insert_ready(&sub).unwrap();
        assert_eq!(
            store.query_ids("x_", DateRange::default()).unwrap(),
            vec![sub.id]
        );
        assert!(
            store
                .query_ids("%", DateRange::default())
                .unwrap()
                .is_empty()
        );
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
    fn search_text_is_raw_without_dollar_wrap() {
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
        assert!(
            !blob.contains("$"),
            "must not depend on Prefs::default wrap"
        );
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
}
