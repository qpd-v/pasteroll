//! Background workers, so nothing slow runs on the UI thread (and the input hooks' heartbeat stays
//! alive):
//!
//! - ingest: after a clipboard change, reads the clipboard and stores the clip;
//! - preview: decodes and downscales the image candidate, and decodes the full picture for the
//!   expanded viewer.
//!
//! Each worker has its own database connection (WAL mode: readers never block the writer). Results
//! come back over a channel and the platform's event loop is woken with [`Waker`].

use crate::platform::{Captured, ClipboardSource, Waker};
use pasteroll_core::image::{self, Rgba};
use pasteroll_core::Store;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Creates a database connection (same file, same key). Called on worker threads.
pub type StoreFactory = Arc<dyn Fn() -> Result<Store, String> + Send + Sync>;
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

use image::{THUMB_H, THUMB_W};

/// What the ingest worker is allowed to record.
#[derive(Clone, Default)]
pub struct IngestPolicy {
    /// Copies made in these programs (lower-case exe names) are never recorded.
    pub exclude_apps: HashSet<String>,
    /// While set, nothing new is recorded.
    pub paused: Arc<AtomicBool>,
}

pub struct PreviewResult {
    pub generation: u64,
    pub id: i64,
    pub image: Option<Rgba>,
    pub prefetch: bool,
    /// The full-size picture (for the expanded viewer), not the preview.
    pub full: bool,
}

struct PreviewJob {
    id: i64,
    generation: u64,
    /// Warming the cache for a neighbour of the candidate: lower priority, nothing is shown for it.
    prefetch: bool,
    /// Decode the whole picture, not the preview.
    full: bool,
}

const COPY_SOURCE: &str = if cfg!(target_os = "windows") { "pasteroll.exe" } else { "pasteroll" };

enum IngestJob {
    ClipboardChanged,
    CopiedText { text: String, at: i64 },
}

impl Drop for IngestJob {
    fn drop(&mut self) {
        if let Self::CopiedText { text, .. } = self {
            use zeroize::Zeroize;
            text.zeroize();
        }
    }
}

pub struct Workers {
    ingest_tx: Option<Sender<IngestJob>>,
    copy_policy: IngestPolicy,
    /// Counts clips the ingest worker has stored (the controller notices changes on a wake).
    pub ingested: Arc<std::sync::atomic::AtomicU64>,
    pub copied: Arc<std::sync::atomic::AtomicU64>,
    preview_tx: Option<Sender<PreviewJob>>,
    threads: Vec<JoinHandle<()>>,
    pub preview_rx: Receiver<PreviewResult>,
}

/// Keeps the newest candidate request and a short queue of neighbours (newest last).
fn queue(
    job: PreviewJob,
    current: &mut Option<PreviewJob>,
    full: &mut Option<PreviewJob>,
    prefetch: &mut std::collections::VecDeque<PreviewJob>,
) {
    if job.full {
        *full = Some(job);
    } else if job.prefetch {
        if !prefetch.iter().any(|j| j.id == job.id) {
            prefetch.push_back(job);
        }
        while prefetch.len() > 24 {
            prefetch.pop_front();
        }
    } else {
        *current = Some(job);
    }
}

