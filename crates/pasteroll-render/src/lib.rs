//! Portable overlay renderer.
//!
//! Turns a [`ViewModel`] into pixels and answers hit-test questions about them. No window, no OS
//! calls: each platform backend creates a non-activating window and presents the [`Frame`].
//!
//! Layout (DIP, scaled by the display factor):
//!
//! ```text
//! +----------------------------------------------+
//! | [ search ............................ ][All v]|   search bar
//! |  - row (condensed)                           |
//! |  - row                                       |
//! | +------------------------------------------+ |
//! | | active item: larger, full preview,       | |   active row
//! | | selectable text or image thumbnail       | |
//! | +------------------------------------------+ |
//! |  - row                                       |
//! |  - row                          3 of 120     |
//! +----------------------------------------------+
//! ```

mod editor;
mod icons;
mod textmap;
mod viewer;

pub use editor::{
    crop_part_at, editor_hit, editor_layout, fit_rect, ButtonView, Col, CropPart, CropView, EHit, ELayout, EditorView, FieldView, Item,
    ItemKind, Placed, PopupAnchor, PopupView, Top, TopView,
};
pub use icons::Icon;
pub use viewer::{
    fit_zoom, viewer_geometry, ConfirmView, FindView, Morph, MenuItemView, Tool, ToolView, VGeom, VHit, ViewerBody, ViewerView,
};

use cosmic_text::{Attrs, Buffer, FeatureTag, Family, FontFeatures, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use textmap::TextMap;
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform,
};

pub use pasteroll_core::image::Rgba;

/// Natural window size in DIP.
mod ui;
pub use ui::{hit, list_row_at, list_visible_rows, slider_index_at, slider_track, Chrome, UiView, WKind, Widget, LIST_ROW};

pub const WIDTH: f32 = 520.0;
pub const HEIGHT: f32 = 440.0;

const OUTER: f32 = 8.0;
const INNER: f32 = 10.0;
const RADIUS: f32 = 8.0;
const BAR_H: f32 = 34.0;
const FILTER_W: f32 = 118.0;
const ROW_H: f32 = 28.0;
const FOOTER_H: f32 = 20.0;
const DROPDOWN_ITEM_H: f32 = 28.0;
const DROPDOWN_W: f32 = 170.0;
const FONT_PX: f32 = 14.0;
const SMALL_PX: f32 = 12.0;
const BODY_LINE: f32 = 19.0;

type Rgb = (u8, u8, u8);
const BACKGROUND: Rgb = (28, 28, 32);
const SURFACE: Rgb = (37, 37, 43);
const SURFACE_BORDER: Rgb = (50, 50, 58);
const FIELD: Rgb = (46, 46, 54);
const ACTIVE_ROW: Rgb = (44, 48, 56);
const FOREGROUND: Rgb = (236, 236, 240);
const MUTED: Rgb = (150, 150, 162);
const ACCENT: Rgb = (47, 174, 151);
const DANGER: Rgb = (232, 96, 96);
const MENU: Rgb = (52, 52, 62);
const MENU_BORDER: Rgb = (74, 74, 88);

// ---------------------------------------------------------------- view model

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Text,
    Image,
    Files,
}

