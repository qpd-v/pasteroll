//! The expanded viewer: one clip, large, in a window of its own.
//!
//! ```text
//! +----------------------------------------------------------------+
//! | [icon] Text, 1,284 characters                 [find....] [ ][⤢]|  header
//! |                                                                 |
//! |        +----------------------------------------------+         |
//! |        | the clip, at its normal size, scrolling      |         |  content
//! |        |                                              |         |
//! |        +----------------------------------------------+         |
//! |                  ( < >  | copy paste pin del | ... x )          |  toolbar
//! +----------------------------------------------------------------+
//! ```
//!
//! Like the carousel it is a pure function of a [`ViewerView`]; the controller keeps the state and
//! decides. Text is shaped once per clip and width (cached), and only the visible lines are drawn,
//! so scrolling a long clip stays cheap. Large pictures are drawn from halved copies when shown
//! small, so fitting a big screenshot neither aliases nor costs much.

use super::{
    paint, rounded, scaled, intersect, tooltip, Canvas, Frame, Rect, Renderer, Rgb, RowKind, Rgba, ACCENT,
    ACTIVE_ROW, BODY_LINE, DANGER, FIELD, FONT_PX, FOREGROUND, MENU, MENU_BORDER, MUTED, SMALL_PX,
    SURFACE, SURFACE_BORDER,
};
use crate::icons::{self, Icon};
use crate::textmap::TextMap;
use cosmic_text::{Buffer, Wrap};
use std::sync::Arc;
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, IntSize, Mask, Pixmap, PixmapPaint, Transform,
};

const HEADER_H: f32 = 52.0;
const TOOLBAR_H: f32 = 44.0;
const TOOLBAR_BOTTOM: f32 = 14.0;
const TOOL: f32 = 34.0;
const TOOL_GAP: f32 = 2.0;
const SEP_W: f32 = 13.0;
const PAD: f32 = 16.0;
const TEXT_PAD: f32 = 16.0;
const INFO_W: f32 = 300.0;
const MENU_W: f32 = 290.0;
const MENU_ITEM_H: f32 = 32.0;
const FIND_W: f32 = 240.0;
const FIND_BTN: f32 = 30.0;
const CONFIRM_W: f32 = 430.0;
const CONFIRM_H: f32 = 176.0;

// ---------------------------------------------------------------- view model

/// A toolbar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Prev,
    Next,
    ZoomOut,
    FitActual,
    ZoomIn,
    Copy,
    Paste,
    Pin,
    Delete,
    Restore,
    Search,
    Wrap,
    More,
    Close,
}

impl Tool {
    /// Buttons of one group sit together; a thin divider separates groups.
    fn group(self) -> u8 {
        match self {
            Tool::Prev | Tool::Next => 0,
            Tool::ZoomOut | Tool::FitActual | Tool::ZoomIn => 1,
            Tool::Copy | Tool::Paste | Tool::Pin | Tool::Delete | Tool::Restore => 2,
            Tool::Search | Tool::Wrap => 3,
            Tool::More | Tool::Close => 4,
        }
    }