/// The picture to show for an image clip: the stored preview if there is one, else the full image
/// shrunk (and the preview stored, so the next time is quick).
fn preview_picture(store: &mut Store, id: i64) -> Option<Rgba> {
    if let Ok(Some(png)) = store.thumb(id) {
        if let Some(img) = image::decode_png(&png) {
            return Some(img);
        }
    }
    let png = store.image_png(id).ok().flatten()?;
    let img = image::decode_png(&png)?;
    let small = image::downscale(&img, THUMB_W, THUMB_H);
    if let Some(thumb) = image::encode_png_fast(&small) {
        let _ = store.set_thumb(id, &thumb);
    }
    Some(small)
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Workers {
    pub fn start(
        factory: StoreFactory,
        waker: Arc<dyn Waker>,
        source: Option<Box<dyn ClipboardSource>>,
        policy: IngestPolicy,
        log: Logger,
    ) -> Workers {
        let mut threads = Vec::new();
        let ingested = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let copied = Arc::new(std::sync::atomic::AtomicU64::new(0));

        let copy_policy = policy.clone();
        let ingest_tx = {
            let mut source = source;
            // The baseline is read here, before the thread exists: a copy made before the worker
            // gets scheduled must count as a change, not as the starting point.
            let mut last_seq = source.as_mut().map_or(0, |s| s.sequence());
            let (tx, rx) = channel::<IngestJob>();
            let factory = factory.clone();
            let log = log.clone();
            let (ingested, waker) = (ingested.clone(), waker.clone());
            let copied = copied.clone();
            threads.push(std::thread::spawn(move || {
                let mut store = match factory() {
                    Ok(s) => s,
                    Err(e) => {
                        log(&format!("capture worker could not open history: {e}"));
                        return;
                    }
                };
                let mut pending = std::collections::VecDeque::new();
                while let Some(mut job) = pending.pop_front().or_else(|| rx.recv().ok()) {
                    if let IngestJob::CopiedText { text, at } = &mut job {
                        if policy.paused.load(Ordering::Relaxed) || policy.exclude_apps.contains(COPY_SOURCE) {
                            continue;
                        }
                        match store.add_text_from(text, *at, Some(COPY_SOURCE)) {
                            Ok(_) => {
                                copied.fetch_add(1, Ordering::SeqCst);
                                ingested.fetch_add(1, Ordering::SeqCst);
                                waker.wake();
                            }
                            Err(e) => log(&format!("could not store copied text: {e}")),
                        }
                        continue;
                    }
                    let Some(source) = source.as_mut() else { continue };
                    // Coalesce clipboard notifications, but never discard explicit Copy jobs.
                    std::thread::sleep(Duration::from_millis(30));
                    while let Ok(next) = rx.try_recv() {
                        if !matches!(&next, IngestJob::ClipboardChanged) { pending.push_back(next); }
                    }
                    if !pending.is_empty() {
                        // Store explicit copies before capturing the latest external
                        // clipboard state, so a later copy remains the newest clip.
                        pending.push_back(IngestJob::ClipboardChanged);
                        continue;
                    }
                    let seq = source.sequence();
                    if seq == last_seq {
                        continue;
                    }
                    last_seq = seq;
                    if policy.paused.load(Ordering::Relaxed) {
                        continue;
                    }
                    // Ask who wrote the clipboard before reading it: an excluded program's copy must
                    // never be read, let alone stored.
                    let app = source.source_app();
                    if app.as_ref().is_some_and(|a| policy.exclude_apps.contains(a)) {
                        continue;
                    }
                    let app = app.as_deref();
                    use zeroize::Zeroize;
                    let stored = match source.capture() {
                        Ok(Captured::Text(mut t)) => {
                            let r = store.add_text_from(&t, now_secs(), app);
                            t.zeroize();
                            r
                        }
                        Ok(Captured::Files(mut f)) => {
                            let r = store.add_files_from(&f, now_secs(), app);
                            f.iter_mut().for_each(|s| s.zeroize());
                            r
                        }
                        Ok(Captured::Image(mut png)) => {
                            let r = store.add_image_png_from(&png, now_secs(), app);
                            png.zeroize();
                            r
                        }
                        Err(_) => continue,
                    };
                    match stored {
                        Ok(_) => {
                            ingested.fetch_add(1, Ordering::SeqCst);
                            waker.wake();
                        }
                        Err(e) => log(&format!("could not store a clip: {e}")),
                    }
                }
            }));
            Some(tx)
        };

        let (ptx, prx) = channel::<PreviewJob>();
        let (rtx, rrx) = channel::<PreviewResult>();
        {
            let log = log.clone();
            threads.push(std::thread::spawn(move || {
                let mut store = match factory() {
                    Ok(s) => s,
                    Err(e) => {
                        log(&format!("preview worker could not open history: {e}"));
                        return;
                    }
                };
                let mut current: Option<PreviewJob> = None;
                let mut full: Option<PreviewJob> = None;
                let mut prefetch: std::collections::VecDeque<PreviewJob> = std::collections::VecDeque::new();
                loop {
                    if current.is_none() && full.is_none() && prefetch.is_empty() {
                        match prx.recv() {
                            Ok(j) => queue(j, &mut current, &mut full, &mut prefetch),
                            Err(_) => return,
                        }
                    }
                    // Only the newest request for the candidate matters (the user may have
                    // scrolled on); neighbours wait their turn.
                    while let Ok(j) = prx.try_recv() {
                        queue(j, &mut current, &mut full, &mut prefetch);
                    }
                    let job = match current.take().or_else(|| full.take()) {
                        Some(j) => j,
                        None => match prefetch.pop_front() {
                            Some(j) => j,
                            None => continue,
                        },
                    };
                    let image = if job.full {
                        store.image_png(job.id).ok().flatten().and_then(|png| image::decode_png(&png))
                    } else {
                        preview_picture(&mut store, job.id)
                    };
                    // a neighbour that is not an image (or could not be decoded) has nothing to cache
                    if job.prefetch && image.is_none() {
                        continue;
                    }
                    let result = PreviewResult {
                        generation: job.generation,
                        id: job.id,
                        image,
                        prefetch: job.prefetch,
                        full: job.full,
                    };
                    if rtx.send(result).is_err() {
                        return; // the app is shutting down
                    }
                    waker.wake();
                }
            }));
        }

        Workers { ingest_tx, copy_policy, ingested, copied, preview_tx: Some(ptx), threads, preview_rx: rrx }
    }

    pub fn clipboard_changed(&self) {
        if let Some(tx) = &self.ingest_tx {
            let _ = tx.send(IngestJob::ClipboardChanged);
        }
    }

    /// Explicitly copied derived text is new history, even though the OS clipboard
    /// write retains privacy/own-write markers. Automatic pastes never use this path.
    pub fn record_copied_text(&self, text: &str, at: i64) {
        if self.copy_policy.paused.load(Ordering::Relaxed) || self.copy_policy.exclude_apps.contains(COPY_SOURCE) {
            return;
        }
        if let Some(tx) = &self.ingest_tx {
            let _ = tx.send(IngestJob::CopiedText { text: text.to_owned(), at });
        }
    }

    pub fn request_preview(&self, id: i64, generation: u64) {
        if let Some(tx) = &self.preview_tx {
            let _ = tx.send(PreviewJob { id, generation, prefetch: false, full: false });
        }
    }

    /// Asks for the whole picture of an image clip (the expanded viewer zooms into it).
    pub fn request_full(&self, id: i64, generation: u64) {
        if let Some(tx) = &self.preview_tx {
            let _ = tx.send(PreviewJob { id, generation, prefetch: false, full: true });
        }
    }

    /// Asks for the picture of a clip next to the candidate, so it is ready when scrolled to.
    pub fn request_prefetch(&self, id: i64) {
        if let Some(tx) = &self.preview_tx {
            let _ = tx.send(PreviewJob { id, generation: 0, prefetch: true, full: false });
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.ingest_tx.take();
        self.preview_tx.take();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
