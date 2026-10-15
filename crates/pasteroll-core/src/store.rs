//! SQLite history, encrypted at rest.
//!
//! One row per distinct clip. Re-copying identical content moves the existing clip back to the top
//! instead of adding a duplicate. Ordering is a floating `ord` (newer = larger) with `id` as the
//! tie-breaker.
//!
//! What is stored in the clear: row id, ordering key, day-granularity dates, clip kind, pinned flag
//! and the padded size. What is sealed (XChaCha20-Poly1305, see [`crate::vault`]): the clip body and
//! a small searchable record. Both are padded to size buckets first ([`crate::padding`]), so
//! ciphertext length does not reveal clip length. Each row has a random `uid` that is part of the
//! associated data, so a body cannot be moved to another row by someone who can write the file.
//! The duplicate fingerprint is a keyed HMAC and reveals nothing about the content.
//!
//! Limits ([`crate::limits::Limits`]): whichever of count, total size or age is hit first removes
//! the oldest *unpinned* clips; pinned clips are exempt from all three.
//!
//! Deleted data: `secure_delete` overwrites freed pages, the write-ahead log is truncated after
//! deletions (otherwise old page images, including deleted ciphertext, would linger there), and
//! `incremental_vacuum` returns freed pages to the OS.
//!
//! A database that was not created by this app, or by another schema version, is refused rather
//! than touched (`application_id` and `user_version`).

use crate::limits::Limits;
use crate::padding::{pad, unpad};
use crate::record::{png_dimensions, Record};
use crate::source::{Dir, HistorySource, Item, Snapshot};
use crate::vault::Vault;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;
use zeroize::Zeroize;

/// "CARS" - marks a database as ours.
const APPLICATION_ID: i64 = 0x4341_5253;
const SCHEMA_VERSION: i64 = 4;
const SECS_PER_DAY: i64 = 86_400;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("this database was not created by Pasteroll (application id {0:#x})")]
    Foreign(i64),
    #[error("unsupported database schema version {0} (this build uses {SCHEMA_VERSION})")]
    Schema(i64),
    #[error("clip is larger than the configured per-clip limit")]
    TooLarge,
    #[error("could not generate a random identifier")]
    Random,
    #[error("invalid limits: {0}")]
    Limits(String),
    #[error("{0} clip(s) could not be opened with the current key; key rotation was not attempted")]
    Unreadable(usize),
    #[error("the history's key has changed since this connection was opened; restart to continue")]
    StaleKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text = 0,
    Image = 1,
    Files = 2,
}

impl Kind {
    fn from_i64(v: i64) -> Kind {
        match v {
            1 => Kind::Image,
            2 => Kind::Files,
            _ => Kind::Text,
        }
    }
    fn label(self) -> &'static [u8] {
        match self {
            Kind::Text => b"carousel/clip/text/",
            Kind::Image => b"carousel/clip/image/",
            Kind::Files => b"carousel/clip/files/",
        }
    }
}

/// Associated data for a body: its kind and the row's random identity.
fn aad_body(kind: Kind, uid: &[u8]) -> Vec<u8> {
    let mut v = kind.label().to_vec();
    v.extend_from_slice(uid);
    v
}

/// The preview picture for `png`: decoded, shrunk to the preview size, re-encoded small.
fn make_thumb(png: &[u8]) -> Option<Vec<u8>> {
    let img = crate::image::decode_png(png)?;
    let small = crate::image::downscale(&img, crate::image::THUMB_W, crate::image::THUMB_H);
    crate::image::encode_png_fast(&small)
}

fn aad_thumb(uid: &[u8]) -> Vec<u8> {
    let mut v = b"carousel/thumb/".to_vec();
    v.extend_from_slice(uid);
    v
}

fn aad_search(uid: &[u8]) -> Vec<u8> {
    let mut v = b"carousel/search/".to_vec();
    v.extend_from_slice(uid);
    v
}

/// What the overlay needs to render a candidate. Deliberately has no `Debug` impl: clip text must
/// never end up in a log line by accident.
#[derive(Clone, PartialEq)]
pub struct Preview {
    pub kind: Kind,
    /// Text clips: the first `max_chars` characters. File clips: the paths, one per line.
    /// Image clips: empty (the overlay decodes the PNG itself).
    pub text: String,
    /// The text was cut at `max_chars`.
    pub truncated: bool,
    /// Total characters in the clip (text and files), so the overlay can say how much is hidden.
    pub total_chars: usize,
}

/// Light description of a clip: enough to draw a row or run a search, without opening the body.
/// No `Debug` impl, for the same reason as [`Preview`].
#[derive(Clone, PartialEq)]
pub struct ClipInfo {
    pub id: i64,
    pub kind: Kind,
    pub pinned: bool,
    pub total_chars: usize,
    pub width: u32,
    pub height: u32,
    pub files: usize,
    /// First part of the text, or the file paths, or "image WxH".
    pub search: String,
}

/// One entry for the in-memory search index.
#[derive(Clone, PartialEq)]
pub struct SearchRow {
    pub id: i64,
    pub order: f64,
    pub kind: Kind,
    pub pinned: bool,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Unpinned clips.
    pub clips: i64,
    /// Stored size of unpinned clips (padded), in bytes.
    pub bytes: i64,
    pub pins: i64,
}

/// What lowering the limits would delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrunePlan {
    pub clips: usize,
    pub bytes: u64,
    /// How many of those are only because of age.
    pub by_age: usize,
}