#[derive(Debug, Clone)]
pub struct RowView {
    pub kind: RowKind,
    pub pinned: bool,
    /// Shown when the row is condensed. One line.
    pub title: String,
    /// Shown instead of `title` when the row is the active one (the body below already shows the
    /// full content): for example "Text, 1,284 characters". Falls back to `title` when empty.
    pub meta: String,
    /// Character ranges (not bytes) of `title` to emphasise: the query matches.
    pub highlights: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub enum Body {
    /// `display` is what is drawn. `selection` is a UTF-16 range into it. `scroll` is in px.
    Text { display: String, selection: Option<(usize, usize)>, scroll: f32 },
    /// `None` while the thumbnail is still being decoded.
    Image(Option<std::sync::Arc<Rgba>>),
    Message(String),
}

#[derive(Debug, Clone)]
pub struct Dropdown {
    pub items: Vec<String>,
    pub selected: usize,
    pub hover: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ViewModel {
    pub query: String,
    pub placeholder: String,
    pub filter_label: String,
    pub dropdown: Option<Dropdown>,
    /// A window of rows around the active one.
    pub rows: Vec<RowView>,
    /// Index into `rows` of the active row (ignored when `rows` is empty).
    pub active: usize,
    pub body: Body,
    /// UTF-16 ranges of the displayed body text to mark as search matches.
    pub body_matches: Vec<(usize, usize)>,
    pub footer: String,
    /// The active card shows its Expand and recycle-bin buttons.
    pub card_buttons: bool,
    /// The clips shown are the recycle bin's: the card offers Restore and permanent deletion.
    pub in_trash: bool,
    /// The active card has been lifted out into the expanded viewer: its place is drawn empty.
    pub lifted: bool,
    /// What the pointer is over (buttons light up and explain themselves).
    pub hover: Option<Hit>,
}

impl ViewModel {
    /// Overwrites every string that may hold clip text. Called after a frame has been drawn: the
    /// view model is rebuilt for each frame and would otherwise be freed with its text intact.
    pub fn wipe(&mut self) {
        use zeroize::Zeroize;
        self.query.zeroize();
        for r in &mut self.rows {
            r.title.zeroize();
            r.meta.zeroize();
        }
        match &mut self.body {
            Body::Text { display, .. } => display.zeroize(),
            Body::Message(m) => m.zeroize(),
            Body::Image(_) => {}
        }
        self.footer.zeroize();
    }
}

impl Default for ViewModel {
    fn default() -> Self {
        ViewModel {
            query: String::new(),
            placeholder: "Search clipboard".into(),
            filter_label: "Everything".into(),
            dropdown: None,
            rows: Vec::new(),
            active: 0,
            body: Body::Message(String::new()),
            body_matches: Vec::new(),
            footer: String::new(),
            card_buttons: false,
            in_trash: false,
            lifted: false,
            hover: None,
        }
    }
}

// ---------------------------------------------------------------- geometry / hit testing

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    fn inset(&self, d: f32) -> Rect {
        Rect { x: self.x + d, y: self.y + d, w: (self.w - 2.0 * d).max(0.0), h: (self.h - 2.0 * d).max(0.0) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Query,
    FilterButton,
    DropdownItem(usize),
    /// Index into `ViewModel::rows`.
    Row(usize),
    /// The active item's body. `pos` is a UTF-16 offset into the displayed text.
    Body { pos: usize },
    /// The active card's buttons.
    Expand,
    Delete,
    Restore,
    Nothing,
}

/// Side of a card button, in DIP.
pub const CARD_BUTTON: f32 = 24.0;

#[derive(Debug, Clone)]
pub struct Geometry {
    pub surface: Rect,
    pub query_box: Rect,
    pub filter_button: Rect,
    pub list: Rect,
    /// One rect per row in `ViewModel::rows` (rows that do not fit are `None`).
    pub rows: Vec<Option<Rect>>,
    /// The body area inside the active row.
    pub body: Option<Rect>,
    /// The active card's buttons (when [`ViewModel::card_buttons`] is set).
    pub expand: Option<Rect>,
    pub delete: Option<Rect>,
    pub restore: Option<Rect>,
    pub footer: Rect,
    pub dropdown: Option<Rect>,
    pub dropdown_items: Vec<Rect>,
    pub scale: f32,
}

pub fn geometry(vm: &ViewModel, size_dip: (f32, f32), scale: f32) -> Geometry {
    let (w, h) = size_dip;
    let surface = Rect { x: OUTER, y: OUTER, w: w - 2.0 * OUTER, h: h - 2.0 * OUTER };
    let inner = surface.inset(INNER);
    let bar = Rect { x: inner.x, y: inner.y, w: inner.w, h: BAR_H };
    let filter_button = Rect { x: bar.x + bar.w - FILTER_W, y: bar.y, w: FILTER_W, h: BAR_H };
    let query_box = Rect { x: bar.x, y: bar.y, w: bar.w - FILTER_W - 6.0, h: BAR_H };
    let footer = Rect { x: inner.x, y: inner.y + inner.h - FOOTER_H, w: inner.w, h: FOOTER_H };
    let list_top = bar.y + BAR_H + 8.0;
    let list = Rect { x: inner.x, y: list_top, w: inner.w, h: (footer.y - 4.0 - list_top).max(0.0) };

    let mut rows: Vec<Option<Rect>> = vec![None; vm.rows.len()];
    let mut body = None;
    let (mut expand, mut delete, mut restore) = (None, None, None);
    if !vm.rows.is_empty() {
        let active = vm.active.min(vm.rows.len() - 1);
        // The active row takes about 55% of the list, never less than 110 DIP; neighbours are
        // condensed one-liners placed above and below it.
        let active_h = (list.h * 0.55).clamp(110.0, 260.0).min(list.h);
        let slots = ((list.h - active_h).max(0.0) / ROW_H).floor() as usize;
        let want_above = active;
        let want_below = vm.rows.len() - 1 - active;
        // Split the slots roughly evenly; whatever one side cannot use goes to the other.
        let mut above = want_above.min(slots / 2);
        let mut below = want_below.min(slots - above);
        above = want_above.min(slots - below);
        let block_h = (above + below) as f32 * ROW_H + active_h;
        let mut y = list.y + (list.h - block_h).max(0.0) / 2.0;
        for slot in rows.iter_mut().take(active).skip(active - above) {
            *slot = Some(Rect { x: list.x, y, w: list.w, h: ROW_H });
            y += ROW_H;
        }
        let active_rect = Rect { x: list.x, y, w: list.w, h: active_h };
        rows[active] = Some(active_rect);
        // the buttons get a narrow column of their own, so they never cover the clip
        let gutter = if vm.card_buttons { CARD_BUTTON + 2.0 } else { 0.0 };
        body = Some(Rect {
            x: active_rect.x + 10.0,
            y: active_rect.y + ROW_H - 4.0,
            w: active_rect.w - 20.0 - gutter,
            h: (active_rect.h - ROW_H + 4.0 - 8.0).max(0.0),
        });
        if vm.card_buttons && !vm.lifted {
            let bx = active_rect.x + active_rect.w - 4.0 - CARD_BUTTON;
            expand = Some(Rect { x: bx, y: active_rect.y + 2.0, w: CARD_BUTTON, h: CARD_BUTTON });
            let by = active_rect.y + active_rect.h - 4.0 - CARD_BUTTON;
            delete = Some(Rect { x: bx, y: by, w: CARD_BUTTON, h: CARD_BUTTON });
            if vm.in_trash {
                restore = Some(Rect { x: bx, y: by - CARD_BUTTON - 4.0, w: CARD_BUTTON, h: CARD_BUTTON });
            }
        }
        y += active_h;
        for slot in rows.iter_mut().skip(active + 1).take(below) {
            *slot = Some(Rect { x: list.x, y, w: list.w, h: ROW_H });
            y += ROW_H;
        }
        below = below.min(want_below);
        let _ = below;
    }

    let (dropdown, dropdown_items) = match &vm.dropdown {
        Some(d) => {
            let rect = Rect {
                x: filter_button.x + filter_button.w - DROPDOWN_W,
                y: filter_button.y + filter_button.h + 4.0,
                w: DROPDOWN_W,
                h: d.items.len() as f32 * DROPDOWN_ITEM_H + 8.0,
            };
            let items = (0..d.items.len())
                .map(|i| Rect {
                    x: rect.x + 4.0,
                    y: rect.y + 4.0 + i as f32 * DROPDOWN_ITEM_H,
                    w: rect.w - 8.0,
                    h: DROPDOWN_ITEM_H,
                })
                .collect();
            (Some(rect), items)
        }
        None => (None, Vec::new()),
    };

    Geometry {
        surface,
        query_box,
        filter_button,
        list,
        rows,
        body,
        expand,
        delete,
        restore,
        footer,
        dropdown,
        dropdown_items,
        scale,
    }
}

// ---------------------------------------------------------------- frame

/// Opaque pixels of the overlay, BGRA, 8 bits per channel, tightly packed.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Frame {
    /// A copy with every colour multiplied by `k` (0 = black, 1 = unchanged): the carousel is shown
    /// dimmed behind the expanded viewer.
    pub fn dimmed(&self, k: f32) -> Frame {
        let k = (k.clamp(0.0, 1.0) * 256.0) as u32;
        let mut bgra = self.bgra.clone();
        for px in bgra.chunks_exact_mut(4) {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * k) >> 8) as u8;
            }
        }
        Frame { width: self.width, height: self.height, bgra }
    }

    /// The pixels of a rectangle (clamped to the frame).
    pub fn crop(&self, x: i32, y: i32, w: u32, h: u32) -> Frame {
        let x0 = x.clamp(0, self.width as i32) as u32;
        let y0 = y.clamp(0, self.height as i32) as u32;
        let w = w.min(self.width - x0).max(1);
        let h = h.min(self.height - y0).max(1);
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for row in y0..y0 + h {
            let start = ((row.min(self.height - 1) * self.width + x0) * 4) as usize;
            let end = (start + (w * 4) as usize).min(self.bgra.len());
            bgra.extend_from_slice(&self.bgra[start..end]);
        }
        bgra.resize((w * h * 4) as usize, 255);
        Frame { width: w, height: h, bgra }
    }

    /// `(r, g, b)` at a pixel.
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * self.width + x) * 4) as usize;
        (self.bgra[i + 2], self.bgra[i + 1], self.bgra[i])
    }
}

