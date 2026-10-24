//! The carousel controller: everything the app *decides*, with no OS calls.
//!
//! A platform backend turns operating-system input into [`Event`]s and calls
//! [`Controller::handle`]; the controller answers through the [`Platform`] it is handed.
//!
//! Safety properties, all deliberate:
//! - paste goes only to the window that was foreground when the carousel opened (or the window the
//!   user clicked), in the same process; the destination is re-checked immediately before the
//!   keystroke, and the session is cancelled if the foreground changes (while the image editor is
//!   open it is re-checked when Paste is pressed and again before the result is delivered);
//! - a more-privileged window is refused up front rather than silently losing the paste;
//! - capture and image decoding run on worker threads, never here;
//! - what is pasted is the real text, even where the preview shows hidden characters as tokens.

mod editor;
mod multi;
mod viewer;

use crate::event::{Event, Key, MouseEvent, MouseKind};
use crate::filters::{self, Filter};
use crate::search::{self, MatchSource, SearchIndex};
use crate::pastekeys::{PasteKey, PasteKeys};
use crate::platform::{ClickPoll, ClipboardSource, OverlayInfo, Payload, Platform, Target, TextRecognizer, Waker};
use crate::terminals::PastePolicy;
use crate::workers::{IngestPolicy, Logger, PreviewResult, StoreFactory, Workers};
use pasteroll_core::config::{CustomFilter, UiMode};
use pasteroll_core::image as core_image;
use pasteroll_core::preview::{sanitize, Sanitized};
use pasteroll_core::store::{Kind, PinResult};
use pasteroll_core::{session::Moved, Dir, HistorySource, Item, Selection, Session, Snapshot, Step, Store};
use pasteroll_render::{geometry, window_dip, Body, Dropdown, Hit, Renderer, RowKind, RowView, ViewModel, HEIGHT, WIDTH};
use editor::{EditMsg, Editor};
use viewer::Viewer;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

pub const PREVIEW_CHARS: usize = 4096;
const IDLE_CANCEL_MS: u64 = 60_000;
/// The expanded viewer is for reading: it closes only after this long without any input.
const VIEWER_IDLE_MS: u64 = 600_000;
const CLICK_WINDOW_MS: u64 = 1_000;
const NOTICE_MS: u64 = 1_500;
const MAINTAIN_EVERY_MS: u64 = 600_000;
const DOUBLE_CLICK_MS: u64 = 500;
const ROWS_BEFORE: usize = 4;
const ROWS_AFTER: usize = 7;
const ROW_CACHE_MAX: usize = 512;
/// The search box holds at most this many characters.
const QUERY_MAX_CHARS: usize = 120;
/// Decoded preview pictures kept while the overlay is open (about 1.2 MB each).
const THUMB_CACHE: usize = 24;
/// How many clips either side of the candidate are fetched ahead.
const PREFETCH_REACH: usize = 3;
/// The search index covers at most this many of the newest clips.
const INDEX_MAX_CLIPS: usize = 50_000;
/// The input hook stops swallowing at 60 s; the overlay closes a little before that so the two
/// never disagree (typing going to the target while the overlay still looks live). Only the
/// expanded viewer extends that time (see `input_keepalive`); closing it starts the 55 s again.
const SESSION_MAX_MS: u64 = 55_000;

/// Lets the session and the UI thread share one store connection.
#[derive(Clone)]
struct SharedStore(Rc<RefCell<Store>>);

impl HistorySource for SharedStore {
    type Error = pasteroll_core::store::StoreError;
    fn snapshot(&mut self) -> Result<Option<Snapshot>, Self::Error> {
        self.0.borrow_mut().snapshot()
    }
    fn page(
        &mut self,
        snap: Snapshot,
        after: Option<(f64, i64)>,
        dir: Dir,
        limit: usize,
    ) -> Result<Vec<Item>, Self::Error> {
        self.0.borrow_mut().page(snap, after, dir, limit)
    }
    fn exists(&mut self, id: i64) -> Result<bool, Self::Error> {
        self.0.borrow_mut().exists(id)
    }
}

/// The recycle bin, for the bin's list of matches: a clip "exists" while it is in the bin.
#[derive(Clone)]
struct TrashStore(SharedStore);

impl HistorySource for TrashStore {
    type Error = pasteroll_core::store::StoreError;
    fn snapshot(&mut self) -> Result<Option<Snapshot>, Self::Error> {
        Ok(None) // the list of matches knows its own extent
    }
    fn page(&mut self, _: Snapshot, _: Option<(f64, i64)>, _: Dir, _: usize) -> Result<Vec<Item>, Self::Error> {
        Ok(Vec::new())
    }
    fn exists(&mut self, id: i64) -> Result<bool, Self::Error> {
        self.0 .0.borrow().is_trashed(id)
    }
}

/// Where the carousel's clips come from: the whole history, the matches of a search, or the
/// recycle bin. Cloneable, so the expanded viewer can browse a copy of the carousel's session.
#[derive(Clone)]
enum Src {
    All(SharedStore),
    Matches(MatchSource<SharedStore>),
    Trash(MatchSource<TrashStore>),
}

impl HistorySource for Src {
    type Error = pasteroll_core::store::StoreError;
    fn snapshot(&mut self) -> Result<Option<Snapshot>, Self::Error> {
        match self {
            Src::All(s) => s.snapshot(),
            Src::Matches(s) => s.snapshot(),
            Src::Trash(s) => s.snapshot(),
        }
    }
    fn page(
        &mut self,
        snap: Snapshot,
        after: Option<(f64, i64)>,
        dir: Dir,
        limit: usize,
    ) -> Result<Vec<Item>, Self::Error> {
        match self {
            Src::All(s) => s.page(snap, after, dir, limit),
            Src::Matches(s) => s.page(snap, after, dir, limit),
            Src::Trash(s) => s.page(snap, after, dir, limit),
        }
    }
    fn exists(&mut self, id: i64) -> Result<bool, Self::Error> {
        match self {
            Src::All(s) => s.exists(id),
            Src::Matches(s) => s.exists(id),
            Src::Trash(s) => s.exists(id),
        }
    }
}

enum BodyState {
    /// `san.display` is drawn; `orig` is the real text (UTF-16) that a selection maps back to.
    Text { san: Sanitized, orig: Vec<u16> },
    Image(Option<Arc<pasteroll_core::image::Rgba>>),
    Message(String),
}

impl Drop for BodyState {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        match self {
            BodyState::Text { san, orig } => {
                san.display.zeroize();
                orig.zeroize();
            }
            BodyState::Message(m) => m.zeroize(),
            BodyState::Image(_) => {}
        }
    }
}

struct RowInfo {
    kind: RowKind,
    pinned: bool,
    title: String,
    meta: String,
}

impl Drop for RowInfo {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.title.zeroize();
        self.meta.zeroize();
    }
}

/// Overwrites the clip content when it goes out of scope, on every path out of `confirm`.
struct WipeOnDrop(Payload);

impl Drop for WipeOnDrop {
    fn drop(&mut self) {
        self.0.wipe();
    }
}

struct ClickWait {
    since_ms: u64,
    first: bool,
}

struct Ui {
    body: BodyState,
    selection: Option<(usize, usize)>,
    anchor: Option<usize>,
    dragging: bool,
    scroll: f32,
    last_dbl_ms: Option<u64>,
    click: Option<ClickWait>,
    dropdown_open: bool,
    /// A multi-line paste into a terminal was announced; the next Enter goes through.
    warned: bool,
    /// The card button under the pointer.
    hover: Option<Hit>,
    /// Deleting this clip for good was asked for; Enter (or another click) confirms.
    confirm_delete: Option<i64>,
    /// Clips chosen for a multiple paste, in the order they were chosen.
    picks: Vec<i64>,
    /// The clip a Ctrl+Shift range grows from: the last one chosen by itself.
    pick_anchor: Option<i64>,
    /// The right-click menu, while it is open.
    menu: Option<multi::Menu>,
}

struct Open {
    opened_ms: u64,
    /// `None` when nothing matches the search.
    session: Option<Session<Src>>,
    target: Target,
    /// Tray browsing has no destination until an application is explicitly clicked.
    browse_only: bool,
    ui: Ui,
    query: String,
    filter: Filter,
    /// Built the first time a query or filter needs it, and dropped with the session.
    index: Option<SearchIndex>,
    /// Number of clips the current search matched, and how many exist (both 0 when unfiltered).
    matched: usize,
    total: usize,
    /// Keyboard highlight in the open dropdown.
    dd_hover: usize,
    /// The search box is being used to type a new custom filter ("Name ext ext").
    adding_filter: bool,
    /// The clip last moved to the recycle bin in this session (Ctrl+Z brings it back).
    last_trashed: Option<Item>,
}

impl Open {
    fn current_id(&self) -> Option<i64> {
        self.session.as_ref().map(|s| s.current().id)
    }

    /// The text that drives the search. While a custom filter is being typed into the box, that
    /// text is the filter's definition, not a query.
    fn search_text(&self) -> &str {
        if self.adding_filter {
            ""
        } else {
            &self.query
        }
    }