/// Whether the key a store was opened with is the one its data is sealed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Ok,
    /// A history without a key check (or an empty one): the check was written just now.
    Initialized,
    Wrong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationState {
    NotDue,
    /// Older than the rotation interval.
    DueByAge,
    /// Requested explicitly (for example after limits were lowered).
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RotateReport {
    pub rows: usize,
    /// The transaction is committed AND the write-ahead log was fully checkpointed, so the new
    /// state is on disk. If false, the caller must keep the old key until the next successful start.
    pub durable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinResult {
    Pinned,
    Unpinned,
    LimitReached,
    Missing,
}

pub struct Store {
    conn: Connection,
    vault: Vault,
    limits: Limits,
    /// Counters for unpinned clips, kept in step with every change so limits never need a scan.
    count: i64,
    bytes: i64,
    pins: i64,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS clips(
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ord         REAL    NOT NULL,
    created_day INTEGER NOT NULL,
    touched_day INTEGER NOT NULL,
    kind        INTEGER NOT NULL,
    pinned      INTEGER NOT NULL DEFAULT 0,
    size        INTEGER NOT NULL,
    uid         BLOB    NOT NULL,
    body        BLOB    NOT NULL,
    search      BLOB    NOT NULL,
    hash        BLOB    NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS clips_order ON clips(ord DESC, id DESC);
CREATE INDEX IF NOT EXISTS clips_unpinned_order ON clips(pinned, ord);
CREATE INDEX IF NOT EXISTS clips_touched ON clips(pinned, touched_day);
CREATE TABLE IF NOT EXISTS meta(
    k TEXT PRIMARY KEY,
    v BLOB NOT NULL
);
-- Derived data: a small sealed preview picture per image clip, so scrolling never has to decode
-- the full image. Removed with its clip; dropped (and rebuilt on demand) when the key rotates.
CREATE TABLE IF NOT EXISTS thumbs(
    id   INTEGER PRIMARY KEY,
    body BLOB NOT NULL
);
CREATE TRIGGER IF NOT EXISTS thumbs_cleanup AFTER DELETE ON clips
BEGIN
    DELETE FROM thumbs WHERE id = old.id;
END;
";

fn random_uid() -> Result<[u8; 16], StoreError> {
    let mut u = [0u8; 16];
    getrandom::fill(&mut u).map_err(|_| StoreError::Random)?;
    Ok(u)
}

fn day(secs: i64) -> i64 {
    secs.div_euclid(SECS_PER_DAY)
}

impl Store {
    pub fn open(path: &Path, vault: Vault) -> Result<Self, StoreError> {
        Self::init(Connection::open(path)?, vault)
    }

    pub fn open_in_memory(vault: Vault) -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?, vault)
    }

    fn init(conn: Connection, vault: Vault) -> Result<Self, StoreError> {
        let app_id: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let has_tables: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table')",
            [],
            |r| r.get(0),
        )?;
        if has_tables {
            if app_id != APPLICATION_ID {
                return Err(StoreError::Foreign(app_id));
            }
            if version != SCHEMA_VERSION {
                return Err(StoreError::Schema(version));
            }
        } else {
            // Only takes effect before the first table exists: freed pages can be handed back
            // incrementally instead of keeping the file at its high-water mark.
            conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        }
        conn.busy_timeout(std::time::Duration::from_millis(250))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Deleted clips must not linger in free pages, and the WAL must not keep growing.
        conn.pragma_update(None, "secure_delete", "ON")?;
        conn.pragma_update(None, "journal_size_limit", 1_048_576)?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "application_id", APPLICATION_ID)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        let mut s = Store { conn, vault, limits: Limits::default(), count: 0, bytes: 0, pins: 0 };
        s.recount()?;
        Ok(s)
    }

    fn recount(&mut self) -> Result<(), StoreError> {
        let (count, bytes): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM clips WHERE pinned = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let pins: i64 = self.conn.query_row("SELECT COUNT(*) FROM clips WHERE pinned = 1", [], |r| r.get(0))?;
        self.count = count;
        self.bytes = bytes;
        self.pins = pins;
        Ok(())
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn set_limits(&mut self, l: Limits) -> Result<(), StoreError> {
        let p = l.problems();
        if !p.is_empty() {
            return Err(StoreError::Limits(p.join("; ")));
        }
        self.limits = l;
        Ok(())
    }

    pub fn stats(&self) -> Stats {
        Stats { clips: self.count, bytes: self.bytes, pins: self.pins }
    }

    // ------------------------------------------------------------------ adding

    /// Inserts a clip, or moves an identical existing one back to the top (re-sealing its body, so a
    /// corrupted body is replaced by the fresh copy). Returns its id.
    fn add(&mut self, kind: Kind, plain: &[u8], record: Record, now: i64) -> Result<i64, StoreError> {
        if plain.len() as u64 > self.limits.max_clip_bytes {
            return Err(StoreError::TooLarge);
        }
        let hash = self.vault.fingerprint(kind as u8, plain);
        let uid = random_uid()?;
        let mut padded = pad(plain);
        let size = padded.len() as i64;
        let body = self.vault.seal(&aad_body(kind, &uid), &padded);
        padded.zeroize();
        let mut record_bytes = record.encode();
        let mut padded_record = pad(&record_bytes);
        let search = self.vault.seal(&aad_search(&uid), &padded_record);
        record_bytes.zeroize();
        padded_record.zeroize();
        // IMMEDIATE: take the write lock up front. A deferred transaction that reads first and
        // writes later fails with SQLITE_BUSY_SNAPSHOT if another connection wrote in between,
        // and the capture would be silently lost.
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // A connection opened before a key rotation must not write rows under the old key: they
        // would be unreadable (and would block every later rotation).
        if !Self::key_current(&tx, &self.vault)? {
            return Err(StoreError::StaleKey);
        }
        let existing: Option<(i64, i64, i64)> = tx
            .query_row("SELECT id, size, pinned FROM clips WHERE hash = ?1", [&hash[..]], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let max: Option<f64> = tx.query_row("SELECT MAX(ord) FROM clips", [], |r| r.get(0))?;
        let ord = max.map_or(1.0, |m| m + 1.0);
        let today = day(now);
        let (id, delta_count, delta_bytes) = match existing {
            Some((id, old_size, pinned)) => {
                tx.execute(
                    "UPDATE clips SET ord = ?1, touched_day = ?2, uid = ?3, body = ?4, search = ?5, size = ?6 \
                     WHERE id = ?7",
                    params![ord, today, &uid[..], body, search, size, id],
                )?;
                (id, 0, if pinned == 0 { size - old_size } else { 0 })
            }
            None => {
                tx.execute(
                    "INSERT INTO clips(ord, created_day, touched_day, kind, pinned, size, uid, body, search, hash) \
                     VALUES(?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8)",
                    params![ord, today, kind as i64, size, &uid[..], body, search, &hash[..]],
                )?;
                (tx.last_insert_rowid(), 1, size)
            }
        };
        tx.commit()?;
        self.count += delta_count;
        self.bytes += delta_bytes;
        self.trim_if_over()?;
        Ok(id)
    }

    pub fn add_text(&mut self, text: &str, now: i64) -> Result<i64, StoreError> {
        self.add(Kind::Text, text.as_bytes(), Record::for_text(text), now)
    }

    pub fn add_image_png(&mut self, png: &[u8], now: i64) -> Result<i64, StoreError> {
        let (w, h) = png_dimensions(png).unwrap_or((0, 0));
        let id = self.add(Kind::Image, png, Record::for_image(w, h), now)?;
        // the preview picture is made now, off the scrolling path (a copy that was already stored
        // keeps the one it has)
        if self.thumb(id)?.is_none() {
            if let Some(thumb) = make_thumb(png) {
                self.set_thumb(id, &thumb)?;
            }
        }
        Ok(id)
    }

    /// The sealed preview picture of an image clip (a small PNG), if one has been made.
    pub fn thumb(&self, id: i64) -> Result<Option<Vec<u8>>, StoreError> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT c.uid, t.body FROM thumbs t JOIN clips c ON c.id = t.id WHERE t.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(uid, sealed)| {
            let padded = self.vault.open(&aad_thumb(&uid), &sealed)?;
            unpad(&padded).map(<[u8]>::to_vec)
        }))
    }

    /// Stores the preview picture of clip `id` (replacing any earlier one).
    pub fn set_thumb(&mut self, id: i64, png: &[u8]) -> Result<(), StoreError> {
        let uid: Option<Vec<u8>> =
            self.conn.query_row("SELECT uid FROM clips WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        let Some(uid) = uid else { return Ok(()) };
        let mut padded = pad(png);
        let sealed = self.vault.seal(&aad_thumb(&uid), &padded);
        padded.zeroize();
        self.conn.execute("INSERT OR REPLACE INTO thumbs(id, body) VALUES(?1, ?2)", params![id, sealed])?;
        Ok(())
    }

    pub fn add_files(&mut self, paths: &[String], now: i64) -> Result<i64, StoreError> {
        self.add(Kind::Files, paths.join("\n").as_bytes(), Record::for_files(paths), now)
    }

    // ------------------------------------------------------------------ limits

    /// Cheap check run after every add: only does work once a limit is exceeded by a margin, then
    /// trims in one batch (so pruning is not paid on every copy).
    fn trim_if_over(&mut self) -> Result<(), StoreError> {
        let l = self.limits;
        let slack_c = ((l.max_clips / 20).max(8)) as i64;
        let slack_b = ((l.max_bytes / 20).max(1 << 20)) as i64;
        if self.count > l.max_clips as i64 + slack_c || self.bytes > l.max_bytes as i64 + slack_b {
            self.trim_to_limits()?;
        }
        Ok(())
    }

    /// Deletes the oldest unpinned clips until the count and size limits hold. Walks the ordering
    /// index from the oldest end, so the cost is proportional to what is deleted.
    fn trim_to_limits(&mut self) -> Result<usize, StoreError> {
        let (max_c, max_b) = (self.limits.max_clips as i64, self.limits.max_bytes as i64);
        let mut removed = 0usize;
        while self.count > max_c || self.bytes > max_b {
            let batch: Vec<(i64, i64)> = {
                let mut stmt = self.conn.prepare_cached(
                    "SELECT id, size FROM clips WHERE pinned = 0 ORDER BY ord ASC, id ASC LIMIT 256",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            if batch.is_empty() {
                break;
            }
            let tx = self.conn.transaction()?;
            let (mut c, mut b) = (self.count, self.bytes);
            let mut n = 0usize;
            for (id, size) in batch {
                if c <= max_c && b <= max_b {
                    break;
                }
                tx.execute("DELETE FROM clips WHERE id = ?1", [id])?;
                c -= 1;
                b -= size;
                n += 1;
            }
            tx.commit()?;
            self.count = c;
            self.bytes = b;
            removed += n;
            if n == 0 {
                break;
            }
        }
        Ok(removed)
    }

    /// Full retention pass: age, count and size limits, then scrub deleted data out of the WAL and
    /// give freed pages back. Run at startup and periodically. Returns how many clips were removed.
    pub fn maintain(&mut self, now: i64) -> Result<usize, StoreError> {
        let mut removed = 0usize;
        if let Some(days) = self.limits.max_age_days {
            let cutoff = day(now) - i64::from(days);
            removed += self
                .conn
                .execute("DELETE FROM clips WHERE pinned = 0 AND touched_day < ?1", [cutoff])?;
            self.recount()?;
        }
        removed += self.trim_to_limits()?;
        if removed > 0 {
            self.scrub()?;
        }
        Ok(removed)
    }

    /// What the given limits would delete right now, without deleting anything.
    pub fn plan_prune(&self, limits: &Limits, now: i64) -> Result<PrunePlan, StoreError> {
        let cutoff = limits.max_age_days.map(|d| day(now) - i64::from(d));
        let mut stmt = self
            .conn
            .prepare("SELECT size, touched_day FROM clips WHERE pinned = 0 ORDER BY ord ASC, id ASC")?;
        let rows: Vec<(i64, i64)> =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        let mut plan = PrunePlan::default();
        let mut keep: Vec<(i64, i64)> = Vec::with_capacity(rows.len());
        for (size, touched) in rows {
            if cutoff.is_some_and(|c| touched < c) {
                plan.clips += 1;
                plan.by_age += 1;
                plan.bytes += size as u64;
            } else {
                keep.push((size, touched));
            }
        }
        let mut count = keep.len() as i64;
        let mut bytes: i64 = keep.iter().map(|(s, _)| s).sum();
        for (size, _) in keep {
            if count <= limits.max_clips as i64 && bytes <= limits.max_bytes as i64 {
                break;
            }
            count -= 1;
            bytes -= size;
            plan.clips += 1;
            plan.bytes += size as u64;
        }
        Ok(plan)
    }

    /// Removes deleted rows' remnants from the WAL and shrinks the file.
    fn scrub(&self) -> Result<(), StoreError> {
        // Both pragmas return a row; read and drop it.
        self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        self.conn.query_row("PRAGMA incremental_vacuum", [], |_| Ok(())).optional()?;
        Ok(())
    }

    // ------------------------------------------------------------------ reading

    fn body(&self, id: i64) -> Result<Option<(Kind, Vec<u8>)>, StoreError> {
        let row: Option<(i64, Vec<u8>, Vec<u8>)> = self
            .conn
            .query_row("SELECT kind, uid, body FROM clips WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        Ok(row.and_then(|(k, uid, sealed)| {
            let kind = Kind::from_i64(k);
            let padded = self.vault.open(&aad_body(kind, &uid), &sealed)?;
            unpad(&padded).map(|p| (kind, p.to_vec()))
        }))
    }

    /// Row description without opening the body.
    pub fn info(&self, id: i64) -> Result<Option<ClipInfo>, StoreError> {
        let row: Option<(i64, i64, Vec<u8>, Vec<u8>)> = self
            .conn
            .query_row("SELECT kind, pinned, uid, search FROM clips WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        Ok(row.and_then(|(k, pinned, uid, sealed)| {
            let rec = self.open_record(&uid, &sealed)?;
            Some(ClipInfo {
                id,
                kind: Kind::from_i64(k),
                pinned: pinned != 0,
                total_chars: rec.total_chars as usize,
                width: rec.width,
                height: rec.height,
                files: rec.files as usize,
                search: rec.text,
            })
        }))
    }

    fn open_record(&self, uid: &[u8], sealed: &[u8]) -> Option<Record> {
        let mut padded = self.vault.open(&aad_search(uid), sealed)?;
        let rec = unpad(&padded).and_then(Record::decode);
        padded.zeroize();
        rec
    }

    /// Everything the search index needs, newest first.
    pub fn search_rows(&self) -> Result<Vec<SearchRow>, StoreError> {
        self.search_rows_newest(i64::MAX as usize)
    }

    /// The newest `limit` clips' search records, newest first (bounds the index's memory).
    pub fn search_rows_newest(&self, limit: usize) -> Result<Vec<SearchRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ord, kind, pinned, uid, search FROM clips ORDER BY ord DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit.min(i64::MAX as usize) as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, Vec<u8>>(4)?, r.get::<_, Vec<u8>>(5)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, order, kind, pinned, uid, sealed) = row?;
            if let Some(rec) = self.open_record(&uid, &sealed) {
                out.push(SearchRow { id, order, kind: Kind::from_i64(kind), pinned: pinned != 0, text: rec.text });
            }
        }
        Ok(out)
    }

    pub fn preview(&self, id: i64, max_chars: usize) -> Result<Option<Preview>, StoreError> {
        Ok(self.body(id)?.map(|(kind, mut plain)| {
            let preview = match kind {
                Kind::Image => Preview { kind, text: String::new(), truncated: false, total_chars: 0 },
                _ => {
                    let s = String::from_utf8_lossy(&plain);
                    let total_chars = s.chars().count();
                    let mut it = s.char_indices();
                    match it.nth(max_chars) {
                        Some((cut, _)) => Preview { kind, text: s[..cut].to_owned(), truncated: true, total_chars },
                        None => Preview { kind, text: s.into_owned(), truncated: false, total_chars },
                    }
                }
            };
            plain.zeroize();
            preview
        }))
    }

    /// Full text of a text clip (for a whole-clip paste).
    pub fn text(&self, id: i64) -> Result<Option<String>, StoreError> {
        Ok(self.body(id)?.and_then(|(k, mut plain)| {
            let text = (k == Kind::Text).then(|| String::from_utf8_lossy(&plain).into_owned());
            plain.zeroize();
            text
        }))
    }

    /// File paths of a files clip.
    pub fn files(&self, id: i64) -> Result<Option<Vec<String>>, StoreError> {
        Ok(self.body(id)?.and_then(|(k, mut plain)| {
            let files = (k == Kind::Files)
                .then(|| String::from_utf8_lossy(&plain).split('\n').map(str::to_owned).collect());
            plain.zeroize();
            files
        }))
    }

    pub fn image_png(&self, id: i64) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.body(id)?.and_then(|(k, plain)| (k == Kind::Image).then_some(plain)))
    }

    pub fn kind(&self, id: i64) -> Result<Option<Kind>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT kind FROM clips WHERE id = ?1", [id], |r| r.get::<_, i64>(0))
            .optional()?
            .map(Kind::from_i64))
    }

    // ------------------------------------------------------------------ changing

    pub fn delete(&mut self, id: i64) -> Result<bool, StoreError> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row("SELECT size, pinned FROM clips WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((size, pinned)) = row else { return Ok(false) };
        self.conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        if pinned != 0 {
            self.pins -= 1;
        } else {
            self.count -= 1;
            self.bytes -= size;
        }
        self.scrub()?;
        Ok(true)
    }

    /// Deletes every clip (optionally keeping pinned ones). Returns how many were removed.
    pub fn clear(&mut self, keep_pinned: bool) -> Result<usize, StoreError> {
        let n = if keep_pinned {
            self.conn.execute("DELETE FROM clips WHERE pinned = 0", [])?
        } else {
            self.conn.execute("DELETE FROM clips", [])?
        };
        self.recount()?;
        self.scrub()?;
        Ok(n)
    }

    /// Pins or unpins a clip. A pinned clip is exempt from every limit; the number of pins is
    /// capped by `limits.max_pins`.
    pub fn set_pinned(&mut self, id: i64, want: bool) -> Result<PinResult, StoreError> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row("SELECT size, pinned FROM clips WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((size, pinned)) = row else { return Ok(PinResult::Missing) };
        let is = pinned != 0;
        if want == is {
            return Ok(if is { PinResult::Pinned } else { PinResult::Unpinned });
        }
        if want {
            if self.pins >= i64::from(self.limits.max_pins) {
                return Ok(PinResult::LimitReached);
            }
            self.conn.execute("UPDATE clips SET pinned = 1 WHERE id = ?1", [id])?;
            self.pins += 1;
            self.count -= 1;
            self.bytes -= size;
            Ok(PinResult::Pinned)
        } else {
            self.conn.execute("UPDATE clips SET pinned = 0 WHERE id = ?1", [id])?;
            self.pins -= 1;
            self.count += 1;
            self.bytes += size;
            self.trim_if_over()?;
            Ok(PinResult::Unpinned)
        }
    }

    /// Records a paste; `bump` also moves the clip to the top of history. Either way the clip's
    /// age restarts: a clip you keep using does not expire.
    pub fn mark_pasted(&mut self, id: i64, now: i64, bump: bool) -> Result<(), StoreError> {
        if bump {
            let max: Option<f64> = self.conn.query_row("SELECT MAX(ord) FROM clips", [], |r| r.get(0))?;
            self.conn.execute(
                "UPDATE clips SET ord = ?1, touched_day = ?2 WHERE id = ?3",
                params![max.map_or(1.0, |m| m + 1.0), day(now), id],
            )?;
        } else {
            self.conn.execute("UPDATE clips SET touched_day = ?1 WHERE id = ?2", params![day(now), id])?;
        }
        Ok(())
    }

    /// Exact row count (pinned and unpinned).
    pub fn count(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0))?)
    }

    // ------------------------------------------------------------------ meta, key check, rotation

    fn meta_get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.conn.query_row("SELECT v FROM meta WHERE k = ?1", [key], |r| r.get(0)).optional()?)
    }

    fn meta_set(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO meta(k, v) VALUES(?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![key, value],
        )?;
        Ok(())
    }

    fn meta_i64(&self, key: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.meta_get(key)?.and_then(|b| b.try_into().ok().map(i64::from_le_bytes)))
    }

    const KEY_CHECK: &'static [u8] = b"carousel key check v1";
    const KEY_CHECK_AAD: &'static [u8] = b"carousel/key-check";

    /// Does the vault this store was opened with match the data? A wrong key never silently
    /// succeeds and never writes: call this right after opening.
    pub fn verify_key(&mut self) -> Result<KeyStatus, StoreError> {
        if let Some(sealed) = self.meta_get("key_check")? {
            let ok = self.vault.open(Self::KEY_CHECK_AAD, &sealed).is_some_and(|p| p == Self::KEY_CHECK);
            return Ok(if ok { KeyStatus::Ok } else { KeyStatus::Wrong });
        }
        // No check yet (an empty history, or one written before checks existed): if there is data,
        // it must open with this key before the check is created from it.
        let first: Option<i64> = self.conn.query_row("SELECT MIN(id) FROM clips", [], |r| r.get(0))?;
        if let Some(id) = first {
            if self.body(id)?.is_none() {
                return Ok(KeyStatus::Wrong);
            }
        }
        let sealed = self.vault.seal(Self::KEY_CHECK_AAD, Self::KEY_CHECK);
        self.meta_set("key_check", &sealed)?;
        Ok(KeyStatus::Initialized)
    }

    /// Is a key rotation due? A history with no recorded rotation starts its clock now.
    pub fn rotation_state(&mut self, now: i64, every_days: i64) -> Result<RotationState, StoreError> {
        if self.meta_i64("rotate_requested")?.is_some() {
            return Ok(RotationState::Requested);
        }
        match self.meta_i64("last_rotation_day")? {
            None => {
                self.meta_set("last_rotation_day", &day(now).to_le_bytes())?;
                Ok(RotationState::NotDue)
            }
            // a recorded date in the future (clock rolled back) counts as due
            Some(d) if day(now) - d >= every_days || d > day(now) + 1 => Ok(RotationState::DueByAge),
            Some(_) => Ok(RotationState::NotDue),
        }
    }

    /// Ask for a rotation at the next start (the running app holds the old key in memory, so a
    /// rotation can only happen while it is not running).
    pub fn request_rotation(&self) -> Result<(), StoreError> {
        self.meta_set("rotate_requested", &1i64.to_le_bytes())
    }

    /// Is `vault` still the key this history is sealed under? Checked inside write transactions so
    /// that a connection opened before a rotation cannot write rows nobody can read.
    fn key_current(conn: &Connection, vault: &Vault) -> Result<bool, StoreError> {
        let sealed: Option<Vec<u8>> = conn
            .query_row("SELECT v FROM meta WHERE k = 'key_check'", [], |r| r.get(0))
            .optional()?;
        Ok(match sealed {
            None => true, // a history that has not had a check yet
            Some(s) => vault.open(Self::KEY_CHECK_AAD, &s).is_some_and(|p| p == Self::KEY_CHECK),
        })
    }

    /// Re-seals every clip under `new_vault` with fresh per-row ids, in one transaction: either all
    /// rows move to the new key or none do. Afterwards the write-ahead log is truncated and the file
    /// is vacuumed, so ciphertext of deleted clips (sealed under the old key) is gone from disk.
    ///
    /// If any row cannot be opened with the current key nothing is changed: a wrong key must never
    /// turn into data loss.
    pub fn rotate(&mut self, new_vault: Vault, now: i64) -> Result<RotateReport, StoreError> {
        self.rotate_with_recovery(new_vault, now, None)
    }

    /// As [`Store::rotate`], and in the same transaction stores `recovery_blob`: the new master key
    /// sealed to the history's recovery public key, so an export made long ago still recovers it.
    pub fn rotate_with_recovery(
        &mut self,
        new_vault: Vault,
        now: i64,
        recovery_blob: Option<Vec<u8>>,
    ) -> Result<RotateReport, StoreError> {
        // The commit and the checkpoint below must reach the disk before the caller drops the old
        // key; NORMAL would let the log lag behind after a power cut.
        self.conn.pragma_update(None, "synchronous", "FULL")?;
        let result = self.rotate_inner(new_vault, now, recovery_blob);
        let _ = self.conn.pragma_update(None, "synchronous", "NORMAL");
        result
    }

    fn rotate_inner(
        &mut self,
        new_vault: Vault,
        now: i64,
        recovery_blob: Option<Vec<u8>>,
    ) -> Result<RotateReport, StoreError> {
        // The write lock is taken first, and the row list is read under it: a writer that sneaks in
        // between listing and locking would otherwise add rows this rotation never sees.
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !Self::key_current(&tx, &self.vault)? {
            return Err(StoreError::StaleKey);
        }
        let ids: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM clips ORDER BY id")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut rows = 0usize;
        let mut unreadable = 0usize;
        for id in ids {
            let row: Option<(i64, Vec<u8>, Vec<u8>, Vec<u8>)> = tx
                .query_row("SELECT kind, uid, body, search FROM clips WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()?;
            let Some((k, uid, body, search)) = row else { continue };
            let kind = Kind::from_i64(k);
            let padded_body = self.vault.open(&aad_body(kind, &uid), &body);
            let padded_search = self.vault.open(&aad_search(&uid), &search);
            let (Some(mut padded_body), Some(mut padded_search)) = (padded_body, padded_search) else {
                unreadable += 1;
                continue;
            };
            let Some(plain) = unpad(&padded_body) else {
                padded_body.zeroize();
                padded_search.zeroize();
                unreadable += 1;
                continue;
            };
            let new_uid = random_uid()?;
            let new_body = new_vault.seal(&aad_body(kind, &new_uid), &padded_body);
            let new_search = new_vault.seal(&aad_search(&new_uid), &padded_search);
            let new_hash = new_vault.fingerprint(kind as u8, plain);
            padded_body.zeroize();
            padded_search.zeroize();
            // the preview picture is bound to the clip's identity: re-seal it under the new one
            let thumb: Option<Vec<u8>> =
                tx.query_row("SELECT body FROM thumbs WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            if let Some(sealed) = thumb {
                match self.vault.open(&aad_thumb(&uid), &sealed) {
                    Some(mut p) => {
                        let resealed = new_vault.seal(&aad_thumb(&new_uid), &p);
                        p.zeroize();
                        tx.execute("UPDATE thumbs SET body = ?1 WHERE id = ?2", params![resealed, id])?;
                    }
                    None => {
                        tx.execute("DELETE FROM thumbs WHERE id = ?1", [id])?;
                    }
                }
            }
            tx.execute(
                "UPDATE clips SET uid = ?1, body = ?2, search = ?3, hash = ?4 WHERE id = ?5",
                params![&new_uid[..], new_body, new_search, &new_hash[..], id],
            )?;
            rows += 1;
        }
        if unreadable > 0 {
            drop(tx); // rolls back
            return Err(StoreError::Unreadable(unreadable));
        }
        let check = new_vault.seal(Self::KEY_CHECK_AAD, Self::KEY_CHECK);
        tx.execute(
            "INSERT INTO meta(k, v) VALUES('key_check', ?1) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [&check[..]],
        )?;
        tx.execute(
            "INSERT INTO meta(k, v) VALUES('last_rotation_day', ?1) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [&day(now).to_le_bytes()[..]],
        )?;
        if let Some(blob) = &recovery_blob {
            tx.execute(
                "INSERT INTO meta(k, v) VALUES('recovery_blob', ?1) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [&blob[..]],
            )?;
        }
        tx.execute("DELETE FROM meta WHERE k = 'rotate_requested'", [])?;
        tx.commit()?;
        self.vault = new_vault;
        // The checkpoint empties the WAL (old page images). It reports whether it could finish: if a
        // reader blocked it, the new state is committed but not yet fully on disk, and the caller
        // must keep the old key for now.
        let (busy, _, _): (i64, i64, i64) = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let durable = busy == 0;
        if durable {
            // VACUUM rewrites the file so freed pages that still hold old ciphertext do not survive.
            self.conn.execute_batch("VACUUM")?;
            self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        }
        Ok(RotateReport { rows, durable })
    }

    // ------------------------------------------------------------------ recovery key pair

    /// The public half of the recovery key pair, if one has been set up (by a key export).
    pub fn recovery_public(&self) -> Result<Option<[u8; 32]>, StoreError> {
        Ok(self.meta_get("recovery_pub")?.and_then(|b| b.try_into().ok()))
    }

    /// The master key sealed to the recovery public key, as of the last rotation or export.
    pub fn recovery_blob(&self) -> Result<Option<Vec<u8>>, StoreError> {
        self.meta_get("recovery_blob")
    }

    /// Installs a recovery key pair's public half and the current master key sealed to it.
    pub fn set_recovery(&self, public: &[u8; 32], blob: &[u8]) -> Result<(), StoreError> {
        self.meta_set("recovery_pub", public)?;
        self.meta_set("recovery_blob", blob)
    }

    // ------------------------------------------------------------------ damaged rows

    /// How many clips cannot be opened with the current key. Nonzero blocks key rotation.
    pub fn count_unreadable(&self) -> Result<usize, StoreError> {
        Ok(self.unreadable_ids()?.len())
    }

    fn unreadable_ids(&self) -> Result<Vec<i64>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT id, kind, uid, body, search FROM clips")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, Vec<u8>>(3)?, r.get::<_, Vec<u8>>(4)?))
        })?;
        let mut bad = Vec::new();
        for row in rows {
            let (id, k, uid, body, search) = row?;
            let kind = Kind::from_i64(k);
            let body_ok = self
                .vault
                .open(&aad_body(kind, &uid), &body)
                .is_some_and(|mut p| {
                    let ok = unpad(&p).is_some();
                    p.zeroize();
                    ok
                });
            let search_ok = self.open_record(&uid, &search).is_some();
            if !(body_ok && search_ok) {
                bad.push(id);
            }
        }
        Ok(bad)
    }

    /// Deletes clips that cannot be opened with the current key. Only call this after
    /// [`Store::verify_key`] said the key is right: with a wrong key every row would "fail".
    pub fn repair_unreadable(&mut self) -> Result<usize, StoreError> {
        let ids = self.unreadable_ids()?;
        for id in &ids {
            self.conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        }
        self.recount()?;
        if !ids.is_empty() {
            self.scrub()?;
        }
        Ok(ids.len())
    }

    /// Test and diagnostics only: the raw sealed body as stored.
    #[cfg(test)]
    fn raw_body(&self, id: i64) -> Vec<u8> {
        self.conn.query_row("SELECT body FROM clips WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }

    #[cfg(test)]
    fn raw_size(&self, id: i64) -> i64 {
        self.conn.query_row("SELECT size FROM clips WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }
}

impl HistorySource for Store {
    type Error = StoreError;

    fn snapshot(&mut self) -> Result<Option<Snapshot>, StoreError> {
        let (max_order, max_id) = self.conn.query_row("SELECT MAX(ord), MAX(id) FROM clips", [], |r| {
            Ok((r.get::<_, Option<f64>>(0)?, r.get::<_, Option<i64>>(1)?))
        })?;
        Ok(max_order.zip(max_id).map(|(max_order, max_id)| Snapshot { max_order, max_id }))
    }

    fn page(
        &mut self,
        snap: Snapshot,
        after: Option<(f64, i64)>,
        dir: Dir,
        limit: usize,
    ) -> Result<Vec<Item>, StoreError> {
        let (cmp, order) = match dir {
            Dir::Older => ("<", "DESC"),
            Dir::Newer => (">", "ASC"),
        };
        let sql = match after {
            None => format!(
                "SELECT id, ord FROM clips WHERE ord <= ?1 AND id <= ?2 \
                 ORDER BY ord {order}, id {order} LIMIT ?3"
            ),
            Some(_) => format!(
                "SELECT id, ord FROM clips WHERE ord <= ?1 AND id <= ?2 \
                 AND (ord {cmp} ?4 OR (ord = ?4 AND id {cmp} ?5)) \
                 ORDER BY ord {order}, id {order} LIMIT ?3"
            ),
        };
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let map = |r: &rusqlite::Row<'_>| Ok(Item { id: r.get(0)?, order: r.get(1)? });
        let rows = match after {
            None => stmt.query_map(params![snap.max_order, snap.max_id, limit as i64], map)?,
            Some((o, i)) => {
                stmt.query_map(params![snap.max_order, snap.max_id, limit as i64, o, i], map)?
            }
        };
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn exists(&mut self, id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM clips WHERE id = ?1)", [id], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{GB, MB};
    use crate::session::{Moved, Session};
    use crate::vault::MASTER_LEN;

    const DAY: i64 = 86_400;

    fn vault() -> Vault {
        Vault::new(&[7; MASTER_LEN])
    }

    fn mem() -> Store {
        Store::open_in_memory(vault()).unwrap()
    }

    fn limits(clips: u64, mb: u64, age: Option<u32>) -> Limits {
        Limits { max_clips: clips, max_bytes: mb * MB, max_age_days: age, ..Limits::default() }
    }

    fn store_with(texts: &[&str]) -> (Store, Vec<i64>) {
        let mut s = mem();
        let ids = texts.iter().map(|t| s.add_text(t, 1_000).unwrap()).collect();
        (s, ids)
    }

    fn temp_db(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pasteroll-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("h.db");
        (dir, db)
    }

    fn png(w: u32, h: u32, shade: u8) -> Vec<u8> {
        let img = crate::image::Rgba { width: w, height: h, pixels: vec![shade; (w * h * 4) as usize] };
        crate::image::encode_png(&img).unwrap()
    }

    // ---------------------------------------------------------------- basics

    #[test]
    fn newest_added_is_first() {
        let (mut s, ids) = store_with(&["one", "two", "three"]);
        let snap = s.snapshot().unwrap().unwrap();
        let page = s.page(snap, None, Dir::Older, 10).unwrap();
        assert_eq!(page.iter().map(|i| i.id).collect::<Vec<_>>(), vec![ids[2], ids[1], ids[0]]);
    }

    #[test]
    fn identical_content_moves_to_top_instead_of_duplicating() {
        let (mut s, ids) = store_with(&["a", "b", "c"]);
        assert_eq!(s.add_text("a", 2_000).unwrap(), ids[0]);
        assert_eq!(s.count().unwrap(), 3);
        let snap = s.snapshot().unwrap().unwrap();
        assert_eq!(s.page(snap, None, Dir::Older, 10).unwrap()[0].id, ids[0]);
        assert_eq!(s.stats().clips, 3, "counters stay in step on a duplicate");
    }

    #[test]
    fn a_duplicate_replaces_the_stored_body() {
        let (mut s, ids) = store_with(&["same"]);
        let before = s.raw_body(ids[0]);
        s.add_text("same", 2_000).unwrap();
        assert_ne!(s.raw_body(ids[0]), before, "body should be re-sealed on a duplicate");
        assert_eq!(s.text(ids[0]).unwrap().as_deref(), Some("same"));
    }

    #[test]
    fn same_text_as_different_kind_is_not_a_duplicate() {
        let mut s = mem();
        let t = s.add_text("x", 1).unwrap();
        let f = s.add_files(&["x".to_string()], 1).unwrap();
        assert_ne!(t, f);
    }

    // ---------------------------------------------------------------- encryption and padding

    #[test]
    fn stored_body_is_encrypted() {
        let (s, ids) = store_with(&["correct horse battery staple"]);
        let raw = s.raw_body(ids[0]);
        assert!(!raw.windows(7).any(|w| w == b"battery"), "plaintext leaked into the database");
        assert_eq!(s.text(ids[0]).unwrap().as_deref(), Some("correct horse battery staple"));
    }

    #[test]
    fn clips_of_different_lengths_in_one_bucket_are_indistinguishable() {
        let (s, ids) = store_with(&["hi", "a longer sentence than the first one, but still short"]);
        assert_eq!(s.raw_size(ids[0]), s.raw_size(ids[1]));
        assert_eq!(s.raw_body(ids[0]).len(), s.raw_body(ids[1]).len());
    }

    #[test]
    fn a_big_clip_looks_big_and_a_small_one_looks_small() {
        let big = "x".repeat(50_000);
        let (s, ids) = store_with(&["tiny", &big]);
        assert!(s.raw_size(ids[1]) > 40 * s.raw_size(ids[0]));
    }

    #[test]
    fn the_search_record_is_sealed_and_padded_too() {
        let (s, ids) = store_with(&["findable words"]);
        let raw: Vec<u8> = s.conn.query_row("SELECT search FROM clips WHERE id = ?1", [ids[0]], |r| r.get(0)).unwrap();
        assert!(!raw.windows(8).any(|w| w == b"findable"));
        assert_eq!(s.info(ids[0]).unwrap().unwrap().search, "findable words");
    }

    #[test]
    fn a_body_cannot_be_moved_to_another_row() {
        let (s, ids) = store_with(&["first secret", "second secret"]);
        let b0 = s.raw_body(ids[0]);
        s.conn.execute("UPDATE clips SET body = ?1 WHERE id = ?2", params![b0, ids[1]]).unwrap();
        assert_eq!(s.text(ids[1]).unwrap(), None, "swapped body must not authenticate");
        assert_eq!(s.text(ids[0]).unwrap().as_deref(), Some("first secret"));
    }

    #[test]
    fn a_search_record_cannot_be_moved_to_another_row() {
        let (s, ids) = store_with(&["alpha", "beta"]);
        let sr: Vec<u8> = s.conn.query_row("SELECT search FROM clips WHERE id = ?1", [ids[0]], |r| r.get(0)).unwrap();
        s.conn.execute("UPDATE clips SET search = ?1 WHERE id = ?2", params![sr, ids[1]]).unwrap();
        assert!(s.info(ids[1]).unwrap().is_none());
    }

    #[test]
    fn a_body_cannot_change_kind() {
        let mut s = mem();
        let id = s.add_text("png-looking", 1).unwrap();
        s.conn.execute("UPDATE clips SET kind = 1 WHERE id = ?1", [id]).unwrap();
        assert_eq!(s.image_png(id).unwrap(), None);
    }

    #[test]
    fn a_different_key_cannot_read_the_clip() {
        let (dir, path) = temp_db("key");
        let id = {
            let mut s = Store::open(&path, Vault::new(&[1; MASTER_LEN])).unwrap();
            s.add_text("secret", 5).unwrap()
        };
        let other = Store::open(&path, Vault::new(&[2; MASTER_LEN])).unwrap();
        assert_eq!(other.text(id).unwrap(), None);
        assert!(other.info(id).unwrap().is_none());
        drop(other);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_foreign_database_is_refused() {
        let (dir, path) = temp_db("foreign");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE notes(x); PRAGMA application_id = 42;").unwrap();
        }
        match Store::open(&path, vault()) {
            Err(StoreError::Foreign(42)) => {}
            other => panic!("expected Foreign(42), got {:?}", other.err()),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_schema_is_refused_not_modified() {
        let (dir, path) = temp_db("old");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(&format!(
                "CREATE TABLE clips(id); PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 3;"
            ))
            .unwrap();
        }
        assert!(matches!(Store::open(&path, vault()), Err(StoreError::Schema(3))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------- info / search rows

    #[test]
    fn info_describes_each_kind_without_the_body() {
        let mut s = mem();
        let t = s.add_text("héllo wörld", 1).unwrap();
        let i = s.add_image_png(&png(30, 20, 9), 2).unwrap();
        let f = s.add_files(&["C:\\a\\x.pdf".into(), "C:\\a\\y.txt".into()], 3).unwrap();
        let ti = s.info(t).unwrap().unwrap();
        assert_eq!((ti.kind, ti.total_chars, ti.pinned), (Kind::Text, 11, false));
        let ii = s.info(i).unwrap().unwrap();
        assert_eq!((ii.kind, ii.width, ii.height), (Kind::Image, 30, 20));
        assert!(ii.search.contains("30x20"));
        let fi = s.info(f).unwrap().unwrap();
        assert_eq!((fi.kind, fi.files), (Kind::Files, 2));
        assert!(fi.search.contains("x.pdf") && fi.search.contains("y.txt"));
    }

    #[test]
    fn long_text_keeps_only_the_search_prefix_but_the_true_length() {
        let (s, ids) = store_with(&[&"w".repeat(9_000)]);
        let i = s.info(ids[0]).unwrap().unwrap();
        assert_eq!(i.total_chars, 9_000);
        assert_eq!(i.search.chars().count(), crate::record::SEARCH_CHARS);
    }

    #[test]
    fn search_rows_come_back_newest_first_with_pins() {
        let (mut s, ids) = store_with(&["one", "two", "three"]);
        s.set_pinned(ids[0], true).unwrap();
        let rows = s.search_rows().unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ids[2], ids[1], ids[0]]);
        assert_eq!(rows.iter().map(|r| r.pinned).collect::<Vec<_>>(), vec![false, false, true]);
        assert_eq!(rows[0].text, "three");
    }

    // ---------------------------------------------------------------- limits

    #[test]
    fn oversized_clips_are_refused_and_the_limit_is_configurable() {
        let mut s = mem();
        s.set_limits(Limits { max_clip_bytes: MB, ..Limits::default() }).unwrap();
        let big = vec![b'a'; (MB as usize) + 1];
        assert!(matches!(s.add_image_png(&big, 1), Err(StoreError::TooLarge)));
        s.set_limits(Limits { max_clip_bytes: 2 * MB, ..Limits::default() }).unwrap();
        assert!(s.add_image_png(&big, 1).is_ok());
    }

    #[test]
    fn invalid_limits_are_refused() {
        let mut s = mem();
        assert!(matches!(s.set_limits(Limits { max_clips: 0, ..Limits::default() }), Err(StoreError::Limits(_))));
        assert_eq!(s.limits(), Limits::default());
    }

    #[test]
    fn count_limit_trims_in_batches_to_the_limit() {
        let mut s = mem();
        s.set_limits(limits(100, 500, None)).unwrap();
        let ids: Vec<i64> = (0..200).map(|i| s.add_text(&format!("c{i}"), 10).unwrap()).collect();
        assert!(s.count().unwrap() <= 100 + 100 / 20 + 1, "count {}", s.count().unwrap());
        assert!(!s.exists(ids[0]).unwrap(), "oldest clip must be gone");
        assert!(s.exists(ids[199]).unwrap(), "newest clip must be kept");
    }

    #[test]
    fn maintain_enforces_the_exact_count() {
        let mut s = mem();
        s.set_limits(limits(50, 500, None)).unwrap();
        for i in 0..53 {
            s.add_text(&format!("c{i}"), 10).unwrap();
        }
        s.maintain(11).unwrap();
        assert_eq!(s.count().unwrap(), 50);
    }

    #[test]
    fn the_size_budget_removes_the_oldest_unpinned_clips() {
        let mut s = mem();
        s.set_limits(Limits { max_bytes: 2 * MB, max_clip_bytes: MB, ..limits(1000, 2, None) }).unwrap();
        // five ~600 KB images: ~3 MB stored, over the 2 MB budget
        let ids: Vec<i64> = (0..5)
            .map(|i| {
                let big: Vec<u8> = (0..600_000u32).map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8 ^ i as u8).collect();
                s.add_image_png(&big, 10).unwrap()
            })
            .collect();
        s.maintain(11).unwrap();
        let st = s.stats();
        assert!(st.bytes <= 2 * MB as i64, "bytes {}", st.bytes);
        assert!(!s.exists(ids[0]).unwrap(), "oldest must go first");
        assert!(s.exists(ids[4]).unwrap(), "newest must stay");
    }

    #[test]
    fn age_is_counted_in_days_from_the_last_use() {
        let mut s = mem();
        s.set_limits(limits(1000, 500, Some(30))).unwrap();
        let old = s.add_text("old", 100 * DAY).unwrap();
        let fresh = s.add_text("fresh", 125 * DAY).unwrap();
        assert_eq!(s.maintain(129 * DAY).unwrap(), 0, "29 days old: kept");
        assert_eq!(s.maintain(131 * DAY).unwrap(), 1, "31 days old: expired");
        assert!(!s.exists(old).unwrap());
        assert!(s.exists(fresh).unwrap());
    }

    #[test]
    fn age_never_means_never() {
        let mut s = mem();
        s.set_limits(limits(1000, 500, None)).unwrap();
        let id = s.add_text("ancient", 0).unwrap();
        assert_eq!(s.maintain(50_000 * DAY).unwrap(), 0);
        assert!(s.exists(id).unwrap());
    }

    #[test]
    fn a_recopied_or_pasted_clip_does_not_expire() {
        let mut s = mem();
        s.set_limits(limits(1000, 500, Some(30))).unwrap();
        let a = s.add_text("copied again", 100 * DAY).unwrap();
        s.add_text("copied again", 125 * DAY).unwrap();
        let b = s.add_text("pasted", 100 * DAY).unwrap();
        s.mark_pasted(b, 125 * DAY, true).unwrap();
        assert_eq!(s.maintain(131 * DAY).unwrap(), 0);
        assert!(s.exists(a).unwrap() && s.exists(b).unwrap());
    }

    #[test]
    fn plan_prune_matches_what_maintain_then_does() {
        let mut s = mem();
        s.set_limits(limits(1000, 500, Some(30))).unwrap();
        for i in 0..10 {
            s.add_text(&format!("old {i}"), 10 * DAY).unwrap();
        }
        for i in 0..40 {
            s.add_text(&format!("new {i}"), 100 * DAY).unwrap();
        }
        let lower = limits(25, 500, Some(30));
        let plan = s.plan_prune(&lower, 110 * DAY).unwrap();
        assert_eq!(plan.by_age, 10);
        assert_eq!(plan.clips, 10 + 15, "10 expired + 15 over the new count");
        assert!(plan.bytes > 0);
        // nothing was deleted by planning
        assert_eq!(s.count().unwrap(), 50);
        // applying the same limits removes exactly the planned amount
        s.set_limits(lower).unwrap();
        assert_eq!(s.maintain(110 * DAY).unwrap(), plan.clips);
        assert_eq!(s.count().unwrap(), 25);
    }

    #[test]
    fn plan_prune_with_current_limits_is_empty() {
        let (s, _) = store_with(&["a", "b"]);
        assert_eq!(s.plan_prune(&Limits::default(), 1_000).unwrap(), PrunePlan::default());
    }

    #[test]
    fn counters_survive_reopening() {
        let (dir, path) = temp_db("counters");
        {
            let mut s = Store::open(&path, vault()).unwrap();
            let a = s.add_text("a", 1).unwrap();
            s.add_text("b", 1).unwrap();
            s.set_pinned(a, true).unwrap();
        }
        let s = Store::open(&path, vault()).unwrap();
        let st = s.stats();
        assert_eq!((st.clips, st.pins), (1, 1));
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------- pins

    #[test]
    fn pinned_clips_survive_every_limit() {
        let mut s = mem();
        s.set_limits(limits(5, 500, Some(30))).unwrap();
        let pinned = s.add_text("keep me", 10 * DAY).unwrap();
        s.set_pinned(pinned, true).unwrap();
        for i in 0..40 {
            s.add_text(&format!("filler {i}"), 10 * DAY).unwrap();
        }
        s.maintain(200 * DAY).unwrap(); // everything unpinned is ancient and over the count
        assert!(s.exists(pinned).unwrap(), "a pinned clip must never be removed automatically");
        assert_eq!(s.count().unwrap(), 1);
    }

    #[test]
    fn pins_do_not_count_toward_the_clip_or_size_limits() {
        let mut s = mem();
        s.set_limits(limits(3, 500, None)).unwrap();
        let ids: Vec<i64> = (0..3).map(|i| s.add_text(&format!("p{i}"), 1).unwrap()).collect();
        for id in &ids {
            s.set_pinned(*id, true).unwrap();
        }
        for i in 0..3 {
            s.add_text(&format!("u{i}"), 1).unwrap();
        }
        s.maintain(2).unwrap();
        assert_eq!(s.count().unwrap(), 6, "3 pins + 3 unpinned within the limit of 3");
        let st = s.stats();
        assert_eq!((st.clips, st.pins), (3, 3));
    }

    #[test]
    fn pin_toggles_and_reports() {
        let (mut s, ids) = store_with(&["a"]);
        assert_eq!(s.set_pinned(ids[0], true).unwrap(), PinResult::Pinned);
        assert!(s.info(ids[0]).unwrap().unwrap().pinned);
        assert_eq!(s.set_pinned(ids[0], true).unwrap(), PinResult::Pinned, "idempotent");
        assert_eq!(s.set_pinned(ids[0], false).unwrap(), PinResult::Unpinned);
        assert!(!s.info(ids[0]).unwrap().unwrap().pinned);
        assert_eq!(s.set_pinned(9999, true).unwrap(), PinResult::Missing);
    }

    #[test]
    fn the_pin_cap_is_enforced() {
        let mut s = mem();
        s.set_limits(Limits { max_pins: 2, ..Limits::default() }).unwrap();
        let ids: Vec<i64> = (0..3).map(|i| s.add_text(&format!("c{i}"), 1).unwrap()).collect();
        assert_eq!(s.set_pinned(ids[0], true).unwrap(), PinResult::Pinned);
        assert_eq!(s.set_pinned(ids[1], true).unwrap(), PinResult::Pinned);
        assert_eq!(s.set_pinned(ids[2], true).unwrap(), PinResult::LimitReached);
        assert_eq!(s.stats().pins, 2);
    }

    #[test]
    fn unpinning_can_put_a_clip_back_under_the_limits() {
        let mut s = mem();
        s.set_limits(limits(2, 500, None)).unwrap();
        let a = s.add_text("oldest", 1).unwrap();
        s.set_pinned(a, true).unwrap();
        for i in 0..2 {
            s.add_text(&format!("u{i}"), 1).unwrap();
        }
        s.set_pinned(a, false).unwrap();
        s.maintain(2).unwrap();
        assert_eq!(s.count().unwrap(), 2);
        assert!(!s.exists(a).unwrap(), "the unpinned oldest clip is now over the limit and goes first");
    }

    #[test]
    fn deleting_and_clearing_keep_the_counters_right() {
        let mut s = mem();
        let a = s.add_text("a", 1).unwrap();
        let b = s.add_text("b", 1).unwrap();
        s.add_text("c", 1).unwrap();
        s.set_pinned(a, true).unwrap();
        assert!(s.delete(b).unwrap());
        assert_eq!((s.stats().clips, s.stats().pins), (1, 1));
        assert_eq!(s.clear(true).unwrap(), 1);
        assert_eq!((s.count().unwrap(), s.stats().clips, s.stats().pins), (1, 0, 1));
        assert_eq!(s.clear(false).unwrap(), 1);
        assert_eq!(s.count().unwrap(), 0);
        assert_eq!(s.stats(), Stats { clips: 0, bytes: 0, pins: 0 });
    }

    // ---------------------------------------------------------------- scrubbing and concurrency

    #[test]
    fn deleting_scrubs_the_wal_and_survives_reopen() {
        let (dir, path) = temp_db("scrub");
        let (keep, gone) = {
            let mut s = Store::open(&path, vault()).unwrap();
            let keep = s.add_text("keep", 5).unwrap();
            let gone = s.add_text("UNIQUE-DELETED-CLIP-TEXT-0123456789", 5).unwrap();
            assert!(s.delete(gone).unwrap());
            (keep, gone)
        };
        let mut s = Store::open(&path, vault()).unwrap();
        assert!(s.exists(keep).unwrap());
        assert!(!s.exists(gone).unwrap());
        let wal = path.with_extension("db-wal");
        if wal.exists() {
            assert!(std::fs::metadata(&wal).unwrap().len() < 200_000);
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_database_uses_incremental_vacuum() {
        let (dir, path) = temp_db("vac");
        let s = Store::open(&path, vault()).unwrap();
        let mode: i64 = s.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 2, "INCREMENTAL");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_connections_can_write_without_losing_captures() {
        let (dir, path) = temp_db("two");
        let mut a = Store::open(&path, vault()).unwrap();
        let mut b = Store::open(&path, vault()).unwrap();
        for i in 0..50 {
            a.add_text(&format!("a{i}"), 10).unwrap();
            b.add_text(&format!("b{i}"), 10).unwrap();
            if i % 7 == 0 {
                let id = a.add_text("shared", 11).unwrap();
                b.mark_pasted(id, 12, false).unwrap();
            }
        }
        assert_eq!(a.count().unwrap(), 101);
        drop((a, b));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------- preview and session

    #[test]
    fn preview_truncates_and_reports_totals() {
        let long = "x".repeat(10_000);
        let (s, ids) = store_with(&[&long]);
        let p = s.preview(ids[0], 4096).unwrap().unwrap();
        assert_eq!(p.text.chars().count(), 4096);
        assert!(p.truncated);
        assert_eq!(p.total_chars, 10_000);
        let (s2, ids2) = store_with(&["short"]);
        let p2 = s2.preview(ids2[0], 4096).unwrap().unwrap();
        assert_eq!((p2.text.as_str(), p2.truncated, p2.total_chars), ("short", false, 5));
    }

    #[test]
    fn preview_truncation_counts_characters_not_bytes() {
        let (s, ids) = store_with(&["héllo wörld 😀 ok"]);
        assert_eq!(s.preview(ids[0], 8).unwrap().unwrap().text, "héllo wö");
    }

    #[test]
    fn full_text_is_not_truncated() {
        let long = "y".repeat(50_000);
        let (s, ids) = store_with(&[&long]);
        assert_eq!(s.text(ids[0]).unwrap().unwrap().len(), 50_000);
    }

    #[test]
    fn image_roundtrip_and_kind() {
        let mut s = mem();
        let p = png(4, 4, 100);
        let id = s.add_image_png(&p, 5).unwrap();
        assert_eq!(s.kind(id).unwrap(), Some(Kind::Image));
        assert_eq!(s.image_png(id).unwrap().unwrap(), p);
        assert_eq!(s.text(id).unwrap(), None);
    }

    #[test]
    fn files_roundtrip() {
        let mut s = mem();
        let id = s.add_files(&["C:\\a.txt".into(), "C:\\b c.txt".into()], 5).unwrap();
        assert_eq!(s.files(id).unwrap().unwrap(), vec!["C:\\a.txt", "C:\\b c.txt"]);
        assert_eq!(s.preview(id, 100).unwrap().unwrap().text, "C:\\a.txt\nC:\\b c.txt");
    }

    #[test]
    fn mark_pasted_can_bump_to_top() {
        let (mut s, ids) = store_with(&["a", "b", "c"]);
        s.mark_pasted(ids[0], 9, true).unwrap();
        let snap = s.snapshot().unwrap().unwrap();
        assert_eq!(s.page(snap, None, Dir::Older, 1).unwrap()[0].id, ids[0]);
    }

    #[test]
    fn delete_removes_the_clip() {
        let (mut s, ids) = store_with(&["a", "b"]);
        assert!(s.delete(ids[0]).unwrap());
        assert!(!s.exists(ids[0]).unwrap());
        assert!(!s.delete(ids[0]).unwrap());
    }

    #[test]
    fn session_over_sqlite_cycles_and_wraps() {
        let (s, ids) = store_with(&["a", "b", "c"]);
        let mut sess = Session::open(s).unwrap().unwrap();
        assert_eq!(sess.current().id, ids[2]);
        assert_eq!(sess.older().unwrap(), Moved::Stepped);
        assert_eq!(sess.current().id, ids[1]);
        assert_eq!(sess.older().unwrap(), Moved::Stepped);
        assert_eq!(sess.older().unwrap(), Moved::Wrapped);
        assert_eq!(sess.current().id, ids[2]);
    }

    #[test]
    fn session_over_sqlite_ignores_clips_copied_mid_session() {
        let (s, ids) = store_with(&["a", "b"]);
        let mut sess = Session::open(s).unwrap().unwrap();
        let fresh = sess.source_mut().add_text("fresh", 3).unwrap();
        sess.older().unwrap();
        sess.older().unwrap(); // wrapped
        assert_eq!(sess.current().id, ids[1]);
        assert_ne!(sess.current().id, fresh);
    }

    #[test]
    fn session_over_sqlite_skips_a_clip_deleted_mid_session() {
        let (s, ids) = store_with(&["a", "b", "c"]);
        let mut sess = Session::open(s).unwrap().unwrap();
        sess.source_mut().delete(ids[1]).unwrap();
        assert_eq!(sess.older().unwrap(), Moved::Stepped);
        assert_eq!(sess.current().id, ids[0]);
    }

    #[test]
    fn large_history_pages_through_in_batches() {
        let mut s = mem();
        s.set_limits(limits(5_000, 500, None)).unwrap();
        for i in 0..1_000 {
            s.add_text(&format!("clip {i}"), i).unwrap();
        }
        let mut sess = Session::open(s).unwrap().unwrap();
        let mut seen = std::collections::HashSet::new();
        seen.insert(sess.current().id);
        for _ in 0..999 {
            assert_eq!(sess.older().unwrap(), Moved::Stepped);
            assert!(seen.insert(sess.current().id), "revisited a clip");
            assert!(sess.window().1 <= crate::session::CACHE_SIZE);
        }
        assert_eq!(seen.len(), 1_000);
        assert_eq!(sess.older().unwrap(), Moved::Wrapped);
    }

    // ---------------------------------------------------------------- key check and rotation

    fn other_vault() -> Vault {
        Vault::new(&[9; MASTER_LEN])
    }

    #[test]
    fn a_fresh_history_gets_a_key_check_and_a_wrong_key_is_caught() {
        let (dir, path) = temp_db("kc");
        {
            let mut s = Store::open(&path, vault()).unwrap();
            assert_eq!(s.verify_key().unwrap(), KeyStatus::Initialized);
            s.add_text("x", 1).unwrap();
            assert_eq!(s.verify_key().unwrap(), KeyStatus::Ok);
        }
        let mut wrong = Store::open(&path, other_vault()).unwrap();
        assert_eq!(wrong.verify_key().unwrap(), KeyStatus::Wrong);
        // and a wrong key never overwrote the check
        drop(wrong);
        let mut right = Store::open(&path, vault()).unwrap();
        assert_eq!(right.verify_key().unwrap(), KeyStatus::Ok);
        drop(right);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_key_on_a_history_without_a_check_is_not_blessed() {
        let (dir, path) = temp_db("kc2");
        {
            let mut s = Store::open(&path, vault()).unwrap();
            s.add_text("data", 1).unwrap();
        }
        let mut wrong = Store::open(&path, other_vault()).unwrap();
        assert_eq!(wrong.verify_key().unwrap(), KeyStatus::Wrong);
        drop(wrong);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn populated() -> (Store, Vec<i64>) {
        let mut s = mem();
        let mut ids = vec![
            s.add_text("alpha secret", 1_700_000_000).unwrap(),
            s.add_text(&"long text ".repeat(500), 1_700_000_000).unwrap(),
            s.add_image_png(&png(8, 8, 3), 1_700_000_000).unwrap(),
            s.add_files(&["C:\\a\\b.txt".into()], 1_700_000_000).unwrap(),
        ];
        s.set_pinned(ids[0], true).unwrap();
        ids.push(s.add_text("omega", 1_700_000_001).unwrap());
        (s, ids)
    }

    #[test]
    fn rotation_moves_every_clip_to_the_new_key() {
        let (mut s, ids) = populated();
        let before = s.stats();
        let old_bodies: Vec<Vec<u8>> = ids.iter().map(|&i| s.raw_body(i)).collect();
        let report = s.rotate(other_vault(), 1_700_100_000).unwrap();
        assert_eq!(report.rows, ids.len());
        // same content, new ciphertext
        assert_eq!(s.text(ids[0]).unwrap().as_deref(), Some("alpha secret"));
        assert_eq!(s.text(ids[4]).unwrap().as_deref(), Some("omega"));
        assert_eq!(s.image_png(ids[2]).unwrap().unwrap(), png(8, 8, 3));
        assert_eq!(s.files(ids[3]).unwrap().unwrap(), vec!["C:\\a\\b.txt"]);
        for (i, &id) in ids.iter().enumerate() {
            assert_ne!(s.raw_body(id), old_bodies[i], "row {id} still has its old ciphertext");
        }
        assert_eq!(s.stats(), before, "counters and pins unchanged");
        assert!(s.info(ids[0]).unwrap().unwrap().pinned);
        assert_eq!(s.verify_key().unwrap(), KeyStatus::Ok);
    }

    #[test]
    fn the_old_key_cannot_read_anything_after_rotation() {
        let (dir, path) = temp_db("rot");
        let mut s = Store::open(&path, vault()).unwrap();
        let id = s.add_text("only the new key may read this", 1_700_000_000).unwrap();
        s.rotate(other_vault(), 1_700_000_001).unwrap();
        drop(s);
        let mut old = Store::open(&path, vault()).unwrap();
        assert_eq!(old.text(id).unwrap(), None);
        assert_eq!(old.verify_key().unwrap(), KeyStatus::Wrong);
        drop(old);
        let new = Store::open(&path, other_vault()).unwrap();
        assert_eq!(new.text(id).unwrap().as_deref(), Some("only the new key may read this"));
        drop(new);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deduplication_still_works_after_rotation() {
        let (mut s, ids) = populated();
        s.rotate(other_vault(), 1_700_100_000).unwrap();
        let again = s.add_text("omega", 1_700_100_001).unwrap();
        assert_eq!(again, ids[4], "the same text must still be recognised as a duplicate");
        assert_eq!(s.count().unwrap(), ids.len() as i64);
    }

    #[test]
    fn rotation_removes_deleted_ciphertext_from_the_disk() {
        let (dir, path) = temp_db("scrubrot");
        let mut s = Store::open(&path, vault()).unwrap();
        let keep = s.add_text("kept", 1_700_000_000).unwrap();
        let gone = s.add_text(&"deleted clip body ".repeat(200), 1_700_000_000).unwrap();
        let ciphertext = s.raw_body(gone);
        let needle = ciphertext[40..72].to_vec();
        s.delete(gone).unwrap();
        s.rotate(other_vault(), 1_700_000_001).unwrap();
        drop(s);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle.as_slice()),
                "old ciphertext of a deleted clip survived on disk"
            );
        }
        let s = Store::open(&path, other_vault()).unwrap();
        assert_eq!(s.text(keep).unwrap().as_deref(), Some("kept"));
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_refuses_and_changes_nothing_if_a_row_is_unreadable() {
        let (mut s, ids) = populated();
        // corrupt one body
        s.conn.execute("UPDATE clips SET body = zeroblob(80) WHERE id = ?1", [ids[1]]).unwrap();
        let before: Vec<Vec<u8>> = ids.iter().map(|&i| s.raw_body(i)).collect();
        match s.rotate(other_vault(), 1_700_100_000) {
            Err(StoreError::Unreadable(1)) => {}
            other => panic!("expected Unreadable(1), got {:?}", other.err()),
        }
        for (i, &id) in ids.iter().enumerate() {
            assert_eq!(s.raw_body(id), before[i], "row {id} changed despite the refused rotation");
        }
        assert_eq!(s.text(ids[4]).unwrap().as_deref(), Some("omega"), "still readable with the old key");
    }

    #[test]
    fn rotation_of_an_empty_history_works() {
        let mut s = mem();
        assert_eq!(s.rotate(other_vault(), 1).unwrap().rows, 0);
        assert_eq!(s.verify_key().unwrap(), KeyStatus::Ok);
    }

    #[test]
    fn rotation_state_starts_its_clock_then_comes_due() {
        let mut s = mem();
        let t0 = 1_700_000_000;
        assert_eq!(s.rotation_state(t0, 30).unwrap(), RotationState::NotDue, "baseline set");
        assert_eq!(s.rotation_state(t0 + 29 * DAY, 30).unwrap(), RotationState::NotDue);
        assert_eq!(s.rotation_state(t0 + 31 * DAY, 30).unwrap(), RotationState::DueByAge);
        s.rotate(other_vault(), t0 + 31 * DAY).unwrap();
        assert_eq!(s.rotation_state(t0 + 32 * DAY, 30).unwrap(), RotationState::NotDue);
    }

    #[test]
    fn a_requested_rotation_is_due_until_it_happens() {
        let mut s = mem();
        s.request_rotation().unwrap();
        assert_eq!(s.rotation_state(1_700_000_000, 30).unwrap(), RotationState::Requested);
        s.rotate(other_vault(), 1_700_000_000).unwrap();
        assert_eq!(s.rotation_state(1_700_000_001, 30).unwrap(), RotationState::NotDue);
    }

    #[test]
    fn a_connection_with_the_old_key_cannot_write_after_a_rotation() {
        let (dir, path) = temp_db("stale");
        let mut a = Store::open(&path, vault()).unwrap();
        a.add_text("before", 1_700_000_000).unwrap();
        a.verify_key().unwrap();
        let mut b = Store::open(&path, vault()).unwrap(); // opened before the rotation, old key
        a.rotate(other_vault(), 1_700_000_100).unwrap();
        match b.add_text("written with the stale key", 1_700_000_200) {
            Err(StoreError::StaleKey) => {}
            other => panic!("expected StaleKey, got {:?}", other.err()),
        }
        assert_eq!(a.count().unwrap(), 1, "no unreadable row may have been written");
        drop((a, b));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rotation_reports_whether_it_is_durable() {
        let (mut s, _) = populated();
        assert!(s.rotate(other_vault(), 1_700_100_000).unwrap().durable);
    }

    #[test]
    fn rotation_stores_the_recovery_blob_in_the_same_transaction() {
        let (mut s, _) = populated();
        s.rotate_with_recovery(other_vault(), 1_700_100_000, Some(vec![7; 90])).unwrap();
        assert_eq!(s.recovery_blob().unwrap(), Some(vec![7; 90]));
    }

    #[test]
    fn recovery_material_can_be_set_and_read_back() {
        let mut s = mem();
        assert_eq!(s.recovery_public().unwrap(), None);
        s.set_recovery(&[3; 32], &[9; 80]).unwrap();
        assert_eq!(s.recovery_public().unwrap(), Some([3; 32]));
        assert_eq!(s.recovery_blob().unwrap(), Some(vec![9; 80]));
        let _ = &mut s;
    }

    #[test]
    fn unreadable_rows_are_counted_and_can_be_repaired() {
        let (mut s, ids) = populated();
        assert_eq!(s.count_unreadable().unwrap(), 0);
        s.conn.execute("UPDATE clips SET body = zeroblob(80) WHERE id = ?1", [ids[1]]).unwrap();
        s.conn.execute("UPDATE clips SET search = zeroblob(80) WHERE id = ?1", [ids[2]]).unwrap();
        assert_eq!(s.count_unreadable().unwrap(), 2);
        // rotation refuses while rows are damaged...
        assert!(matches!(s.rotate(other_vault(), 1_700_100_000), Err(StoreError::Unreadable(2))));
        // ...and works after the damaged rows are removed
        assert_eq!(s.repair_unreadable().unwrap(), 2);
        assert_eq!(s.count_unreadable().unwrap(), 0);
        assert_eq!(s.count().unwrap(), 3);
        s.rotate(other_vault(), 1_700_100_000).unwrap();
        assert_eq!(s.text(ids[4]).unwrap().as_deref(), Some("omega"));
    }

    #[test]
    fn a_rotation_date_in_the_future_counts_as_due() {
        let mut s = mem();
        let t = 1_700_000_000;
        s.rotation_state(t, 30).unwrap(); // baseline = today
        // the clock was rolled back a year
        assert_eq!(s.rotation_state(t - 365 * DAY, 30).unwrap(), RotationState::DueByAge);
    }

    #[test]
    fn a_hundred_thousand_clips_do_not_slow_adds_down() {
        // Adds must not pay for history size: trimming is batched and counters are incremental.
        let mut s = mem();
        s.set_limits(limits(1_000_000, 100 * GB / MB, None)).unwrap();
        let start = std::time::Instant::now();
        for i in 0..5_000 {
            s.add_text(&format!("clip number {i}"), 1).unwrap();
        }
        let first = start.elapsed();
        let start = std::time::Instant::now();
        for i in 5_000..10_000 {
            s.add_text(&format!("clip number {i}"), 1).unwrap();
        }
        let second = start.elapsed();
        assert!(second < first * 4 + std::time::Duration::from_millis(500), "{first:?} then {second:?}");
    }
    // ---------------------------------------------------------------- preview pictures

    fn picture(w: u32, h: u32, shade: u8) -> Vec<u8> {
        let img = crate::image::Rgba { width: w, height: h, pixels: [shade, 100, 200, 255].repeat((w * h) as usize) };
        crate::image::encode_png(&img).unwrap()
    }

    #[test]
    fn an_image_gets_a_small_preview_when_it_is_stored() {
        let mut s = mem();
        let id = s.add_image_png(&picture(1600, 900, 40), 1_700_000_000).unwrap();
        let thumb = s.thumb(id).unwrap().expect("a preview is made at capture time");
        let small = crate::image::decode_png(&thumb).unwrap();
        assert!(small.width <= crate::image::THUMB_W && small.height <= crate::image::THUMB_H);
        assert!(thumb.len() < s.image_png(id).unwrap().unwrap().len() + 1);
    }

    #[test]
    fn previews_are_sealed_and_bound_to_their_clip() {
        let mut s = mem();
        let a = s.add_image_png(&picture(900, 500, 10), 1_700_000_000).unwrap();
        let b = s.add_image_png(&picture(900, 500, 80), 1_700_000_001).unwrap();
        let raw: Vec<u8> = s.conn.query_row("SELECT body FROM thumbs WHERE id = ?1", [a], |r| r.get(0)).unwrap();
        assert!(!raw.windows(4).any(|w| w == b"\x89PNG"), "the stored preview must not be a plain PNG");
        // moving one clip's preview onto another clip must not open
        s.conn.execute("UPDATE thumbs SET body = ?1 WHERE id = ?2", params![raw, b]).unwrap();
        assert!(s.thumb(b).unwrap().is_none(), "a preview sealed for another clip does not open");
        assert!(s.thumb(a).unwrap().is_some());
    }

    #[test]
    fn a_preview_goes_with_its_clip() {
        let mut s = mem();
        let id = s.add_image_png(&picture(900, 500, 10), 1_700_000_000).unwrap();
        assert!(s.thumb(id).unwrap().is_some());
        s.delete(id).unwrap();
        let n: i64 = s.conn.query_row("SELECT COUNT(*) FROM thumbs", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "no orphaned preview");
    }

    #[test]
    fn previews_survive_a_key_rotation() {
        let mut s = mem();
        let id = s.add_image_png(&picture(900, 500, 10), 1_700_000_000).unwrap();
        let new_vault = Vault::new(&[9; MASTER_LEN]);
        s.rotate_with_recovery(new_vault, 1_700_000_100, None).unwrap();
        let t = s.thumb(id).unwrap().expect("the preview is re-sealed under the new key");
        assert!(crate::image::decode_png(&t).is_some());
    }

    #[test]
    fn a_missing_preview_can_be_set_later() {
        let mut s = mem();
        let id = s.add_image_png(&picture(900, 500, 10), 1_700_000_000).unwrap();
        s.conn.execute("DELETE FROM thumbs", []).unwrap(); // like a clip stored before previews existed
        assert!(s.thumb(id).unwrap().is_none());
        s.set_thumb(id, &picture(50, 30, 1)).unwrap();
        assert!(s.thumb(id).unwrap().is_some());
        s.set_thumb(9999, b"x").unwrap(); // a clip that is gone: nothing happens
    }

    /// `cargo test --release -p pasteroll-core --lib time_preview_paths -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn time_preview_paths() {
        let (w, h) = (3840u32, 2160u32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        let mut seed = 7u32;
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let n = ((seed >> 24) & 7) as u8;
                px.extend_from_slice(&[(x * 255 / w) as u8 / 2 + n, (y * 255 / h) as u8 / 2, 120 + n, 255]);
            }
        }
        let png = crate::image::encode_png(&crate::image::Rgba { width: w, height: h, pixels: px }).unwrap();
        let mut s = mem();
        let t = std::time::Instant::now();
        let id = s.add_image_png(&png, 1_000).unwrap();
        eprintln!("store a {:.1} MB 4K clip incl. preview: {:?}", png.len() as f64 / 1e6, t.elapsed());
        let t = std::time::Instant::now();
        let full = s.image_png(id).unwrap().unwrap();
        let img = crate::image::decode_png(&full).unwrap();
        let _ = crate::image::downscale(&img, crate::image::THUMB_W, crate::image::THUMB_H);
        eprintln!("old way (open, decode, shrink):  {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let thumb = s.thumb(id).unwrap().unwrap();
        let _ = crate::image::decode_png(&thumb).unwrap();
        eprintln!("new way (open the preview):      {:?}  ({} KB)", t.elapsed(), thumb.len() / 1024);
    }
}
