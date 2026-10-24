//! The expanded viewer: the active card grows into a large window of its own, and back.
//!
//! - It is modal: while it is open the carousel underneath is drawn dimmed, with the card's place
//!   empty, and takes no input. The viewer browses a *fork* of the carousel's session, so stepping
//!   through clips here never moves the carousel; closing it shrinks the card back into the exact
//!   place, selection and scroll it came from.
//! - Opening, closing and fullscreen are short morphs between two frames (the card as it looked and
//!   the viewer as it will look), stretched over a window that moves and grows every tick. Nothing
//!   is laid out mid-animation, so it stays smooth on large screens.
//! - Everything slow (decoding the full picture, text recognition) runs on worker threads.

use super::*;
use crate::event::Wheel;
use crate::platform::SaveChoice;
use pasteroll_core::preview::Sanitized;
use pasteroll_core::store::ClipInfo;
use pasteroll_render::{
    fit_zoom, viewer_geometry, ConfirmView, FindView, Frame, MenuItemView, Morph, Tool, ToolView, VHit, ViewerBody,
    ViewerView,
};

pub(super) const OPEN_MS: u64 = 260;
pub(super) const CLOSE_MS: u64 = 230;
const RESIZE_MS: u64 = 210;
/// How dark the carousel gets behind the viewer (1 = unchanged).
pub(super) const DIM: f32 = 0.42;
/// The viewer shows this much of a very long text (Copy, Paste and Save as still use all of it).
const VIEWER_MAX_CHARS: usize = 200_000;
const TOAST_MS: u64 = 1_800;
const ZOOM_STEP: f32 = 1.25;
const MAX_MATCHES: usize = 5_000;
const SCROLL_LINES: f32 = 3.0;
const LINE_DIP: f32 = 19.0;
/// Animation frames are drawn with at most this many pixels and stretched by the window: a frame
/// the size of a large screen takes too long to blend for a smooth animation.
pub(super) const ANIM_MAX_PIXELS: f32 = 600_000.0;

type RectPx = (i32, i32, u32, u32);

pub(crate) struct OcrDone {
    pub id: i64,
    pub request: u64,
    pub result: Result<String, String>,
}

impl Drop for OcrDone {
    fn drop(&mut self) {
        if let Ok(text) = &mut self.result {
            use zeroize::Zeroize;
            text.zeroize();
        }
    }
}

struct OcrJob(Arc<std::sync::atomic::AtomicUsize>);
impl OcrJob {
    fn acquire(jobs: &Arc<std::sync::atomic::AtomicUsize>) -> Option<Self> {
        use std::sync::atomic::Ordering;
        let mut count = jobs.load(Ordering::Acquire);
        loop {
            if count >= 2 { return None; }
            match jobs.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(Self(jobs.clone())),
                Err(current) => count = current,
            }
        }
    }
}
impl Drop for OcrJob {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Release);
    }
}

enum Content {
    /// A text clip, or a files clip shown as its list of paths.
    Text { san: Sanitized, orig: Vec<u16>, files: bool, chars: usize, lines: usize, words: usize, bytes: usize },
    Image { thumb: Option<Arc<core_image::Rgba>>, full: Option<Arc<core_image::Rgba>>, w: u32, h: u32 },
    Message(String),
}

impl Drop for Content {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        match self {
            Content::Text { san, orig, .. } => {
                san.display.zeroize();
                orig.zeroize();
            }
            Content::Message(m) => m.zeroize(),
            Content::Image { .. } => {}
        }
    }
}

/// Text recognised in the picture, shown instead of it until Esc.
struct OcrText {
    san: Sanitized,
    orig: Vec<u16>,
}

impl Drop for OcrText {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.san.display.zeroize();
        self.orig.zeroize();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AnimKind {
    Open,
    Close,
    Resize,
}

pub(super) struct Anim {
    pub(super) kind: AnimKind,
    pub(super) start_ms: u64,
    pub(super) dur_ms: u64,
    pub(super) from: RectPx,
    pub(super) to: RectPx,
    pub(super) morph: Morph,
    pub(super) dim: (f32, f32),
}

/// Entries of the More menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    SaveAs,
    Info,
    ResetZoom,
    ExtractText,
    ClearOcrRegions,
    ShowPicture,
    Restore,
    DeleteForever,
}

enum Confirm {
    DeleteForever(i64),
    TerminalPaste(usize),
}

struct Find {
    query: String,
    matches: Vec<(usize, usize)>,
    current: Option<usize>,
}

impl Drop for Find {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.query.zeroize();
    }
}

pub(crate) struct Viewer {
    session: Session<Src>,
    /// Browsing the recycle bin.
    trash: bool,
    id: i64,
    info: Option<ClipInfo>,
    info_lines: Vec<(String, String)>,
    content: Content,
    ocr_text: Option<OcrText>,
    ocr_running: Option<u64>,
    can_ocr: bool,
    ocr_selecting: bool,
    ocr_regions: Vec<Vec<(f32, f32)>>,
    ocr_stroke: Vec<(f32, f32)>,
    /// Changes with the text shown (the renderer caches the shaped text by it).
    gen: u64,
    full_gen: u64,
    pub(crate) rect: RectPx,
    home: RectPx,
    monitor: RectPx,
    pub(crate) fullscreen: bool,
    anim: Option<Anim>,
    /// The carousel with the card's place left empty, undimmed.
    lifted: Frame,
    dim: f32,
    last_frame: Option<Frame>,
    /// Size of the text and of the area showing it, as of the last frame (px).
    extent: Option<((f32, f32), (f32, f32))>,
    hover: Option<VHit>,
    menu: Option<Vec<Action>>,
    menu_hover: Option<usize>,
    info_open: bool,
    find: Option<Find>,
    confirm: Option<Confirm>,
    toast: Option<(String, u64)>,
    wrap: bool,
    selection: Option<(usize, usize)>,
    anchor: Option<usize>,
    selecting: bool,
    last_dbl_ms: Option<u64>,
    scroll: (f32, f32),
    zoom: Option<f32>,
    center: (f32, f32),
    /// Pointer (px) and picture centre when a pan drag began.
    pan: Option<((f32, f32), (f32, f32))>,
}

pub(super) fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub(super) fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub(super) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub(super) fn lerp_rect(a: RectPx, b: RectPx, t: f32) -> RectPx {
    (
        lerp(a.0 as f32, b.0 as f32, t).round() as i32,
        lerp(a.1 as f32, b.1 as f32, t).round() as i32,
        lerp(a.2 as f32, b.2 as f32, t).round().max(1.0) as u32,
        lerp(a.3 as f32, b.3 as f32, t).round().max(1.0) as u32,
    )
}

/// 75% of the monitor, centred in its free area.
pub(crate) fn home_rect(info: &OverlayInfo) -> RectPx {
    let (mx, my, mw, mh) = info.monitor_px;
    let (wx, wy, ww, wh) = if info.work_px.2 > 0 && info.work_px.3 > 0 { info.work_px } else { (mx, my, mw, mh) };
    let w = ((mw as f32 * 0.75).round() as u32).min(ww).max(200);
    let h = ((mh as f32 * 0.75).round() as u32).min(wh).max(150);
    (wx + (ww as i32 - w as i32) / 2, wy + (wh as i32 - h as i32) / 2, w, h)
}

/// Character ranges where `query` occurs in `text`, ignoring case, as UTF-16 ranges.
fn find_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let q: Vec<char> = query.chars().map(fold).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let t: Vec<char> = text.chars().map(fold).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= t.len() && out.len() < MAX_MATCHES {
        if t[i..i + q.len()] == q[..] {
            out.push((i, i + q.len()));
            i += q.len();
        } else {
            i += 1;
        }
    }
    utf16_ranges(text, &out)
}