// ---------------------------------------------------------------- renderer

pub struct Renderer {
    fs: FontSystem,
    cache: SwashCache,
    family: String,
    /// The expanded viewer's shaped text and picture.
    vtext: Option<viewer::VText>,
    vimg: viewer::ImgCache,
}

fn color(rgb: Rgb, a: f32) -> Color {
    Color::from_rgba8(rgb.0, rgb.1, rgb.2, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn paint(rgb: Rgb, a: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(rgb, a));
    p.anti_alias = true;
    p
}

fn rounded(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let k = 0.552_284_75 * radius;
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + radius, y);
    pb.line_to(x + w - radius, y);
    pb.cubic_to(x + w - radius + k, y, x + w, y + radius - k, x + w, y + radius);
    pb.line_to(x + w, y + h - radius);
    pb.cubic_to(x + w, y + h - radius + k, x + w - radius + k, y + h, x + w - radius, y + h);
    pb.line_to(x + radius, y + h);
    pb.cubic_to(x + radius - k, y + h, x, y + h - radius + k, x, y + h - radius);
    pb.line_to(x, y + radius);
    pb.cubic_to(x, y + radius - k, x + radius - k, y, x + radius, y);
    pb.close();
    pb.finish()
}

struct Canvas {
    pm: Pixmap,
    s: f32,
}

impl Canvas {
    fn fill(&mut self, r: Rect, radius: f32, rgb: Rgb, a: f32) {
        let r = scaled(r, self.s);
        if let Some(path) = rounded(r, radius * self.s) {
            self.pm.fill_path(&path, &paint(rgb, a), FillRule::Winding, Transform::identity(), None);
        }
    }

    fn stroke(&mut self, r: Rect, radius: f32, rgb: Rgb, a: f32, width: f32) {
        let r = scaled(r, self.s);
        if let Some(path) = rounded(r, radius * self.s) {
            let st = Stroke { width: width * self.s, ..Stroke::default() };
            self.pm.stroke_path(&path, &paint(rgb, a), &st, Transform::identity(), None);
        }
    }