    fn icon(self, on: bool) -> Icon {
        match self {
            Tool::Prev => Icon::Prev,
            Tool::Next => Icon::Next,
            Tool::ZoomOut => Icon::ZoomOut,
            Tool::ZoomIn => Icon::ZoomIn,
            // shows what a click will do: "1:1" while fitted, "fit" while at another size
            Tool::FitActual => {
                if on {
                    Icon::Fit
                } else {
                    Icon::ActualSize
                }
            }
            Tool::Copy => Icon::Copy,
            Tool::Paste => Icon::Paste,
            Tool::Pin => {
                if on {
                    Icon::StarFilled
                } else {
                    Icon::Star
                }
            }
            Tool::Delete => Icon::Trash,
            Tool::Restore => Icon::Restore,
            Tool::Search => Icon::Search,
            Tool::Wrap => Icon::Wrap,
            Tool::More => Icon::More,
            Tool::Close => Icon::Close,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolView {
    pub tool: Tool,
    /// Toggle state (pinned, wrapping on, search open, zoomed away from fit).
    pub on: bool,
    pub enabled: bool,
    /// Shown above the button while the pointer is on it ("Copy (Ctrl+C)").
    pub tip: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItemView {
    pub label: String,
    pub shortcut: String,
    pub enabled: bool,
    /// Destructive: drawn in red.
    pub danger: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FindView {
    pub query: String,
    pub count: usize,
    pub current: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmView {
    pub title: String,
    pub message: String,
    pub ok: String,
    pub danger: bool,
}

#[derive(Debug, Clone)]
pub enum ViewerBody {
    Text {
        /// What is drawn (hidden characters shown as tokens).
        display: String,
        /// Changes whenever `display` does: the shaped text is cached by it.
        gen: u64,
        /// UTF-16 range into `display`.
        selection: Option<(usize, usize)>,
        /// Scroll offset in px (x matters only without wrapping).
        scroll: (f32, f32),
        wrap: bool,
        /// UTF-16 ranges of the find matches, and which one is current.
        matches: Vec<(usize, usize)>,
        current: Option<usize>,
    },
    Image {
        /// `None` while the picture is still being decoded. May be the small preview while the
        /// whole picture loads: it is drawn at the size of `size`.
        img: Option<Arc<Rgba>>,
        /// The picture's real size in pixels (what zoom and centre refer to).
        size: (u32, u32),
        /// DIP per image pixel; `None` fits the picture to the viewing area.
        zoom: Option<f32>,
        /// The image point (in image pixels) at the middle of the viewing area.
        center: (f32, f32),
    },
    Message(String),
}

#[derive(Debug, Clone)]
pub struct ViewerView {
    pub kind: RowKind,
    pub title: String,
    pub subtitle: String,
    pub fullscreen: bool,
    pub body: ViewerBody,
    pub tools: Vec<ToolView>,
    pub hover: Option<VHit>,
    /// The More menu, when open.
    pub menu: Option<Vec<MenuItemView>>,
    pub menu_hover: Option<usize>,
    /// The Info panel's lines (label, value), when open.
    pub info: Option<Vec<(String, String)>>,
    pub find: Option<FindView>,
    pub confirm: Option<ConfirmView>,
    /// A short message above the toolbar ("Copied").
    pub toast: String,
    /// "Fit" or "250%" for pictures.
    pub zoom_label: String,
}

impl Default for ViewerView {
    fn default() -> Self {
        ViewerView {
            kind: RowKind::Text,
            title: String::new(),
            subtitle: String::new(),
            fullscreen: false,
            body: ViewerBody::Message(String::new()),
            tools: Vec::new(),
            hover: None,
            menu: None,
            menu_hover: None,
            info: None,
            find: None,
            confirm: None,
            toast: String::new(),
            zoom_label: String::new(),
        }
    }
}

impl ViewerView {
    /// Overwrites every string that may hold clip text (after the frame is drawn).
    pub fn wipe(&mut self) {
        use zeroize::Zeroize;
        self.title.zeroize();
        self.subtitle.zeroize();
        match &mut self.body {
            ViewerBody::Text { display, .. } => display.zeroize(),
            ViewerBody::Message(m) => m.zeroize(),
            ViewerBody::Image { .. } => {}
        }
        if let Some(f) = &mut self.find {
            f.query.zeroize();
        }
        if let Some(info) = &mut self.info {
            for (_, v) in info.iter_mut() {
                v.zeroize();
            }
        }
    }
}

/// What is under a point of the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VHit {
    Tool(Tool),
    /// The back arrow at the top left: back to the carousel.
    Back,
    Fullscreen,
    MenuItem(usize),
    /// Inside the menu, between items.
    Menu,
    FindField,
    FindPrev,
    FindNext,
    FindClose,
    InfoClose,
    /// Inside the Info panel.
    Info,
    ConfirmOk,
    ConfirmCancel,
    /// Inside the confirmation, not on a button.
    Dialog,
    /// Over the toolbar, between buttons.
    Toolbar,
    /// The text, at this UTF-16 offset of the displayed text.
    Text { pos: usize },
    Image,
    Nothing,
}

// ---------------------------------------------------------------- geometry

#[derive(Debug, Clone)]
pub struct VGeom {
    pub header: Rect,
    pub back: Rect,
    pub fullscreen: Rect,
    /// Where content may go: below the header, above the toolbar, beside the Info panel.
    pub area: Rect,
    /// The text container (text bodies) or the picture's viewport (images): never under the toolbar.
    pub content: Rect,
    /// Inside the text container: where the text itself is drawn.
    pub text: Rect,
    pub toolbar: Rect,
    pub tools: Vec<Rect>,
    pub separators: Vec<f32>,
    pub menu: Option<Rect>,
    pub menu_items: Vec<Rect>,
    pub info: Option<Rect>,
    pub info_close: Option<Rect>,
    pub find_field: Option<Rect>,
    pub find_prev: Option<Rect>,
    pub find_next: Option<Rect>,
    pub find_close: Option<Rect>,
    pub confirm: Option<Rect>,
    pub confirm_ok: Option<Rect>,
    pub confirm_cancel: Option<Rect>,
}

pub fn viewer_geometry(v: &ViewerView, size_dip: (f32, f32)) -> VGeom {
    let (w, h) = size_dip;
    let header = Rect { x: 0.0, y: 0.0, w, h: HEADER_H };
    let back = Rect { x: 10.0, y: 10.0, w: 32.0, h: 32.0 };
    let fullscreen = Rect { x: w - 12.0 - 32.0, y: 10.0, w: 32.0, h: 32.0 };

    // toolbar: groups of buttons with dividers, centred at the bottom
    let mut tw = 12.0;
    let mut prev_group = None;
    for t in &v.tools {
        if prev_group.is_some_and(|g| g != t.tool.group()) {
            tw += SEP_W;
        }
        prev_group = Some(t.tool.group());
        tw += TOOL + TOOL_GAP;
    }
    tw += 12.0 - TOOL_GAP;
    let toolbar = Rect { x: (w - tw) / 2.0, y: h - TOOLBAR_BOTTOM - TOOLBAR_H, w: tw, h: TOOLBAR_H };
    let mut tools = Vec::new();
    let mut separators = Vec::new();
    let mut x = toolbar.x + 12.0;
    let mut prev_group = None;
    for t in &v.tools {
        if prev_group.is_some_and(|g| g != t.tool.group()) {
            separators.push(x + (SEP_W - TOOL_GAP) / 2.0);
            x += SEP_W;
        }
        prev_group = Some(t.tool.group());
        tools.push(Rect { x, y: toolbar.y + (TOOLBAR_H - TOOL) / 2.0, w: TOOL, h: TOOL });
        x += TOOL + TOOL_GAP;
    }

    let info = v.info.as_ref().map(|_| Rect {
        x: w - PAD - INFO_W,
        y: HEADER_H,
        w: INFO_W,
        h: (toolbar.y - 12.0 - HEADER_H).max(60.0),
    });
    let info_close = info.map(|r| Rect { x: r.x + r.w - 32.0, y: r.y + 6.0, w: 26.0, h: 26.0 });
    let right = info.map_or(w - PAD, |r| r.x - 12.0);
    let area = Rect {
        x: PAD,
        y: HEADER_H,
        w: (right - PAD).max(40.0),
        h: (toolbar.y - 12.0 - HEADER_H).max(40.0),
    };
    let content = match v.body {
        ViewerBody::Image { .. } => area,
        _ => {
            // a container of about 75% x 80% of the viewer, never outside the free area
            let cw = (w * 0.75).min(area.w);
            let ch = (h * 0.80).min(area.h);
            Rect { x: area.x + (area.w - cw) / 2.0, y: area.y + (area.h - ch) / 2.0, w: cw, h: ch }
        }
    };
    let text = Rect {
        x: content.x + TEXT_PAD,
        y: content.y + TEXT_PAD - 4.0,
        w: (content.w - 2.0 * TEXT_PAD - 6.0).max(10.0),
        h: (content.h - 2.0 * TEXT_PAD + 8.0).max(10.0),
    };

    let (menu, menu_items) = match (&v.menu, v.tools.iter().position(|t| t.tool == Tool::More)) {
        (Some(items), Some(i)) => {
            let anchor = tools[i];
            let mh = items.len() as f32 * MENU_ITEM_H + 8.0;
            let mx = (anchor.x + anchor.w - MENU_W).clamp(8.0, (w - MENU_W - 8.0).max(8.0));
            let my = (toolbar.y - 8.0 - mh).max(8.0);
            let rect = Rect { x: mx, y: my, w: MENU_W, h: mh };
            let rows = (0..items.len())
                .map(|k| Rect { x: mx + 4.0, y: my + 4.0 + k as f32 * MENU_ITEM_H, w: MENU_W - 8.0, h: MENU_ITEM_H })
                .collect();
            (Some(rect), rows)
        }
        _ => (None, Vec::new()),
    };

    let (find_field, find_prev, find_next, find_close) = match &v.find {
        Some(_) => {
            let x0 = (fullscreen.x - 10.0 - FIND_W - 3.0 * FIND_BTN).max(w * 0.35);
            let field = Rect { x: x0, y: 10.0, w: FIND_W, h: 32.0 };
            let b = |k: f32| Rect { x: x0 + FIND_W + 2.0 + k * FIND_BTN, y: 10.0, w: FIND_BTN - 2.0, h: 32.0 };
            (Some(field), Some(b(0.0)), Some(b(1.0)), Some(b(2.0)))
        }
        None => (None, None, None, None),
    };

    let (confirm, confirm_ok, confirm_cancel) = match &v.confirm {
        Some(_) => {
            let cw = CONFIRM_W.min(w - 32.0);
            let r = Rect { x: (w - cw) / 2.0, y: (h - CONFIRM_H) / 2.0, w: cw, h: CONFIRM_H };
            let ok = Rect { x: r.x + r.w - 16.0 - 130.0, y: r.y + r.h - 16.0 - 34.0, w: 130.0, h: 34.0 };
            let cancel = Rect { x: ok.x - 10.0 - 100.0, w: 100.0, ..ok };
            (Some(r), Some(ok), Some(cancel))
        }
        None => (None, None, None),
    };

    VGeom {
        header,
        back,
        fullscreen,
        area,
        content,
        text,
        toolbar,
        tools,
        separators,
        menu,
        menu_items,
        info,
        info_close,
        find_field,
        find_prev,
        find_next,
        find_close,
        confirm,
        confirm_ok,
        confirm_cancel,
    }
}

/// DIP per image pixel that fits a `w x h` picture inside the viewing area.
pub fn fit_zoom(g: &VGeom, w: u32, h: u32) -> f32 {
    if w == 0 || h == 0 {
        return 1.0;
    }
    (g.content.w / w as f32).min(g.content.h / h as f32)
}

// ---------------------------------------------------------------- caches

/// The shaped text of the clip in the viewer.
pub(crate) struct VText {
    key: (u64, u32, bool, u32),
    buffer: Buffer,
    map: TextMap,
    width: f32,
    height: f32,
}

/// The picture in the viewer, premultiplied, at full size and at halved sizes.
#[derive(Default)]
pub(crate) struct ImgCache {
    img: Option<Arc<Rgba>>,
    levels: Vec<(u32, Pixmap)>,
}

fn to_pixmap(img: &Rgba) -> Option<Pixmap> {
    let mut pm = Pixmap::new(img.width, img.height)?;
    let d = pm.data_mut();
    for (i, p) in img.pixels.chunks_exact(4).enumerate() {
        let a = u32::from(p[3]);
        d[i * 4] = ((u32::from(p[0]) * a + 127) / 255) as u8;
        d[i * 4 + 1] = ((u32::from(p[1]) * a + 127) / 255) as u8;
        d[i * 4 + 2] = ((u32::from(p[2]) * a + 127) / 255) as u8;
        d[i * 4 + 3] = p[3];
    }
    Some(pm)
}

impl ImgCache {
    /// The picture reduced by `2^level`, made on first use.
    pub(crate) fn level(&mut self, img: &Arc<Rgba>, level: u32) -> Option<&Pixmap> {
        if !self.img.as_ref().is_some_and(|i| Arc::ptr_eq(i, img)) {
            self.img = Some(img.clone());
            self.levels.clear();
        }
        if !self.levels.iter().any(|(l, _)| *l == level) {
            let pm = if level == 0 {
                to_pixmap(img)?
            } else {
                let f = 1u32 << level;
                let small = pasteroll_core::image::downscale(img, (img.width / f).max(1), (img.height / f).max(1));
                to_pixmap(&small)?
            };
            self.levels.push((level, pm));
        }
        self.levels.iter().find(|(l, _)| *l == level).map(|(_, p)| p)
    }
}

// ---------------------------------------------------------------- morph (the expand / collapse animation)

/// Two frames, blended and stretched into any size: the card growing into the viewer (or the
/// reverse). Frames are opaque BGRA, which tiny-skia can blend as if it were RGBA.
pub struct Morph {
    from: Pixmap,
    to: Pixmap,
}

fn frame_pixmap(f: &Frame) -> Option<Pixmap> {
    Pixmap::from_vec(f.bgra.clone(), IntSize::from_wh(f.width, f.height)?)
}

impl Morph {
    pub fn new(from: &Frame, to: &Frame) -> Option<Morph> {
        Some(Morph { from: frame_pixmap(from)?, to: frame_pixmap(to)? })
    }

    /// The in-between picture at `t` (0 = `from`, 1 = `to`), `w x h` pixels. Each frame covers the
    /// size from the top-left corner, so the card's title stays where it is while the card grows.
    pub fn frame(&self, t: f32, w: u32, h: u32) -> Frame {
        let (w, h) = (w.max(1), h.max(1));
        let mut pm = Pixmap::new(w, h).expect("pixmap");
        // BGRA: the colour components are given in that order
        pm.fill(Color::from_rgba8(ACTIVE_ROW.2, ACTIVE_ROW.1, ACTIVE_ROW.0, 255));
        let t = t.clamp(0.0, 1.0);
        for (src, opacity) in [(&self.from, 1.0), (&self.to, t)] {
            if opacity <= 0.0 {
                continue;
            }
            let k = (w as f32 / src.width() as f32).max(h as f32 / src.height() as f32);
            let pp = PixmapPaint { opacity, blend_mode: BlendMode::SourceOver, quality: FilterQuality::Bilinear };
            pm.draw_pixmap(0, 0, src.as_ref(), &pp, Transform::from_scale(k, k), None);
        }
        Frame { width: w, height: h, bgra: pm.take() }
    }
}

// ---------------------------------------------------------------- drawing

const TOOLBAR_BG: Rgb = (22, 22, 26);
const BUTTON_HOT: Rgb = (66, 70, 82);
const ICON: Rgb = (214, 214, 222);
const ICON_OFF: Rgb = (96, 96, 106);
const MATCH: Rgb = (230, 190, 60);
const MATCH_CURRENT: Rgb = (255, 150, 40);

impl Renderer {
    /// Forgets the viewer's shaped text and picture (the viewer closed).
    pub fn forget_viewer(&mut self) {
        self.vtext = None;
        self.vimg = ImgCache::default();
    }

    fn ensure_vtext(&mut self, display: &str, gen: u64, width_px: f32, wrap: bool, scale: f32) {
        let key = (gen, width_px.round() as u32, wrap, scale.to_bits());
        if self.vtext.as_ref().is_some_and(|t| t.key == key) {
            return;
        }
        let w = if wrap { Some(width_px) } else { None };
        let mode = if wrap { Wrap::WordOrGlyph } else { Wrap::None };
        let buffer = self.shape(display, FONT_PX * scale, BODY_LINE * scale, w, mode);
        let (mut width, mut height) = (0.0f32, 0.0f32);
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height = height.max(run.line_top + run.line_height);
        }
        self.vtext = Some(VText { key, buffer, map: TextMap::new(display), width, height });
    }

    /// Size in px of the whole text, and of the area it is shown in (for scrolling).
    pub fn viewer_text_extent(&mut self, v: &ViewerView, size_dip: (f32, f32), scale: f32) -> Option<((f32, f32), (f32, f32))> {
        let ViewerBody::Text { display, gen, wrap, .. } = &v.body else { return None };
        let g = viewer_geometry(v, size_dip);
        self.ensure_vtext(display, *gen, g.text.w * scale, *wrap, scale);
        let t = self.vtext.as_ref()?;
        Some(((t.width, t.height), (g.text.w * scale, g.text.h * scale)))
    }

    /// UTF-16 offset of the displayed text nearest to a pixel of the viewer (clamped to the first
    /// or last line above or below the text, which is what a drag needs).
    pub fn viewer_text_position(&mut self, v: &ViewerView, size_dip: (f32, f32), scale: f32, px: f32, py: f32) -> Option<usize> {
        let ViewerBody::Text { display, gen, wrap, scroll, .. } = &v.body else { return None };
        let g = viewer_geometry(v, size_dip);
        self.ensure_vtext(display, *gen, g.text.w * scale, *wrap, scale);
        let t = self.vtext.as_ref()?;
        let (lx, ly) = (px - g.text.x * scale + scroll.0, py - g.text.y * scale + scroll.1);
        Some(match t.buffer.hit(lx.max(0.0), ly) {
            Some(c) => t.map.u16_of(display, c),
            None => {
                if ly < 0.0 {
                    0
                } else {
                    t.map.len_u16()
                }
            }
        })
    }

    /// Where the character at UTF-16 offset `pos` is, in px within the whole text: `(x, top, height)`.
    pub fn viewer_char_box(&mut self, v: &ViewerView, size_dip: (f32, f32), scale: f32, pos: usize) -> Option<(f32, f32, f32)> {
        let ViewerBody::Text { display, gen, wrap, .. } = &v.body else { return None };
        let g = viewer_geometry(v, size_dip);
        self.ensure_vtext(display, *gen, g.text.w * scale, *wrap, scale);
        let t = self.vtext.as_ref()?;
        let c = t.map.cursor(display, pos);
        for run in t.buffer.layout_runs() {
            if run.line_i != c.line {
                continue;
            }
            let at_end = run.glyphs.last().is_some_and(|gl| c.index >= gl.end);
            for glyph in run.glyphs {
                if glyph.start <= c.index && c.index < glyph.end {
                    return Some((glyph.x, run.line_top, run.line_height));
                }
            }
            if !at_end {
                continue;
            }
            return Some((run.line_w, run.line_top, run.line_height));
        }
        None
    }

    pub fn render_viewer(&mut self, v: &ViewerView, size_dip: (f32, f32), scale: f32) -> Frame {
        let s = scale;
        let (wpx, hpx) = ((size_dip.0 * s).round().max(1.0) as u32, (size_dip.1 * s).round().max(1.0) as u32);
        let mut cv = Canvas { pm: Pixmap::new(wpx, hpx).expect("pixmap"), s };
        let g = viewer_geometry(v, size_dip);
        cv.pm.fill(super::color(ACTIVE_ROW, 1.0));
        if !v.fullscreen {
            // the card's own edge, as in the carousel
            let edge = Rect { x: 0.6, y: 0.6, w: size_dip.0 - 1.2, h: size_dip.1 - 1.2 };
            cv.stroke(edge, 8.0, ACCENT, 0.9, 1.2);
        }
        self.draw_header(&mut cv, v, &g, size_dip);
        match &v.body {
            ViewerBody::Text { .. } => self.draw_viewer_text(&mut cv, v, &g),
            ViewerBody::Image { .. } => self.draw_viewer_image(&mut cv, v, &g),
            ViewerBody::Message(m) => {
                let tw = self.measure(m, FONT_PX * s, 18.0 * s);
                let c = g.area;
                self.draw_text(
                    &mut cv,
                    m,
                    ((c.x + c.w / 2.0) * s - tw / 2.0, (c.y + c.h / 2.0 - 9.0) * s),
                    FONT_PX * s,
                    18.0 * s,
                    None,
                    Wrap::None,
                    MUTED,
                    scaled(c, s),
                );
            }
        }
        if let (Some(r), Some(lines)) = (g.info, &v.info) {
            self.draw_info(&mut cv, v, r, g.info_close, lines);
        }
        self.draw_toolbar(&mut cv, v, &g);
        if let (Some(r), Some(items)) = (g.menu, &v.menu) {
            cv.fill(r, 8.0, MENU, 1.0);
            cv.stroke(r, 8.0, MENU_BORDER, 1.0, 1.0);
            for (i, it) in items.iter().enumerate() {
                let ir = g.menu_items[i];
                if v.menu_hover == Some(i) && it.enabled {
                    cv.fill(ir, 5.0, ACCENT, 0.28);
                }
                let col = match (it.enabled, it.danger) {
                    (false, _) => ICON_OFF,
                    (true, true) => DANGER,
                    (true, false) => FOREGROUND,
                };
                self.draw_text(
                    &mut cv,
                    &it.label,
                    ((ir.x + 12.0) * s, (ir.y + 7.0) * s),
                    FONT_PX * s,
                    18.0 * s,
                    None,
                    Wrap::None,
                    col,
                    scaled(ir, s),
                );
                if !it.shortcut.is_empty() {
                    let sw = self.measure(&it.shortcut, SMALL_PX * s, 16.0 * s);
                    self.draw_text(
                        &mut cv,
                        &it.shortcut,
                        ((ir.x + ir.w - 12.0) * s - sw, (ir.y + 8.0) * s),
                        SMALL_PX * s,
                        16.0 * s,
                        None,
                        Wrap::None,
                        MUTED,
                        scaled(ir, s),
                    );
                }
            }
        }
        // the explanation of the button under the pointer
        match v.hover {
            Some(VHit::Tool(t)) if v.menu.is_none() && v.confirm.is_none() => {
                if let Some(i) = v.tools.iter().position(|x| x.tool == t) {
                    let tip = v.tools[i].tip.clone();
                    centred_tip(self, &mut cv, &tip, g.tools[i], size_dip);
                }
            }
            Some(VHit::Back) if v.confirm.is_none() => {
                let tip = "Back to the carousel (Esc)";
                let tw = self.measure(tip, SMALL_PX * s, 16.0 * s) / s + 16.0;
                // left-aligned under the button
                let anchor = Rect { x: g.back.x + tw - 1.0, w: 1.0, ..g.back };
                tooltip(self, &mut cv, tip, anchor, size_dip, true);
            }
            Some(VHit::Fullscreen) if v.confirm.is_none() => {
                let tip = if v.fullscreen { "Exit fullscreen (F11)" } else { "Fullscreen (F11)" };
                tooltip(self, &mut cv, tip, g.fullscreen, size_dip, true);
            }
            _ => {}
        }
        if !v.toast.is_empty() {
            let tw = self.measure(&v.toast, FONT_PX * s, 18.0 * s) / s;
            let r = Rect { x: (size_dip.0 - tw - 32.0) / 2.0, y: g.toolbar.y - 12.0 - 32.0, w: tw + 32.0, h: 32.0 };
            cv.fill(r, 16.0, TOOLBAR_BG, 0.96);
            cv.stroke(r, 16.0, MENU_BORDER, 1.0, 1.0);
            self.draw_text(
                &mut cv,
                &v.toast,
                ((r.x + 16.0) * s, (r.y + 7.0) * s),
                FONT_PX * s,
                18.0 * s,
                None,
                Wrap::None,
                FOREGROUND,
                scaled(r, s),
            );
        }
        if let (Some(r), Some(c)) = (g.confirm, &v.confirm) {
            self.draw_confirm(&mut cv, v, &g, r, c, size_dip);
        }
        let mut bgra = cv.pm.take();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Frame { width: wpx, height: hpx, bgra }
    }

    fn icon_button(&mut self, cv: &mut Canvas, r: Rect, icon: Icon, hot: bool, on: bool, enabled: bool) {
        if on {
            cv.fill(r, 7.0, ACCENT, 0.24);
        }
        if hot && enabled {
            cv.fill(r, 7.0, BUTTON_HOT, 1.0);
        }
        let col = match (enabled, hot, on) {
            (false, _, _) => ICON_OFF,
            (true, true, _) => FOREGROUND,
            (true, false, true) => ACCENT,
            _ => ICON,
        };
        icons::draw(cv, icon, r, 16.0, col);
    }

    fn draw_header(&mut self, cv: &mut Canvas, v: &ViewerView, g: &VGeom, size_dip: (f32, f32)) {
        let s = cv.s;
        self.icon_button(cv, g.back, Icon::Back, v.hover == Some(VHit::Back), false, true);
        let title_x = g.back.x + g.back.w + 10.0;
        self.draw_icon(cv, v.kind, title_x, 13.0, ACCENT);
        let mut right = g.fullscreen.x - 12.0;
        if let Some(f) = g.find_field {
            right = f.x - 12.0;
        }
        if !v.zoom_label.is_empty() && v.find.is_none() {
            let zw = self.measure(&v.zoom_label, SMALL_PX * s, 16.0 * s) / s;
            let x = g.fullscreen.x - 12.0 - zw;
            self.draw_text(
                cv,
                &v.zoom_label,
                (x * s, 18.0 * s),
                SMALL_PX * s,
                16.0 * s,
                None,
                Wrap::None,
                MUTED,
                scaled(Rect { x, y: 10.0, w: zw + 2.0, h: 30.0 }, s),
            );
            right = x - 12.0;
        }
        let clip = Rect { x: title_x + 22.0, y: 4.0, w: (right - title_x - 22.0).max(0.0), h: HEADER_H - 8.0 };
        self.draw_text(cv, &v.title, ((title_x + 24.0) * s, 9.0 * s), FONT_PX * s, 18.0 * s, None, Wrap::None, FOREGROUND, scaled(clip, s));
        if !v.subtitle.is_empty() {
            self.draw_text(
                cv,
                &v.subtitle,
                ((title_x + 24.0) * s, 29.0 * s),
                SMALL_PX * s,
                16.0 * s,
                None,
                Wrap::None,
                MUTED,
                scaled(clip, s),
            );
        }
        let fs_icon = if v.fullscreen { Icon::ExitFullscreen } else { Icon::Fullscreen };
        self.icon_button(cv, g.fullscreen, fs_icon, v.hover == Some(VHit::Fullscreen), false, true);
        if let (Some(field), Some(find)) = (g.find_field, &v.find) {
            cv.fill(field, 6.0, FIELD, 1.0);
            cv.stroke(field, 6.0, ACCENT, 0.8, 1.0);
            icons::draw(cv, Icon::Search, Rect { x: field.x + 4.0, y: field.y, w: 24.0, h: field.h }, 13.0, MUTED);
            let count = match (find.count, find.current) {
                (0, _) if !find.query.is_empty() => "No matches".to_owned(),
                (0, _) => String::new(),
                (n, Some(c)) => format!("{} of {}", c + 1, n),
                (n, None) => format!("{n}"),
            };
            let cw = self.measure(&count, SMALL_PX * s, 16.0 * s) / s;
            let text_clip = Rect { x: field.x + 28.0, y: field.y, w: (field.w - 36.0 - cw).max(10.0), h: field.h };
            let tp = ((field.x + 30.0) * s, (field.y + 7.0) * s);
            if find.query.is_empty() {
                self.draw_text(cv, "Find in clip", tp, FONT_PX * s, 18.0 * s, None, Wrap::None, MUTED, scaled(text_clip, s));
            } else {
                let w = self.draw_text(cv, &find.query, tp, FONT_PX * s, 18.0 * s, None, Wrap::None, FOREGROUND, scaled(text_clip, s));
                let cx = (tp.0 + w + 1.0 * s).min((text_clip.x + text_clip.w) * s - 2.0 * s);
                if let Some(r) = tiny_skia::Rect::from_xywh(cx, tp.1 + 1.0 * s, 1.5 * s, 16.0 * s) {
                    cv.pm.fill_rect(r, &paint(ACCENT, 1.0), Transform::identity(), None);
                }
            }
            self.draw_text(
                cv,
                &count,
                ((field.x + field.w - 8.0 - cw) * s, (field.y + 9.0) * s),
                SMALL_PX * s,
                16.0 * s,
                None,
                Wrap::None,
                MUTED,
                scaled(field, s),
            );
            let has = find.count > 0;
            for (r, icon, hit, enabled) in [
                (g.find_prev, Icon::ChevronUp, VHit::FindPrev, has),
                (g.find_next, Icon::ChevronDown, VHit::FindNext, has),
                (g.find_close, Icon::Close, VHit::FindClose, true),
            ] {
                if let Some(r) = r {
                    self.icon_button(cv, r, icon, v.hover == Some(hit), false, enabled);
                }
            }
        }
        let _ = size_dip;
    }

    fn draw_viewer_text(&mut self, cv: &mut Canvas, v: &ViewerView, g: &VGeom) {
        let ViewerBody::Text { display, gen, selection, scroll, wrap, matches, current } = &v.body else { return };
        let s = cv.s;
        cv.fill(g.content, 8.0, SURFACE, 1.0);
        cv.stroke(g.content, 8.0, SURFACE_BORDER, 1.0, 1.0);
        let tr = g.text;
        self.ensure_vtext(display, *gen, tr.w * s, *wrap, s);
        let Renderer { fs, cache, vtext, .. } = self;
        let Some(t) = vtext.as_ref() else { return };
        let clip = scaled(tr, s);
        let (ox, oy) = (tr.x * s - scroll.0, tr.y * s - scroll.1);
        let (y0, y1) = (scroll.1, scroll.1 + tr.h * s);
        let visible: Vec<_> =
            t.buffer.layout_runs().filter(|r| r.line_top + r.line_height >= y0 && r.line_top <= y1).collect();
        if let (Some(first), Some(last)) = (visible.first(), visible.last()) {
            let (l0, l1) = (first.line_i, last.line_i);
            // find matches, then the selection on top
            let mut ranges: Vec<((usize, usize), Rgb, f32)> = matches
                .iter()
                .enumerate()
                .map(|(i, r)| (*r, if Some(i) == *current { MATCH_CURRENT } else { MATCH }, if Some(i) == *current { 0.62 } else { 0.30 }))
                .collect();
            if let Some((a, z)) = selection {
                ranges.push(((*a.min(z), *a.max(z)), ACCENT, 0.55));
            }
            for ((a, z), rgb, alpha) in ranges {
                if z <= a {
                    continue;
                }
                let c0 = t.map.cursor(display, a);
                let c1 = t.map.cursor(display, z);
                if c1.line < l0 || c0.line > l1 {
                    continue;
                }
                for run in visible.iter().filter(|run| c0.line <= run.line_i && run.line_i <= c1.line) {
                    let (top, h) = (run.line_top, run.line_height);
                    for (x, w) in run.highlight(c0, c1) {
                        let rect = Rect { x: ox + x, y: oy + top, w, h };
                        if let Some(r) = intersect(rect, clip) {
                            if let Some(path) = rounded(r, 2.0 * s) {
                                cv.pm.fill_path(&path, &paint(rgb, alpha), FillRule::Winding, Transform::identity(), None);
                            }
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
        for run in &visible {
            for glyph in run.glyphs {
                let gx = ox + glyph.x;
                if gx + glyph.w < clip.x || gx > clip.x + clip.w {
                    continue; // a long unwrapped line: only what is in view
                }
                let pg = glyph.physical((0.0, run.line_y), 1.0);
                let col = glyph.color_opt.unwrap_or(base);
                cache.with_pixels(fs, pg.cache_key, col, |x, y, c| {
                    cv.blend_px(oxi + pg.x + x, oyi + pg.y + y, 1, 1, c, clip_px);
                });
            }
        }
        // scroll indicators
        let (vw, vh) = (tr.w * s, tr.h * s);
        if t.height > vh + 1.0 {
            let track = Rect { x: g.content.x + g.content.w - 7.0, y: g.content.y + 6.0, w: 3.0, h: g.content.h - 12.0 };
            let th = (track.h * vh / t.height).max(24.0).min(track.h);
            let ty = track.y + (track.h - th) * (scroll.1 / (t.height - vh)).clamp(0.0, 1.0);
            cv.fill(Rect { y: ty, h: th, ..track }, 1.5, MUTED, 0.6);
        }
        if !wrap && t.width > vw + 1.0 {
            let track = Rect { x: g.content.x + 6.0, y: g.content.y + g.content.h - 7.0, w: g.content.w - 12.0, h: 3.0 };
            let tw = (track.w * vw / t.width).max(24.0).min(track.w);
            let tx = track.x + (track.w - tw) * (scroll.0 / (t.width - vw)).clamp(0.0, 1.0);
            cv.fill(Rect { x: tx, w: tw, ..track }, 1.5, MUTED, 0.6);
        }
    }

    fn draw_viewer_image(&mut self, cv: &mut Canvas, v: &ViewerView, g: &VGeom) {
        let ViewerBody::Image { img, size, zoom, center } = &v.body else { return };
        let s = cv.s;
        let Some(img) = img else {
            let c = g.content;
            let m = "Loading...";
            let tw = self.measure(m, FONT_PX * s, 18.0 * s);
            self.draw_text(
                cv,
                m,
                ((c.x + c.w / 2.0) * s - tw / 2.0, (c.y + c.h / 2.0 - 9.0) * s),
                FONT_PX * s,
                18.0 * s,
                None,
                Wrap::None,
                MUTED,
                scaled(c, s),
            );
            return;
        };
        if img.width == 0 || img.height == 0 {
            return;
        }
        // the real picture's size; a preview stands in for it at that size
        let (lw, lh) = if size.0 > 0 && size.1 > 0 { *size } else { (img.width, img.height) };
        let stand_in = lw as f32 / img.width as f32;
        let center = (center.0 / stand_in, center.1 / stand_in);
        let k = zoom.unwrap_or_else(|| fit_zoom(g, lw, lh)) * s * stand_in; // px per pixmap px
        // shown small: draw from a halved copy (bilinear filtering only averages 2 x 2)
        let mut level = 0u32;
        while level < 6 && k * (1u32 << (level + 1)) as f32 <= 1.0 && (img.width >> (level + 1)) > 0 {
            level += 1;
        }
        let f = (1u32 << level) as f32;
        let kk = k * f;
        let (cx, cy) = ((g.content.x + g.content.w / 2.0) * s, (g.content.y + g.content.h / 2.0) * s);
        let tx = cx - center.0 / f * kk;
        let ty = cy - center.1 / f * kk;
        let (w, h) = (cv.pm.width(), cv.pm.height());
        let mut mask = Mask::new(w, h);
        if let (Some(mask), Some(path)) = (mask.as_mut(), rounded(scaled(g.content, s), 6.0 * s)) {
            mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
        }
        let quality = if kk >= 2.0 { FilterQuality::Nearest } else { FilterQuality::Bilinear };
        let Renderer { vimg, .. } = self;
        if let Some(pm) = vimg.level(img, level) {
            let pp = PixmapPaint { opacity: 1.0, blend_mode: BlendMode::SourceOver, quality };
            cv.pm.draw_pixmap(0, 0, pm.as_ref(), &pp, Transform::from_row(kk, 0.0, 0.0, kk, tx, ty), mask.as_ref());
        }
    }

    fn draw_info(&mut self, cv: &mut Canvas, v: &ViewerView, r: Rect, close: Option<Rect>, lines: &[(String, String)]) {
        let s = cv.s;
        cv.fill(r, 8.0, SURFACE, 1.0);
        cv.stroke(r, 8.0, SURFACE_BORDER, 1.0, 1.0);
        self.draw_text(cv, "Info", ((r.x + 14.0) * s, (r.y + 10.0) * s), FONT_PX * s, 18.0 * s, None, Wrap::None, FOREGROUND, scaled(r, s));
        if let Some(c) = close {
            self.icon_button(cv, c, Icon::Close, v.hover == Some(VHit::InfoClose), false, true);
        }
        let mut y = r.y + 40.0;
        let inner_w = r.w - 28.0;
        for (label, value) in lines {
            if y > r.y + r.h - 20.0 {
                break;
            }
            self.draw_text(cv, label, ((r.x + 14.0) * s, y * s), SMALL_PX * s, 16.0 * s, None, Wrap::None, MUTED, scaled(r, s));
            y += 17.0;
            let buf = self.shape(value, 13.0 * s, 18.0 * s, Some(inner_w * s), Wrap::WordOrGlyph);
            let lines_n = buf.layout_runs().count().max(1).min(4);
            let clip = Rect { x: r.x + 10.0, y, w: r.w - 20.0, h: (lines_n as f32 * 18.0).min(r.y + r.h - y - 6.0).max(0.0) };
            self.draw_text(cv, value, ((r.x + 14.0) * s, y * s), 13.0 * s, 18.0 * s, Some(inner_w * s), Wrap::WordOrGlyph, FOREGROUND, scaled(clip, s));
            y += lines_n as f32 * 18.0 + 10.0;
        }
    }

    fn draw_toolbar(&mut self, cv: &mut Canvas, v: &ViewerView, g: &VGeom) {
        let s = cv.s;
        cv.fill(g.toolbar, TOOLBAR_H / 2.0, TOOLBAR_BG, 0.97);
        cv.stroke(g.toolbar, TOOLBAR_H / 2.0, MENU_BORDER, 1.0, 1.0);
        for x in &g.separators {
            cv.line(&[(*x, g.toolbar.y + 12.0), (*x, g.toolbar.y + TOOLBAR_H - 12.0)], (60, 60, 70), 1.0);
        }
        for (i, t) in v.tools.iter().enumerate() {
            let hot = v.hover == Some(VHit::Tool(t.tool)) && v.confirm.is_none();
            let open = t.tool == Tool::More && v.menu.is_some();
            let on = match t.tool {
                Tool::Pin | Tool::Wrap | Tool::Search => t.on,
                _ => open,
            };
            let r = g.tools[i];
            self.icon_button(cv, r, t.tool.icon(t.on), hot, on, t.enabled);
            if t.tool == Tool::Delete && hot && t.enabled {
                icons::draw(cv, Icon::Trash, r, 16.0, DANGER);
            }
        }
        let _ = s;
    }

    fn draw_confirm(&mut self, cv: &mut Canvas, v: &ViewerView, g: &VGeom, r: Rect, c: &ConfirmView, size_dip: (f32, f32)) {
        let s = cv.s;
        cv.fill(Rect { x: 0.0, y: 0.0, w: size_dip.0, h: size_dip.1 }, 0.0, (0, 0, 0), 0.5);
        cv.fill(r, 10.0, MENU, 1.0);
        cv.stroke(r, 10.0, MENU_BORDER, 1.0, 1.0);
        self.draw_text(cv, &c.title, ((r.x + 20.0) * s, (r.y + 18.0) * s), 15.0 * s, 20.0 * s, None, Wrap::None, FOREGROUND, scaled(r, s));
        let msg_clip = Rect { x: r.x + 16.0, y: r.y + 46.0, w: r.w - 32.0, h: 64.0 };
        self.draw_text(
            cv,
            &c.message,
            ((r.x + 20.0) * s, (r.y + 48.0) * s),
            13.0 * s,
            18.0 * s,
            Some((r.w - 40.0) * s),
            Wrap::WordOrGlyph,
            MUTED,
            scaled(msg_clip, s),
        );
        for (rect, label, hit, primary) in [
            (g.confirm_cancel, "Cancel", VHit::ConfirmCancel, false),
            (g.confirm_ok, c.ok.as_str(), VHit::ConfirmOk, true),
        ] {
            let Some(b) = rect else { continue };
            let hot = v.hover == Some(hit);
            let (fill, alpha) = match (primary, c.danger) {
                (true, true) => (DANGER, if hot { 1.0 } else { 0.85 }),
                (true, false) => (ACCENT, if hot { 1.0 } else { 0.85 }),
                (false, _) => (if hot { BUTTON_HOT } else { FIELD }, 1.0),
            };
            cv.fill(b, 7.0, fill, alpha);
            let lw = self.measure(label, FONT_PX * s, 18.0 * s);
            let col = if primary { (255, 255, 255) } else { FOREGROUND };
            self.draw_text(
                cv,
                label,
                ((b.x + b.w / 2.0) * s - lw / 2.0, (b.y + 8.0) * s),
                FONT_PX * s,
                18.0 * s,
                None,
                Wrap::None,
                col,
                scaled(b, s),
            );
        }
    }

    // ------------------------------------------------------------------ hit testing

    /// What is under `(px, py)` (pixels of the viewer window).
    pub fn viewer_hit(&mut self, v: &ViewerView, size_dip: (f32, f32), scale: f32, px: f32, py: f32) -> VHit {
        let g = viewer_geometry(v, size_dip);
        let (x, y) = (px / scale, py / scale);
        if let Some(r) = g.confirm {
            // modal: nothing else answers while it is up
            if g.confirm_ok.is_some_and(|b| b.contains(x, y)) {
                return VHit::ConfirmOk;
            }
            if g.confirm_cancel.is_some_and(|b| b.contains(x, y)) {
                return VHit::ConfirmCancel;
            }
            return if r.contains(x, y) { VHit::Dialog } else { VHit::Nothing };
        }
        if let Some(m) = g.menu {
            if m.contains(x, y) {
                return g.menu_items.iter().position(|r| r.contains(x, y)).map_or(VHit::Menu, VHit::MenuItem);
            }
        }
        if g.toolbar.contains(x, y) {
            return g
                .tools
                .iter()
                .position(|r| r.contains(x, y))
                .map_or(VHit::Toolbar, |i| VHit::Tool(v.tools[i].tool));
        }
        if g.fullscreen.contains(x, y) {
            return VHit::Fullscreen;
        }
        if g.back.contains(x, y) {
            return VHit::Back;
        }
        for (r, hit) in [
            (g.find_prev, VHit::FindPrev),
            (g.find_next, VHit::FindNext),
            (g.find_close, VHit::FindClose),
            (g.find_field, VHit::FindField),
            (g.info_close, VHit::InfoClose),
            (g.info, VHit::Info),
        ] {
            if r.is_some_and(|r| r.contains(x, y)) {
                return hit;
            }
        }
        match &v.body {
            ViewerBody::Text { .. } if g.content.contains(x, y) => {
                VHit::Text { pos: self.viewer_text_position(v, size_dip, scale, px, py).unwrap_or(0) }
            }
            ViewerBody::Image { .. } if g.content.contains(x, y) => VHit::Image,
            _ => VHit::Nothing,
        }
    }
}

/// A tooltip centred above a toolbar button.
fn centred_tip(r: &mut Renderer, cv: &mut Canvas, text: &str, button: Rect, size_dip: (f32, f32)) {
    if text.is_empty() {
        return;
    }
    let s = cv.s;
    let tw = r.measure(text, SMALL_PX * s, 16.0 * s) / s + 16.0;
    // `tooltip` right-aligns to its anchor: give it an anchor whose right edge centres the label
    let anchor = Rect { x: button.x + button.w / 2.0 + tw / 2.0 - 1.0, y: button.y - 4.0, w: 1.0, h: button.h };
    tooltip(r, cv, text, anchor, size_dip, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(list: &[Tool]) -> Vec<ToolView> {
        list.iter().map(|t| ToolView { tool: *t, on: false, enabled: true, tip: format!("{t:?}") }).collect()
    }

    fn text_view(display: &str) -> ViewerView {
        ViewerView {
            title: "Text, 12 characters".into(),
            body: ViewerBody::Text {
                display: display.into(),
                gen: 1,
                selection: None,
                scroll: (0.0, 0.0),
                wrap: true,
                matches: vec![],
                current: None,
            },
            tools: tools(&[Tool::Prev, Tool::Next, Tool::Copy, Tool::Paste, Tool::Pin, Tool::Delete, Tool::Search, Tool::Wrap, Tool::More, Tool::Close]),
            ..ViewerView::default()
        }
    }

    const SIZE: (f32, f32) = (1200.0, 800.0);

    #[test]
    fn the_text_container_is_three_quarters_wide_and_clear_of_the_toolbar() {
        let v = text_view("hello");
        let g = viewer_geometry(&v, SIZE);
        assert!((g.content.w - 900.0).abs() < 0.5, "{:?}", g.content);
        assert!(g.content.h <= 640.0 + 0.5);
        assert!(g.content.y + g.content.h <= g.toolbar.y, "toolbar must not cover the text");
        assert!(g.content.y >= g.header.h);
        // the toolbar is centred and its buttons do not overlap
        assert!(((g.toolbar.x + g.toolbar.w / 2.0) - 600.0).abs() < 0.5);
        for w in g.tools.windows(2) {
            assert!(w[0].x + w[0].w <= w[1].x);
        }
        assert_eq!(g.separators.len(), 3);
    }

    #[test]
    fn an_open_info_panel_moves_the_content_aside() {
        let mut v = text_view("hello");
        v.info = Some(vec![("Type".into(), "Text".into())]);
        let g = viewer_geometry(&v, SIZE);
        let info = g.info.unwrap();
        assert!(g.content.x + g.content.w <= info.x);
    }

    #[test]
    fn hits_find_buttons_text_and_menu_items() {
        let mut r = Renderer::new();
        let mut v = text_view("hello world, this is a clip");
        let g = viewer_geometry(&v, SIZE);
        let t = g.tools[2];
        assert_eq!(r.viewer_hit(&v, SIZE, 1.0, t.x + 5.0, t.y + 5.0), VHit::Tool(Tool::Copy));
        assert_eq!(r.viewer_hit(&v, SIZE, 1.0, g.fullscreen.x + 3.0, g.fullscreen.y + 3.0), VHit::Fullscreen);
        assert_eq!(r.viewer_hit(&v, SIZE, 1.0, g.back.x + 3.0, g.back.y + 3.0), VHit::Back);
        assert!(matches!(r.viewer_hit(&v, SIZE, 1.0, g.text.x + 2.0, g.text.y + 5.0), VHit::Text { pos: 0 }));
        v.menu = Some(vec![MenuItemView { label: "Save as...".into(), shortcut: "Ctrl+S".into(), enabled: true, danger: false }]);
        let g = viewer_geometry(&v, SIZE);
        let m = g.menu_items[0];
        assert_eq!(r.viewer_hit(&v, SIZE, 1.0, m.x + 5.0, m.y + 5.0), VHit::MenuItem(0));
        // a confirmation is modal
        v.confirm = Some(ConfirmView { title: "Delete?".into(), message: "x".into(), ok: "Delete".into(), danger: true });
        let g = viewer_geometry(&v, SIZE);
        assert_eq!(r.viewer_hit(&v, SIZE, 1.0, t.x + 5.0, t.y + 5.0), VHit::Nothing);
        let ok = g.confirm_ok.unwrap();
        assert_eq!(r.viewer_hit(&v, SIZE, 1.0, ok.x + 5.0, ok.y + 5.0), VHit::ConfirmOk);
    }

    #[test]
    fn long_text_scrolls_and_unwrapped_text_is_wider() {
        let mut r = Renderer::new();
        let long: String = (0..400).map(|i| format!("line {i} with some words in it that go on for a while\n")).collect();
        let mut v = text_view(&long);
        let ((w, h), (vw, vh)) = r.viewer_text_extent(&v, SIZE, 1.0).unwrap();
        assert!(h > vh * 5.0 && w <= vw + 1.0);
        if let ViewerBody::Text { wrap, gen, display, .. } = &mut v.body {
            *wrap = false;
            *gen = 2;
            display.push_str(&"x".repeat(3_000));
        }
        let ((w2, _), (vw2, _)) = r.viewer_text_extent(&v, SIZE, 1.0).unwrap();
        assert!(w2 > vw2 * 2.0);
        // drawing a scrolled frame touches only what is visible and still produces the frame
        if let ViewerBody::Text { scroll, .. } = &mut v.body {
            *scroll = (100.0, 2_000.0);
        }
        let f = r.render_viewer(&v, SIZE, 1.0);
        assert_eq!((f.width, f.height), (1200, 800));
        // the character box of a later line lies below an earlier one
        let a = r.viewer_char_box(&v, SIZE, 1.0, 0).unwrap();
        let b = r.viewer_char_box(&v, SIZE, 1.0, 200).unwrap();
        assert!(b.1 > a.1);
    }

    #[test]
    fn a_picture_fits_and_zooms_around_its_centre() {
        let mut r = Renderer::new();
        // left half red, right half blue
        let (w, h) = (400u32, 200u32);
        let mut px = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                px.extend_from_slice(if x < w / 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let img = Arc::new(Rgba { width: w, height: h, pixels: px });
        let mut v = ViewerView {
            body: ViewerBody::Image { img: Some(img.clone()), size: (w, h), zoom: None, center: (200.0, 100.0) },
            tools: tools(&[Tool::ZoomOut, Tool::FitActual, Tool::ZoomIn, Tool::Close]),
            ..ViewerView::default()
        };
        let g = viewer_geometry(&v, SIZE);
        let fit = fit_zoom(&g, w, h);
        assert!((fit * w as f32 - g.content.w).abs() < 1.0, "a wide picture fills the width");
        let f = r.render_viewer(&v, SIZE, 1.0);
        let (cx, cy) = ((g.content.x + g.content.w / 2.0) as u32, (g.content.y + g.content.h / 2.0) as u32);
        assert_eq!(f.pixel(cx - 20, cy), (255, 0, 0));
        assert_eq!(f.pixel(cx + 20, cy), (0, 0, 255));
        // zoomed in on the left quarter: the middle of the view is red, all of it
        v.body = ViewerBody::Image { img: Some(img.clone()), size: (w, h), zoom: Some(8.0), center: (100.0, 100.0) };
        let f = r.render_viewer(&v, SIZE, 1.0);
        assert_eq!(f.pixel(cx + 300, cy), (255, 0, 0));
        // a half-size preview standing in for the picture lands in the same place
        let half = Arc::new(pasteroll_core::image::downscale(&img, w / 2, h / 2));
        v.body = ViewerBody::Image { img: Some(half), size: (w, h), zoom: None, center: (200.0, 100.0) };
        let f = r.render_viewer(&v, SIZE, 1.0);
        assert_eq!(f.pixel(cx - 20, cy), (255, 0, 0));
        assert_eq!(f.pixel(cx + 20, cy), (0, 0, 255));
        let left_edge = (g.content.x + 3.0) as u32;
        assert_eq!(f.pixel(left_edge, cy), (255, 0, 0), "it fills the width as the real picture would");
        // nothing is drawn under the toolbar
        let tb = g.toolbar;
        assert_ne!(f.pixel((tb.x - 40.0) as u32, (tb.y + 10.0) as u32), (255, 0, 0));
    }

    #[test]
    fn the_morph_goes_from_one_frame_to_the_other() {
        let a = Frame { width: 10, height: 10, bgra: [0u8, 0, 255, 255].repeat(100) };
        let b = Frame { width: 40, height: 20, bgra: [255u8, 0, 0, 255].repeat(800) };
        let m = Morph::new(&a, &b).unwrap();
        assert_eq!(m.frame(0.0, 10, 10).pixel(5, 5), (255, 0, 0));
        let end = m.frame(1.0, 40, 20);
        assert_eq!((end.width, end.pixel(30, 10)), (40, (0, 0, 255)));
        let mid = m.frame(0.5, 25, 15).pixel(10, 7);
        assert!(mid.0 > 60 && mid.2 > 60, "{mid:?}");
    }

    #[test]
    fn the_wipe_clears_clip_text() {
        let mut v = text_view("secret");
        v.find = Some(FindView { query: "sec".into(), count: 1, current: Some(0) });
        v.wipe();
        match &v.body {
            ViewerBody::Text { display, .. } => assert!(display.is_empty()),
            _ => unreachable!(),
        }
        assert!(v.find.unwrap().query.is_empty());
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use std::time::Instant;

    /// `cargo test --release -p pasteroll-render timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn timing_viewer_frames() {
        let mut r = Renderer::new();
        let long: String = (0..400).map(|i| format!("line {i} with words in it that go on for a while\n")).collect();
        let v = ViewerView {
            body: ViewerBody::Text { display: long, gen: 1, selection: None, scroll: (0.0, 0.0), wrap: true, matches: vec![], current: None },
            tools: vec![ToolView { tool: Tool::Close, on: false, enabled: true, tip: String::new() }],
            ..ViewerView::default()
        };
        let size = (2880.0 / 2.5, 1620.0 / 2.5);
        for _ in 0..2 {
            let t = Instant::now();
            let f = r.render_viewer(&v, size, 2.5);
            eprintln!("render 2880x1620 text: {:?}", t.elapsed());
            let small = Frame { width: 1210, height: 465, bgra: vec![255; 1210 * 465 * 4] };
            let t = Instant::now();
            let m = Morph::new(&small, &f).unwrap();
            eprintln!("morph setup: {:?}", t.elapsed());
            for (w, h) in [(2880u32, 1620u32), (1440, 810), (960, 540)] {
                let t = Instant::now();
                let _ = m.frame(0.5, w, h);
                eprintln!("morph frame {w}x{h}: {:?}", t.elapsed());
            }
            let t = Instant::now();
            let _ = f.clone();
            eprintln!("clone frame: {:?}", t.elapsed());
        }
    }
}