fn human_bytes(n: usize) -> String {
    let n = n as f64;
    if n < 1024.0 {
        format!("{n} bytes")
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else {
        format!("{:.1} MB", n / (1024.0 * 1024.0))
    }
}

impl Viewer {
    fn size_dip(&self, scale: f32) -> (f32, f32) {
        let s = scale.max(0.1);
        (self.rect.2 as f32 / s, self.rect.3 as f32 / s)
    }

    fn animating(&self) -> bool {
        self.anim.is_some()
    }

    /// The text being shown (the recognised text instead of a picture, when there is some).
    fn shown_text(&self) -> Option<(&Sanitized, &[u16])> {
        if let Some(o) = &self.ocr_text {
            return Some((&o.san, &o.orig));
        }
        match &self.content {
            Content::Text { san, orig, .. } => Some((san, orig)),
            _ => None,
        }
    }

    fn is_picture(&self) -> bool {
        matches!(self.content, Content::Image { .. }) && self.ocr_text.is_none()
    }

    fn picture(&self) -> Option<(Option<Arc<core_image::Rgba>>, u32, u32)> {
        match &self.content {
            Content::Image { thumb, full, w, h } => Some((full.clone().or_else(|| thumb.clone()), *w, *h)),
            _ => None,
        }
    }

    fn pinned(&self) -> bool {
        self.info.as_ref().is_some_and(|i| i.pinned && i.trashed.is_none())
    }

    fn toast(&mut self, text: impl Into<String>, now: u64) {
        self.toast = Some((text.into(), now + TOAST_MS));
    }

    fn menu_actions(&self, has_ocr: bool) -> Vec<Action> {
        let mut v = vec![Action::SaveAs, Action::Info];
        if matches!(self.content, Content::Image { .. }) {
            if self.ocr_text.is_some() {
                v.push(Action::ShowPicture);
            } else {
                v.push(Action::ResetZoom);
                if has_ocr {
                    v.push(Action::ExtractText);
                    if self.ocr_selecting {
                        v.push(Action::ClearOcrRegions);
                    }
                }
            }
        }
        if self.trash {
            v.push(Action::Restore);
        }
        v.push(Action::DeleteForever);
        v
    }

    fn menu_item(&self, a: Action) -> MenuItemView {
        let (label, shortcut, danger) = match a {
            Action::SaveAs => ("Save as...".to_owned(), "Ctrl+S", false),
            Action::Info => ((if self.info_open { "Hide info" } else { "Info" }).to_owned(), "Ctrl+I", false),
            Action::ResetZoom => ("Reset zoom".to_owned(), "0", false),
            Action::ExtractText => (
                (if self.ocr_running.is_some() { "Reading text..." }
                 else if self.ocr_selecting { "Extract selected text" }
                 else { "Extract text (OCR)" }).to_owned(),
                "",
                false,
            ),
            Action::ShowPicture => ("Show the picture".to_owned(), "Esc", false),
            Action::ClearOcrRegions => ("Clear OCR areas".to_owned(), "Del", false),
            Action::Restore => ("Restore to history".to_owned(), "Ctrl+Z", false),
            Action::DeleteForever => ("Delete permanently".to_owned(), "Shift+Del", true),
        };
        let enabled = match a {
            Action::ExtractText => self.ocr_running.is_none() && (!self.ocr_selecting || !self.ocr_regions.is_empty()),
            Action::ClearOcrRegions => self.ocr_running.is_none() && !self.ocr_regions.is_empty(),
            _ => true,
        };
        MenuItemView { label, shortcut: shortcut.to_owned(), enabled, danger }
    }

    fn view(&self, target_exe: &str, scale: f32) -> ViewerView {
        let kind = match (&self.content, self.ocr_text.is_some()) {
            (Content::Image { .. }, false) => RowKind::Image,
            (Content::Text { files: true, .. }, _) => RowKind::Files,
            _ => RowKind::Text,
        };
        let title = match (&self.content, &self.ocr_text) {
            (_, Some(_)) => "Text from the picture".to_owned(),
            (Content::Text { files: true, lines, .. }, _) => format!("{} file(s)", thousands(*lines)),
            (Content::Text { chars, .. }, _) => format!("Text, {} characters", thousands(*chars)),
            (Content::Image { w, h, .. }, _) if *w > 0 => format!("Image, {w} x {h}"),
            (Content::Image { .. }, _) => "Image".to_owned(),
            (Content::Message(_), _) => "Clip".to_owned(),
        };
        let mut sub = Vec::new();
        if let Some(i) = &self.info {
            if i.trashed.is_some() {
                sub.push("In the recycle bin".to_owned());
            }
            if i.pinned && i.trashed.is_none() {
                sub.push("Pinned".to_owned());
            }
            if !i.source.is_empty() {
                sub.push(format!("from {}", i.source));
            }
        }
        let body = if let Some((san, _)) = self.shown_text() {
            let (matches, current) = match &self.find {
                Some(f) => (f.matches.clone(), f.current),
                None => (Vec::new(), None),
            };
            ViewerBody::Text {
                display: san.display.clone(),
                gen: self.gen,
                selection: self.selection,
                scroll: self.scroll,
                wrap: self.wrap,
                matches,
                current,
            }
        } else {
            match &self.content {
                Content::Image { .. } => {
                    let (img, w, h) = self.picture().unwrap_or((None, 0, 0));
                    ViewerBody::Image { img, size: (w, h), zoom: self.zoom, center: self.center }
                }
                Content::Message(m) => ViewerBody::Message(m.clone()),
                Content::Text { .. } => ViewerBody::Message(String::new()),
            }
        };
        if self.ocr_selecting && self.is_picture() {
            sub.clear();
            sub.push(format!("Draw around text; Enter extracts; Ctrl+Z undoes; Esc cancels. {} area(s)", self.ocr_regions.len()));
        }
        let picture = self.is_picture();
        let paste_tip = if target_exe.is_empty() { "Paste (Enter)".to_owned() } else { format!("Paste into {target_exe} (Enter)") };
        let t = |tool: Tool, on: bool, enabled: bool, tip: &str| ToolView { tool, on, enabled, tip: tip.to_owned() };
        let mut tools = vec![t(Tool::Prev, false, true, "Previous clip (Left)"), t(Tool::Next, false, true, "Next clip (Right)")];
        if picture {
            let fitted = self.zoom.is_none();
            tools.push(t(Tool::ZoomOut, false, true, "Zoom out (-)"));
            tools.push(t(Tool::FitActual, !fitted, true, if fitted { "Actual size (1)" } else { "Fit to window (0)" }));
            tools.push(t(Tool::ZoomIn, false, true, "Zoom in (+)"));
        }
        let has_clip = !matches!(self.content, Content::Message(_));
        if picture && self.can_ocr {
            if self.ocr_selecting {
                tools.retain(|t| matches!(t.tool, Tool::ZoomIn | Tool::ZoomOut | Tool::FitActual));
            }
            tools.push(t(Tool::OcrSelect, self.ocr_selecting, self.ocr_running.is_none(),
                "Circle text for OCR (Esc cancels)"));
            if self.ocr_selecting {
                tools.push(t(Tool::OcrRead, false, !self.ocr_regions.is_empty() && self.ocr_running.is_none(),
                    "Extract selected text (Enter)"));
                tools.push(t(Tool::OcrUndo, false, !self.ocr_regions.is_empty() && self.ocr_running.is_none(),
                    "Undo last OCR area (Ctrl+Z)"));
            }
        }
        tools.push(t(Tool::Copy, false, has_clip, "Copy (Ctrl+C)"));
        tools.push(ToolView { tool: Tool::Paste, on: false, enabled: has_clip, tip: paste_tip });
        if self.trash {
            tools.push(t(Tool::Pin, false, false, "Restore it first to pin it"));
            tools.push(t(Tool::Restore, false, true, "Restore to history (Ctrl+Z)"));
        } else {
            let pinned = self.pinned();
            tools.push(t(Tool::Pin, pinned, true, if pinned { "Unpin (Ctrl+P)" } else { "Pin: keep it until you unpin it (Ctrl+P)" }));
            tools.push(t(Tool::Delete, false, true, "Move to recycle bin (Del)"));
        }
        if self.shown_text().is_some() {
            tools.push(t(Tool::Search, self.find.is_some(), true, "Find in clip (Ctrl+F)"));
            tools.push(t(Tool::Wrap, self.wrap, true, if self.wrap { "Word wrap: on" } else { "Word wrap: off" }));
        }
        tools.push(t(Tool::More, false, true, "More"));
        tools.push(t(Tool::Close, false, true, "Close (Esc)"));
        if self.ocr_selecting {
            tools.retain(|t| !matches!(t.tool, Tool::Copy | Tool::Paste | Tool::Pin | Tool::Delete | Tool::Restore));
        }
        let zoom_label = match (picture, self.picture()) {
            (true, Some((_, w, h))) if w > 0 && h > 0 => {
                // percent of real pixels: DIP per image pixel times device pixels per DIP
                let probe = ViewerView {
                    body: ViewerBody::Image { img: None, size: (w, h), zoom: None, center: (0.0, 0.0) },
                    info: self.info_open.then(Vec::new),
                    tools: tools.clone(),
                    ..ViewerView::default()
                };
                let g = viewer_geometry(&probe, self.size_dip(scale));
                let z = self.zoom.unwrap_or_else(|| fit_zoom(&g, w, h));
                let pct = (z * scale * 100.0).round();
                if self.zoom.is_none() {
                    format!("Fit, {pct}%")
                } else {
                    format!("{pct}%")
                }
            }
            _ => String::new(),
        };
        let menu = self.menu.as_ref().map(|items| items.iter().map(|a| self.menu_item(*a)).collect());
        ViewerView {
            kind,
            title,
            subtitle: sub.join("  \u{b7}  "),
            fullscreen: self.fullscreen,
            body,
            tools,
            hover: self.hover,
            menu,
            menu_hover: self.menu_hover,
            info: self.info_open.then(|| self.info_lines.clone()),
            find: self.find.as_ref().map(|f| FindView { query: f.query.clone(), count: f.matches.len(), current: f.current }),
            confirm: self.confirm.as_ref().map(|c| match c {
                Confirm::DeleteForever(_) => ConfirmView {
                    title: "Delete permanently?".into(),
                    message: "This clip will be removed for good. It does not go to the recycle bin and cannot be brought back.".into(),
                    ok: "Delete".into(),
                    danger: true,
                },
                Confirm::TerminalPaste(lines) => ConfirmView {
                    title: "Paste into a terminal?".into(),
                    message: format!("{lines} line(s) pasted into a terminal may run as commands."),
                    ok: "Paste".into(),
                    danger: false,
                },
            }),
            toast: self.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default(),
            zoom_label,
            ocr_selection: (picture && self.ocr_selecting).then(|| pasteroll_render::OcrSelectionView {
                regions: self.ocr_regions.clone(), drawing: self.ocr_stroke.clone(),
            }),
        }
    }
}

impl Controller {
    // ------------------------------------------------------------------ queries (tests, platforms)

    pub fn viewer_is_open(&self) -> bool {
        self.viewer.is_some()
    }

    /// The clip the viewer shows.
    pub fn viewer_clip(&self) -> Option<i64> {
        self.viewer.as_ref().map(|v| v.id)
    }

    pub fn viewer_fullscreen(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| v.fullscreen)
    }

    pub fn viewer_animating(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| v.animating())
    }

    /// The viewer window's rectangle in screen pixels.
    pub fn viewer_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.viewer.as_ref().map(|v| v.rect)
    }

    /// What the viewer shows right now (tests aim input with it).
    pub fn viewer_view(&self) -> Option<ViewerView> {
        let target = self.open.as_ref().map(|o| o.target.exe.clone()).unwrap_or_default();
        self.viewer.as_ref().map(|v| v.view(&target, self.overlay.scale))
    }

    /// The viewer's size in DIP (for hit tests in self-tests).
    pub fn viewer_size_dip(&self) -> Option<(f32, f32)> {
        self.viewer.as_ref().map(|v| v.size_dip(self.overlay.scale))
    }

    pub fn viewer_hit(&mut self, px: f32, py: f32) -> Option<VHit> {
        let mut view = self.viewer_view()?;
        let size = self.viewer_size_dip()?;
        let h = self.renderer.viewer_hit(&view, size, self.overlay.scale, px, py);
        view.wipe();
        Some(h)
    }

    // ------------------------------------------------------------------ open, close, fullscreen

    /// The card under the carousel's pointer grows into the viewer.
    pub(super) fn open_viewer(&mut self, p: &mut dyn Platform, now: u64) {
        if self.viewer.is_some() {
            return;
        }
        let Some(open) = self.open.as_ref() else { return };
        if open.ui.click.is_some() {
            return;
        }
        let Some(session) = open.session.as_ref() else { return };
        let fork = session.fork();
        let trash = open.in_trash();
        self.withdraw_warning();
        if let Some(o) = self.open.as_mut() {
            o.ui.dropdown_open = false;
            o.ui.hover = None;
            if o.ui.confirm_delete.take().is_some() {
                self.status = None;
            }
        }
        let Some((card_img, card_rect, lifted)) = self.card_pictures() else { return };
        let home = home_rect(&self.overlay);
        let (mx, my, mw, mh) = self.overlay.monitor_px;
        self.viewer = Some(Viewer {
            session: fork,
            trash,
            id: 0,
            info: None,
            info_lines: Vec::new(),
            content: Content::Message(String::new()),
            ocr_text: None,
            ocr_running: None,
            can_ocr: self.ocr.is_some(),
            ocr_selecting: false,
            ocr_regions: Vec::new(),
            ocr_stroke: Vec::new(),
            gen: 0,
            full_gen: 0,
            rect: card_rect,
            home,
            monitor: (mx, my, mw, mh),
            fullscreen: false,
            anim: None,
            lifted,
            dim: 1.0,
            last_frame: None,
            extent: None,
            hover: None,
            menu: None,
            menu_hover: None,
            info_open: false,
            find: None,
            confirm: None,
            toast: None,
            wrap: true,
            selection: None,
            anchor: None,
            selecting: false,
            last_dbl_ms: None,
            scroll: (0.0, 0.0),
            zoom: None,
            center: (0.0, 0.0),
            pan: None,
        });
        self.viewer_load(p);
        let to = self.render_viewer_at(home, true);
        let Some(morph) = Morph::new(&card_img, &to) else {
            self.viewer = None;
            return;
        };
        // the clock starts once the pictures are ready, so no frame of the animation is skipped
        let now = p.now_ms().max(now);
        if let Some(v) = self.viewer.as_mut() {
            v.anim = Some(Anim { kind: AnimKind::Open, start_ms: now, dur_ms: OPEN_MS, from: card_rect, to: home, morph, dim: (1.0, DIM) });
        }
        p.viewer_mode(true);
        p.animating(true);
        p.log("viewer opened");
        self.step_anim(p, now);
    }

    /// The active card as it looks now (and where it is on screen), and the carousel with its
    /// place empty.
    pub(super) fn card_pictures(&mut self) -> Option<(Frame, RectPx, Frame)> {
        let mut vm = self.view_model();
        vm.hover = None;
        let size = self.size_dip();
        let scale = self.overlay.scale;
        let g = geometry(&vm, size, scale);
        let card = g.rows.get(vm.active).copied().flatten();
        let Some(card) = card else {
            vm.wipe();
            return None;
        };
        let whole = self.renderer.render(&vm, size, scale);
        vm.lifted = true;
        let lifted = self.renderer.render(&vm, size, scale);
        vm.wipe();
        let (x, y) = ((card.x * scale).round() as i32, (card.y * scale).round() as i32);
        let (w, h) = ((card.w * scale).round().max(1.0) as u32, (card.h * scale).round().max(1.0) as u32);
        let img = whole.crop(x, y, w, h);
        let o = self.overlay.origin_px;
        Some((img, (o.0 + x, o.1 + y, w, h), lifted))
    }

    /// The viewer drawn for `rect`. A `draft` (the end picture of an animation, seen only for a
    /// moment) is drawn at half resolution on large screens: the real frame follows at once.
    fn render_viewer_at(&mut self, rect: RectPx, draft: bool) -> Frame {
        let scale = self.overlay.scale;
        let res = if draft && rect.2 > 1600 { 0.5 } else { 1.0 };
        let target = self.open.as_ref().map(|o| o.target.exe.clone()).unwrap_or_default();
        let Some(v) = self.viewer.as_mut() else {
            return Frame { width: 1, height: 1, bgra: vec![0, 0, 0, 255] };
        };
        let saved = v.rect;
        v.rect = rect;
        let size = v.size_dip(scale);
        let mut view = v.view(&target, scale);
        v.rect = saved;
        let f = self.renderer.render_viewer(&view, size, scale * res);
        view.wipe();
        f
    }

    /// Shrinks the viewer back into the card's place (after bringing the carousel up to date: the
    /// clip it showed may have been binned or deleted from the viewer).
    pub(super) fn close_viewer(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_ref() else { return };
        if v.anim.as_ref().is_some_and(|a| a.kind == AnimKind::Close) {
            return;
        }
        // the carousel's own clip
        let gone = match self.open.as_mut().and_then(|o| o.session.as_mut()) {
            Some(s) => {
                let id = s.current().id;
                !s.source_mut().exists(id).unwrap_or(false)
            }
            None => false,
        };
        if gone {
            self.step(p, true);
            if self.open.is_none() || self.viewer.is_none() {
                return; // history became empty: the session is over
            }
        } else if self.open.as_ref().is_some_and(|o| matches!(o.ui.body, BodyState::Image(None))) {
            self.load_body();
        }
        let Some((card_img, card_rect, lifted)) = self.card_pictures() else {
            self.drop_viewer(p);
            self.dirty = true;
            return;
        };
        let from_img = match self.viewer.as_mut().and_then(|v| v.last_frame.take()) {
            Some(f) => f,
            None => {
                let r = self.viewer.as_ref().map(|v| v.rect).unwrap_or(card_rect);
                self.render_viewer_at(r, true)
            }
        };
        let now = p.now_ms().max(now);
        let Some(v) = self.viewer.as_mut() else { return };
        let Some(morph) = Morph::new(&from_img, &card_img) else {
            self.drop_viewer(p);
            self.dirty = true;
            return;
        };
        v.menu = None;
        v.confirm = None;
        v.hover = None;
        v.lifted = lifted;
        v.anim = Some(Anim { kind: AnimKind::Close, start_ms: now, dur_ms: CLOSE_MS, from: v.rect, to: card_rect, morph, dim: (v.dim, 1.0) });
        v.last_frame = Some(from_img);
        p.animating(true);
        self.step_anim(p, now);
    }

    fn toggle_fullscreen(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.animating() {
            return;
        }
        v.fullscreen = !v.fullscreen;
        v.menu = None;
        let to = if v.fullscreen { v.monitor } else { v.home };
        let from = v.rect;
        let from_img = match v.last_frame.take() {
            Some(f) => f,
            None => self.render_viewer_at(from, true),
        };
        let to_img = self.render_viewer_at(to, true);
        let now = p.now_ms().max(now);
        let Some(v) = self.viewer.as_mut() else { return };
        // the picture keeps its zoom; fitted pictures refit
        let Some(morph) = Morph::new(&from_img, &to_img) else { return };
        v.anim = Some(Anim { kind: AnimKind::Resize, start_ms: now, dur_ms: RESIZE_MS, from, to, morph, dim: (v.dim, v.dim) });
        p.animating(true);
        self.step_anim(p, now);
    }

    /// Draws the animation's frame for `now`; finishes it when its time is up.
    pub(super) fn step_anim(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_mut() else { return };
        let Some(a) = v.anim.as_ref() else { return };
        let t = (now.saturating_sub(a.start_ms) as f32 / a.dur_ms.max(1) as f32).clamp(0.0, 1.0);
        let e = match a.kind {
            AnimKind::Open => ease_out(t),
            _ => ease_in_out(t),
        };
        let rect = lerp_rect(a.from, a.to, e);
        let shrink = (ANIM_MAX_PIXELS / (rect.2 as f32 * rect.3 as f32)).sqrt().min(1.0);
        let (fw, fh) = ((rect.2 as f32 * shrink).round().max(1.0) as u32, (rect.3 as f32 * shrink).round().max(1.0) as u32);
        let frame = a.morph.frame(e, fw, fh);
        let dim = lerp(a.dim.0, a.dim.1, e);
        let kind = a.kind;
        p.viewer_show(rect, !v.fullscreen);
        p.viewer_present(&frame);
        v.rect = rect;
        v.last_frame = Some(frame);
        if (dim - v.dim).abs() > 0.004 || t >= 1.0 {
            v.dim = dim;
            p.overlay_present(&v.lifted.dimmed(dim));
        }
        if t < 1.0 {
            return;
        }
        match kind {
            AnimKind::Open | AnimKind::Resize => {
                v.anim = None;
                v.last_frame = None;
                p.animating(false);
                self.dirty = true;
            }
            AnimKind::Close => {
                self.drop_viewer(p);
                self.dirty = true;
                p.log("viewer closed");
            }
        }
    }

    /// Removes the viewer at once (no animation).
    pub(super) fn drop_viewer(&mut self, p: &mut dyn Platform) {
        if self.viewer.take().is_some() {
            p.viewer_hide();
            p.viewer_mode(false);
            p.animating(false);
            self.renderer.forget_viewer();
            // back in the carousel: its usual time limit starts again, in step with the hooks
            let now = p.now_ms();
            if let Some(o) = self.open.as_mut() {
                o.opened_ms = now;
                self.last_activity_ms = now;
                p.input_keepalive();
            }
            self.on_new_clips(p);
        }
    }

    pub(super) fn present_viewer(&mut self, p: &mut dyn Platform) {
        let scale = self.overlay.scale;
        let target = self.open.as_ref().map(|o| o.target.exe.clone()).unwrap_or_default();
        let Some(v) = self.viewer.as_mut() else { return };
        if v.animating() {
            return; // the animation draws itself
        }
        let size = v.size_dip(scale);
        let mut view = v.view(&target, scale);
        let frame = self.renderer.render_viewer(&view, size, scale);
        v.extent = self.renderer.viewer_text_extent(&view, size, scale);
        view.wipe();
        p.viewer_show(v.rect, !v.fullscreen);
        p.viewer_present(&frame);
        v.last_frame = Some(frame);
    }

    // ------------------------------------------------------------------ loading a clip

    /// Shows the clip the viewer's session is on.
    fn viewer_load(&mut self, p: &mut dyn Platform) {
        let Some(id) = self.viewer.as_ref().map(|v| v.session.current().id) else { return };
        let info = self.store.borrow().info(id).ok().flatten();
        let content = match info.as_ref().map(|i| i.kind) {
            Some(Kind::Image) => {
                let thumb = self.cached_thumb(id);
                let (w, h) = info.as_ref().map_or((0, 0), |i| (i.width, i.height));
                Content::Image { thumb, full: None, w, h }
            }
            Some(k @ (Kind::Text | Kind::Files)) => {
                let text = {
                    let st = self.store.borrow();
                    if k == Kind::Files {
                        st.files(id).ok().flatten().map(|f| f.join("\n"))
                    } else {
                        st.text(id).ok().flatten()
                    }
                };
                match text {
                    Some(mut text) => {
                        let chars = text.chars().count();
                        let lines = if k == Kind::Files { text.split('\n').filter(|l| !l.is_empty()).count() } else { text.lines().count().max(1) };
                        let words = text.split_whitespace().count();
                        let bytes = text.len();
                        let (shown, more) = match text.char_indices().nth(VIEWER_MAX_CHARS) {
                            Some((cut, _)) => (&text[..cut], chars - VIEWER_MAX_CHARS),
                            None => (text.as_str(), 0),
                        };
                        let mut san = sanitize(shown);
                        if more > 0 {
                            san = san.with_footer(&format!(
                                "\n... {} more characters not shown here (Copy, Paste and Save as include them)",
                                thousands(more)
                            ));
                        }
                        let orig = shown.encode_utf16().collect();
                        {
                            use zeroize::Zeroize;
                            text.zeroize();
                        }
                        Content::Text { san, orig, files: k == Kind::Files, chars, lines, words, bytes }
                    }
                    None => Content::Message("This clip could not be read.".into()),
                }
            }
            None => Content::Message("This clip no longer exists.".into()),
        };
        let lines = self.info_lines(p, info.as_ref(), &content);
        let Some(v) = self.viewer.as_mut() else { return };
        v.id = id;
        v.info = info;
        v.info_lines = lines;
        v.content = content;
        v.ocr_text = None;
        v.ocr_running = None;
        v.ocr_selecting = false;
        v.ocr_regions.clear();
        v.ocr_stroke.clear();
        v.gen += 1;
        v.selection = None;
        v.anchor = None;
        v.selecting = false;
        v.scroll = (0.0, 0.0);
        v.zoom = None;
        v.pan = None;
        v.extent = None;
        v.menu = None;
        v.confirm = None;
        if let Content::Image { w, h, .. } = v.content {
            v.center = (w as f32 / 2.0, h as f32 / 2.0);
            v.full_gen += 1;
            self.workers.request_full(id, v.full_gen);
        }
        self.find_recompute(true);
        self.dirty = true;
    }

    fn info_lines(&self, p: &dyn Platform, info: Option<&ClipInfo>, content: &Content) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let kind = match content {
            Content::Text { files: true, .. } => "Files",
            Content::Text { .. } => "Text",
            Content::Image { .. } => "Image (PNG)",
            Content::Message(_) => "Unknown",
        };
        out.push(("Type".to_owned(), kind.to_owned()));
        match content {
            Content::Text { files: true, lines, .. } => out.push(("Size".into(), format!("{} file(s)", thousands(*lines)))),
            Content::Text { chars, lines, words, bytes, .. } => out.push((
                "Size".into(),
                format!("{} characters, {} lines, {} words ({})", thousands(*chars), thousands(*lines), thousands(*words), human_bytes(*bytes)),
            )),
            Content::Image { w, h, .. } => out.push(("Dimensions".into(), format!("{w} x {h} pixels"))),
            Content::Message(_) => {}
        }
        if let Some(i) = info {
            let copied = if i.copied_at > 0 { p.format_time(i.copied_at) } else { utc_date(i.touched_day) };
            out.push(("Copied".into(), copied));
            out.push(("From".into(), if i.source.is_empty() { "Not recorded".into() } else { i.source.clone() }));
            if i.created_day < i.touched_day {
                out.push(("First copied".into(), utc_date(i.created_day)));
            }
            out.push(("Pinned".into(), (if i.pinned && i.trashed.is_none() { "Yes" } else { "No" }).into()));
            if let Some(t) = i.trashed {
                out.push((
                    "In the recycle bin since".into(),
                    format!("{} (removed for good after {} days)", p.format_time(t), pasteroll_core::store::TRASH_DAYS),
                ));
            }
        }
        out
    }

    pub(super) fn viewer_on_preview(&mut self, r: &PreviewResult, img: Option<Arc<core_image::Rgba>>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if r.id != v.id {
            return;
        }
        let Content::Image { thumb, full, .. } = &mut v.content else { return };
        if r.full {
            if r.generation == v.full_gen {
                *full = img;
                self.dirty = true;
            }
        } else if thumb.is_none() && img.is_some() {
            *thumb = img;
            self.dirty = true;
        }
    }

    pub(super) fn drain_ocr(&mut self, now: u64) {
        while let Ok(mut done) = self.ocr_rx.try_recv() {
            let Some(v) = self.viewer.as_mut() else { continue };
            if v.ocr_running != Some(done.request) || v.id != done.id {
                if let Ok(text) = &mut done.result {
                    use zeroize::Zeroize;
                    text.zeroize();
                }
                continue;
            }
            v.ocr_running = None;
            if v.id != done.id {
                continue;
            }
            match std::mem::replace(&mut done.result, Err(String::new())) {
                Ok(mut text) if !text.trim().is_empty() => {
                    let san = sanitize(&text);
                    let orig = text.encode_utf16().collect();
                    {
                        use zeroize::Zeroize;
                        text.zeroize();
                    }
                    v.ocr_text = Some(OcrText { san, orig });
                    v.ocr_selecting = false;
                    v.ocr_stroke.clear();
                    v.gen += 1;
                    v.selection = None;
                    v.scroll = (0.0, 0.0);
                    v.toast("Text extracted. Copy and Paste use it; Esc shows the picture again.", now);
                }
                Ok(_) => v.toast("No text found in this picture.", now),
                Err(e) => v.toast(e, now),
            }
            self.dirty = true;
        }
    }

    // ------------------------------------------------------------------ actions

    fn viewer_toast(&mut self, text: impl Into<String>, now: u64) {
        if let Some(v) = self.viewer.as_mut() {
            v.toast(text, now);
            self.dirty = true;
        }
    }

    /// The selected part of the text shown, as the real text.
    fn viewer_selected_text(&self) -> Option<String> {
        let v = self.viewer.as_ref()?;
        let (a, b) = v.selection?;
        let (a, b) = (a.min(b), a.max(b));
        let (san, orig) = v.shown_text()?;
        let (x, y) = san.original_range(a, b);
        (y > x && y <= orig.len()).then(|| String::from_utf16_lossy(&orig[x..y]))
    }

    /// What Copy and Paste take: the selection, the recognised text, or the whole clip.
    fn viewer_payload(&self) -> Option<(Payload, bool)> {
        if let Some(t) = self.viewer_selected_text() {
            return Some((Payload::Text(t), false));
        }
        let v = self.viewer.as_ref()?;
        if let Some(o) = &v.ocr_text {
            return Some((Payload::Text(String::from_utf16_lossy(&o.orig)), false));
        }
        let st = self.store.borrow();
        let id = v.id;
        let p = match st.kind(id).ok().flatten()? {
            Kind::Text => st.text(id).ok().flatten().map(Payload::Text),
            Kind::Files => st.files(id).ok().flatten().map(Payload::Files),
            Kind::Image => st.image_png(id).ok().flatten().map(Payload::Image),
        };
        p.map(|p| (p, true))
    }

    fn viewer_copy(&mut self, p: &mut dyn Platform, now: u64) {
        let partial = self.viewer.as_ref().is_some_and(|v| v.selection.is_some_and(|(a, b)| a != b));
        let Some((payload, whole)) = self.viewer_payload() else {
            self.viewer_toast("Nothing to copy.", now);
            return;
        };
        let payload = WipeOnDrop(payload);
        match p.write_clipboard(&payload.0) {
            Ok(()) => {
                if !whole {
                    if let Payload::Text(text) = &payload.0 {
                        self.workers.record_copied_text(text, p.now_secs());
                    }
                }
                self.viewer_toast(if partial { "Copied the selection" } else { "Copied" }, now);
            }
            Err(e) => {
                p.log(&format!("could not write clipboard: {e}"));
                self.viewer_toast("Could not copy: the clipboard is busy.", now);
            }
        }
    }

    fn viewer_paste(&mut self, p: &mut dyn Platform, warned: bool) {
        let Some(id) = self.viewer.as_ref().map(|v| v.id) else { return };
        let partial = self.viewer_selected_text();
        let ocr = self.viewer.as_ref().and_then(|v| v.ocr_text.as_ref().map(|o| String::from_utf16_lossy(&o.orig)));
        let selection = partial.or(ocr);
        if let PasteCheck::Warn(lines) = self.paste_item(p, id, selection, warned) {
            if let Some(v) = self.viewer.as_mut() {
                v.confirm = Some(Confirm::TerminalPaste(lines));
                self.dirty = true;
            }
        }
    }

    fn viewer_pin(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_ref() else { return };
        if v.trash {
            self.viewer_toast("Restore it from the recycle bin first to pin it.", now);
            return;
        }
        let id = v.id;
        if let Some(msg) = self.set_pin(p, id, now) {
            let info = self.store.borrow().info(id).ok().flatten();
            if let Some(v) = self.viewer.as_mut() {
                if let (Some(i), Some(new)) = (v.info.as_mut(), info) {
                    i.pinned = new.pinned;
                }
                if let Some(line) = v.info_lines.iter_mut().find(|(l, _)| l == "Pinned") {
                    line.1 = (if v.info.as_ref().is_some_and(|i| i.pinned) { "Yes" } else { "No" }).into();
                }
            }
            self.viewer_toast(msg, now);
        }
    }

    /// The viewer's clip went away: show the next older one, or close when there is none.
    fn viewer_advance(&mut self, p: &mut dyn Platform, now: u64) {
        let moved = match self.viewer.as_mut() {
            Some(v) => v.session.older(),
            None => return,
        };
        match moved {
            Ok(Moved::Empty) | Err(_) => {
                if let Some(v) = self.viewer.as_mut() {
                    v.content = Content::Message("Nothing left to show.".into());
                    v.info = None;
                    v.info_lines.clear();
                    v.gen += 1;
                }
                self.close_viewer(p, now);
            }
            Ok(_) => self.viewer_load(p),
        }
    }

    fn viewer_trash(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.trash {
            v.confirm = Some(Confirm::DeleteForever(v.id));
            self.dirty = true;
            return;
        }
        let id = v.id;
        if self.trash_clip(p, id) {
            self.viewer_advance(p, now);
            self.viewer_toast("Moved to the recycle bin. Ctrl+Z brings it back.", now);
        }
    }

    fn viewer_restore_or_undo(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_ref() else { return };
        if v.trash {
            let id = v.id;
            if self.restore_clip(p, id) {
                self.viewer_advance(p, now);
                self.viewer_toast("Restored to history.", now);
            }
            return;
        }
        let Some(item) = self.open.as_mut().and_then(|o| o.last_trashed.take()) else {
            self.viewer_toast("Nothing to undo.", now);
            return;
        };
        if !self.restore_clip(p, item.id) {
            return;
        }
        if let Some(v) = self.viewer.as_mut() {
            let src = v.session.source_mut().clone();
            if let Ok(Some(s)) = Session::open_at(src, &item) {
                v.session = s;
            }
        }
        self.viewer_load(p);
        self.viewer_toast("Restored.", now);
    }

    fn viewer_delete_forever(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(id) = self.viewer.as_ref().map(|v| v.id) else { return };
        if self.delete_forever(p, id) {
            self.viewer_advance(p, now);
            self.viewer_toast("Deleted permanently.", now);
        }
    }

    fn viewer_step_clip(&mut self, p: &mut dyn Platform, older: bool, now: u64) {
        let moved = match self.viewer.as_mut() {
            Some(v) if older => v.session.older(),
            Some(v) => v.session.newer(),
            None => return,
        };
        match moved {
            Ok(Moved::Empty) | Err(_) => self.close_viewer(p, now),
            Ok(_) => self.viewer_load(p),
        }
    }

    fn viewer_save(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_ref() else { return };
        let id = v.id;
        let (bytes, ext, word): (Vec<u8>, &str, &str) = if let Some(o) = &v.ocr_text {
            (String::from_utf16_lossy(&o.orig).into_bytes(), "txt", "text")
        } else {
            let st = self.store.borrow();
            match st.kind(id).ok().flatten() {
                Some(Kind::Text) => match st.text(id).ok().flatten() {
                    Some(t) => (t.into_bytes(), "txt", "text"),
                    None => return,
                },
                Some(Kind::Files) => match st.files(id).ok().flatten() {
                    Some(f) => (f.join(if cfg!(windows) { "\r\n" } else { "\n" }).into_bytes(), "txt", "file list"),
                    None => return,
                },
                Some(Kind::Image) => {
                    // re-encoded: another program's PNG may carry metadata or trailing data
                    match st.image_png(id).ok().flatten().and_then(|png| core_image::decode_png(&png)).and_then(|i| core_image::encode_png(&i)) {
                        Some(png) => (png, "png", "image"),
                        None => return,
                    }
                }
                None => return,
            }
        };
        let mut bytes = bytes;
        let stem = format!("Pasteroll {word} {}", p.local_timestamp());
        let Some(target) = self.open.as_ref().map(|o| o.target.clone()) else { return };
        let choice = p.save_dialog(&format!("{stem}.{ext}"), ext, &target);
        // the dialog took a while: that was use, not idleness
        self.last_activity_ms = p.now_ms();
        p.input_keepalive();
        let result = match &choice {
            SaveChoice::Path(path) => crate::files::save_replacing(path, &bytes).map(|_| Some(path.clone())),
            SaveChoice::Cancelled => Ok(None),
            SaveChoice::Unavailable => match p.desktop_folder() {
                Some(folder) => crate::files::save_unique(&folder, &stem, ext, &bytes).map(Some),
                None => {
                    {
                        use zeroize::Zeroize;
                        bytes.zeroize();
                    }
                    p.log("could not save the clip: no save dialog and no Desktop folder");
                    self.viewer_toast("Nowhere to save: install zenity or kdialog for a save dialog.", now);
                    return;
                }
            },
        };
        {
            use zeroize::Zeroize;
            bytes.zeroize();
        }
        match result {
            Ok(Some(path)) => {
                p.file_created(&path);
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                p.log("saved a clip to a file");
                let unasked = matches!(choice, SaveChoice::Unavailable);
                self.viewer_toast(if unasked { format!("Saved to the Desktop as {name}") } else { format!("Saved as {name}") }, now);
            }
            Ok(None) => {}
            Err(e) => {
                p.log(&format!("could not save the clip: {e}"));
                self.viewer_toast("Could not save the file.", now);
            }
        }
    }

    fn viewer_extract_text(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(v) = self.viewer.as_ref() else { return };
        if !matches!(v.content, Content::Image { .. }) || v.ocr_running.is_some() {
            return;
        }
        let Some(engine) = self.ocr.clone() else {
            self.viewer_toast("Text recognition is not available on this computer.", now);
            return;
        };
        let id = v.id;
        if v.ocr_selecting && (v.ocr_regions.is_empty() || !v.ocr_stroke.is_empty()) {
            self.viewer_toast("Finish drawing an area around the text first.", now);
            return;
        }
        let regions = v.ocr_selecting.then(|| v.ocr_regions.clone());
        let img = match &v.content {
            Content::Image { full: Some(f), .. } => Some(f.clone()),
            _ => None,
        };
        let img = match img {
            Some(i) => i,
            None => {
                let png = self.store.borrow().image_png(id).ok().flatten();
                match png.and_then(|png| core_image::decode_png(&png)) {
                    Some(i) => Arc::new(i),
                    None => {
                        self.viewer_toast("The picture could not be read.", now);
                        return;
                    }
                }
            }
        };
        // A cancelled OS OCR call may still be finishing. Bound outstanding jobs
        // so repeated cancel/retry cannot accumulate full-size masked images.
        let Some(job) = OcrJob::acquire(&self.ocr_jobs) else {
            self.viewer_toast("Previous text recognition is still finishing. Please try again shortly.", now);
            return;
        };
        self.ocr_request = self.ocr_request.wrapping_add(1);
        let request = self.ocr_request;
        if let Some(v) = self.viewer.as_mut() {
            v.ocr_running = Some(request);
            v.toast = Some(("Reading text...".into(), now + 30_000));
        }
        self.dirty = true;
        let tx = self.ocr_tx.clone();
        let waker = self.waker.clone();
        p.log("text recognition started");
        std::thread::spawn(move || {
            let _job = job;
            let result = if let Some(regions) = regions {
                pasteroll_core::ocr_regions::masked(&img, &regions).and_then(|mut selected| {
                    let result = engine.recognize(&selected);
                    use zeroize::Zeroize;
                    selected.pixels.zeroize();
                    result
                })
            } else {
                engine.recognize(&img)
            };
            let _ = tx.send(OcrDone { id, request, result });
            waker.wake();
        });
    }

    fn show_picture(&mut self) {
        if let Some(v) = self.viewer.as_mut() {
            v.ocr_text = None;
            v.gen += 1;
            v.selection = None;
            v.find = None;
            self.dirty = true;
        }
    }

    fn run_action(&mut self, p: &mut dyn Platform, a: Action, now: u64) {
        if let Some(v) = self.viewer.as_mut() {
            v.menu = None;
            v.menu_hover = None;
        }
        self.dirty = true;
        match a {
            Action::SaveAs => self.viewer_save(p, now),
            Action::Info => {
                if let Some(v) = self.viewer.as_mut() {
                    v.info_open = !v.info_open;
                }
                self.clamp_center();
            }
            Action::ResetZoom => self.set_zoom(None, None),
            Action::ExtractText => self.viewer_extract_text(p, now),
            Action::ClearOcrRegions => {
                if let Some(v) = self.viewer.as_mut() {
                    v.ocr_regions.clear();
                    v.ocr_stroke.clear();
                }
            }
            Action::ShowPicture => self.show_picture(),
            Action::Restore => self.viewer_restore_or_undo(p, now),
            Action::DeleteForever => {
                if let Some(v) = self.viewer.as_mut() {
                    v.confirm = Some(Confirm::DeleteForever(v.id));
                }
            }
        }
    }

    fn run_tool(&mut self, p: &mut dyn Platform, t: Tool, now: u64) {
        match t {
            Tool::OcrSelect => {
                if let Some(v) = self.viewer.as_mut() {
                    v.ocr_selecting = !v.ocr_selecting;
                    v.ocr_regions.clear();
                    v.ocr_stroke.clear();
                    v.pan = None;
                }
                self.dirty = true;
            }
            Tool::OcrRead => self.viewer_extract_text(p, now),
            Tool::OcrUndo => {
                if let Some(v) = self.viewer.as_mut() { v.ocr_regions.pop(); }
                self.dirty = true;
            }
            Tool::Prev => self.viewer_step_clip(p, false, now),
            Tool::Next => self.viewer_step_clip(p, true, now),
            Tool::ZoomIn => self.zoom_by(ZOOM_STEP, None),
            Tool::ZoomOut => self.zoom_by(1.0 / ZOOM_STEP, None),
            Tool::FitActual => {
                let fitted = self.viewer.as_ref().is_some_and(|v| v.zoom.is_none());
                if fitted {
                    self.set_zoom(Some(1.0 / self.overlay.scale.max(0.1)), None);
                } else {
                    self.set_zoom(None, None);
                }
            }
            Tool::Copy => self.viewer_copy(p, now),
            Tool::Paste => self.viewer_paste(p, false),
            Tool::Pin => self.viewer_pin(p, now),
            Tool::Delete => self.viewer_trash(p, now),
            Tool::Restore => self.viewer_restore_or_undo(p, now),
            Tool::Search => {
                if self.viewer.as_ref().is_some_and(|v| v.find.is_some()) {
                    if let Some(v) = self.viewer.as_mut() {
                        v.find = None;
                    }
                } else {
                    self.find_open(None);
                }
            }
            Tool::Wrap => {
                if let Some(v) = self.viewer.as_mut() {
                    v.wrap = !v.wrap;
                    v.scroll = (0.0, 0.0);
                    v.extent = None;
                }
                self.find_scroll_to_current();
            }
            Tool::More => {
                let has_ocr = self.ocr.is_some();
                if let Some(v) = self.viewer.as_mut() {
                    v.menu = if v.menu.is_some() { None } else { Some(v.menu_actions(has_ocr)) };
                    v.menu_hover = None;
                }
            }
            Tool::Close => self.close_viewer(p, now),
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------ find

    fn find_open(&mut self, first: Option<char>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.shown_text().is_none() {
            return;
        }
        let f = v.find.get_or_insert_with(|| Find { query: String::new(), matches: Vec::new(), current: None });
        if let Some(c) = first {
            f.query.push(c);
        }
        self.find_recompute(true);
        self.dirty = true;
    }

    /// Finds the query again in the text shown (after typing, or on another clip).
    fn find_recompute(&mut self, jump: bool) {
        let Some(v) = self.viewer.as_mut() else { return };
        let display = v.shown_text().map(|(s, _)| s.display.clone());
        let Some(f) = v.find.as_mut() else { return };
        f.matches = match display {
            Some(mut d) => {
                let m = find_matches(&d, &f.query);
                use zeroize::Zeroize;
                d.zeroize();
                m
            }
            None => Vec::new(),
        };
        f.current = (!f.matches.is_empty()).then_some(0);
        if jump {
            self.find_scroll_to_current();
        }
    }

    fn find_step(&mut self, forward: bool) {
        let Some(f) = self.viewer.as_mut().and_then(|v| v.find.as_mut()) else { return };
        let n = f.matches.len();
        if n == 0 {
            return;
        }
        f.current = Some(match f.current {
            Some(c) if forward => (c + 1) % n,
            Some(c) => (c + n - 1) % n,
            None => 0,
        });
        self.find_scroll_to_current();
        self.dirty = true;
    }

    fn find_scroll_to_current(&mut self) {
        let scale = self.overlay.scale;
        let target = self.open.as_ref().map(|o| o.target.exe.clone()).unwrap_or_default();
        let Some(v) = self.viewer.as_mut() else { return };
        let Some(pos) = v.find.as_ref().and_then(|f| f.current.map(|c| f.matches[c].0)) else { return };
        let size = v.size_dip(scale);
        let mut view = v.view(&target, scale);
        let bx = self.renderer.viewer_char_box(&view, size, scale, pos);
        let ext = self.renderer.viewer_text_extent(&view, size, scale);
        view.wipe();
        let (Some((x, top, h)), Some(((cw, ch), (vw, vh)))) = (bx, ext) else { return };
        if top < v.scroll.1 || top + h > v.scroll.1 + vh {
            v.scroll.1 = (top - vh / 3.0).clamp(0.0, (ch - vh).max(0.0));
        }
        if !v.wrap && (x < v.scroll.0 || x > v.scroll.0 + vw - 40.0) {
            v.scroll.0 = (x - vw / 3.0).clamp(0.0, (cw - vw).max(0.0));
        }
        v.extent = ext;
    }

    // ------------------------------------------------------------------ scrolling, zoom, pan

    fn scroll_text(&mut self, dx: f32, dy: f32) {
        let Some(v) = self.viewer.as_mut() else { return };
        let ((cw, ch), (vw, vh)) = v.extent.unwrap_or(((0.0, f32::MAX), (0.0, 0.0)));
        v.scroll.1 = (v.scroll.1 + dy).clamp(0.0, (ch - vh).max(0.0));
        v.scroll.0 = if v.wrap { 0.0 } else { (v.scroll.0 + dx).clamp(0.0, (cw - vw).max(0.0)) };
        self.dirty = true;
    }

    fn picture_geometry(&self) -> Option<(pasteroll_render::VGeom, u32, u32, f32)> {
        let v = self.viewer.as_ref()?;
        if !v.is_picture() {
            return None;
        }
        let (_, w, h) = v.picture()?;
        if w == 0 || h == 0 {
            return None;
        }
        let mut view = v.view("", self.overlay.scale);
        let g = viewer_geometry(&view, v.size_dip(self.overlay.scale));
        view.wipe();
        let fit = fit_zoom(&g, w, h);
        Some((g, w, h, fit))
    }

    /// Sets the zoom (DIP per image pixel; `None` = fit), keeping the image point under `at` (px)
    /// where it is.
    fn set_zoom(&mut self, zoom: Option<f32>, at: Option<(f32, f32)>) {
        let scale = self.overlay.scale.max(0.1);
        let Some((g, w, h, fit)) = self.picture_geometry() else { return };
        let Some(v) = self.viewer.as_mut() else { return };
        let old = v.zoom.unwrap_or(fit);
        // from a quarter of fit (or of real size) up to 32 screen pixels per picture pixel, or 8x
        // fit for pictures so small that fitting them already magnifies more than that
        let new = zoom.map(|z| z.clamp(fit.min(1.0 / scale) * 0.25, (32.0 / scale).max(fit * 8.0)));
        let new_z = new.unwrap_or(fit);
        match (new, at) {
            (None, _) => v.center = (w as f32 / 2.0, h as f32 / 2.0),
            (Some(_), Some((px, py))) => {
                let (cx, cy) = (g.content.x + g.content.w / 2.0, g.content.y + g.content.h / 2.0);
                let (dx, dy) = (px / scale - cx, py / scale - cy);
                let q = (v.center.0 + dx / old, v.center.1 + dy / old);
                v.center = (q.0 - dx / new_z, q.1 - dy / new_z);
            }
            (Some(_), None) => {}
        }
        v.zoom = new;
        self.clamp_center();
        self.dirty = true;
    }

    fn zoom_by(&mut self, factor: f32, at: Option<(f32, f32)>) {
        let Some((_, _, _, fit)) = self.picture_geometry() else { return };
        let cur = self.viewer.as_ref().and_then(|v| v.zoom).unwrap_or(fit);
        self.set_zoom(Some(cur * factor), at);
    }

    /// Keeps the picture in view: a side smaller than the view stays centred, a larger one cannot
    /// be dragged past its edge.
    fn clamp_center(&mut self) {
        let Some((g, w, h, fit)) = self.picture_geometry() else { return };
        let Some(v) = self.viewer.as_mut() else { return };
        let z = v.zoom.unwrap_or(fit);
        let (vw, vh) = (g.content.w / z, g.content.h / z);
        let clamp = |c: f32, size: f32, view: f32| if size <= view { size / 2.0 } else { c.clamp(view / 2.0, size - view / 2.0) };
        v.center = (clamp(v.center.0, w as f32, vw), clamp(v.center.1, h as f32, vh));
    }

    fn pan_by(&mut self, dx_dip: f32, dy_dip: f32) {
        let Some((_, _, _, fit)) = self.picture_geometry() else { return };
        if let Some(v) = self.viewer.as_mut() {
            let z = v.zoom.unwrap_or(fit);
            v.center = (v.center.0 + dx_dip / z, v.center.1 + dy_dip / z);
        }
        self.clamp_center();
        self.dirty = true;
    }

    // ------------------------------------------------------------------ input

    pub(super) fn viewer_key(&mut self, p: &mut dyn Platform, key: Key, now: u64) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.animating() {
            if key == Key::Escape {
                self.close_viewer(p, now);
            }
            return;
        }
        if v.confirm.is_some() {
            match key {
                Key::Enter => self.confirm_ok(p, now),
                Key::Escape => {
                    v.confirm = None;
                    self.dirty = true;
                }
                _ => {}
            }
            return;
        }
        if v.menu.is_some() {
            match key {
                Key::Enter => {
                    let pick = v.menu_hover.and_then(|i| v.menu.as_ref().and_then(|m| m.get(i).copied()));
                    match pick {
                        Some(a) => self.run_action(p, a, now),
                        None => v.menu = None,
                    }
                    self.dirty = true;
                    return;
                }
                Key::Escape | Key::Tab => {
                    v.menu = None;
                    self.dirty = true;
                    return;
                }
                _ => {
                    v.menu = None;
                    self.dirty = true;
                }
            }
        }
        let Some(v) = self.viewer.as_mut() else { return };
        if v.ocr_selecting {
            match key {
                Key::Escape => {
                    v.ocr_selecting = false;
                    v.ocr_regions.clear();
                    v.ocr_stroke.clear();
                    v.ocr_running = None;
                    v.toast = None;
                    self.dirty = true;
                    return;
                }
                Key::Enter => {
                    self.viewer_extract_text(p, now);
                    return;
                }
                Key::CtrlZ => {
                    if v.ocr_running.is_none() {
                        if !v.ocr_stroke.is_empty() { v.ocr_stroke.clear(); }
                        else { v.ocr_regions.pop(); }
                    }
                    self.dirty = true;
                    return;
                }
                Key::Delete => {
                    if v.ocr_running.is_none() {
                        v.ocr_regions.clear();
                        v.ocr_stroke.clear();
                    }
                    self.dirty = true;
                    return;
                }
                _ => {}
            }
        }
        let finding = v.find.is_some();
        let picture = v.is_picture();
        let page = v.extent.map_or(300.0, |(_, (_, vh))| vh * 0.9);
        let line = LINE_DIP * self.overlay.scale;
        match key {
            Key::Escape => {
                if v.find.is_some() {
                    v.find = None;
                } else if v.info_open {
                    v.info_open = false;
                    self.clamp_center();
                } else if v.ocr_text.is_some() {
                    self.show_picture();
                } else if v.fullscreen {
                    self.toggle_fullscreen(p, now);
                } else {
                    self.close_viewer(p, now);
                }
                self.dirty = true;
            }
            Key::Enter => {
                if finding {
                    self.find_step(true);
                } else {
                    self.viewer_paste(p, false);
                }
            }
            Key::Left => self.viewer_step_clip(p, false, now),
            Key::Right => self.viewer_step_clip(p, true, now),
            Key::F11 => self.toggle_fullscreen(p, now),
            Key::F3 => self.find_step(true),
            Key::ShiftF3 => self.find_step(false),
            Key::CtrlF => self.find_open(None),
            Key::CtrlC => self.viewer_copy(p, now),
            Key::CtrlS => self.viewer_save(p, now),
            Key::CtrlP => self.viewer_pin(p, now),
            Key::CtrlZ => self.viewer_restore_or_undo(p, now),
            Key::CtrlY => {}
            Key::CtrlE => self.close_viewer(p, now),
            Key::CtrlI => self.run_action(p, Action::Info, now),
            Key::CtrlA => {
                if let Some((san, _)) = v.shown_text() {
                    let n = san.display.encode_utf16().count();
                    v.selection = (n > 0).then_some((0, n));
                    self.dirty = true;
                }
            }
            Key::Delete | Key::CtrlDelete => self.viewer_trash(p, now),
            Key::ShiftDelete => {
                v.confirm = Some(Confirm::DeleteForever(v.id));
                self.dirty = true;
            }
            Key::PageUp | Key::PageDown => {
                let d = if key == Key::PageDown { page } else { -page };
                if picture {
                    self.pan_by(0.0, d / self.overlay.scale);
                } else {
                    self.scroll_text(0.0, d);
                }
            }
            Key::Home | Key::End => {
                if !picture {
                    let d = if key == Key::End { f32::MAX / 4.0 } else { -f32::MAX / 4.0 };
                    self.scroll_text(0.0, d);
                }
            }
            Key::Backspace | Key::CtrlBackspace => {
                if let Some(f) = v.find.as_mut() {
                    if key == Key::CtrlBackspace {
                        f.query.clear();
                    } else {
                        f.query.pop();
                    }
                    self.find_recompute(true);
                    self.dirty = true;
                }
            }
            Key::Char(c) => {
                if let Some(f) = v.find.as_mut() {
                    if f.query.chars().count() < QUERY_MAX_CHARS {
                        f.query.push(c);
                        self.find_recompute(true);
                        self.dirty = true;
                    }
                } else if picture {
                    match c {
                        '+' | '=' => self.zoom_by(ZOOM_STEP, None),
                        '-' | '_' => self.zoom_by(1.0 / ZOOM_STEP, None),
                        '0' => self.set_zoom(None, None),
                        '1' => self.set_zoom(Some(1.0 / self.overlay.scale.max(0.1)), None),
                        _ => {}
                    }
                } else if !c.is_whitespace() {
                    // typing starts a search in the clip, as typing searches history in the carousel
                    self.find_open(Some(c));
                }
            }
            Key::Up | Key::Down => {
                let d = if key == Key::Down { 1.0 } else { -1.0 };
                if picture {
                    self.pan_by(0.0, d * 40.0);
                } else {
                    self.scroll_text(0.0, d * line);
                }
            }
            Key::Tab => {}
        }
    }

    /// Up / Down arrive as steps (they move through the carousel otherwise).
    pub(super) fn viewer_step(&mut self, p: &mut dyn Platform, older: bool, now: u64) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.animating() || v.confirm.is_some() {
            return;
        }
        if let Some(items) = &v.menu {
            let n = items.len();
            v.menu_hover = Some(match v.menu_hover {
                Some(i) if older => (i + 1) % n,
                Some(i) => (i + n - 1) % n,
                None => {
                    if older {
                        0
                    } else {
                        n - 1
                    }
                }
            });
            self.dirty = true;
            return;
        }
        self.viewer_key(p, if older { Key::Down } else { Key::Up }, now);
    }

    fn confirm_ok(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(c) = self.viewer.as_mut().and_then(|v| v.confirm.take()) else { return };
        self.dirty = true;
        match c {
            Confirm::DeleteForever(id) => {
                if self.viewer.as_ref().is_some_and(|v| v.id == id) {
                    self.viewer_delete_forever(p, now);
                }
            }
            Confirm::TerminalPaste(_) => self.viewer_paste(p, true),
        }
    }

    pub(super) fn viewer_wheel(&mut self, w: Wheel) {
        let Some(v) = self.viewer.as_ref() else { return };
        if v.animating() || v.confirm.is_some() || v.menu.is_some() || !v.ocr_stroke.is_empty() {
            return;
        }
        let notches = w.delta as f32 / 120.0;
        let scale = self.overlay.scale;
        if v.is_picture() {
            if w.ctrl {
                let (px, py) = self.pointer_in_viewer();
                self.zoom_by(ZOOM_STEP.powf(notches), Some((px, py)));
            } else if w.horizontal || w.shift {
                self.pan_by(notches * 60.0, 0.0);
            } else {
                self.pan_by(0.0, -notches * 60.0);
            }
        } else if !w.ctrl {
            let d = notches * SCROLL_LINES * LINE_DIP * scale;
            if w.horizontal {
                self.scroll_text(d, 0.0);
            } else if w.shift {
                self.scroll_text(-d, 0.0);
            } else {
                self.scroll_text(0.0, -d);
            }
        }
    }

    /// The pointer in the viewer's pixels (wheel events carry no position).
    fn pointer_in_viewer(&self) -> (f32, f32) {
        let r = self.viewer.as_ref().map_or((0, 0, 0, 0), |v| v.rect);
        let c = self.last_pointer;
        ((c.0 - r.0) as f32, (c.1 - r.1) as f32)
    }

    pub(super) fn viewer_mouse(&mut self, p: &mut dyn Platform, m: MouseEvent, now: u64) {
        let Some(v) = self.viewer.as_ref() else { return };
        if v.animating() {
            return;
        }
        let r = v.rect;
        self.last_pointer = (r.0 + m.x as i32, r.1 + m.y as i32);
        let scale = self.overlay.scale;
        match m.kind {
            MouseKind::Leave => {
                if let Some(v) = self.viewer.as_mut() {
                    if v.hover.take().is_some() || v.menu_hover.take().is_some() {
                        self.dirty = true;
                    }
                }
            }
            MouseKind::Up => {
                self.finish_ocr_stroke(m.x, m.y, now);
                if let Some(v) = self.viewer.as_mut() {
                    v.selecting = false;
                    v.pan = None;
                }
            }
            MouseKind::Move => {
                if !v.ocr_stroke.is_empty() {
                    self.append_ocr_point(m.x, m.y);
                    return;
                }
                let (selecting, pan) = (v.selecting, v.pan);
                if selecting {
                    self.viewer_drag_select(m.x, m.y);
                } else if let Some((start, center)) = pan {
                    let fit = self.picture_geometry().map_or(1.0, |g| g.3);
                    if let Some(v) = self.viewer.as_mut() {
                        let z = v.zoom.unwrap_or(fit);
                        v.center = (center.0 - (m.x - start.0) / scale / z, center.1 - (m.y - start.1) / scale / z);
                    }
                    self.clamp_center();
                    self.dirty = true;
                } else {
                    let hit = self.viewer_hit(m.x, m.y);
                    // the exact character under the pointer does not change how anything looks
                    let hit = match hit {
                        Some(VHit::Text { .. }) => Some(VHit::Text { pos: 0 }),
                        other => other,
                    };
                    if let Some(v) = self.viewer.as_mut() {
                        let menu_hover = match hit {
                            Some(VHit::MenuItem(i)) => Some(i),
                            _ => v.menu_hover.filter(|_| matches!(hit, Some(VHit::Menu))),
                        };
                        if v.hover != hit || v.menu_hover != menu_hover {
                            v.hover = hit;
                            v.menu_hover = menu_hover;
                            self.dirty = true;
                        }
                    }
                }
            }
            MouseKind::Down | MouseKind::DoubleClick => {
                let Some(hit) = self.viewer_hit(m.x, m.y) else { return };
                self.viewer_press(p, hit, m, now);
            }
            // the right button belongs to the carousel's Copy menu
            MouseKind::RightDown => {}
        }
    }

    fn viewer_press(&mut self, p: &mut dyn Platform, hit: VHit, m: MouseEvent, now: u64) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.confirm.is_some() {
            match hit {
                VHit::ConfirmOk => self.confirm_ok(p, now),
                VHit::ConfirmCancel => {
                    v.confirm = None;
                    self.dirty = true;
                }
                _ => {}
            }
            return;
        }
        if v.menu.is_some() {
            match hit {
                VHit::MenuItem(i) => {
                    let a = v.menu.as_ref().and_then(|m| m.get(i).copied());
                    let enabled = a.is_some_and(|a| v.menu_item(a).enabled);
                    if let (Some(a), true) = (a, enabled) {
                        self.run_action(p, a, now);
                    }
                }
                VHit::Menu => {}
                _ => {
                    // a click anywhere else only closes the menu
                    v.menu = None;
                    self.dirty = true;
                }
            }
            return;
        }
        match hit {
            VHit::Tool(t) => {
                let enabled = v.view("", self.overlay.scale).tools.iter().any(|x| x.tool == t && x.enabled);
                if enabled {
                    self.run_tool(p, t, now);
                }
            }
            VHit::Fullscreen => self.toggle_fullscreen(p, now),
            VHit::Back => self.close_viewer(p, now),
            VHit::FindPrev => self.find_step(false),
            VHit::FindNext => self.find_step(true),
            VHit::FindClose => {
                v.find = None;
                self.dirty = true;
            }
            VHit::InfoClose => {
                v.info_open = false;
                self.clamp_center();
                self.dirty = true;
            }
            VHit::Text { pos } => self.viewer_text_press(m.kind, pos, now),
            VHit::Image => {
                if v.ocr_selecting {
                    if v.ocr_running.is_some() { return; }
                    if v.ocr_regions.len() >= pasteroll_core::ocr_regions::MAX_REGIONS {
                        v.toast("Up to 16 areas; undo or clear an area to draw another.", now);
                        self.dirty = true;
                        return;
                    }
                    if let Some(point) = self.ocr_image_point(m.x, m.y, false) {
                        if let Some(v) = self.viewer.as_mut() { v.ocr_stroke = vec![point]; }
                        self.dirty = true;
                    }
                    return;
                }
                if m.kind == MouseKind::DoubleClick {
                    // double-click: real size at that point, or back to fit
                    if v.zoom.is_none() {
                        self.set_zoom(Some(1.0 / self.overlay.scale.max(0.1)), Some((m.x, m.y)));
                    } else {
                        self.set_zoom(None, None);
                    }
                } else {
                    v.pan = Some(((m.x, m.y), v.center));
                }
            }
            _ => {}
        }
    }

    fn ocr_image_point(&self, x: f32, y: f32, clamp: bool) -> Option<(f32, f32)> {
        if !x.is_finite() || !y.is_finite() { return None; }
        let (g, w, h, fit) = self.picture_geometry()?;
        let v = self.viewer.as_ref()?;
        let s = self.overlay.scale.max(0.1);
        let (mut x, mut y) = (x / s, y / s);
        if !clamp && !g.content.contains(x, y) { return None; }
        x = x.clamp(g.content.x, g.content.x + g.content.w);
        y = y.clamp(g.content.y, g.content.y + g.content.h);
        let zoom = v.zoom.unwrap_or(fit);
        let point = (v.center.0 + (x - g.content.x - g.content.w / 2.0) / zoom,
            v.center.1 + (y - g.content.y - g.content.h / 2.0) / zoom);
        if !clamp && (point.0 < 0.0 || point.1 < 0.0 || point.0 > w as f32 || point.1 > h as f32) {
            return None;
        }
        Some((point.0.clamp(0.0, w as f32), point.1.clamp(0.0, h as f32)))
    }

    fn append_ocr_point(&mut self, x: f32, y: f32) {
        let Some(point) = self.ocr_image_point(x, y, true) else { return };
        let Some(v) = self.viewer.as_mut() else { return };
        if v.ocr_stroke.last() == Some(&point) { return; }
        if v.ocr_stroke.len() >= pasteroll_core::ocr_regions::MAX_POINTS {
            v.ocr_stroke = v.ocr_stroke.iter().step_by(2).copied().collect();
        }
        v.ocr_stroke.push(point);
        self.dirty = true;
    }

    fn finish_ocr_stroke(&mut self, x: f32, y: f32, now: u64) {
        if !self.viewer.as_ref().is_some_and(|v| !v.ocr_stroke.is_empty()) { return; }
        self.append_ocr_point(x, y);
        let Some(v) = self.viewer.as_mut() else { return };
        let region = std::mem::take(&mut v.ocr_stroke);
        if pasteroll_core::ocr_regions::valid(&region) {
            v.ocr_regions.push(region);
        } else {
            v.toast("Draw a loop around the text; a click or line is not an area.", now);
        }
        self.dirty = true;
    }

    fn viewer_text_press(&mut self, kind: MouseKind, pos: usize, now: u64) {
        let Some(v) = self.viewer.as_mut() else { return };
        let Some((san, _)) = v.shown_text() else { return };
        let units: Vec<u16> = san.display.encode_utf16().collect();
        let triple = kind == MouseKind::Down && v.last_dbl_ms.is_some_and(|t| now.saturating_sub(t) <= DOUBLE_CLICK_MS);
        if kind == MouseKind::DoubleClick {
            v.last_dbl_ms = Some(now);
            let s = Selection::word_at(&units, pos);
            v.selection = (!s.is_empty()).then(|| s.range());
            v.selecting = false;
        } else if triple {
            v.last_dbl_ms = None;
            let s = Selection::line_at(&units, pos);
            v.selection = (!s.is_empty()).then(|| s.range());
            v.selecting = false;
        } else {
            let pos = pasteroll_core::select::snap(&units, pos);
            v.anchor = Some(pos);
            v.selection = None;
            v.selecting = true;
        }
        self.dirty = true;
    }

    fn viewer_drag_select(&mut self, x: f32, y: f32) {
        let scale = self.overlay.scale;
        let Some(v) = self.viewer.as_ref() else { return };
        let mut view = v.view("", scale);
        let size = v.size_dip(scale);
        let g = viewer_geometry(&view, size);
        // dragging past the top or bottom scrolls toward the pointer
        let (top, bottom) = (g.text.y * scale, (g.text.y + g.text.h) * scale);
        let dy = if y > bottom { (y - bottom) * 0.5 } else if y < top { -(top - y) * 0.5 } else { 0.0 };
        if dy != 0.0 {
            self.scroll_text(0.0, dy);
            if let (Some(v), ViewerBody::Text { scroll, .. }) = (self.viewer.as_ref(), &mut view.body) {
                *scroll = v.scroll;
            }
        }
        let pos = self.renderer.viewer_text_position(&view, size, scale, x, y);
        view.wipe();
        let Some(v) = self.viewer.as_mut() else { return };
        if let (Some(pos), Some((san, _))) = (pos, v.shown_text()) {
            let units: Vec<u16> = san.display.encode_utf16().collect();
            let pos = pasteroll_core::select::snap(&units, pos);
            if let Some(a) = v.anchor {
                v.selection = (a != pos).then_some((a.min(pos), a.max(pos)));
            }
        }
        self.dirty = true;
    }

    /// Timer work while the viewer is open: the animation, and the toast running out.
    pub(super) fn viewer_tick(&mut self, p: &mut dyn Platform, now: u64) {
        if self.viewer.as_ref().is_some_and(|v| v.animating()) {
            self.step_anim(p, now);
            return;
        }
        if let Some(v) = self.viewer.as_mut() {
            if v.toast.as_ref().is_some_and(|t| now >= t.1) {
                v.toast = None;
                self.dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_ocr_jobs_remain_bounded_until_the_worker_finishes() {
        let jobs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let first = OcrJob::acquire(&jobs).unwrap();
        let second = OcrJob::acquire(&jobs).unwrap();
        assert!(OcrJob::acquire(&jobs).is_none());
        drop(first);
        assert!(OcrJob::acquire(&jobs).is_some());
        drop(second);
        assert_eq!(jobs.load(std::sync::atomic::Ordering::Acquire), 0);
    }

    #[test]
    fn finding_ignores_case_and_reports_utf16_ranges() {
        assert_eq!(find_matches("Hello hello HELLO", "hello"), vec![(0, 5), (6, 11), (12, 17)]);
        assert_eq!(find_matches("😀ab😀AB", "ab"), vec![(2, 4), (6, 8)]);
        assert!(find_matches("abc", "").is_empty());
        assert!(find_matches("abc", "abcd").is_empty());
    }

    #[test]
    fn the_viewer_takes_three_quarters_of_the_monitor_inside_its_free_area() {
        let mut info = OverlayInfo::new(1.0, (520, 440));
        info.monitor_px = (1920, 0, 2560, 1440);
        info.work_px = (1920, 0, 2560, 1392);
        let r = home_rect(&info);
        assert_eq!((r.2, r.3), (1920, 1080));
        assert_eq!(r.0, 1920 + 320);
        assert!(r.1 >= 0 && r.1 as u32 + r.3 <= 1392);
    }

    #[test]
    fn the_animation_curves_start_and_end_in_place() {
        for f in [ease_out, ease_in_out] {
            assert!(f(0.0).abs() < 1e-6 && (f(1.0) - 1.0).abs() < 1e-6);
            assert!(f(0.5) > 0.0 && f(0.5) < 1.0);
        }
        assert_eq!(lerp_rect((0, 0, 10, 10), (100, 50, 110, 60), 0.5), (50, 25, 60, 35));
    }
}