    fn line(&mut self, pts: &[(f32, f32)], rgb: Rgb, width: f32) {
        let s = self.s;
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(x * s, y * s);
            } else {
                pb.line_to(x * s, y * s);
            }
        }
        if let Some(p) = pb.finish() {
            let st = Stroke { width: width * s, ..Stroke::default() };
            self.pm.stroke_path(&p, &paint(rgb, 1.0), &st, Transform::identity(), None);
        }
    }

    /// Source-over blend of a straight-alpha colour into a pixel rectangle, clipped to `clip`.
    fn blend_px(&mut self, x: i32, y: i32, w: u32, h: u32, c: cosmic_text::Color, clip: (i32, i32, i32, i32)) {
        let (cx0, cy0, cx1, cy1) = clip;
        let pw = self.pm.width() as i32;
        let ph = self.pm.height() as i32;
        let a = c.a() as u32;
        if a == 0 {
            return;
        }
        let data = self.pm.data_mut();
        let inv = 255 - a;
        for yy in y.max(cy0).max(0)..(y + h as i32).min(cy1).min(ph) {
            for xx in x.max(cx0).max(0)..(x + w as i32).min(cx1).min(pw) {
                let i = ((yy * pw + xx) * 4) as usize;
                // premultiplied destination, opaque result
                data[i] = ((c.r() as u32 * a + data[i] as u32 * inv + 127) / 255) as u8;
                data[i + 1] = ((c.g() as u32 * a + data[i + 1] as u32 * inv + 127) / 255) as u8;
                data[i + 2] = ((c.b() as u32 * a + data[i + 2] as u32 * inv + 127) / 255) as u8;
                data[i + 3] = 255;
            }
        }
    }
}

