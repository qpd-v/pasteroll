//! Background workers, so nothing slow runs on the UI thread (and the input hooks' heartbeat stays
//! alive):
//!
//! - ingest: after a clipboard change, reads the clipboard and stores the clip;
//! - preview: decodes and downscales the image candidate.
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
}

struct PreviewJob {
    id: i64,
    generation: u64,
    /// Warming the cache for a neighbour of the candidate: lower priority, nothing is shown for it.
    prefetch: bool,
}

pub struct Workers {
    ingest_tx: Option<Sender<()>>,
    /// Counts clips the ingest worker has stored (the controller notices changes on a wake).
    pub ingested: Arc<std::sync::atomic::AtomicU64>,
    preview_tx: Option<Sender<PreviewJob>>,
    threads: Vec<JoinHandle<()>>,
    pub preview_rx: Receiver<PreviewResult>,
}

/// Keeps the newest candidate request and a short queue of neighbours (newest last).
fn queue(
    job: PreviewJob,
    current: &mut Option<PreviewJob>,
    prefetch: &mut std::collections::VecDeque<PreviewJob>,
) {
    if job.prefetch {
        if !prefetch.iter().any(|j| j.id == job.id) {
            prefetch.push_back(job);
        }
        while prefetch.len() > 6 {
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

        let ingest_tx = source.map(|mut source| {
            // The baseline is read here, before the thread exists: a copy made before the worker
            // gets scheduled must count as a change, not as the starting point.
            let mut last_seq = source.sequence();
            let (tx, rx) = channel::<()>();
            let factory = factory.clone();
            let log = log.clone();
            let (ingested, waker) = (ingested.clone(), waker.clone());
            threads.push(std::thread::spawn(move || {
                let mut store = match factory() {
                    Ok(s) => s,
                    Err(e) => {
                        log(&format!("capture worker could not open history: {e}"));
                        return;
                    }
                };
                while rx.recv().is_ok() {
                    // Apps often set the clipboard several times in a burst: take the last.
                    std::thread::sleep(Duration::from_millis(30));
                    while rx.try_recv().is_ok() {}
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
                    if let Some(app) = source.source_app() {
                        if policy.exclude_apps.contains(&app) {
                            continue;
                        }
                    }
                    use zeroize::Zeroize;
                    let stored = match source.capture() {
                        Ok(Captured::Text(mut t)) => {
                            let r = store.add_text(&t, now_secs());
                            t.zeroize();
                            r
                        }
                        Ok(Captured::Files(mut f)) => {
                            let r = store.add_files(&f, now_secs());
                            f.iter_mut().for_each(|s| s.zeroize());
                            r
                        }
                        Ok(Captured::Image(mut png)) => {
                            let r = store.add_image_png(&png, now_secs());
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
            tx
        });

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
                let mut prefetch: std::collections::VecDeque<PreviewJob> = std::collections::VecDeque::new();
                loop {
                    if current.is_none() && prefetch.is_empty() {
                        match prx.recv() {
                            Ok(j) => queue(j, &mut current, &mut prefetch),
                            Err(_) => return,
                        }
                    }
                    // Only the newest request for the candidate matters (the user may have
                    // scrolled on); neighbours wait their turn.
                    while let Ok(j) = prx.try_recv() {
                        queue(j, &mut current, &mut prefetch);
                    }
                    let job = match current.take() {
                        Some(j) => j,
                        None => match prefetch.pop_front() {
                            Some(j) => j,
                            None => continue,
                        },
                    };
                    let image = preview_picture(&mut store, job.id);
                    // a neighbour that is not an image (or could not be decoded) has nothing to cache
                    if job.prefetch && image.is_none() {
                        continue;
                    }
                    let result = PreviewResult { generation: job.generation, id: job.id, image, prefetch: job.prefetch };
                    if rtx.send(result).is_err() {
                        return; // the app is shutting down
                    }
                    waker.wake();
                }
            }));
        }

        Workers { ingest_tx, ingested, preview_tx: Some(ptx), threads, preview_rx: rrx }
    }

    pub fn clipboard_changed(&self) {
        if let Some(tx) = &self.ingest_tx {
            let _ = tx.send(());
        }
    }

    pub fn request_preview(&self, id: i64, generation: u64) {
        if let Some(tx) = &self.preview_tx {
            let _ = tx.send(PreviewJob { id, generation, prefetch: false });
        }
    }

    /// Asks for the picture of a clip next to the candidate, so it is ready when scrolled to.
    pub fn request_prefetch(&self, id: i64) {
        if let Some(tx) = &self.preview_tx {
            let _ = tx.send(PreviewJob { id, generation: 0, prefetch: true });
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