    fn is_all(&self) -> bool {
        self.search_text().trim().is_empty() && self.filter == Filter::Everything
    }

    fn in_trash(&self) -> bool {
        self.filter == Filter::Trash
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.query.zeroize();
    }
}

struct Notice {
    text: String,
    until_ms: u64,
}

/// A short message in the footer while the overlay is open ("Pin limit reached").
struct Status {
    text: String,
    until_ms: u64,
}

pub struct Controller {
    store: Rc<RefCell<Store>>,
    renderer: Renderer,
    paste_keys: PasteKeys,
    workers: Workers,
    open: Option<Open>,
    notice: Option<Notice>,
    status: Option<Status>,
    paste_policy: PastePolicy,
    custom_filters: Vec<CustomFilter>,
    seen_ingested: u64,
    seen_copied: u64,
    pending_copy_refresh: bool,
    /// Decoded preview pictures of image clips, newest use last (cleared when the overlay closes).
    thumbs: Vec<(i64, Arc<core_image::Rgba>)>,
    paused: Arc<std::sync::atomic::AtomicBool>,
    last_activity_ms: u64,
    last_maintain_ms: Option<u64>,
    row_cache: HashMap<i64, RowInfo>,
    preview_gen: u64,
    overlay: OverlayInfo,
    dirty: bool,
    /// The expanded viewer, while it is open (or animating).
    viewer: Option<Viewer>,
    /// Where the pointer last was over the viewer (screen px): wheel zoom centres on it.
    last_pointer: (i32, i32),
    ocr: Option<Arc<dyn TextRecognizer>>,
    ocr_request: u64,
    ocr_jobs: Arc<std::sync::atomic::AtomicUsize>,
    waker: Arc<dyn Waker>,
    ocr_tx: std::sync::mpsc::Sender<viewer::OcrDone>,
    ocr_rx: std::sync::mpsc::Receiver<viewer::OcrDone>,
    /// The image editor (Advanced Image Paste), while it is open (or animating).
    editor: Option<Editor>,
    edit_tx: std::sync::mpsc::Sender<EditMsg>,
    edit_rx: std::sync::mpsc::Receiver<EditMsg>,
    /// The key that, held while clicking the desktop or a window, opens the image editor.
    advanced_modifier: pasteroll_core::config::ClickModifier,
    /// Where Paste Path and the image editor save pictures ("" = the Desktop).
    image_folder: String,
    /// How the carousel looks: everything on show, or only the clip (every feature works in both).
    ui_mode: UiMode,
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Character ranges of `text` as UTF-16 ranges (what the renderer's body positions use).
fn utf16_ranges(text: &str, ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let mut at = Vec::with_capacity(text.chars().count() + 1);
    let mut u = 0usize;
    for c in text.chars() {
        at.push(u);
        u += c.len_utf16();
    }
    at.push(u);
    ranges
        .iter()
        .filter_map(|(a, b)| Some((*at.get(*a)?, *at.get(*b)?)))
        .collect()
}

/// Year, month, day of a day number (days since 1970-01-01), proleptic Gregorian.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "YYYY-MM-DD" of a day number (UTC).
pub(crate) fn utc_date(day: i64) -> String {
    let (y, m, d) = civil(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// "YYYY-MM-DD HH:MM" (UTC) of a Unix time; platforms show local time instead.
pub fn utc_date_time(secs: i64) -> String {
    let rem = secs.rem_euclid(86_400);
    format!("{} {:02}:{:02}", utc_date(secs.div_euclid(86_400)), rem / 3_600, rem % 3_600 / 60)
}

/// Whether a paste went ahead or needs the user's go-ahead first.
enum PasteCheck {
    Done,
    /// A multi-line text into a terminal: this many lines.
    Warn(usize),
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).find(|s| !s.is_empty()).unwrap_or(path)
}

impl Controller {
    pub fn new(
        store: Store,
        factory: StoreFactory,
        paste_keys: PasteKeys,
        waker: Arc<dyn Waker>,
        source: Option<Box<dyn ClipboardSource>>,
        policy: IngestPolicy,
        log: Logger,
    ) -> Controller {
        let paused = policy.paused.clone();
        let workers = Workers::start(factory, waker.clone(), source, policy, log);
        let (ocr_tx, ocr_rx) = std::sync::mpsc::channel();
        let (edit_tx, edit_rx) = std::sync::mpsc::channel();
        Controller {
            store: Rc::new(RefCell::new(store)),
            renderer: Renderer::new(),
            paste_keys,
            workers,
            open: None,
            notice: None,
            status: None,
            paste_policy: PastePolicy::default(),
            custom_filters: Vec::new(),
            seen_ingested: 0,
            seen_copied: 0,
            pending_copy_refresh: false,
            thumbs: Vec::new(),
            paused,
            last_activity_ms: 0,
            last_maintain_ms: None,
            row_cache: HashMap::new(),
            preview_gen: 0,
            overlay: OverlayInfo::new(1.0, (WIDTH as u32, HEIGHT as u32)),
            dirty: false,
            viewer: None,
            last_pointer: (0, 0),
            ocr: None,
            ocr_request: 0,
            ocr_jobs: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            waker,
            ocr_tx,
            ocr_rx,
            editor: None,
            edit_tx,
            edit_rx,
            advanced_modifier: pasteroll_core::config::ClickModifier::Ctrl,
            image_folder: String::new(),
            ui_mode: UiMode::Complete,
        }
    }

    /// The mode the program starts in (the platform sizes its window the same way itself). Use
    /// [`Controller::set_ui_mode`] to change it while running.
    pub fn init_ui_mode(&mut self, mode: UiMode) {
        self.ui_mode = mode;
        self.overlay = OverlayInfo::new(1.0, {
            let (w, h) = window_dip(mode);
            (w as u32, h as u32)
        });
    }

    pub fn ui_mode(&self) -> UiMode {
        self.ui_mode
    }

    /// Switches between Complete and No Bloat (a different look, the same features). A carousel
    /// that is open is closed first, so the window never changes shape under the pointer; the next
    /// one opens in the new mode.
    pub fn set_ui_mode(&mut self, p: &mut dyn Platform, mode: UiMode) {
        if mode == self.ui_mode {
            return;
        }
        self.cancel(p, "mode changed");
        self.init_ui_mode(mode);
        p.ui_mode_changed(mode);
    }

    /// Text recognition for the viewer's Extract text (none: the action says it is unavailable).
    pub fn set_ocr(&mut self, ocr: Arc<dyn TextRecognizer>) {
        self.ocr = Some(ocr);
    }

    // ------------------------------------------------------------------ queries (tests, platforms)

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn current_id(&self) -> Option<i64> {
        self.open.as_ref().and_then(Open::current_id)
    }

    /// The text in the search box.
    pub fn query(&self) -> Option<&str> {
        self.open.as_ref().map(|o| o.query.as_str())
    }

    /// How many clips the current search matches, and how many exist.
    pub fn match_counts(&self) -> Option<(usize, usize)> {
        self.open.as_ref().map(|o| (o.matched, o.total))
    }

    pub fn filter_label(&self) -> Option<String> {
        self.open.as_ref().map(|o| filters::label(&o.filter, &self.custom_filters))
    }

    /// The user's custom filters (from the config file), shown at the end of the dropdown.
    pub fn set_custom_filters(&mut self, filters: Vec<CustomFilter>) {
        self.custom_filters = filters;
    }

    pub fn notice_text(&self) -> Option<&str> {
        self.notice.as_ref().map(|n| n.text.as_str())
    }

    /// The part of the clip the user selected, as the real text.
    pub fn selected_text(&self) -> Option<String> {
        let open = self.open.as_ref()?;
        let (a, b) = open.ui.selection?;
        let (a, b) = (a.min(b), a.max(b));
        if b <= a {
            return None;
        }
        match &open.ui.body {
            BodyState::Text { san, orig } => {
                let (x, y) = san.original_range(a, b);
                (y > x && y <= orig.len()).then(|| String::from_utf16_lossy(&orig[x..y]))
            }
            _ => None,
        }
    }

    pub fn set_paste_policy(&mut self, policy: PastePolicy) {
        self.paste_policy = policy;
    }

    /// Pauses or resumes recording (the tray menu and the config file drive this).
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// The platform saw the system clipboard change: let the ingest worker look at it.
    pub fn clipboard_changed(&self) {
        self.workers.clipboard_changed();
    }

    pub fn store(&self) -> Rc<RefCell<Store>> {
        self.store.clone()
    }

    pub fn overlay_info(&self) -> OverlayInfo {
        self.overlay
    }

    fn size_dip(&self) -> (f32, f32) {
        let s = self.overlay.scale.max(0.1);
        (self.overlay.size_px.0 as f32 / s, self.overlay.size_px.1 as f32 / s)
    }

    /// Pixel centre of the character at display offset `pos` in the active body (for self-tests
    /// that aim real input at a character).
    pub fn char_centre_px(&mut self, pos: usize) -> Option<(f32, f32)> {
        let vm = self.view_model();
        let size = self.size_dip();
        self.renderer.char_centre(&vm, size, self.overlay.scale, pos)
    }

    /// The displayed text of the active clip (tokens for hidden characters included).
    pub fn displayed_text(&self) -> Option<String> {
        match &self.open.as_ref()?.ui.body {
            BodyState::Text { san, .. } => Some(san.display.clone()),
            _ => None,
        }
    }

    // ------------------------------------------------------------------ event entry point

    pub fn handle(&mut self, p: &mut dyn Platform, ev: Event) {
        let now = p.now_ms();
        // Processing an event proves the UI thread is alive just as a tick does; under key repeat
        // the timer message can be starved by the posted ones.
        if !matches!(ev, Event::Tick) {
            p.heartbeat();
        }
        let viewing = self.viewer.is_some();
        // the image editor is modal: it takes every event meant for the carousel
        let consumed = self.editor.is_some() && self.editor_event(p, ev, now);
        match ev {
            _ if consumed => {}
            Event::Activate => {
                self.last_activity_ms = now;
                self.activate(p, now);
            }
            Event::BrowseAt(x, y) => {
                p.begin_browse();
                self.end_session(p);
                self.last_activity_ms = now;
                self.open_at(p, now, Target { id: 0, pid: 0, exe: String::new(),
                    class: String::new(), desktop: false }, (x, y), true);
            }
            Event::Step(step, gen) => {
                if self.live(p, gen) {
                    self.touch(p, now);
                    if viewing {
                        self.viewer_step(p, step == Step::Older, now);
                    } else if self.open.as_ref().is_some_and(|o| o.ui.click.is_none()) {
                        self.step(p, step == Step::Older);
                    }
                }
            }
            Event::Key(key, gen) => {
                if self.live(p, gen) {
                    self.touch(p, now);
                    if viewing {
                        self.viewer_key(p, key, now);
                    } else {
                        self.key(p, key, now);
                    }
                }
            }
            Event::ClickPaste(gen) => {
                // the viewer is modal: no paste-at-click while it is open
                if self.live(p, gen) && !viewing {
                    self.touch(p, now);
                    if let Some(o) = self.open.as_mut() {
                        if o.ui.click.is_none() {
                            o.ui.click = Some(ClickWait { since_ms: now, first: true });
                        }
                    }
                }
            }
            Event::ClickCancel(gen) | Event::ClickOutside(gen) => {
                if self.live(p, gen) {
                    if viewing {
                        self.close_viewer(p, now);
                    } else if matches!(ev, Event::ClickCancel(_)) {
                        self.cancel(p, "clicked elsewhere");
                    }
                }
            }
            Event::Mouse(m) => {
                // the carousel takes no input while the viewer is open
                if self.open.is_some() && !viewing {
                    self.touch(p, now);
                    self.mouse(p, m, now);
                }
            }
            Event::ViewerMouse(m) => {
                if self.open.is_some() && viewing {
                    self.touch(p, now);
                    self.viewer_mouse(p, m, now);
                }
            }
            Event::Wheel(w, gen) => {
                if self.live(p, gen) && viewing {
                    self.touch(p, now);
                    self.viewer_wheel(w);
                }
            }
            Event::Tick => self.tick(p, now),
            Event::Wake => {
                self.drain_previews();
                self.drain_ocr(now);
                self.drain_edit(p, now);
                self.on_new_clips(p);
            }
        }
        if self.dirty {
            self.dirty = false;
            self.present(p);
        }
    }

    fn live(&self, p: &dyn Platform, gen: u64) -> bool {
        self.open.is_some() && gen == p.input_generation()
    }

    /// The user did something: the idle clock starts again (and, in the viewer, the hooks' clock).
    fn touch(&mut self, p: &mut dyn Platform, now: u64) {
        self.last_activity_ms = now;
        if self.viewer.is_some() || self.editor.is_some() {
            p.input_keepalive();
        }
    }

    // ------------------------------------------------------------------ session lifecycle

    fn notify(&mut self, p: &mut dyn Platform, text: &str, now: u64) {
        let at = p.cursor();
        self.overlay = p.overlay_show(at);
        self.notice = Some(Notice { text: text.to_owned(), until_ms: now + NOTICE_MS });
        self.dirty = true;
    }

    fn activate(&mut self, p: &mut dyn Platform, now: u64) {
        if self.open.is_some() {
            return;
        }
        let Some(target) = p.foreground() else {
            p.log("activation ignored: no suitable target window");
            return;
        };
        if !target.desktop && p.target_privileged(&target) {
            self.notify(p, "Can't paste into an administrator window.", now);
            p.log("target runs at a higher privilege level: refused");
            return;
        }
        let at = p.cursor();
        self.open_at(p, now, target, at, false);
    }

    fn open_at(&mut self, p: &mut dyn Platform, now: u64, target: Target, at: (i32, i32), browse_only: bool) {
        let session = match Session::open(Src::All(SharedStore(self.store.clone()))) {
            Ok(Some(s)) => s,
            Ok(None) => {
                self.notify(p, "Clipboard history is empty.", now);
                return;
            }
            Err(e) => {
                p.log(&format!("could not open history: {e}"));
                return;
            }
        };
        self.notice = None;
        self.open = Some(Open {
            opened_ms: now,
            session: Some(session),
            query: String::new(),
            filter: Filter::Everything,
            index: None,
            matched: 0,
            total: 0,
            dd_hover: 0,
            adding_filter: false,
            last_trashed: None,
            target,
            browse_only,
            ui: Ui {
                body: BodyState::Message(String::new()),
                selection: None,
                anchor: None,
                dragging: false,
                scroll: 0.0,
                last_dbl_ms: None,
                click: None,
                dropdown_open: false,
                warned: false,
                hover: None,
                confirm_delete: None,
                picks: Vec::new(),
                pick_anchor: None,
                menu: None,
            },
        });
        p.input_active(true);
        self.load_body();
        self.overlay = p.overlay_show(at);
        if browse_only {
            self.status = Some(Status { text: "Click a destination window to paste; Esc closes.".into(),
                until_ms: now + NOTICE_MS });
        }
        self.dirty = true;
        p.log("session opened");
    }

    fn end_session(&mut self, p: &mut dyn Platform) {
        self.pending_copy_refresh = false;
        self.drop_editor(p);
        self.drop_viewer(p);
        if self.open.take().is_some() {
            p.input_active(false);
            p.overlay_hide();
        }
        // Cached row titles are the first part of recent clips: do not keep them between sessions.
        self.row_cache.clear();
        self.thumbs.clear();
        // A footer message (a terminal warning, say) belongs to the session that raised it.
        self.status = None;
    }

    fn cancel(&mut self, p: &mut dyn Platform, why: &str) {
        if self.open.is_some() {
            p.log(&format!("session closed: {why}"));
        }
        self.end_session(p);
    }

    // ------------------------------------------------------------------ terminal warning

    fn withdraw_warning(&mut self) {
        if let Some(o) = self.open.as_mut() {
            if o.ui.warned {
                o.ui.warned = false;
                self.status = None;
                self.dirty = true;
            }
        }
    }

    // ------------------------------------------------------------------ pin and delete

    fn flash(&mut self, text: &str, now: u64) {
        self.status = Some(Status { text: text.to_owned(), until_ms: now + NOTICE_MS });
        self.dirty = true;
    }

    fn toggle_pin(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(id) = self.current_id() else { return };
        if self.open.as_ref().is_some_and(Open::in_trash) {
            self.flash("Restore it from the recycle bin first to pin it", now);
            return;
        }
        if let Some(msg) = self.set_pin(p, id, now) {
            self.flash(&msg, now);
        }
    }

    /// Pins or unpins clip `id`. Returns what to tell the user.
    fn set_pin(&mut self, p: &mut dyn Platform, id: i64, _now: u64) -> Option<String> {
        let pinned = self.store.borrow().info(id).ok().flatten().is_some_and(|i| i.pinned);
        let result = self.store.borrow_mut().set_pinned(id, !pinned);
        let mut now_pinned = pinned;
        let msg = match result {
            Ok(PinResult::Pinned) => {
                now_pinned = true;
                Some("Pinned: kept until you unpin it".to_owned())
            }
            Ok(PinResult::Unpinned) => {
                now_pinned = false;
                Some("Unpinned".to_owned())
            }
            Ok(PinResult::LimitReached) => {
                let max = self.store.borrow().limits().max_pins;
                Some(format!("Pin limit reached ({max}). Unpin something first."))
            }
            Ok(PinResult::Missing) => Some("That clip no longer exists".to_owned()),
            Err(e) => {
                p.log(&format!("could not change the pin: {e}"));
                None
            }
        };
        self.row_cache.remove(&id);
        if let Some(idx) = self.open.as_mut().and_then(|o| o.index.as_mut()) {
            idx.set_pinned(id, now_pinned);
        }
        // In the Pinned filter, pinning state is membership: the list and the counts must follow
        // (not under the viewer, which must give the carousel back as it was).
        if self.open.as_ref().is_some_and(|o| o.filter == Filter::Pinned) && now_pinned != pinned && self.viewer.is_none() {
            self.refresh(p);
        }
        self.dirty = true;
        msg
    }

    /// Delete key and the card's bin button: the candidate goes to the recycle bin (in the bin
    /// itself, deleting is for good and is confirmed first).
    fn delete_current(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(id) = self.current_id() else { return };
        if self.open.as_ref().is_some_and(Open::in_trash) {
            self.ask_delete_forever(id);
            return;
        }
        if self.trash_clip(p, id) {
            self.step(p, true);
            if self.open.is_some() {
                self.flash("Moved to the recycle bin. Ctrl+Z brings it back.", now);
            }
        }
    }

    /// Moves clip `id` from history to the recycle bin. `false` if nothing changed.
    fn trash_clip(&mut self, p: &mut dyn Platform, id: i64) -> bool {
        let order = self
            .open
            .as_ref()
            .and_then(|o| o.session.as_ref())
            .and_then(|s| s.window_items().0.iter().find(|i| i.id == id).map(|i| i.order));
        let order = order.or_else(|| self.store.borrow_mut().order_of(id).ok().flatten());
        match self.store.borrow_mut().trash(id, p.now_secs()) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                p.log(&format!("could not move the clip to the recycle bin: {e}"));
                return false;
            }
        }
        self.forget_clip(id);
        if let (Some(o), Some(order)) = (self.open.as_mut(), order) {
            o.last_trashed = Some(Item { id, order });
        }
        true
    }

    /// Deletes clip `id` for good (from history or the bin). `false` if nothing changed.
    fn delete_forever(&mut self, p: &mut dyn Platform, id: i64) -> bool {
        match self.store.borrow_mut().delete(id) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                p.log(&format!("could not delete the clip: {e}"));
                return false;
            }
        }
        self.forget_clip(id);
        if let Some(o) = self.open.as_mut() {
            if o.last_trashed.as_ref().is_some_and(|i| i.id == id) {
                o.last_trashed = None;
            }
        }
        true
    }

    /// Brings clip `id` back from the bin. `false` if nothing changed.
    fn restore_clip(&mut self, p: &mut dyn Platform, id: i64) -> bool {
        match self.store.borrow_mut().restore(id) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                p.log(&format!("could not restore the clip: {e}"));
                return false;
            }
        }
        self.row_cache.remove(&id);
        if let Some(o) = self.open.as_mut() {
            // the index no longer covers history: rebuilt on the next search
            o.index = None;
            if o.last_trashed.as_ref().is_some_and(|i| i.id == id) {
                o.last_trashed = None;
            }
        }
        true
    }

    /// A clip left the list being shown: drop what is cached about it.
    fn forget_clip(&mut self, id: i64) {
        self.row_cache.remove(&id);
        self.thumbs.retain(|(k, _)| *k != id);
        if let Some(idx) = self.open.as_mut().and_then(|o| o.index.as_mut()) {
            idx.remove(id);
        }
        if let Some(o) = self.open.as_mut() {
            o.matched = o.matched.saturating_sub(1);
            o.total = o.total.saturating_sub(1);
            // a clip that is gone cannot be part of a multiple paste
            o.ui.picks.retain(|k| *k != id);
            if o.ui.pick_anchor == Some(id) {
                o.ui.pick_anchor = None;
            }
        }
    }

    /// Shift+Delete, Shift+click on the bin button, Delete in the bin: deleting for good is
    /// confirmed first (Enter, or the same click again).
    fn ask_delete_forever(&mut self, id: i64) {
        if let Some(o) = self.open.as_mut() {
            o.ui.confirm_delete = Some(id);
        }
        self.status = Some(Status {
            text: "Delete this clip permanently? Enter deletes, Esc keeps it.".into(),
            until_ms: u64::MAX,
        });
        self.dirty = true;
    }

    fn confirm_delete_forever(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(id) = self.open.as_mut().and_then(|o| o.ui.confirm_delete.take()) else { return };
        self.status = None;
        if self.delete_forever(p, id) {
            if self.current_id() == Some(id) {
                self.step(p, true);
            }
            if self.open.is_some() {
                self.flash("Deleted permanently", now);
            }
        }
        self.dirty = true;
    }

    fn withdraw_delete_question(&mut self) {
        if self.open.as_mut().and_then(|o| o.ui.confirm_delete.take()).is_some() {
            self.status = None;
            self.dirty = true;
        }
    }

    /// Ctrl+Z / the card's Restore button. In the bin: the candidate goes back to history.
    /// Elsewhere: the clip last moved to the bin comes back, and the carousel shows it.
    fn undo_trash(&mut self, p: &mut dyn Platform, now: u64) {
        if self.open.as_ref().is_some_and(Open::in_trash) {
            let Some(id) = self.current_id() else { return };
            if self.restore_clip(p, id) {
                self.step(p, true);
                if self.open.is_some() {
                    self.flash("Restored to history", now);
                }
            }
            return;
        }
        let Some(item) = self.open.as_mut().and_then(|o| o.last_trashed.take()) else { return };
        if self.restore_clip(p, item.id) {
            self.refresh_at(p, Some(item));
            self.flash("Restored", now);
        }
    }

    // ------------------------------------------------------------------ search and filters

    /// Re-runs the search for the current query and filter and shows the newest match.
    fn refresh(&mut self, p: &mut dyn Platform) {
        self.refresh_at(p, None);
    }

    /// As [`Controller::refresh`], landing on `at` when it is in the new list.
    fn refresh_at(&mut self, p: &mut dyn Platform, at: Option<Item>) {
        // the clip the warning was about may no longer be the candidate
        self.withdraw_warning();
        self.withdraw_delete_question();
        let open_src = |src: Src| match &at {
            Some(item) => Session::open_at(src, item),
            None => Session::open(src),
        };
        let Some(open) = self.open.as_mut() else { return };
        if open.in_trash() {
            let rows = self.store.borrow().trash_rows();
            match rows {
                Ok(rows) => {
                    let index = SearchIndex::from_rows(rows);
                    let items = index.search(open.search_text(), &Filter::Trash, &self.custom_filters);
                    open.matched = items.len();
                    open.total = index.len();
                    let src = Src::Trash(MatchSource::new(items, TrashStore(SharedStore(self.store.clone()))));
                    open.session = open_src(src).ok().flatten();
                }
                Err(e) => {
                    p.log(&format!("could not read the recycle bin: {e}"));
                    open.session = None;
                }
            }
            if open.session.is_none() && open.search_text().trim().is_empty() {
                self.status = Some(Status { text: "The recycle bin is empty.".into(), until_ms: p.now_ms() + NOTICE_MS });
            }
        } else if open.is_all() {
            let session = open_src(Src::All(SharedStore(self.store.clone()))).ok().flatten();
            open.matched = 0;
            open.total = 0;
            open.session = session;
        } else {
            if open.index.is_none() {
                p.heartbeat();
                let rows = self.store.borrow().search_rows_newest(INDEX_MAX_CLIPS);
                p.heartbeat();
                match rows {
                    Ok(rows) => open.index = Some(SearchIndex::from_rows(rows)),
                    Err(e) => {
                        p.log(&format!("could not build the search index: {e}"));
                        open.session = None;
                        open.matched = 0;
                        open.total = 0;
                        self.status = Some(Status {
                            text: "Search is unavailable.".into(),
                            until_ms: p.now_ms() + NOTICE_MS,
                        });
                        self.load_body();
                        self.dirty = true;
                        return;
                    }
                }
            }
            let index = open.index.as_ref().expect("built above");
            let items = index.search(open.search_text(), &open.filter, &self.custom_filters);
            open.matched = items.len();
            open.total = index.len();
            let src = Src::Matches(MatchSource::new(items, SharedStore(self.store.clone())));
            open.session = match open_src(src) {
                Ok(s) => s,
                Err(e) => {
                    p.log(&format!("search failed: {e}"));
                    None
                }
            };
        }
        self.row_cache.clear();
        self.load_body();
        self.dirty = true;
    }

    fn set_filter(&mut self, p: &mut dyn Platform, filter: Filter) {
        if let Some(o) = self.open.as_mut() {
            o.filter = filter;
            o.ui.dropdown_open = false;
            o.adding_filter = false;
        }
        self.refresh(p);
    }

    fn dropdown_pick(&mut self, p: &mut dyn Platform, index: usize) {
        match filters::at(index, &self.custom_filters) {
            Some(f) => self.set_filter(p, f),
            None => {
                // the "Add custom filter..." entry: the search box takes "name ext ext"
                if let Some(o) = self.open.as_mut() {
                    o.ui.dropdown_open = false;
                    o.adding_filter = true;
                    o.query.clear();
                }
                self.refresh(p);
            }
        }
    }

    fn finish_add_filter(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(o) = self.open.as_ref() else { return };
        let mut words = o.query.split_whitespace();
        let name = words.next().unwrap_or("").to_owned();
        let exts: Vec<String> = words.map(str::to_owned).collect();
        match p.add_custom_filter(&name, &exts) {
            Ok(list) => {
                self.custom_filters = list;
                let idx = self.custom_filters.iter().position(|f| f.name.eq_ignore_ascii_case(&name));
                if let Some(o) = self.open.as_mut() {
                    o.adding_filter = false;
                    o.query.clear();
                }
                match idx {
                    Some(i) => self.set_filter(p, Filter::Custom(i)),
                    None => self.dirty = true,
                }
            }
            Err(e) => self.flash(&e, now),
        }
    }

    /// A character typed into the search box.
    fn type_char(&mut self, p: &mut dyn Platform, c: char) {
        if c.is_control() {
            return;
        }
        let Some(o) = self.open.as_mut() else { return };
        if o.query.chars().count() >= QUERY_MAX_CHARS {
            return;
        }
        o.query.push(c);
        self.query_edited(p);
    }

    /// The box's text changed: search again, unless it is a filter definition being typed.
    fn query_edited(&mut self, p: &mut dyn Platform) {
        if self.open.as_ref().is_some_and(|o| o.adding_filter) {
            self.dirty = true;
        } else {
            self.refresh(p);
        }
    }

    fn erase(&mut self, p: &mut dyn Platform, word: bool) {
        let Some(o) = self.open.as_mut() else { return };
        if o.query.is_empty() {
            return;
        }
        if word {
            while o.query.ends_with(char::is_whitespace) {
                o.query.pop();
            }
            while o.query.chars().last().is_some_and(|c| !c.is_whitespace()) {
                o.query.pop();
            }
        } else {
            o.query.pop();
        }
        self.query_edited(p);
    }

    // ------------------------------------------------------------------ navigation and body

    fn step(&mut self, p: &mut dyn Platform, older: bool) {
        self.withdraw_warning();
        let moved = match self.open.as_mut() {
            Some(o) => match o.session.as_mut() {
                Some(s) if older => s.older(),
                Some(s) => s.newer(),
                None => return,
            },
            None => return,
        };
        match moved {
            Ok(Moved::Empty) => {
                if self.open.as_ref().is_some_and(|o| !o.is_all()) {
                    // every match is gone (or the index is stale): rebuild it and say so
                    if let Some(o) = self.open.as_mut() {
                        o.index = None;
                    }
                    self.refresh(p);
                } else {
                    self.cancel(p, "history is empty");
                }
            }
            Ok(_) => {
                self.load_body();
                self.dirty = true;
            }
            Err(e) => {
                p.log(&format!("history error: {e}"));
                self.cancel(p, "error");
            }
        }
    }

    fn load_body(&mut self) {
        if self.open.as_ref().is_some_and(|o| o.session.is_none()) {
            if let Some(o) = self.open.as_mut() {
                o.ui.body = BodyState::Message("No matching clips.".into());
                o.ui.selection = None;
                o.ui.anchor = None;
                o.ui.dragging = false;
                o.ui.scroll = 0.0;
            }
            return;
        }
        let Some(id) = self.open.as_ref().and_then(Open::current_id) else { return };
        let kind = self.store.borrow().kind(id);
        let body = match kind {
            Ok(Some(Kind::Image)) => {
                self.preview_gen += 1;
                match self.cached_thumb(id) {
                    // seen already, or fetched ahead while the user was elsewhere: no waiting
                    Some(img) => BodyState::Image(Some(img)),
                    None => {
                        // Decoding happens on the preview worker; an empty frame is shown meanwhile.
                        self.workers.request_preview(id, self.preview_gen);
                        BodyState::Image(None)
                    }
                }
            }
            Ok(Some(_)) => {
                let p = self.store.borrow().preview(id, PREVIEW_CHARS);
                match p {
                    Ok(Some(mut p)) => {
                        let mut san = sanitize(&p.text);
                        if p.truncated {
                            // Say how much is hidden, so a long clip cannot look shorter than it is.
                            let hidden = p.total_chars.saturating_sub(PREVIEW_CHARS);
                            san = san.with_footer(&format!("\n... {} more characters not shown", thousands(hidden)));
                        }
                        let orig = p.text.encode_utf16().collect();
                        {
                            use zeroize::Zeroize;
                            p.text.zeroize();
                        }
                        BodyState::Text { san, orig }
                    }
                    _ => BodyState::Message("No preview available.".into()),
                }
            }
            _ => BodyState::Message("No preview available.".into()),
        };
        if let Some(o) = self.open.as_mut() {
            o.ui.body = body;
            o.ui.selection = None;
            o.ui.anchor = None;
            o.ui.dragging = false;
            o.ui.scroll = 0.0;
        }
        self.prefetch_neighbours();
    }

    /// A clip was stored while the overlay may be open: the search index no longer covers
    /// everything, so rebuild it (and re-run the search that is showing).
    fn on_new_clips(&mut self, p: &mut dyn Platform) {
        let copied = self.workers.copied.load(Ordering::SeqCst);
        if copied != self.seen_copied {
            self.seen_copied = copied;
            self.pending_copy_refresh = self.open.is_some();
        }
        let n = self.workers.ingested.load(Ordering::SeqCst);
        let changed = n != self.seen_ingested;
        self.seen_ingested = n;
        if self.pending_copy_refresh {
            if let Some(o) = self.open.as_mut() { o.index = None; }
            if self.viewer.is_none() && self.editor.is_none() {
                self.pending_copy_refresh = false;
                self.refresh(p);
            }
            return;
        }
        if !changed {
            return;
        }
        let Some(o) = self.open.as_mut() else { return };
        if o.index.is_some() {
            o.index = None;
            if !o.is_all() {
                self.refresh(p);
            }
        }
    }

    fn drain_previews(&mut self) {
        let mut got = Vec::new();
        while let Ok(r) = self.workers.preview_rx.try_recv() {
            got.push(r);
        }
        for r in got {
            self.on_preview(r);
        }
    }

    fn on_preview(&mut self, mut r: PreviewResult) {
        if self.open.is_none() {
            return;
        }
        let img = r.image.take().map(Arc::new);
        if let (Some(i), false) = (&img, r.full) {
            self.remember_thumb(r.id, i.clone());
        }
        if self.viewer.is_some() {
            self.viewer_on_preview(&r, img.clone());
        }
        if r.full {
            return; // only the viewer asks for whole pictures
        }
        if r.prefetch {
            return; // only for the cache
        }
        let Some(open) = self.open.as_mut() else { return };
        if open.current_id() != Some(r.id) || r.generation != self.preview_gen {
            return; // the user has scrolled on
        }
        open.ui.body = match img {
            Some(img) => BodyState::Image(Some(img)),
            None => BodyState::Message("No preview available.".into()),
        };
        self.dirty = true;
    }

    fn cached_thumb(&mut self, id: i64) -> Option<Arc<core_image::Rgba>> {
        let i = self.thumbs.iter().position(|(k, _)| *k == id)?;
        let entry = self.thumbs.remove(i);
        let img = entry.1.clone();
        self.thumbs.push(entry);
        Some(img)
    }

    fn remember_thumb(&mut self, id: i64, img: Arc<core_image::Rgba>) {
        self.thumbs.retain(|(k, _)| *k != id);
        self.thumbs.push((id, img));
        while self.thumbs.len() > THUMB_CACHE {
            self.thumbs.remove(0);
        }
    }

    /// Warms the cache for the clips either side of the candidate (older first: that is the way
    /// people scroll), so the next step shows its picture at once.
    fn prefetch_neighbours(&mut self) {
        let Some(s) = self.open.as_ref().and_then(|o| o.session.as_ref()) else { return };
        let (items, cur) = s.window_items();
        let mut ids = Vec::new();
        for d in 1..=PREFETCH_REACH {
            for idx in [cur.checked_add(d), cur.checked_sub(d)].into_iter().flatten() {
                if let Some(item) = items.get(idx) {
                    ids.push(item.id);
                }
            }
        }
        for id in ids {
            if self.thumbs.iter().any(|(k, _)| *k == id) {
                continue;
            }
            if matches!(self.store.borrow().kind(id), Ok(Some(Kind::Image))) {
                self.workers.request_prefetch(id);
            }
        }
    }

    // ------------------------------------------------------------------ keyboard

    fn key(&mut self, p: &mut dyn Platform, key: Key, now: u64) {
        // A click-paste is pending: the clip it will paste must not change underneath it.
        if self.open.as_ref().is_some_and(|o| o.ui.click.is_some()) && key != Key::Escape {
            return;
        }
        if key != Key::Enter {
            self.withdraw_warning();
        }
        // an open right-click menu: any key closes it, and Esc does nothing more
        if self.close_menu() && key == Key::Escape {
            return;
        }
        // a pending "delete permanently?": Enter confirms, Esc keeps the clip, anything else
        // carries on as usual
        if self.open.as_ref().is_some_and(|o| o.ui.confirm_delete.is_some()) {
            match key {
                Key::Enter => {
                    self.confirm_delete_forever(p, now);
                    return;
                }
                Key::Escape => {
                    self.withdraw_delete_question();
                    return;
                }
                _ => self.withdraw_delete_question(),
            }
        }
        let (dropdown, adding) = match self.open.as_ref() {
            Some(o) => (o.ui.dropdown_open, o.adding_filter),
            None => return,
        };
        // While the dropdown is open the arrows and Enter drive it.
        if dropdown {
            let n = filters::labels(&self.custom_filters).len();
            match key {
                Key::Up | Key::Down => {
                    if let Some(o) = self.open.as_mut() {
                        o.dd_hover = if key == Key::Down { (o.dd_hover + 1) % n } else { (o.dd_hover + n - 1) % n };
                        self.dirty = true;
                    }
                }
                Key::Enter => {
                    let i = self.open.as_ref().map_or(0, |o| o.dd_hover);
                    self.dropdown_pick(p, i);
                }
                Key::Escape | Key::Tab => {
                    if let Some(o) = self.open.as_mut() {
                        o.ui.dropdown_open = false;
                        self.dirty = true;
                    }
                }
                _ => {}
            }
            return;
        }
        match key {
            Key::Escape => {
                let query_empty = self.open.as_ref().is_none_or(|o| o.query.is_empty());
                if adding {
                    if let Some(o) = self.open.as_mut() {
                        o.adding_filter = false;
                        o.query.clear();
                    }
                    self.refresh(p);
                } else if !query_empty {
                    if let Some(o) = self.open.as_mut() {
                        o.query.clear();
                    }
                    self.refresh(p);
                } else if !self.clear_picks() {
                    self.cancel(p, "cancelled");
                }
            }
            Key::Enter => {
                if self.open.as_ref().is_some_and(|o| o.ui.click.is_none()) {
                    if adding {
                        self.finish_add_filter(p, now);
                    } else {
                        self.confirm(p);
                    }
                }
            }
            Key::Up | Key::Down => {
                if self.open.as_ref().is_some_and(|o| o.ui.click.is_none()) {
                    self.step(p, key == Key::Down);
                }
            }
            Key::Tab => {
                if let Some(o) = self.open.as_mut() {
                    o.ui.dropdown_open = true;
                    o.dd_hover = filters::index_of(&o.filter, &self.custom_filters);
                    self.dirty = true;
                }
            }
            Key::Char(c) => self.type_char(p, c),
            Key::Backspace => self.erase(p, false),
            Key::CtrlBackspace => self.erase(p, true),
            Key::CtrlA => {}
            // copies the chosen clips (or the one shown) without pasting them anywhere
            Key::CtrlC => {
                if !adding {
                    self.copy_picks(p, now);
                }
            }
            Key::CtrlP => self.toggle_pin(p, now),
            // Delete is destructive and irreversible; with text in the search box it is more
            // likely meant for the box (Ctrl+A, Delete), so it only acts on an empty box.
            Key::Delete => {
                if self.open.as_ref().is_some_and(|o| o.query.is_empty() && !o.adding_filter) {
                    self.delete_current(p, now);
                }
            }
            Key::CtrlDelete => {
                if !adding {
                    self.delete_current(p, now);
                }
            }
            Key::ShiftDelete => {
                if let (Some(id), false) = (self.current_id(), adding) {
                    self.ask_delete_forever(id);
                }
            }
            Key::CtrlZ => {
                if !adding {
                    self.undo_trash(p, now);
                }
            }
            Key::CtrlY => {}
            Key::CtrlE => {
                if !adding {
                    self.open_viewer(p, now);
                }
            }
            // keys for the viewer; the carousel has no use for them
            Key::Left
            | Key::Right
            | Key::Home
            | Key::End
            | Key::F3
            | Key::ShiftF3
            | Key::F11
            | Key::CtrlF
            | Key::CtrlI
            | Key::CtrlS => {}
            Key::PageUp | Key::PageDown => {
                let mut vm = self.view_model();
                let size = self.size_dip();
                let scale = self.overlay.scale;
                let page = geometry(&vm, size, scale).body.map_or(100.0, |b| b.h * scale * 0.9);
                let max = self.renderer.max_scroll(&vm, size, scale);
                vm.wipe();
                if let Some(o) = self.open.as_mut() {
                    let delta = if key == Key::PageDown { page } else { -page };
                    o.ui.scroll = (o.ui.scroll + delta).clamp(0.0, max);
                    self.dirty = true;
                }
            }
        }
    }

    // ------------------------------------------------------------------ mouse on the overlay

    fn mouse(&mut self, p: &mut dyn Platform, m: MouseEvent, now: u64) {
        if m.kind == MouseKind::Leave {
            if let Some(o) = self.open.as_mut() {
                if o.ui.hover.take().is_some() {
                    self.dirty = true;
                }
            }
            return;
        }
        // Outside a drag, a move only lights up the card's buttons: a cheap geometry test.
        if m.kind == MouseKind::Move && !self.open.as_ref().is_some_and(|o| o.ui.dragging) {
            let mut vm = self.view_model();
            let hover = pasteroll_render::button_at(&vm, self.size_dip(), self.overlay.scale, m.x, m.y);
            let menu_hover = vm
                .menu
                .as_ref()
                .and_then(|_| pasteroll_render::menu_item_at(&vm, self.size_dip(), self.overlay.scale, m.x, m.y));
            vm.wipe();
            if let Some(o) = self.open.as_mut() {
                if o.ui.hover != hover {
                    o.ui.hover = hover;
                    self.dirty = true;
                }
                if let Some(menu) = o.ui.menu.as_mut() {
                    if menu.hover != menu_hover {
                        menu.hover = menu_hover;
                        self.dirty = true;
                    }
                }
            }
            return;
        }
        let mut vm = self.view_model();
        self.mouse_with(p, m, now, &vm);
        vm.wipe();
    }

    fn mouse_with(&mut self, p: &mut dyn Platform, m: MouseEvent, now: u64, vm: &ViewModel) {
        let size = self.size_dip();
        let scale = self.overlay.scale;
        let (x, y) = (m.x, m.y);
        match m.kind {
            MouseKind::Down | MouseKind::DoubleClick => {
                let hit = self.renderer.hit_test(vm, size, scale, x, y);
                // Ctrl+click chooses a clip, a click on the right-click menu acts on it
                if self.multi_mouse(p, m, hit, scale, now) {
                    return;
                }
                // an open dropdown closes on any click that is not on one of its items
                if let Hit::DropdownItem(i) = hit {
                    self.dropdown_pick(p, i);
                    return;
                }
                if !matches!(hit, Hit::Delete) {
                    self.withdraw_delete_question();
                }
                match hit {
                    Hit::Expand => self.open_viewer(p, now),
                    Hit::Delete => {
                        let id = self.current_id();
                        let pending = self.open.as_ref().and_then(|o| o.ui.confirm_delete);
                        let forever = p.shift_held() || self.open.as_ref().is_some_and(Open::in_trash);
                        match (id, pending) {
                            (Some(id), Some(q)) if q == id => self.confirm_delete_forever(p, now),
                            (Some(id), _) if forever => self.ask_delete_forever(id),
                            (Some(_), _) => {
                                self.withdraw_delete_question();
                                self.delete_current(p, now);
                            }
                            _ => {}
                        }
                    }
                    Hit::Restore => self.undo_trash(p, now),
                    Hit::FilterButton => {
                        if let Some(o) = self.open.as_mut() {
                            o.ui.dropdown_open = !o.ui.dropdown_open;
                            o.dd_hover = filters::index_of(&o.filter, &self.custom_filters);
                            self.dirty = true;
                        }
                    }
                    Hit::Row(i) => self.activate_row(p, i, vm),
                    Hit::Body { pos } => self.body_press(vm, m.kind, pos, now),
                    _ => {
                        if let Some(o) = self.open.as_mut() {
                            if o.ui.dropdown_open {
                                o.ui.dropdown_open = false;
                                self.dirty = true;
                            }
                        }
                    }
                }
            }
            MouseKind::RightDown => {
                let hit = self.renderer.hit_test(vm, size, scale, x, y);
                self.multi_mouse(p, m, hit, scale, now);
            }
            MouseKind::Move => {
                let dragging = self.open.as_ref().is_some_and(|o| o.ui.dragging);
                if dragging {
                    self.drag_to(vm, size, scale, x, y);
                }
            }
            MouseKind::Up => {
                if let Some(o) = self.open.as_mut() {
                    o.ui.dragging = false;
                }
            }
            MouseKind::Leave => {}
        }
    }

    fn activate_row(&mut self, p: &mut dyn Platform, i: usize, vm: &ViewModel) {
        // Clicking a neighbouring row makes it the active one.
        let delta = i as isize - vm.active as isize;
        for _ in 0..delta.unsigned_abs() {
            if self.open.is_none() {
                return;
            }
            self.step(p, delta > 0);
        }
    }

    fn body_press(&mut self, vm: &ViewModel, kind: MouseKind, pos: usize, now: u64) {
        self.withdraw_warning();
        let Body::Text { display, .. } = &vm.body else { return };
        let units: Vec<u16> = display.encode_utf16().collect();
        let Some(o) = self.open.as_mut() else { return };
        let triple = kind == MouseKind::Down
            && o.ui.last_dbl_ms.is_some_and(|t| now.saturating_sub(t) <= DOUBLE_CLICK_MS);
        if kind == MouseKind::DoubleClick {
            o.ui.last_dbl_ms = Some(now);
            let s = Selection::word_at(&units, pos);
            o.ui.selection = (!s.is_empty()).then(|| s.range());
            o.ui.dragging = false;
        } else if triple {
            o.ui.last_dbl_ms = None;
            let s = Selection::line_at(&units, pos);
            o.ui.selection = (!s.is_empty()).then(|| s.range());
            o.ui.dragging = false;
        } else {
            let pos = pasteroll_core::select::snap(&units, pos);
            o.ui.anchor = Some(pos);
            o.ui.selection = None;
            o.ui.dragging = true;
        }
        self.dirty = true;
    }

    fn drag_to(&mut self, vm: &ViewModel, size: (f32, f32), scale: f32, x: f32, y: f32) {
        let g = geometry(vm, size, scale);
        let max = self.renderer.max_scroll(vm, size, scale);
        let Some(o) = self.open.as_mut() else { return };
        if let Some(b) = g.body {
            // Dragging past the top or bottom scrolls toward the pointer.
            let (top, bottom) = (b.y * scale, (b.y + b.h) * scale);
            if y > bottom {
                o.ui.scroll = (o.ui.scroll + (y - bottom) * 0.5).min(max);
            } else if y < top {
                o.ui.scroll = (o.ui.scroll - (top - y) * 0.5).max(0.0);
            }
        }
        // Positions are computed with the scroll that will be drawn.
        let mut vm2 = vm.clone();
        if let Body::Text { scroll, .. } = &mut vm2.body {
            *scroll = o.ui.scroll;
        }
        let pos = self.renderer.text_position(&vm2, size, scale, x, y);
        if let (Some(o), Some(pos)) = (self.open.as_mut(), pos) {
            if let BodyState::Text { san, .. } = &o.ui.body {
                let units: Vec<u16> = san.display.encode_utf16().collect();
                let pos = pasteroll_core::select::snap(&units, pos);
                if let Some(a) = o.ui.anchor {
                    o.ui.selection = (a != pos).then_some((a.min(pos), a.max(pos)));
                }
            }
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------ paste

    /// Whole clip, or just the selected part if the user selected one.
    fn confirm(&mut self, p: &mut dyn Platform) {
        let Some(open) = self.open.as_ref() else { return };
        if !open.ui.picks.is_empty() {
            self.confirm_picks(p);
            return;
        }
        let Some(id) = open.current_id() else {
            return; // nothing matches the search: Enter has nothing to paste
        };
        let warned = open.ui.warned;
        let selection = self.selected_text();
        if let PasteCheck::Warn(lines) = self.paste_item(p, id, selection, warned) {
            if let Some(o) = self.open.as_mut() {
                o.ui.warned = true;
            }
            // Stays up until the user decides: Enter to paste, anything else to back out.
            self.status = Some(Status {
                text: format!("{lines} line(s) into a terminal may run commands. Enter pastes, Esc cancels."),
                until_ms: u64::MAX,
            });
            self.dirty = true;
        }
    }

    /// Writes `payload` to the clipboard and sends `target` its paste keystroke, with the last
    /// checks every paste gets. The key that was sent, or `None` when nothing was pasted.
    fn deliver(&mut self, p: &mut dyn Platform, target: &Target, payload: &Payload) -> Option<PasteKey> {
        // Alt/Win held at this moment would turn the paste chord into something else: wait
        // briefly, then give up rather than send a wrong chord.
        if !p.modifiers_released(Duration::from_millis(300)) {
            p.log("Alt/Win still held; not pasting");
            return None;
        }
        // Write the clipboard first (it can take a moment if another process holds it), then
        // re-check the destination as late as possible before the keystroke.
        if let Err(e) = p.write_clipboard(payload) {
            p.log(&format!("could not write clipboard: {e}"));
            return None;
        }
        if !p.target_is_current(target) {
            p.log("foreground changed while preparing the paste; not pasting");
            return None;
        }
        let key = self.paste_keys.for_exe(&target.exe);
        if !p.send_paste(key) {
            p.log("paste keystroke was blocked by the system; is the target elevated?");
            return None;
        }
        Some(key)
    }

    /// Pastes clip `id` (or `selection`, part of it) into the session's target. A multi-line text
    /// into a terminal is not pasted until `warned` says the user has been told.
    fn paste_item(&mut self, p: &mut dyn Platform, id: i64, selection: Option<String>, warned: bool) -> PasteCheck {
        let Some(open) = self.open.as_ref() else { return PasteCheck::Done };
        if open.browse_only {
            self.status = Some(Status { text: "Click a destination window to paste.".into(),
                until_ms: p.now_ms() + NOTICE_MS });
            self.dirty = true;
            return PasteCheck::Done;
        }
        let target = open.target.clone();
        if target.desktop {
            self.cancel(p, "Enter does not paste onto the desktop (click it to save an image)");
            return PasteCheck::Done;
        }
        if !p.target_is_current(&target) {
            self.cancel(p, "foreground changed; not pasting");
            return PasteCheck::Done;
        }
        if p.target_privileged(&target) {
            self.cancel(p, "target runs at a higher privilege level; not pasting");
            return PasteCheck::Done;
        }
        let (payload, whole) = match selection {
            Some(t) => (Payload::Text(t), false),
            None => {
                match self.clip_payload(id) {
                    Some(pl) => (pl, true),
                    None => {
                        self.cancel(p, "clip could not be read");
                        return PasteCheck::Done;
                    }
                }
            }
        };
        let payload = WipeOnDrop(payload);
        let payload = &payload.0;
        if let Payload::Text(t) = payload {
            if !warned && self.paste_policy.needs_warning(&target.exe, &target.class, t) {
                return PasteCheck::Warn(t.lines().count().max(1));
            }
        }
        self.end_session(p);
        let Some(key) = self.deliver(p, &target, payload) else { return PasteCheck::Done };
        if whole {
            let now = p.now_secs();
            let mut st = self.store.borrow_mut();
            // pasting a clip from the recycle bin brings it back
            if st.is_trashed(id).unwrap_or(false) {
                let _ = st.restore(id);
            }
            let _ = st.mark_pasted(id, now, true);
        }
        let what = match (payload, whole) {
            (Payload::Text(t), true) => format!("whole clip ({} chars)", t.chars().count()),
            (Payload::Text(t), false) => format!("selection ({} chars)", t.chars().count()),
            (Payload::Files(f), _) => format!("{} file(s)", f.len()),
            (Payload::Image(_), _) => "image".to_owned(),
        };
        p.log(&format!("pasted {what} with {key:?}"));
        PasteCheck::Done
    }

    /// Saves the image candidate as a new PNG on the Desktop. Not a clipboard operation: nothing
    /// is loaded onto the clipboard and history is not reordered.
    fn save_desktop_image(&mut self, p: &mut dyn Platform) {
        let Some(id) = self.current_id() else { return };
        let png = self.store.borrow().image_png(id);
        let Ok(Some(png)) = png else {
            self.cancel(p, "the candidate is not an image");
            return;
        };
        // Re-encode: another program's PNG may carry text chunks, metadata or trailing data, and
        // the Desktop is often synced to the cloud.
        let Some(png) = pasteroll_core::image::decode_png(&png).and_then(|i| pasteroll_core::image::encode_png(&i))
        else {
            self.cancel(p, "the image could not be decoded");
            return;
        };
        self.end_session(p);
        let Some(folder) = p.desktop_folder() else {
            p.log("could not locate the Desktop folder");
            return;
        };
        let stem = format!("Pasteroll image {}", p.local_timestamp());
        match crate::files::save_unique_png(&folder, &stem, &png) {
            Ok(path) => {
                p.file_created(&path);
                p.log(&format!(
                    "saved image to the Desktop as {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            Err(e) => p.log(&format!("could not save the image to the Desktop: {e}")),
        }
    }

    // ------------------------------------------------------------------ timer

    fn tick(&mut self, p: &mut dyn Platform, now: u64) {
        p.heartbeat();
        self.on_new_clips(p);
        if self.viewer.is_some() {
            self.viewer_tick(p, now);
        }
        if self.editor.is_some() {
            self.editor_tick(p, now);
        }
        if let Some(n) = &self.notice {
            if now >= n.until_ms {
                self.notice = None;
                if self.open.is_none() {
                    p.overlay_hide();
                }
            } else if self.open.is_none() {
                self.dirty = true;
            }
        }
        if self.status.as_ref().is_some_and(|st| now >= st.until_ms) {
            self.status = None;
            self.dirty = true;
        }
        if self.last_maintain_ms.is_none_or(|t| now.saturating_sub(t) >= MAINTAIN_EVERY_MS) {
            self.last_maintain_ms = Some(now);
            // Expiry must not depend on new copies arriving (capture may even be off).
            if let Err(e) = self.store.borrow_mut().maintain(p.now_secs()) {
                p.log(&format!("maintenance failed: {e}"));
            }
        }
        let Some(open) = self.open.as_ref() else { return };
        if self.viewer.is_some() || self.editor.is_some() {
            // reading takes time: the viewer has its own, longer limit, and keeps the hooks alive
            if now.saturating_sub(self.last_activity_ms) > VIEWER_IDLE_MS {
                self.cancel(p, "idle timeout");
                return;
            }
            p.input_keepalive();
        } else {
            if now.saturating_sub(open.opened_ms) > SESSION_MAX_MS {
                self.cancel(p, "session time limit");
                return;
            }
            if now.saturating_sub(self.last_activity_ms) > IDLE_CANCEL_MS {
                self.cancel(p, "idle timeout");
                return;
            }
        }
        match &open.ui.click {
            None => {
                // The foreground is not swallowed: if the user leaves the target, stop swallowing.
                let target = open.target.clone();
                // (the image editor stands on the desktop, whatever program the session began in)
                if self.editor.is_none() {
                    let current = if open.browse_only { p.browse_focus_current() }
                        else { p.target_is_current(&target) };
                    if !current { self.cancel(p, "foreground changed"); }
                }
            }
            Some(_) => self.poll_click(p, now),
        }
    }

    /// Paste-at-click: after the user's click has landed, wait until the app has settled and the
    /// click was on a client area, then paste.
    fn poll_click(&mut self, p: &mut dyn Platform, now: u64) {
        let Some((since, first)) = self
            .open
            .as_mut()
            .and_then(|o| o.ui.click.as_mut().map(|c| (c.since_ms, std::mem::replace(&mut c.first, false))))
        else {
            return;
        };
        if now.saturating_sub(since) > CLICK_WINDOW_MS {
            self.cancel(p, "click paste timed out");
            return;
        }
        // Ctrl+click (by default) on a picture opens the editor, which types nothing yet
        let editor_click = self.current_is_image()
            && self.picks_allow_editor()
            && p.click_modifiers().has(self.advanced_modifier);
        let polled = if editor_click { p.poll_click_editor(first) } else { p.poll_click(first) };
        match polled {
            ClickPoll::Pending => {}
            ClickPoll::Rejected(why) => self.cancel(p, why),
            ClickPoll::Desktop => {
                if self.current_is_image() {
                    if p.click_modifiers().has(self.advanced_modifier) {
                        // Ctrl+click (by default): the editor, instead of saving at once
                        self.open_editor(p, now, None);
                    } else {
                        self.save_desktop_image(p);
                    }
                } else {
                    self.cancel(p, "only images can be saved to the desktop");
                }
            }
            ClickPoll::Target(t) => {
                // Ctrl+click (by default) with a picture: the editor first, then the result is
                // pasted into the window that was clicked
                if self.current_is_image() && self.picks_allow_editor() && p.click_modifiers().has(self.advanced_modifier) {
                    self.status = None;
                    self.open_editor(p, now, Some(t));
                    return;
                }
                if let Some(o) = self.open.as_mut() {
                    if o.target.id != t.id {
                        // a warning given for another window does not carry over
                        o.ui.warned = false;
                    }
                    o.target = t;
                    o.browse_only = false;
                    o.ui.click = None;
                }
                self.status = None;
                self.confirm(p);
            }
        }
    }

    /// The clip the carousel is on is a picture.
    fn current_is_image(&self) -> bool {
        self.current_id().and_then(|id| self.store.borrow().kind(id).ok().flatten()) == Some(Kind::Image)
    }

    // ------------------------------------------------------------------ rendering

    fn row_info(&mut self, id: i64) -> (RowKind, bool, String, String) {
        if !self.row_cache.contains_key(&id) {
            if self.row_cache.len() > ROW_CACHE_MAX {
                self.row_cache.clear();
            }
            let info = match self.store.borrow().info(id) {
                Ok(Some(i)) => match i.kind {
                    Kind::Image => RowInfo {
                        kind: RowKind::Image,
                        pinned: i.pinned,
                        title: if i.width > 0 { format!("Image {} x {}", i.width, i.height) } else { "Image".into() },
                        meta: if i.width > 0 { format!("Image, {} x {}", i.width, i.height) } else { "Image".into() },
                    },
                    Kind::Files => {
                        let files: Vec<&str> = i.search.split('\n').filter(|s| !s.is_empty()).collect();
                        let title = match files.as_slice() {
                            [] => "Files".to_owned(),
                            [one] => file_name(one).to_owned(),
                            [first, rest @ ..] => {
                                format!("{} +{} more", file_name(first), rest.len().max(i.files.saturating_sub(1)))
                            }
                        };
                        RowInfo { kind: RowKind::Files, pinned: i.pinned, title, meta: format!("{} file(s)", i.files) }
                    }
                    Kind::Text => RowInfo {
                        kind: RowKind::Text,
                        pinned: i.pinned,
                        title: i.search,
                        meta: format!("Text, {} characters", thousands(i.total_chars)),
                    },
                },
                _ => RowInfo { kind: RowKind::Text, pinned: false, title: "(unreadable)".into(), meta: String::new() },
            };
            self.row_cache.insert(id, info);
        }
        let i = &self.row_cache[&id];
        (i.kind, i.pinned, i.title.clone(), i.meta.clone())
    }

    pub fn view_model(&mut self) -> ViewModel {
        let mut vm = ViewModel::default();
        if let Some(n) = &self.notice {
            if self.open.is_none() {
                vm.body = Body::Message(n.text.clone());
                return vm;
            }
        }
        let Some(open) = self.open.as_ref() else { return vm };
        let (ids, active) = self.row_ids();
        let picks = open.ui.picks.clone();
        let menu_state = open.ui.menu;
        vm.active = active;
        vm.slim = self.ui_mode == UiMode::NoBloat;
        vm.card_buttons = open.session.is_some() && !vm.slim;
        // No Bloat shows the search bar and filter only while they are in use
        vm.slim_bar = vm.slim
            && (!open.query.is_empty()
                || open.adding_filter
                || open.filter != Filter::Everything
                || open.ui.dropdown_open);
        vm.in_trash = open.in_trash();
        vm.hover = open.ui.hover;
        let (selection, scroll, dropdown_open) =
            (open.ui.selection, open.ui.scroll, open.ui.dropdown_open);
        let (query, adding, hover, filter) =
            (open.query.clone(), open.adding_filter, open.dd_hover, open.filter.clone());
        let counts = (!open.is_all() && !open.adding_filter).then_some((open.matched, open.total));
        let words = search::terms(open.search_text());
        vm.body = match &open.ui.body {
            BodyState::Text { san, .. } => Body::Text { display: san.display.clone(), selection, scroll },
            BodyState::Image(img) => Body::Image(img.clone()),
            BodyState::Message(m) => Body::Message(m.clone()),
        };
        if let (BodyState::Text { san, .. }, false) = (&open.ui.body, words.is_empty()) {
            vm.body_matches = utf16_ranges(
                &san.display,
                &search::highlight_ranges_limited(&san.display, &words, usize::MAX, 300),
            );
        }
        vm.query = query;
        vm.filter_label = filters::label(&filter, &self.custom_filters);
        if adding {
            vm.placeholder = "Name, then extensions: Design psd ai".into();
            vm.footer = "Enter saves the filter. Esc cancels.".into();
        }
        if dropdown_open {
            vm.dropdown = Some(Dropdown {
                items: filters::labels(&self.custom_filters),
                selected: filters::index_of(&filter, &self.custom_filters),
                hover: Some(hover),
            });
        }
        if let Some((m, t)) = counts {
            vm.footer = format!("{} of {} clips", thousands(m), thousands(t));
        }
        if !picks.is_empty() && !vm.slim {
            vm.footer = multi::footer_text(picks.len());
        }
        if let Some(st) = &self.status {
            vm.footer = st.text.clone();
        }
        let words = if adding { Vec::new() } else { search::terms(&vm.query) };
        if !vm.slim {
            vm.picked = ids.iter().map(|id| picks.contains(id)).collect();
            vm.menu = menu_state.map(|m| m.view(picks.len()));
        }
        for id in ids {
            let (kind, pinned, title, meta) = self.row_info(id);
            if vm.slim {
                // the clip is drawn from `body`: its row keeps only the pin marker
                vm.rows.push(RowView { kind, pinned, title: String::new(), meta: String::new(), highlights: Vec::new() });
                continue;
            }
            let highlights = search::highlight_ranges(&title, &words);
            vm.rows.push(RowView { kind, pinned, title, meta, highlights });
        }
        vm
    }

    fn present(&mut self, p: &mut dyn Platform) {
        if self.editor.is_some() {
            self.present_editor(p);
            return;
        }
        if self.viewer.is_some() {
            self.present_viewer(p);
            return;
        }
        if self.open.is_none() && self.notice.is_none() {
            return;
        }
        let mut vm = self.view_model();
        let frame = self.renderer.render(&vm, self.size_dip(), self.overlay.scale);
        vm.wipe();
        p.overlay_present(&frame);
    }
}