impl Renderer {
    pub fn new() -> Renderer {
        let fs = FontSystem::new();
        let wanted = ["Segoe UI", "SF Pro Text", ".AppleSystemUIFont", "Noto Sans", "DejaVu Sans", "Ubuntu", "Arial"];
        let family = {
            let db = fs.db();
            wanted
                .iter()
                .find(|w| db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(w))))
                .map(|s| (*s).to_owned())
                .unwrap_or_default()
        };
        Renderer { fs, cache: SwashCache::new(), family, vtext: None, vimg: viewer::ImgCache::default() }
    }

    fn shape(&mut self, text: &str, px: f32, line: f32, width: Option<f32>, wrap: Wrap) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fs, Metrics::new(px, line));
        buffer.set_wrap(wrap);
        buffer.set_size(width, None);
        let fam = self.family.clone();
        // No ligatures: a selection is made per character, and a ligature ("fi") would make two
        // characters one indivisible glyph that cannot be selected in parts.
        let mut features = FontFeatures::new();
        features.disable(FeatureTag::STANDARD_LIGATURES);
        features.disable(FeatureTag::CONTEXTUAL_LIGATURES);
        features.disable(FeatureTag::DISCRETIONARY_LIGATURES);
        let attrs = if fam.is_empty() { Attrs::new() } else { Attrs::new().family(Family::Name(&fam)) };
        let attrs = attrs.font_features(features);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fs, false);
        buffer
    }

    fn measure(&mut self, text: &str, px: f32, line: f32) -> f32 {
        let buffer = self.shape(text, px, line, None, Wrap::None);
        buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }

    /// Draws `text` with its top-left at `(x, y)` px, clipped to `clip` (px rect). Returns the
    /// width of the widest line in px.
    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        cv: &mut Canvas,
        text: &str,
        (x, y): (f32, f32),
        px: f32,
        line: f32,
        width: Option<f32>,
        wrap: Wrap,
        rgb: Rgb,
        clip: Rect,
    ) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let mut buffer = self.shape(text, px, line, width, wrap);
        let clip_px = (
            clip.x.floor() as i32,
            clip.y.floor() as i32,
            (clip.x + clip.w).ceil() as i32,
            (clip.y + clip.h).ceil() as i32,
        );
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        let base = cosmic_text::Color::rgb(rgb.0, rgb.1, rgb.2);
        buffer.draw(&mut self.fs, &mut self.cache, base, |gx, gy, gw, gh, c| {
            cv.blend_px(ox + gx, oy + gy, gw, gh, c, clip_px);
        });
        buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }

    // ------------------------------------------------------------------ public API

    pub fn render(&mut self, vm: &ViewModel, size_dip: (f32, f32), scale: f32) -> Frame {
        let (wpx, hpx) = ((size_dip.0 * scale).round().max(1.0) as u32, (size_dip.1 * scale).round().max(1.0) as u32);
        let mut cv = Canvas { pm: Pixmap::new(wpx, hpx).expect("pixmap"), s: scale };
        cv.pm.fill(color(BACKGROUND, 1.0));
        let g = geometry(vm, size_dip, scale);
        let s = scale;

        cv.fill(g.surface, RADIUS, SURFACE, 1.0);
        cv.stroke(g.surface, RADIUS, SURFACE_BORDER, 1.0, 1.0);

        self.draw_search_bar(&mut cv, vm, &g);
        self.draw_rows(&mut cv, vm, &g);
        self.draw_card_buttons(&mut cv, vm, &g);

        if !vm.footer.is_empty() {
            let f = g.footer;
            let tw = self.measure(&vm.footer, SMALL_PX * s, 16.0 * s);
            self.draw_text(
                &mut cv,
                &vm.footer,
                ((f.x + f.w) * s - tw, (f.y + 2.0) * s),
                SMALL_PX * s,
                16.0 * s,
                None,
                Wrap::None,
                MUTED,
                scaled(f, s),
            );
        }

        self.draw_card_tooltip(&mut cv, vm, &g, size_dip);

        if let (Some(rect), Some(dd)) = (g.dropdown, &vm.dropdown) {
            cv.fill(rect, 6.0, (52, 52, 62), 1.0);
            cv.stroke(rect, 6.0, (74, 74, 88), 1.0, 1.0);
            for (i, item) in dd.items.iter().enumerate() {
                let r = g.dropdown_items[i];
                if dd.hover == Some(i) {
                    cv.fill(r, 4.0, ACCENT, 0.28);
                }
                let col = if i == dd.selected { ACCENT } else { FOREGROUND };
                self.draw_text(
                    &mut cv,
                    item,
                    ((r.x + 10.0) * s, (r.y + 6.0) * s),
                    FONT_PX * s,
                    18.0 * s,
                    None,
                    Wrap::None,
                    col,
                    scaled(r, s),
                );
            }
        }

        // tiny-skia holds premultiplied RGBA; the frame is opaque, so swizzle to BGRA directly.
        let mut bgra = cv.pm.take();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Frame { width: wpx, height: hpx, bgra }
    }

    fn draw_search_bar(&mut self, cv: &mut Canvas, vm: &ViewModel, g: &Geometry) {
        let s = g.scale;
        cv.fill(g.query_box, 6.0, FIELD, 1.0);
        cv.fill(g.filter_button, 6.0, FIELD, 1.0);
        if vm.dropdown.is_some() {
            cv.stroke(g.filter_button, 6.0, ACCENT, 1.0, 1.0);
        }
        // magnifier glyph
        let (mx, my) = (g.query_box.x + 14.0, g.query_box.y + BAR_H / 2.0 - 1.0);
        if let Some(path) = {
            let mut pb = PathBuilder::new();
            pb.push_circle(mx * s, my * s, 5.0 * s);
            pb.finish()
        } {
            let st = Stroke { width: 1.6 * s, ..Stroke::default() };
            cv.pm.stroke_path(&path, &paint(MUTED, 1.0), &st, Transform::identity(), None);
        }
        cv.line(&[(mx + 3.6, my + 3.6), (mx + 8.0, my + 8.0)], MUTED, 1.8);

        let text_pos = ((g.query_box.x + 32.0) * s, (g.query_box.y + 8.0) * s);
        let clip = scaled(
            Rect { x: g.query_box.x + 28.0, y: g.query_box.y, w: g.query_box.w - 36.0, h: g.query_box.h },
            s,
        );
        if vm.query.is_empty() {
            self.draw_text(cv, &vm.placeholder, text_pos, FONT_PX * s, 18.0 * s, None, Wrap::None, MUTED, clip);
        } else {
            let w = self.draw_text(cv, &vm.query, text_pos, FONT_PX * s, 18.0 * s, None, Wrap::None, FOREGROUND, clip);
            let cx = (text_pos.0 + w + 1.0 * s).min(clip.x + clip.w - 2.0 * s);
            if let Some(r) = tiny_skia::Rect::from_xywh(cx, text_pos.1 + 1.0 * s, 1.5 * s, 16.0 * s) {
                cv.pm.fill_rect(r, &paint(ACCENT, 1.0), Transform::identity(), None);
            }
        }
        let f = g.filter_button;
        self.draw_text(
            cv,
            &vm.filter_label,
            ((f.x + 10.0) * s, (f.y + 8.0) * s),
            FONT_PX * s,
            18.0 * s,
            None,
            Wrap::None,
            FOREGROUND,
            scaled(Rect { x: f.x + 6.0, y: f.y, w: f.w - 26.0, h: f.h }, s),
        );
        let (cx, cy) = (f.x + f.w - 14.0, f.y + BAR_H / 2.0);
        cv.line(&[(cx - 4.0, cy - 2.0), (cx, cy + 2.0), (cx + 4.0, cy - 2.0)], MUTED, 1.6);
    }

    fn draw_rows(&mut self, cv: &mut Canvas, vm: &ViewModel, g: &Geometry) {
        let s = g.scale;
        if vm.rows.is_empty() {
            if let Body::Message(m) = &vm.body {
                let l = g.list;
                let tw = self.measure(m, FONT_PX * s, 18.0 * s);
                self.draw_text(
                    cv,
                    m,
                    ((l.x + l.w / 2.0) * s - tw / 2.0, (l.y + l.h / 2.0 - 9.0) * s),
                    FONT_PX * s,
                    18.0 * s,
                    None,
                    Wrap::None,
                    MUTED,
                    scaled(l, s),
                );
            }
            return;
        }
        let active = vm.active.min(vm.rows.len() - 1);
        for (i, row) in vm.rows.iter().enumerate() {
            let Some(r) = g.rows[i] else { continue };
            let is_active = i == active;
            if is_active && vm.lifted {
                // the card is out in the viewer: leave its place empty
                cv.fill(r, 8.0, (22, 22, 26), 1.0);
                cv.stroke(r, 8.0, SURFACE_BORDER, 1.0, 1.0);
                continue;
            }
            if is_active {
                cv.fill(r, 8.0, ACTIVE_ROW, 1.0);
                cv.stroke(r, 8.0, ACCENT, 0.9, 1.2);
            }
            self.draw_icon(cv, row.kind, r.x + 12.0, r.y + (ROW_H - 12.0) / 2.0, if is_active { ACCENT } else { MUTED });
            let title_x = r.x + 32.0;
            // the active card's Expand button sits at the right of its title line
            let buttons = if is_active && vm.card_buttons { CARD_BUTTON + 6.0 } else { 0.0 };
            let right_pad = buttons + if row.pinned { 28.0 } else { 12.0 };
            let clip = scaled(Rect { x: title_x, y: r.y, w: (r.w - 32.0 - right_pad).max(0.0), h: ROW_H }, s);
            let col = if is_active { FOREGROUND } else { MUTED };
            let title = if is_active && !row.meta.is_empty() { row.meta.clone() } else { first_line(&row.title) };
            // query matches are highlighted behind the title text (condensed rows only)
            let chars: Vec<char> = title.chars().collect();
            let highlights: &[(usize, usize)] = if is_active && !row.meta.is_empty() { &[] } else { &row.highlights };
            for &(a, b) in highlights {
                let (a, b) = (a.min(chars.len()), b.min(chars.len()));
                if b <= a {
                    continue;
                }
                let before: String = chars[..a].iter().collect();
                let mid: String = chars[a..b].iter().collect();
                let x0 = self.measure(&before, FONT_PX * s, 18.0 * s);
                let wmid = self.measure(&mid, FONT_PX * s, 18.0 * s);
                cv.fill(
                    Rect { x: title_x + x0 / s - 1.0, y: r.y + 5.0, w: wmid / s + 2.0, h: ROW_H - 10.0 },
                    3.0,
                    ACCENT,
                    0.30,
                );
            }
            self.draw_text(
                cv,
                &title,
                (title_x * s, (r.y + 5.0) * s),
                FONT_PX * s,
                18.0 * s,
                None,
                Wrap::None,
                col,
                clip,
            );
            if row.pinned {
                draw_star(cv, r.x + r.w - 18.0 - buttons, r.y + ROW_H / 2.0, 6.0, ACCENT);
            }
        }
        if let (Some(b), false) = (g.body, vm.lifted) {
            self.draw_body(cv, vm, b, s);
        }
    }

    fn draw_card_buttons(&mut self, cv: &mut Canvas, vm: &ViewModel, g: &Geometry) {
        let buttons = [
            (g.expand, Hit::Expand, Icon::Expand),
            (g.restore, Hit::Restore, Icon::Restore),
            (g.delete, Hit::Delete, if vm.in_trash { Icon::TrashForever } else { Icon::Trash }),
        ];
        for (rect, hit, icon) in buttons {
            let Some(r) = rect else { continue };
            let hot = vm.hover == Some(hit);
            // a backing plate keeps the icon legible over an image preview
            cv.fill(r, 6.0, if hot { (62, 68, 80) } else { ACTIVE_ROW }, if hot { 1.0 } else { 0.85 });
            let col = match (hot, hit) {
                (true, Hit::Delete) => DANGER,
                (true, _) => FOREGROUND,
                _ => MUTED,
            };
            icons::draw(cv, icon, r, 14.0, col);
        }
    }

    /// A short explanation next to the card button under the pointer.
    fn draw_card_tooltip(&mut self, cv: &mut Canvas, vm: &ViewModel, g: &Geometry, size_dip: (f32, f32)) {
        let (rect, text) = match vm.hover {
            Some(Hit::Expand) => (g.expand, "Expand (Ctrl+E)"),
            Some(Hit::Restore) => (g.restore, "Restore from the recycle bin (Ctrl+Z)"),
            Some(Hit::Delete) if vm.in_trash => (g.delete, "Delete permanently (Del)"),
            Some(Hit::Delete) => (g.delete, "Move to recycle bin (Del). Shift+click deletes permanently"),
            _ => return,
        };
        let Some(r) = rect else { return };
        tooltip(self, cv, text, r, size_dip, true);
    }

    fn draw_icon(&mut self, cv: &mut Canvas, kind: RowKind, x: f32, y: f32, rgb: Rgb) {
        match kind {
            RowKind::Text => {
                for k in 0..3 {
                    let yy = y + 2.0 + k as f32 * 4.0;
                    let w = if k == 2 { 7.0 } else { 12.0 };
                    cv.line(&[(x, yy), (x + w, yy)], rgb, 1.4);
                }
            }
            RowKind::Image => {
                cv.stroke(Rect { x, y: y + 1.0, w: 12.0, h: 10.0 }, 2.0, rgb, 1.0, 1.4);
                cv.line(
                    &[(x + 2.0, y + 9.0), (x + 5.0, y + 5.0), (x + 7.5, y + 8.0), (x + 9.0, y + 6.5), (x + 11.0, y + 9.0)],
                    rgb,
                    1.4,
                );
            }
            RowKind::Files => {
                cv.stroke(Rect { x, y: y + 3.0, w: 12.0, h: 8.0 }, 1.5, rgb, 1.0, 1.4);
                cv.line(&[(x, y + 3.0), (x + 4.0, y + 3.0), (x + 5.5, y + 1.0), (x + 9.0, y + 1.0)], rgb, 1.4);
            }
        }
    }

    fn draw_body(&mut self, cv: &mut Canvas, vm: &ViewModel, b: Rect, s: f32) {
        let clip = scaled(b, s);
        match &vm.body {
            Body::Text { display, selection, scroll } => {
                let map = TextMap::new(display);
                let mut buffer = self.shape(display, FONT_PX * s, BODY_LINE * s, Some(b.w * s), Wrap::WordOrGlyph);
                let (ox, oy) = (b.x * s, b.y * s - scroll);
                // search matches first, so a selection draws over them
                let mut ranges: Vec<((usize, usize), Rgb, f32)> =
                    vm.body_matches.iter().map(|r| (*r, (230, 190, 60), 0.32)).collect();
                if let Some((a, z)) = selection {
                    ranges.push(((*a.min(z), *a.max(z)), ACCENT, 0.55));
                }
                for ((a, z), rgb, alpha) in ranges {
                    if z > a {
                        let c0 = map.cursor(display, a);
                        let c1 = map.cursor(display, z);
                        let spans: Vec<(f32, f32, f32, f32)> = buffer
                            .layout_runs()
                            // `highlight` treats every line outside the cursors' line range as
                            // fully selected, so only ask it about lines inside the range.
                            .filter(|run| c0.line <= run.line_i && run.line_i <= c1.line)
                            .flat_map(|run| {
                                let (top, h) = (run.line_top, run.line_height);
                                run.highlight(c0, c1).map(move |(x, w)| (x, top, w, h)).collect::<Vec<_>>()
                            })
                            .collect();
                        for (x, top, w, h) in spans {
                            let rect = Rect { x: ox + x, y: oy + top, w, h };
                            if let Some(r) = intersect(rect, clip) {
                                if let Some(path) = rounded(r, 2.0 * s) {
                                    cv.pm.fill_path(
                                        &path,
                                        &paint(rgb, alpha),
                                        FillRule::Winding,
                                        Transform::identity(),
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
                let clip_px = (
                    clip.x.floor() as i32,
                    clip.y.floor() as i32,
                    (clip.x + clip.w).ceil() as i32,
                    (clip.y + clip.h).ceil() as i32,
                );
                let base = cosmic_text::Color::rgb(FOREGROUND.0, FOREGROUND.1, FOREGROUND.2);
                let (oxi, oyi) = (ox.round() as i32, oy.round() as i32);
                buffer.draw(&mut self.fs, &mut self.cache, base, |gx, gy, gw, gh, c| {
                    cv.blend_px(oxi + gx, oyi + gy, gw, gh, c, clip_px);
                });
            }
            Body::Message(m) => {
                self.draw_text(
                    cv,
                    m,
                    (b.x * s, b.y * s),
                    FONT_PX * s,
                    BODY_LINE * s,
                    Some(b.w * s),
                    Wrap::WordOrGlyph,
                    MUTED,
                    clip,
                );
            }
            Body::Image(None) => {}
            Body::Image(Some(img)) => {
                if img.width == 0 || img.height == 0 {
                    return;
                }
                // Fit inside the body; allow up to device scale so small thumbnails stay crisp.
                let k = (b.w * s / img.width as f32).min(b.h * s / img.height as f32).min(s.max(1.0));
                let (dw, dh) = (img.width as f32 * k, img.height as f32 * k);
                let (dx, dy) = (b.x * s + (b.w * s - dw) / 2.0, b.y * s + (b.h * s - dh) / 2.0);
                if let Some(mut src) = Pixmap::new(img.width, img.height) {
                    let d = src.data_mut();
                    for (i, p) in img.pixels.chunks_exact(4).enumerate() {
                        let a = p[3] as u32;
                        d[i * 4] = ((p[0] as u32 * a + 127) / 255) as u8;
                        d[i * 4 + 1] = ((p[1] as u32 * a + 127) / 255) as u8;
                        d[i * 4 + 2] = ((p[2] as u32 * a + 127) / 255) as u8;
                        d[i * 4 + 3] = p[3];
                    }
                    let pp = PixmapPaint { opacity: 1.0, blend_mode: BlendMode::SourceOver, quality: FilterQuality::Bilinear };
                    cv.pm.draw_pixmap(0, 0, src.as_ref(), &pp, Transform::from_row(k, 0.0, 0.0, k, dx, dy), None);
                }
            }
        }
    }

    /// What is under `(px, py)` (pixels, frame coordinates).
    pub fn hit_test(&mut self, vm: &ViewModel, size_dip: (f32, f32), scale: f32, px: f32, py: f32) -> Hit {
        let g = geometry(vm, size_dip, scale);
        let (x, y) = (px / scale, py / scale);
        if let Some(r) = g.dropdown {
            if r.contains(x, y) {
                for (i, it) in g.dropdown_items.iter().enumerate() {
                    if it.contains(x, y) {
                        return Hit::DropdownItem(i);
                    }
                }
                return Hit::Nothing;
            }
        }
        if g.filter_button.contains(x, y) {
            return Hit::FilterButton;
        }
        if g.query_box.contains(x, y) {
            return Hit::Query;
        }
        for (rect, hit) in [(g.expand, Hit::Expand), (g.restore, Hit::Restore), (g.delete, Hit::Delete)] {
            if rect.is_some_and(|r| r.contains(x, y)) {
                return hit;
            }
        }
        if !vm.rows.is_empty() {
            let active = vm.active.min(vm.rows.len() - 1);
            if let Some(b) = g.body {
                if b.contains(x, y) {
                    if let Body::Text { .. } = vm.body {
                        return Hit::Body { pos: self.text_position(vm, size_dip, scale, px, py).unwrap_or(0) };
                    }
                    return Hit::Row(active);
                }
            }
            for (i, r) in g.rows.iter().enumerate() {
                if let Some(r) = r {
                    if r.contains(x, y) {
                        return Hit::Row(i);
                    }
                }
            }
        }
        Hit::Nothing
    }

    /// UTF-16 offset in the active body's displayed text nearest to the pixel position. Positions
    /// above or below the body clamp to its first or last line, which is what a drag needs.
    pub fn text_position(&mut self, vm: &ViewModel, size_dip: (f32, f32), scale: f32, px: f32, py: f32) -> Option<usize> {
        let Body::Text { display, scroll, .. } = &vm.body else { return None };
        let g = geometry(vm, size_dip, scale);
        let b = g.body?;
        let buffer = self.shape(display, FONT_PX * scale, BODY_LINE * scale, Some(b.w * scale), Wrap::WordOrGlyph);
        let (lx, ly) = (px - b.x * scale, py - b.y * scale + scroll);
        let map = TextMap::new(display);
        match buffer.hit(lx, ly) {
            Some(c) => Some(map.u16_of(display, c)),
            None => Some(if ly < 0.0 { 0 } else { map.len_u16() }),
        }
    }

    /// Height in px of the active body's text.
    pub fn body_text_height(&mut self, vm: &ViewModel, size_dip: (f32, f32), scale: f32) -> f32 {
        let Body::Text { display, .. } = &vm.body else { return 0.0 };
        let g = geometry(vm, size_dip, scale);
        let Some(b) = g.body else { return 0.0 };
        let buffer = self.shape(display, FONT_PX * scale, BODY_LINE * scale, Some(b.w * scale), Wrap::WordOrGlyph);
        buffer.layout_runs().map(|r| r.line_top + r.line_height).fold(0.0, f32::max)
    }

    /// Largest useful scroll offset in px for the active body.
    pub fn max_scroll(&mut self, vm: &ViewModel, size_dip: (f32, f32), scale: f32) -> f32 {
        let g = geometry(vm, size_dip, scale);
        let Some(b) = g.body else { return 0.0 };
        (self.body_text_height(vm, size_dip, scale) - b.h * scale).max(0.0)
    }

    /// Pixel centre of the character at UTF-16 offset `pos` in the active body (for tests and
    /// self-tests that need to aim real input at a specific character).
    pub fn char_centre(&mut self, vm: &ViewModel, size_dip: (f32, f32), scale: f32, pos: usize) -> Option<(f32, f32)> {
        let Body::Text { display, scroll, .. } = &vm.body else { return None };
        let g = geometry(vm, size_dip, scale);
        let b = g.body?;
        let map = TextMap::new(display);
        let c = map.cursor(display, pos);
        let buffer = self.shape(display, FONT_PX * scale, BODY_LINE * scale, Some(b.w * scale), Wrap::WordOrGlyph);
        for run in buffer.layout_runs() {
            if run.line_i != c.line {
                continue;
            }
            for glyph in run.glyphs {
                if glyph.start <= c.index && c.index < glyph.end {
                    let x = b.x * scale + glyph.x + glyph.w / 2.0;
                    let y = b.y * scale + run.line_top + run.line_height / 2.0 - scroll;
                    return Some((x, y));
                }
            }
        }
        None
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

fn scaled(r: Rect, s: f32) -> Rect {
    Rect { x: r.x * s, y: r.y * s, w: r.w * s, h: r.h * s }
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    (x1 > x0 && y1 > y0).then_some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// Which card button (if any) is under `(px, py)` (pixels). Cheap: geometry only, no text shaping,
/// so it can run on every pointer move.
pub fn button_at(vm: &ViewModel, size_dip: (f32, f32), scale: f32, px: f32, py: f32) -> Option<Hit> {
    let g = geometry(vm, size_dip, scale);
    let (x, y) = (px / scale, py / scale);
    [(g.expand, Hit::Expand), (g.restore, Hit::Restore), (g.delete, Hit::Delete)]
        .into_iter()
        .find(|(r, _)| r.is_some_and(|r| r.contains(x, y)))
        .map(|(_, h)| h)
}

/// A dark label beside `anchor` (above it, or below when `below`), kept inside the frame.
fn tooltip(r: &mut Renderer, cv: &mut Canvas, text: &str, anchor: Rect, size_dip: (f32, f32), below: bool) {
    let s = cv.s;
    let tw = r.measure(text, SMALL_PX * s, 16.0 * s) / s;
    let w = tw + 16.0;
    let h = 24.0;
    let x = (anchor.x + anchor.w - w).clamp(4.0, (size_dip.0 - w - 4.0).max(4.0));
    let y = if below { anchor.y + anchor.h + 6.0 } else { anchor.y - h - 6.0 };
    let y = y.clamp(4.0, (size_dip.1 - h - 4.0).max(4.0));
    let rect = Rect { x, y, w, h };
    cv.fill(rect, 5.0, (16, 16, 20), 0.96);
    cv.stroke(rect, 5.0, MENU_BORDER, 1.0, 1.0);
    r.draw_text(
        cv,
        text,
        ((x + 8.0) * s, (y + 4.0) * s),
        SMALL_PX * s,
        16.0 * s,
        None,
        Wrap::None,
        FOREGROUND,
        scaled(rect, s),
    );
}

/// A row title is one line: cut at the first line break and collapse whitespace.
pub fn first_line(s: &str) -> String {
    let line = s.split(['\r', '\n']).next().unwrap_or("");
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn draw_star(cv: &mut Canvas, cx: f32, cy: f32, r: f32, rgb: Rgb) {
    let s = cv.s;
    let mut pb = PathBuilder::new();
    for i in 0..10 {
        let ang = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let rad = if i % 2 == 0 { r } else { r * 0.42 };
        let (x, y) = ((cx + rad * ang.cos()) * s, (cy + rad * ang.sin()) * s);
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    if let Some(p) = pb.finish() {
        cv.pm.fill_path(&p, &paint(rgb, 1.0), FillRule::Winding, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests;
