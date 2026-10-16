//! The Advanced Image Paste window: a live preview on the left, the adjustments in a panel on the
//! right, the file name and the Paste / Cancel buttons along the bottom.
//!
//! The controller describes what is shown as an [`EditorView`] (a flat list of [`Item`]s for the
//! panel, much like a form); [`editor_layout`] places everything, [`Renderer::render_editor`] draws
//! it and [`editor_hit`] says what a pointer position means. Drawing and hit testing share the same
//! layout, so they cannot disagree.

use super::icons::{self, Icon};
use super::{
    scaled, tooltip, Canvas, Frame, Rect, Renderer, Rgb, ACCENT, ACTIVE_ROW, DANGER, FIELD, FOREGROUND, MENU,
    MENU_BORDER, MUTED, SMALL_PX, SURFACE, SURFACE_BORDER,
};
use cosmic_text::Wrap;
use pasteroll_core::image::Rgba;
use std::sync::Arc;
use tiny_skia::{BlendMode, FilterQuality, PixmapPaint, Transform};

pub type Col = [u8; 4];

const HEADER_H: f32 = 52.0;
const FOOTER_H: f32 = 76.0;
const PANEL_W: f32 = 336.0;
const LABEL_W: f32 = 84.0;
const PAD: f32 = 12.0;
const TEXT_PX: f32 = 13.0;
const TOOLBAR_BG: Rgb = (22, 22, 26);
const BUTTON_HOT: Rgb = (66, 70, 82);
const ICON: Rgb = (214, 214, 222);
const ICON_OFF: Rgb = (96, 96, 106);
/// The picture is shown at most this many times larger than its real size.
const MAX_UPSCALE: f32 = 4.0;
const HANDLE: f32 = 9.0;

// ---------------------------------------------------------------- what the controller describes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Top {
    Undo,
    Redo,
    Reset,
    Presets,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopView {
    pub kind: Top,
    pub enabled: bool,
    pub tip: String,
}

/// A text box. `sel_all` shows the whole text selected (typing replaces it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldView {
    /// A short caption inside the box, before the value ("W").
    pub label: String,
    pub text: String,
    /// Shown muted after the value ("px").
    pub suffix: String,
    pub focused: bool,
    pub sel_all: bool,
    /// Grey hint when `text` is empty.
    pub placeholder: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ButtonView {
    pub label: String,
    pub icon: Option<Icon>,
    pub enabled: bool,
    pub primary: bool,
    /// Shown pressed (a toggle that is on).
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// A section's title row. `switch` is the on/off toggle of effects that have one.
    Header { title: String, summary: String, open: bool, switch: Option<bool> },
    /// Part 0 is the track, part 1 the value box (type a number in it).
    Slider { label: String, field: FieldView, frac: f32 },
    /// One or more text boxes in a row; the part is the box.
    Fields { label: String, fields: Vec<FieldView> },
    Chips { label: String, options: Vec<String>, selected: Option<usize> },
    Check { label: String, on: bool },
    /// A colour swatch with its hex code; clicking opens the colour popup.
    Color { label: String, color: Col, open: bool },
    Buttons { buttons: Vec<ButtonView> },
    Note { text: String },
    /// A grid of colours (the colour popup); the part is the swatch.
    Palette { colors: Vec<Col>, selected: Option<usize> },
    /// A labelled text box on its own row.
    Text { label: String, field: FieldView },
    /// A row of a menu: part 0 the row, part 1 its delete cross.
    MenuRow { label: String, can_delete: bool, on: bool },
    Gap(f32),
    Separator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: u32,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PopupAnchor {
    Top(Top),
    Item(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PopupView {
    pub anchor: PopupAnchor,
    pub title: String,
    pub w: f32,
    pub items: Vec<Item>,
}

/// The crop rectangle, in pixels of the picture shown.
#[derive(Debug, Clone, PartialEq)]
pub struct CropView {
    pub rect: (f32, f32, f32, f32),
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct EditorView {
    pub title: String,
    pub subtitle: String,
    pub tops: Vec<TopView>,
    /// The picture to show (the adjusted result, or the original while cropping).
    pub preview: Option<Arc<Rgba>>,
    /// The size the preview stands for, in real pixels (the preview may be a reduced copy).
    pub dims: (u32, u32),
    /// The preview is being redrawn.
    pub busy: bool,
    pub crop: Option<CropView>,
    pub items: Vec<Item>,
    pub scroll: f32,
    pub name: FieldView,
    pub ext: String,
    pub folder: FieldView,
    pub paste_label: String,
    pub paste_enabled: bool,
    pub popup: Option<PopupView>,
    pub toast: String,
    /// A line under the preview (why Paste is unavailable, or the size of the result).
    pub note: String,
    pub hover: Option<EHit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropPart {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EHit {
    Back,
    Top(Top),
    /// `part` and `frac` depend on the item (see [`ItemKind`]); `frac` is where along a slider's track.
    Item { id: u32, part: u16, frac: f32 },
    Name,
    Folder,
    Browse,
    Cancel,
    Paste,
    Crop(CropPart),
    Image,
    Panel,
    /// Inside the popup, on nothing in particular.
    Popup,
    /// A popup is open and the pointer is outside it.
    OutsidePopup,
    Nothing,
}

impl EHit {
    /// The same hit without the position along a slider (what hover compares).
    pub fn plain(self) -> EHit {
        match self {
            EHit::Item { id, part, .. } => EHit::Item { id, part, frac: 0.0 },
            other => other,
        }
    }
}

// ---------------------------------------------------------------- layout

#[derive(Debug, Clone)]
pub struct Placed {
    pub id: u32,
    pub rect: Rect,
    pub parts: Vec<Rect>,
}

#[derive(Debug, Clone)]
pub struct ELayout {
    pub size: (f32, f32),
    pub back: Rect,
    pub tops: Vec<Rect>,
    pub preview: Rect,
    /// Where the picture is drawn (inside `preview`).
    pub image: Option<Rect>,
    /// The crop rectangle on screen.
    pub crop: Option<Rect>,
    pub panel: Rect,
    pub placed: Vec<Placed>,
    pub content_h: f32,
    pub max_scroll: f32,
    pub name: Rect,
    pub ext: Rect,
    pub folder: Rect,
    pub browse: Rect,
    pub cancel: Rect,
    pub paste: Rect,
    pub popup: Option<(Rect, Vec<Placed>)>,
}

fn chip_w(text: &str) -> f32 {
    (text.chars().count() as f32 * 7.0 + 22.0).max(38.0)
}

fn chip_rows(label: &str, options: &[String], w: f32) -> Vec<Vec<usize>> {
    let start = PAD + if label.is_empty() { 0.0 } else { LABEL_W };
    let avail = w - start - PAD;
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0.0;
    for (i, o) in options.iter().enumerate() {
        let cw = chip_w(o);
        if used > 0.0 && used + cw > avail {
            rows.push(Vec::new());
            used = 0.0;
        }
        rows.last_mut().expect("a row").push(i);
        used += cw + 4.0;
    }
    rows
}

fn note_lines(text: &str, w: f32) -> usize {
    let per_line = ((w - 2.0 * PAD) / 6.2).max(8.0);
    text.split('\n').map(|l| (l.chars().count() as f32 / per_line).ceil().max(1.0) as usize).sum()
}

fn palette_cols(w: f32) -> usize {
    (((w - 2.0 * PAD + 4.0) / 26.0).floor() as usize).max(1)
}

/// Places one item with its top-left corner at `(x, y)`, `w` wide.
fn place(item: &Item, x: f32, y: f32, w: f32) -> Placed {
    let mut parts = Vec::new();
    let h = match &item.kind {
        ItemKind::Header { switch, .. } => {
            let r = Rect { x, y, w, h: 36.0 };
            parts.push(Rect { w: if switch.is_some() { w - 56.0 } else { w }, ..r });
            if switch.is_some() {
                parts.push(Rect { x: x + w - 46.0, y: y + 9.0, w: 34.0, h: 18.0 });
            }
            36.0
        }
        ItemKind::Slider { .. } => {
            parts.push(Rect { x: x + PAD + LABEL_W, y, w: w - 2.0 * PAD - LABEL_W - 62.0, h: 30.0 });
            parts.push(Rect { x: x + w - PAD - 54.0, y: y + 3.0, w: 54.0, h: 24.0 });
            30.0
        }
        ItemKind::Fields { label, fields } => {
            let start = x + PAD + if label.is_empty() { 0.0 } else { LABEL_W };
            let avail = x + w - PAD - start;
            let n = fields.len().max(1) as f32;
            let fw = (avail - 6.0 * (n - 1.0)) / n;
            for i in 0..fields.len() {
                parts.push(Rect { x: start + i as f32 * (fw + 6.0), y: y + 3.0, w: fw, h: 28.0 });
            }
            34.0
        }
        ItemKind::Chips { label, options, .. } => {
            let rows = chip_rows(label, options, w);
            let start = x + PAD + if label.is_empty() { 0.0 } else { LABEL_W };
            parts = vec![Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; options.len()];
            for (ri, row) in rows.iter().enumerate() {
                let mut cx = start;
                for &i in row {
                    let cw = chip_w(&options[i]);
                    parts[i] = Rect { x: cx, y: y + 2.0 + ri as f32 * 30.0, w: cw, h: 26.0 };
                    cx += cw + 4.0;
                }
            }
            rows.len() as f32 * 30.0 + 4.0
        }
        ItemKind::Check { .. } => {
            parts.push(Rect { x: x + PAD - 2.0, y, w: w - 2.0 * PAD + 4.0, h: 28.0 });
            28.0
        }
        ItemKind::Color { .. } => {
            parts.push(Rect { x: x + PAD + LABEL_W, y: y + 4.0, w: 150.0, h: 24.0 });
            32.0
        }
        ItemKind::Buttons { buttons } => {
            let n = buttons.len().max(1) as f32;
            let bw = (w - 2.0 * PAD - 6.0 * (n - 1.0)) / n;
            for i in 0..buttons.len() {
                parts.push(Rect { x: x + PAD + i as f32 * (bw + 6.0), y: y + 2.0, w: bw, h: 28.0 });
            }
            32.0
        }
        ItemKind::Note { text } => note_lines(text, w) as f32 * 15.0 + 6.0,
        ItemKind::Palette { colors, .. } => {
            let cols = palette_cols(w);
            for i in 0..colors.len() {
                parts.push(Rect { x: x + PAD + (i % cols) as f32 * 26.0, y: y + 2.0 + (i / cols) as f32 * 26.0, w: 22.0, h: 22.0 });
            }
            colors.len().div_ceil(cols) as f32 * 26.0 + 6.0
        }
        ItemKind::Text { label, .. } => {
            let start = x + PAD + if label.is_empty() { 0.0 } else { LABEL_W };
            parts.push(Rect { x: start, y: y + 3.0, w: x + w - PAD - start, h: 28.0 });
            34.0
        }
        ItemKind::MenuRow { can_delete, .. } => {
            parts.push(Rect { x: x + 4.0, y, w: w - 8.0, h: 30.0 });
            if *can_delete {
                parts.push(Rect { x: x + w - 34.0, y: y + 3.0, w: 24.0, h: 24.0 });
            }
            30.0
        }
        ItemKind::Gap(g) => *g,
        ItemKind::Separator => 10.0,
    };
    Placed { id: item.id, rect: Rect { x, y, w, h }, parts }
}

fn place_all(items: &[Item], x: f32, mut y: f32, w: f32) -> (Vec<Placed>, f32) {
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let p = place(it, x, y, w);
        y += p.rect.h + 2.0;
        out.push(p);
    }
    (out, y)
}

/// The picture's rectangle: as large as fits, never more than [`MAX_UPSCALE`] times its real size.
pub fn fit_rect(area: Rect, dims: (u32, u32)) -> Rect {
    let (iw, ih) = (dims.0.max(1) as f32, dims.1.max(1) as f32);
    let k = (area.w / iw).min(area.h / ih).min(MAX_UPSCALE).max(0.0);
    let (w, h) = (iw * k, ih * k);
    Rect { x: area.x + (area.w - w) / 2.0, y: area.y + (area.h - h) / 2.0, w, h }
}

pub fn editor_layout(v: &EditorView, size: (f32, f32)) -> ELayout {
    let (w, h) = size;
    let back = Rect { x: 12.0, y: 9.0, w: 34.0, h: 34.0 };
    let mut tops = vec![Rect { x: 0.0, y: 9.0, w: 0.0, h: 34.0 }; v.tops.len()];
    let mut x = w - 12.0;
    for (i, t) in v.tops.iter().enumerate().rev() {
        let bw = if t.kind == Top::Presets { 112.0 } else { 34.0 };
        x -= bw;
        tops[i] = Rect { x, y: 9.0, w: bw, h: 34.0 };
        x -= 4.0;
    }
    let footer_y = h - FOOTER_H;
    let panel = Rect { x: w - PANEL_W - 12.0, y: HEADER_H, w: PANEL_W, h: (footer_y - HEADER_H - 6.0).max(0.0) };
    let preview = Rect { x: 12.0, y: HEADER_H, w: (panel.x - 24.0).max(0.0), h: panel.h };
    let area = Rect { x: preview.x + 14.0, y: preview.y + 14.0, w: (preview.w - 28.0).max(1.0), h: (preview.h - 52.0).max(1.0) };
    let image = (v.dims.0 > 0 && v.dims.1 > 0).then(|| fit_rect(area, v.dims));
    let crop = match (&v.crop, image) {
        (Some(c), Some(img)) => {
            let k = img.w / v.dims.0.max(1) as f32;
            Some(Rect { x: img.x + c.rect.0 * k, y: img.y + c.rect.1 * k, w: c.rect.2 * k, h: c.rect.3 * k })
        }
        _ => None,
    };
    let (mut placed, end) = place_all(&v.items, panel.x, panel.y + 8.0 - v.scroll, panel.w);
    let content_h = end + v.scroll - panel.y + 8.0;
    let max_scroll = (content_h - panel.h).max(0.0);
    placed.shrink_to_fit();
    let fy = footer_y + 32.0;
    let fh = 32.0;
    let paste = Rect { x: w - 14.0 - 156.0, y: fy, w: 156.0, h: fh };
    let cancel = Rect { x: paste.x - 8.0 - 96.0, y: fy, w: 96.0, h: fh };
    let browse = Rect { x: cancel.x - 20.0 - 34.0, y: fy, w: 34.0, h: fh };
    let left = 14.0;
    let right = browse.x - 6.0;
    let name_w = ((right - left - 48.0 - 14.0) * 0.4).max(80.0);
    let name = Rect { x: left, y: fy, w: name_w, h: fh };
    let ext = Rect { x: name.x + name.w + 4.0, y: fy, w: 48.0, h: fh };
    let folder_x = ext.x + ext.w + 10.0;
    let folder = Rect { x: folder_x, y: fy, w: (right - folder_x).max(40.0), h: fh };

    let popup = v.popup.as_ref().map(|p| {
        let anchor = match p.anchor {
            PopupAnchor::Top(t) => v.tops.iter().position(|x| x.kind == t).map(|i| tops[i]),
            PopupAnchor::Item(id) => placed.iter().find(|pl| pl.id == id).map(|pl| pl.rect),
        }
        .unwrap_or(Rect { x: w - 200.0, y: HEADER_H, w: 100.0, h: 20.0 });
        let title_h = if p.title.is_empty() { 8.0 } else { 34.0 };
        let (items_probe, end) = place_all(&p.items, 0.0, 0.0, p.w);
        let _ = items_probe;
        let ph = end + title_h + 6.0;
        let mut px = match p.anchor {
            PopupAnchor::Top(_) => anchor.x + anchor.w - p.w,
            PopupAnchor::Item(_) => anchor.x,
        };
        px = px.clamp(8.0, (w - p.w - 8.0).max(8.0));
        let mut py = anchor.y + anchor.h + 6.0;
        if py + ph > h - 8.0 {
            py = (anchor.y - ph - 6.0).max(8.0);
        }
        let rect = Rect { x: px, y: py, w: p.w, h: ph };
        let (items, _) = place_all(&p.items, px, py + title_h, p.w);
        (rect, items)
    });
    ELayout {
        size,
        back,
        tops,
        preview,
        image,
        crop,
        panel,
        placed,
        content_h,
        max_scroll,
        name,
        ext,
        folder,
        browse,
        cancel,
        paste,
        popup,
    }
}

fn part_at(parts: &[Rect], x: f32, y: f32) -> Option<usize> {
    parts.iter().position(|r| r.contains(x, y))
}

/// What is under `(px, py)`, in pixels of the editor window.
pub fn editor_hit(v: &EditorView, size: (f32, f32), scale: f32, px: f32, py: f32) -> EHit {
    let l = editor_layout(v, size);
    let (x, y) = (px / scale, py / scale);
    let item_hit = |pl: &Placed, kind: &ItemKind| -> Option<EHit> {
        if !pl.rect.contains(x, y) {
            return None;
        }
        let part = match kind {
            ItemKind::Header { .. } => part_at(&pl.parts, x, y).or(Some(0)),
            ItemKind::Slider { .. } => {
                // the track answers over its whole height and a little past its ends
                let track = pl.parts[0];
                if pl.parts[1].contains(x, y) {
                    Some(1)
                } else if x >= track.x - 6.0 && x <= track.x + track.w + 6.0 {
                    Some(0)
                } else {
                    None
                }
            }
            ItemKind::Fields { .. } | ItemKind::Chips { .. } | ItemKind::Buttons { .. } | ItemKind::Palette { .. } => {
                part_at(&pl.parts, x, y)
            }
            ItemKind::Check { .. } | ItemKind::Color { .. } | ItemKind::Text { .. } => part_at(&pl.parts, x, y),
            ItemKind::MenuRow { .. } => part_at(&pl.parts[1..], x, y).map(|i| i + 1).or_else(|| part_at(&pl.parts[..1], x, y)),
            ItemKind::Note { .. } | ItemKind::Gap(_) | ItemKind::Separator => None,
        }?;
        let frac = match kind {
            ItemKind::Slider { .. } if part == 0 => {
                let t = pl.parts[0];
                ((x - t.x - 7.0) / (t.w - 14.0).max(1.0)).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        Some(EHit::Item { id: pl.id, part: part as u16, frac })
    };
    if let (Some((rect, items)), Some(p)) = (&l.popup, &v.popup) {
        if rect.contains(x, y) {
            for (pl, it) in items.iter().zip(&p.items) {
                if let Some(h) = item_hit(pl, &it.kind) {
                    return h;
                }
            }
            return EHit::Popup;
        }
        return EHit::OutsidePopup;
    }
    if l.back.contains(x, y) {
        return EHit::Back;
    }
    for (r, t) in l.tops.iter().zip(&v.tops) {
        if r.contains(x, y) {
            return EHit::Top(t.kind);
        }
    }
    for (r, hit) in [(l.paste, EHit::Paste), (l.cancel, EHit::Cancel), (l.browse, EHit::Browse), (l.name, EHit::Name), (l.folder, EHit::Folder)] {
        if r.contains(x, y) {
            return hit;
        }
    }
    if l.panel.contains(x, y) {
        for (pl, it) in l.placed.iter().zip(&v.items) {
            if let Some(h) = item_hit(pl, &it.kind) {
                return h;
            }
        }
        return EHit::Panel;
    }
    if let Some(c) = l.crop {
        if let Some(part) = crop_part_at(c, x, y) {
            return EHit::Crop(part);
        }
    }
    if l.image.is_some_and(|r| r.contains(x, y)) {
        return EHit::Image;
    }
    EHit::Nothing
}

/// Which handle of the crop rectangle `c` (on screen, DIP) is at `(x, y)`.
pub fn crop_part_at(c: Rect, x: f32, y: f32) -> Option<CropPart> {
    let near = |a: f32, b: f32| (a - b).abs() <= HANDLE;
    let (l, r, t, b) = (c.x, c.x + c.w, c.y, c.y + c.h);
    let (mx, my) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
    // corners first, then the middle of each edge, then the edges themselves
    for (hx, hy, part) in [(l, t, CropPart::NW), (r, t, CropPart::NE), (r, b, CropPart::SE), (l, b, CropPart::SW)] {
        if near(x, hx) && near(y, hy) {
            return Some(part);
        }
    }
    for (hx, hy, part) in [(mx, t, CropPart::N), (r, my, CropPart::E), (mx, b, CropPart::S), (l, my, CropPart::W)] {
        if near(x, hx) && near(y, hy) {
            return Some(part);
        }
    }
    let inside_x = x > l - 6.0 && x < r + 6.0;
    let inside_y = y > t - 6.0 && y < b + 6.0;
    if inside_x && (y - t).abs() <= 6.0 {
        return Some(CropPart::N);
    }
    if inside_x && (y - b).abs() <= 6.0 {
        return Some(CropPart::S);
    }
    if inside_y && (x - l).abs() <= 6.0 {
        return Some(CropPart::W);
    }
    if inside_y && (x - r).abs() <= 6.0 {
        return Some(CropPart::E);
    }
    c.contains(x, y).then_some(CropPart::Move)
}

// ---------------------------------------------------------------- drawing

fn rgb_of(c: Col) -> Rgb {
    (c[0], c[1], c[2])
}

impl Renderer {
    fn etext(&mut self, cv: &mut Canvas, text: &str, x: f32, y: f32, px: f32, rgb: Rgb, clip: Rect) -> f32 {
        let s = cv.s;
        self.draw_text(cv, text, (x * s, y * s), px * s, (px + 5.0) * s, None, Wrap::None, rgb, scaled(clip, s)) / s
    }

    fn ewidth(&mut self, text: &str, px: f32, s: f32) -> f32 {
        self.measure(text, px * s, (px + 5.0) * s) / s
    }

    /// Text vertically centred in `r`.
    fn centred(&mut self, cv: &mut Canvas, text: &str, r: Rect, px: f32, rgb: Rgb) {
        let y = r.y + (r.h - px - 5.0) / 2.0 + 0.5;
        self.etext(cv, text, r.x, y, px, rgb, r);
    }

    fn icon_btn(&mut self, cv: &mut Canvas, r: Rect, icon: Icon, hot: bool, on: bool, enabled: bool) {
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

    fn field_box(&mut self, cv: &mut Canvas, r: Rect, f: &FieldView, hot: bool) {
        let s = cv.s;
        cv.fill(r, 5.0, FIELD, 1.0);
        cv.stroke(r, 5.0, if f.focused { ACCENT } else if hot { (96, 96, 112) } else { SURFACE_BORDER }, 1.0, 1.0);
        let mut x = r.x + 8.0;
        if !f.label.is_empty() {
            let lw = self.ewidth(&f.label, 12.0, s);
            self.centred(cv, &f.label, Rect { x, ..r }, 12.0, MUTED);
            x += lw + 7.0;
        }
        let inner = Rect { x: r.x + 2.0, y: r.y, w: r.w - 4.0, h: r.h };
        let suffix_w = if f.suffix.is_empty() { 0.0 } else { self.ewidth(&f.suffix, 12.0, s) + 6.0 };
        let clip = Rect { w: (inner.w - (x - inner.x) - suffix_w - 4.0).max(0.0), x, ..inner };
        if f.text.is_empty() {
            self.centred(cv, &f.placeholder, clip, TEXT_PX, MUTED);
        } else {
            let tw = self.ewidth(&f.text, TEXT_PX, s);
            if f.focused && f.sel_all {
                cv.fill(Rect { x: x - 1.0, y: r.y + 5.0, w: tw.min(clip.w) + 2.0, h: r.h - 10.0 }, 2.0, ACCENT, 0.45);
            }
            // a long text shows its end while it is being typed
            let tx = if f.focused && tw > clip.w { x - (tw - clip.w) } else { x };
            self.centred(cv, &f.text, Rect { x: tx, ..clip }, TEXT_PX, FOREGROUND);
            if f.focused && !f.sel_all {
                let cx = (tx + tw + 1.0).min(clip.x + clip.w);
                cv.line(&[(cx, r.y + 6.0), (cx, r.y + r.h - 6.0)], ACCENT, 1.4);
            }
        }
        if f.focused && f.text.is_empty() {
            cv.line(&[(x, r.y + 6.0), (x, r.y + r.h - 6.0)], ACCENT, 1.4);
        }
        if !f.suffix.is_empty() {
            self.centred(cv, &f.suffix, Rect { x: r.x + r.w - suffix_w - 2.0, ..inner }, 12.0, MUTED);
        }
    }

    fn swatch(&mut self, cv: &mut Canvas, r: Rect, c: Col, selected: bool, hot: bool) {
        checker(cv, r, 5.0, 3.0);
        if c[3] > 0 {
            cv.fill(r, 4.0, rgb_of(c), f32::from(c[3]) / 255.0);
        }
        cv.stroke(r, 4.0, if selected { ACCENT } else if hot { (200, 200, 210) } else { (90, 90, 104) }, 1.0, if selected { 2.0 } else { 1.0 });
    }

    fn draw_item(&mut self, cv: &mut Canvas, it: &Item, pl: &Placed, hover: Option<EHit>, clip: Rect) {
        let s = cv.s;
        let r = pl.rect;
        let hot_part = match hover {
            Some(EHit::Item { id, part, .. }) if id == it.id => Some(part as usize),
            _ => None,
        };
        let label_y = |r: Rect| Rect { x: r.x + PAD, w: LABEL_W - 6.0, ..r };
        match &it.kind {
            ItemKind::Header { title, summary, open, switch } => {
                let row = Rect { x: r.x + 4.0, y: r.y + 1.0, w: r.w - 8.0, h: r.h - 2.0 };
                if *open {
                    cv.fill(row, 7.0, ACCENT, 0.14);
                } else if hot_part.is_some() {
                    cv.fill(row, 7.0, BUTTON_HOT, 0.7);
                }
                icons::draw(cv, if *open { Icon::ChevronDown } else { Icon::Next }, Rect { x: r.x + 8.0, y: r.y + 4.0, w: 28.0, h: 28.0 }, 13.0, if *open { ACCENT } else { MUTED });
                let tw = self.ewidth(title, 14.0, s);
                self.etext(cv, title, r.x + 36.0, r.y + 9.0, 14.0, FOREGROUND, clip);
                let sx = r.x + 36.0 + tw + 10.0;
                let right = r.x + r.w - if switch.is_some() { 62.0 } else { 16.0 };
                if !summary.is_empty() && sx < right {
                    let sw = self.ewidth(summary, 12.0, s);
                    self.etext(cv, summary, (right - sw).max(sx), r.y + 10.0, 12.0, MUTED, clip);
                }
                if let Some(on) = switch {
                    let sw = pl.parts[1];
                    cv.fill(sw, 9.0, if *on { ACCENT } else { (70, 70, 82) }, 1.0);
                    let kx = if *on { sw.x + sw.w - 16.0 } else { sw.x + 2.0 };
                    cv.fill(Rect { x: kx, y: sw.y + 2.0, w: 14.0, h: 14.0 }, 7.0, (240, 240, 244), 1.0);
                }
            }
            ItemKind::Slider { label, field, frac } => {
                self.centred(cv, label, label_y(r), TEXT_PX, MUTED);
                let t = pl.parts[0];
                let cy = t.y + t.h / 2.0;
                let (x0, x1) = (t.x + 7.0, t.x + t.w - 7.0);
                cv.fill(Rect { x: x0, y: cy - 2.0, w: x1 - x0, h: 4.0 }, 2.0, FIELD, 1.0);
                let kx = x0 + frac.clamp(0.0, 1.0) * (x1 - x0);
                cv.fill(Rect { x: x0, y: cy - 2.0, w: kx - x0, h: 4.0 }, 2.0, ACCENT, 1.0);
                cv.fill(Rect { x: kx - 7.0, y: cy - 7.0, w: 14.0, h: 14.0 }, 7.0, if hot_part == Some(0) { (255, 255, 255) } else { FOREGROUND }, 1.0);
                self.field_box(cv, pl.parts[1], field, hot_part == Some(1));
            }
            ItemKind::Fields { label, fields } => {
                if !label.is_empty() {
                    self.centred(cv, label, label_y(r), TEXT_PX, MUTED);
                }
                for (i, f) in fields.iter().enumerate() {
                    self.field_box(cv, pl.parts[i], f, hot_part == Some(i));
                }
            }
            ItemKind::Chips { label, options, selected } => {
                if !label.is_empty() {
                    self.centred(cv, label, Rect { h: 30.0, ..label_y(r) }, TEXT_PX, MUTED);
                }
                for (i, o) in options.iter().enumerate() {
                    let c = pl.parts[i];
                    let on = *selected == Some(i);
                    cv.fill(c, 6.0, if on { ACCENT } else { FIELD }, if on { 0.28 } else { 1.0 });
                    cv.stroke(c, 6.0, if on { ACCENT } else if hot_part == Some(i) { (120, 120, 136) } else { SURFACE_BORDER }, 1.0, 1.0);
                    let tw = self.ewidth(o, 12.0, s);
                    self.centred(cv, o, Rect { x: c.x + (c.w - tw) / 2.0, w: tw + 2.0, ..c }, 12.0, if on { FOREGROUND } else { (200, 200, 210) });
                }
            }
            ItemKind::Check { label, on } => {
                let b = Rect { x: r.x + PAD, y: r.y + (r.h - 16.0) / 2.0, w: 16.0, h: 16.0 };
                if *on {
                    cv.fill(b, 3.0, ACCENT, 1.0);
                    cv.line(&[(b.x + 3.5, b.y + 8.5), (b.x + 6.5, b.y + 11.5), (b.x + 12.5, b.y + 4.5)], (20, 20, 24), 2.0);
                } else {
                    cv.fill(b, 3.0, FIELD, 1.0);
                    cv.stroke(b, 3.0, if hot_part.is_some() { ACCENT } else { (110, 110, 124) }, 1.0, 1.2);
                }
                self.centred(cv, label, Rect { x: r.x + PAD + 26.0, w: r.w - PAD - 30.0, ..r }, TEXT_PX, FOREGROUND);
            }
            ItemKind::Color { label, color, open } => {
                self.centred(cv, label, label_y(r), TEXT_PX, MUTED);
                let p = pl.parts[0];
                let sw = Rect { x: p.x, y: p.y, w: 40.0, h: p.h };
                self.swatch(cv, sw, *color, *open, hot_part.is_some());
                let hex = if color[3] == 0 { "None".to_owned() } else { pasteroll_core::imgedit::Color(*color).hex() };
                self.centred(cv, &hex, Rect { x: p.x + 50.0, w: 100.0, ..p }, TEXT_PX, if hot_part.is_some() { FOREGROUND } else { (190, 190, 200) });
            }
            ItemKind::Buttons { buttons } => {
                for (i, b) in buttons.iter().enumerate() {
                    let c = pl.parts[i];
                    let hot = hot_part == Some(i) && b.enabled;
                    let (fill, a) = if b.primary { (ACCENT, if hot { 1.0 } else { 0.85 }) } else { (if hot { BUTTON_HOT } else { FIELD }, 1.0) };
                    cv.fill(c, 6.0, fill, a);
                    if !b.primary {
                        cv.stroke(c, 6.0, if b.on { ACCENT } else if hot { (120, 120, 136) } else { SURFACE_BORDER }, 1.0, 1.0);
                    }
                    if b.on && !b.primary {
                        cv.fill(c, 6.0, ACCENT, 0.24);
                    }
                    let col = if !b.enabled { ICON_OFF } else if b.primary { (20, 20, 24) } else { FOREGROUND };
                    let tw = self.ewidth(&b.label, 12.0, s);
                    let iw = if b.icon.is_some() { 20.0 } else { 0.0 };
                    let x0 = c.x + (c.w - tw - iw) / 2.0;
                    if let Some(ic) = b.icon {
                        icons::draw(cv, ic, Rect { x: x0, y: c.y, w: 18.0, h: c.h }, 14.0, col);
                    }
                    self.centred(cv, &b.label, Rect { x: x0 + iw, w: tw + 4.0, ..c }, 12.0, col);
                }
            }
            ItemKind::Note { text } => {
                let w = r.w - 2.0 * PAD;
                let box_ = Rect { x: r.x + PAD, y: r.y + 2.0, w, h: r.h - 2.0 };
                self.draw_text(cv, text, (box_.x * s, box_.y * s), 12.0 * s, 15.0 * s, Some(w * s), Wrap::WordOrGlyph, MUTED, scaled(box_.intersect_or(clip), s));
            }
            ItemKind::Palette { colors, selected } => {
                for (i, c) in colors.iter().enumerate() {
                    self.swatch(cv, pl.parts[i], *c, *selected == Some(i), hot_part == Some(i));
                    if c[3] == 0 {
                        let p = pl.parts[i];
                        cv.line(&[(p.x + 4.0, p.y + p.h - 4.0), (p.x + p.w - 4.0, p.y + 4.0)], DANGER, 1.6);
                    }
                }
            }
            ItemKind::Text { label, field } => {
                if !label.is_empty() {
                    self.centred(cv, label, label_y(r), TEXT_PX, MUTED);
                }
                self.field_box(cv, pl.parts[0], field, hot_part == Some(0));
            }
            ItemKind::MenuRow { label, can_delete, on } => {
                let row = pl.parts[0];
                if hot_part == Some(0) {
                    cv.fill(row, 5.0, ACCENT, 0.28);
                }
                if *on {
                    icons::draw(cv, Icon::Check, Rect { x: row.x + 4.0, y: row.y, w: 22.0, h: row.h }, 13.0, ACCENT);
                }
                let right = if *can_delete { 34.0 } else { 10.0 };
                self.centred(cv, label, Rect { x: row.x + 28.0, w: row.w - 28.0 - right, ..row }, TEXT_PX, FOREGROUND);
                if *can_delete {
                    let d = pl.parts[1];
                    let hot = hot_part == Some(1);
                    if hot {
                        cv.fill(d, 5.0, DANGER, 0.3);
                    }
                    icons::draw(cv, Icon::Close, d, 11.0, if hot { DANGER } else { MUTED });
                }
            }
            ItemKind::Gap(_) => {}
            ItemKind::Separator => {
                cv.line(&[(r.x + PAD, r.y + r.h / 2.0), (r.x + r.w - PAD, r.y + r.h / 2.0)], SURFACE_BORDER, 1.0);
            }
        }
    }

    pub fn render_editor(&mut self, v: &EditorView, size: (f32, f32), scale: f32) -> Frame {
        let s = scale;
        let (wpx, hpx) = ((size.0 * s).round().max(1.0) as u32, (size.1 * s).round().max(1.0) as u32);
        let mut cv = Canvas { pm: tiny_skia::Pixmap::new(wpx, hpx).expect("pixmap"), s };
        cv.pm.fill(super::color(ACTIVE_ROW, 1.0));
        let edge = Rect { x: 0.6, y: 0.6, w: size.0 - 1.2, h: size.1 - 1.2 };
        cv.stroke(edge, 8.0, ACCENT, 0.9, 1.2);
        let l = editor_layout(v, size);
        let hover = v.hover;
        let modal = v.popup.is_some();

        // ---- header
        self.icon_btn(&mut cv, l.back, Icon::Back, hover == Some(EHit::Back) && !modal, false, true);
        let title_x = l.back.x + l.back.w + 12.0;
        let right = l.tops.first().map_or(size.0 - 12.0, |r| r.x) - 12.0;
        let clip = Rect { x: title_x, y: 4.0, w: (right - title_x).max(0.0), h: HEADER_H - 8.0 };
        self.etext(&mut cv, &v.title, title_x, 9.0, 14.0, FOREGROUND, clip);
        if !v.subtitle.is_empty() {
            self.etext(&mut cv, &v.subtitle, title_x, 29.0, 12.0, MUTED, clip);
        }
        for (r, t) in l.tops.iter().zip(&v.tops) {
            let hot = hover == Some(EHit::Top(t.kind)) && !modal;
            match t.kind {
                Top::Undo => self.icon_btn(&mut cv, *r, Icon::Undo, hot, false, t.enabled),
                Top::Redo => self.icon_btn(&mut cv, *r, Icon::Redo, hot, false, t.enabled),
                Top::Reset => self.icon_btn(&mut cv, *r, Icon::Reset, hot, false, t.enabled),
                Top::Presets => {
                    let open = matches!(v.popup.as_ref().map(|p| p.anchor), Some(PopupAnchor::Top(Top::Presets)));
                    if open {
                        cv.fill(*r, 7.0, ACCENT, 0.24);
                    } else if hot {
                        cv.fill(*r, 7.0, BUTTON_HOT, 1.0);
                    }
                    icons::draw(&mut cv, Icon::Bookmark, Rect { w: 30.0, ..*r }, 15.0, if hot || open { FOREGROUND } else { ICON });
                    self.centred(&mut cv, "Presets", Rect { x: r.x + 30.0, w: 60.0, ..*r }, TEXT_PX, if hot || open { FOREGROUND } else { ICON });
                    icons::draw(&mut cv, Icon::ChevronDown, Rect { x: r.x + r.w - 26.0, w: 22.0, ..*r }, 11.0, MUTED);
                }
            }
        }

        // ---- preview
        let pv = l.preview;
        cv.fill(pv, 8.0, (20, 20, 24), 1.0);
        match (&v.preview, l.image) {
            (Some(img), Some(ir)) if img.width > 0 && img.height > 0 => {
                checker(&mut cv, ir, 0.0, 8.0);
                let k = ir.w * s / img.width as f32; // px per picture pixel
                let mut level = 0u32;
                while level < 6 && k * (1u32 << (level + 1)) as f32 <= 1.0 && (img.width >> (level + 1)) > 0 {
                    level += 1;
                }
                let f = (1u32 << level) as f32;
                let quality = if k * f >= 2.0 { FilterQuality::Nearest } else { FilterQuality::Bilinear };
                let Renderer { vimg, .. } = self;
                if let Some(pm) = vimg.level(img, level) {
                    let pp = PixmapPaint { opacity: 1.0, blend_mode: BlendMode::SourceOver, quality };
                    // fill the rectangle exactly even when the reduced copy rounds its size
                    let (kx, ky) = (ir.w * s / pm.width() as f32, ir.h * s / pm.height() as f32);
                    cv.pm.draw_pixmap(0, 0, pm.as_ref(), &pp, Transform::from_row(kx, 0.0, 0.0, ky, ir.x * s, ir.y * s), None);
                }
                cv.stroke(ir, 0.0, (90, 90, 104), 0.7, 1.0);
            }
            _ => {
                let m = "Preparing the picture...";
                let tw = self.ewidth(m, 14.0, s);
                self.etext(&mut cv, m, pv.x + (pv.w - tw) / 2.0, pv.y + pv.h / 2.0 - 10.0, 14.0, MUTED, pv);
            }
        }
        if let (Some(c), Some(cr), Some(ir)) = (&v.crop, l.crop, l.image) {
            // dim everything outside the crop
            for d in [
                Rect { x: ir.x, y: ir.y, w: ir.w, h: (cr.y - ir.y).max(0.0) },
                Rect { x: ir.x, y: cr.y + cr.h, w: ir.w, h: (ir.y + ir.h - cr.y - cr.h).max(0.0) },
                Rect { x: ir.x, y: cr.y, w: (cr.x - ir.x).max(0.0), h: cr.h },
                Rect { x: cr.x + cr.w, y: cr.y, w: (ir.x + ir.w - cr.x - cr.w).max(0.0), h: cr.h },
            ] {
                if d.w > 0.0 && d.h > 0.0 {
                    cv.fill(d, 0.0, (0, 0, 0), 0.58);
                }
            }
            cv.stroke(cr, 0.0, (255, 255, 255), 0.95, 1.4);
            for i in 1..3 {
                let t = i as f32 / 3.0;
                cv.line(&[(cr.x + cr.w * t, cr.y), (cr.x + cr.w * t, cr.y + cr.h)], (255, 255, 255), 0.6);
                cv.line(&[(cr.x, cr.y + cr.h * t), (cr.x + cr.w, cr.y + cr.h * t)], (255, 255, 255), 0.6);
            }
            let (l_, r_, t_, b_) = (cr.x, cr.x + cr.w, cr.y, cr.y + cr.h);
            let (mx, my) = (cr.x + cr.w / 2.0, cr.y + cr.h / 2.0);
            for (hx, hy) in [(l_, t_), (r_, t_), (r_, b_), (l_, b_), (mx, t_), (r_, my), (mx, b_), (l_, my)] {
                let hr = Rect { x: hx - 5.0, y: hy - 5.0, w: 10.0, h: 10.0 };
                cv.fill(hr, 2.0, (250, 250, 252), 1.0);
                cv.stroke(hr, 2.0, (30, 30, 36), 1.0, 1.0);
            }
            if !c.label.is_empty() {
                let tw = self.ewidth(&c.label, 12.0, s);
                let badge = Rect { x: cr.x + 6.0, y: (cr.y - 26.0).max(ir.y + 4.0).min(cr.y + 6.0), w: tw + 14.0, h: 22.0 };
                let badge = if cr.y - 26.0 >= ir.y { Rect { y: cr.y - 26.0, ..badge } } else { Rect { y: cr.y + 6.0, ..badge } };
                cv.fill(badge, 5.0, TOOLBAR_BG, 0.92);
                self.centred(&mut cv, &c.label, Rect { x: badge.x + 7.0, w: tw + 2.0, ..badge }, 12.0, FOREGROUND);
            }
        }
        if v.busy {
            let m = "Updating...";
            let tw = self.ewidth(m, 12.0, s);
            let b = Rect { x: pv.x + 10.0, y: pv.y + 10.0, w: tw + 16.0, h: 22.0 };
            cv.fill(b, 5.0, TOOLBAR_BG, 0.9);
            self.centred(&mut cv, m, Rect { x: b.x + 8.0, w: tw + 2.0, ..b }, 12.0, MUTED);
        }
        if !v.note.is_empty() {
            let note = Rect { x: pv.x + 12.0, y: pv.y + pv.h - 30.0, w: pv.w - 24.0, h: 22.0 };
            let tw = self.ewidth(&v.note, 12.0, s);
            self.etext(&mut cv, &v.note, note.x + (note.w - tw).max(0.0) / 2.0, note.y + 3.0, 12.0, MUTED, note);
        }

        // ---- panel
        cv.fill(l.panel, 8.0, SURFACE, 1.0);
        cv.stroke(l.panel, 8.0, SURFACE_BORDER, 1.0, 1.0);
        let inner = Rect { x: l.panel.x, y: l.panel.y + 1.0, w: l.panel.w, h: l.panel.h - 2.0 };
        for (pl, it) in l.placed.iter().zip(&v.items) {
            if pl.rect.y + pl.rect.h < inner.y || pl.rect.y > inner.y + inner.h {
                continue;
            }
            let h = if modal { None } else { hover };
            self.draw_item(&mut cv, it, pl, h, inner);
        }
        // a thin scroll bar when the panel is longer than the window
        if l.max_scroll > 0.0 {
            let track = Rect { x: l.panel.x + l.panel.w - 5.0, y: l.panel.y + 6.0, w: 3.0, h: l.panel.h - 12.0 };
            let th = (track.h * l.panel.h / l.content_h).max(24.0);
            let ty = track.y + (track.h - th) * (v.scroll / l.max_scroll).clamp(0.0, 1.0);
            cv.fill(Rect { y: ty, h: th, ..track }, 1.5, MUTED, 0.55);
        }

        // ---- footer
        let fy = size.1 - FOOTER_H;
        cv.fill(Rect { x: 0.0, y: fy, w: size.0, h: FOOTER_H }, 0.0, TOOLBAR_BG, 0.55);
        cv.line(&[(0.0, fy), (size.0, fy)], SURFACE_BORDER, 1.0);
        let cap = Rect { x: 0.0, y: fy + 9.0, w: size.0, h: 18.0 };
        self.etext(&mut cv, "File name", l.name.x + 2.0, fy + 11.0, 11.5, MUTED, cap);
        self.etext(&mut cv, "Save to", l.folder.x + 2.0, fy + 11.0, 11.5, MUTED, cap);
        let nhot = hover == Some(EHit::Name) && !modal;
        self.field_box(&mut cv, l.name, &v.name, nhot);
        self.centred(&mut cv, &v.ext, Rect { x: l.ext.x + 2.0, w: l.ext.w - 2.0, ..l.ext }, TEXT_PX, MUTED);
        self.field_box(&mut cv, l.folder, &v.folder, hover == Some(EHit::Folder) && !modal);
        let bhot = hover == Some(EHit::Browse) && !modal;
        cv.fill(l.browse, 7.0, if bhot { BUTTON_HOT } else { FIELD }, 1.0);
        cv.stroke(l.browse, 7.0, SURFACE_BORDER, 1.0, 1.0);
        icons::draw(&mut cv, Icon::Folder, l.browse, 16.0, if bhot { FOREGROUND } else { ICON });
        for (r, label, primary, hit, on) in [
            (l.cancel, "Cancel", false, EHit::Cancel, true),
            (l.paste, v.paste_label.as_str(), true, EHit::Paste, v.paste_enabled),
        ] {
            let hot = hover == Some(hit) && on && !modal;
            let (fill, a) = if primary {
                (ACCENT, if !on { 0.3 } else if hot { 1.0 } else { 0.88 })
            } else {
                (if hot { BUTTON_HOT } else { FIELD }, 1.0)
            };
            cv.fill(r, 7.0, fill, a);
            if !primary {
                cv.stroke(r, 7.0, SURFACE_BORDER, 1.0, 1.0);
            }
            let tw = self.ewidth(label, 14.0, s);
            let col = if primary { if on { (16, 24, 22) } else { (110, 120, 118) } } else { FOREGROUND };
            self.centred(&mut cv, label, Rect { x: r.x + (r.w - tw) / 2.0, w: tw + 4.0, ..r }, 14.0, col);
        }

        // ---- tooltips
        if !modal {
            match hover {
                Some(EHit::Back) => {
                    let tip = "Back to the carousel (Esc)";
                    let tw = self.ewidth(tip, SMALL_PX, s) + 16.0;
                    let anchor = Rect { x: l.back.x + tw - 1.0, w: 1.0, ..l.back };
                    tooltip(self, &mut cv, tip, anchor, size, true);
                }
                Some(EHit::Top(t)) => {
                    if let (Some(i), Some(tv)) = (v.tops.iter().position(|x| x.kind == t), v.tops.iter().find(|x| x.kind == t)) {
                        if !tv.tip.is_empty() {
                            tooltip(self, &mut cv, &tv.tip, l.tops[i], size, true);
                        }
                    }
                }
                Some(EHit::Browse) => tooltip(self, &mut cv, "Choose a different folder", l.browse, size, false),
                _ => {}
            }
        }

        // ---- popup
        if let (Some((rect, placed)), Some(p)) = (&l.popup, &v.popup) {
            cv.fill(Rect { x: 0.0, y: 0.0, w: size.0, h: size.1 }, 0.0, (0, 0, 0), 0.32);
            cv.fill(Rect { x: rect.x + 2.0, y: rect.y + 4.0, ..*rect }, 10.0, (0, 0, 0), 0.4);
            cv.fill(*rect, 10.0, MENU, 1.0);
            cv.stroke(*rect, 10.0, MENU_BORDER, 1.0, 1.0);
            if !p.title.is_empty() {
                self.etext(&mut cv, &p.title, rect.x + 16.0, rect.y + 10.0, 14.0, FOREGROUND, *rect);
            }
            let clip = *rect;
            for (pl, it) in placed.iter().zip(&p.items) {
                self.draw_item(&mut cv, it, pl, hover, clip);
            }
        }

        // ---- toast
        if !v.toast.is_empty() {
            let tw = self.ewidth(&v.toast, 14.0, s);
            let r = Rect { x: (size.0 - tw - 32.0) / 2.0, y: fy - 50.0, w: tw + 32.0, h: 32.0 };
            cv.fill(r, 16.0, TOOLBAR_BG, 0.97);
            cv.stroke(r, 16.0, MENU_BORDER, 1.0, 1.0);
            self.centred(&mut cv, &v.toast, Rect { x: r.x + 16.0, w: tw + 4.0, ..r }, 14.0, FOREGROUND);
        }

        let mut bgra = cv.pm.take();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Frame { width: wpx, height: hpx, bgra }
    }
}

impl Rect {
    /// This rectangle cut down to `other` (itself if they do not overlap).
    fn intersect_or(self, other: Rect) -> Rect {
        super::intersect(self, other).unwrap_or(self)
    }
}

/// A grey chequer pattern (what shows through transparent parts), drawn straight into the pixels.
fn checker(cv: &mut Canvas, r: Rect, radius: f32, tile: f32) {
    let s = cv.s;
    let (x0, y0) = ((r.x * s).round() as i32, (r.y * s).round() as i32);
    let (x1, y1) = (((r.x + r.w) * s).round() as i32, ((r.y + r.h) * s).round() as i32);
    let t = ((tile * s).round() as i32).max(2);
    let (pw, ph) = (cv.pm.width() as i32, cv.pm.height() as i32);
    let rr = radius * s;
    let data = cv.pm.data_mut();
    for y in y0.max(0)..y1.min(ph) {
        for x in x0.max(0)..x1.min(pw) {
            if rr > 0.5 {
                // cut the corners
                let cx = if (x - x0) < rr as i32 { rr - (x - x0) as f32 } else if (x1 - 1 - x) < rr as i32 { rr - (x1 - 1 - x) as f32 } else { 0.0 };
                let cy = if (y - y0) < rr as i32 { rr - (y - y0) as f32 } else if (y1 - 1 - y) < rr as i32 { rr - (y1 - 1 - y) as f32 } else { 0.0 };
                if cx > 0.0 && cy > 0.0 && cx * cx + cy * cy > rr * rr {
                    continue;
                }
            }
            let dark = ((x - x0) / t + (y - y0) / t) % 2 == 0;
            let c: u8 = if dark { 58 } else { 78 };
            let i = ((y * pw + x) * 4) as usize;
            data[i] = c;
            data[i + 1] = c;
            data[i + 2] = c + 4;
            data[i + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str) -> FieldView {
        FieldView { text: text.into(), ..FieldView::default() }
    }

    fn view() -> EditorView {
        let mut v = EditorView {
            title: "Advanced Image Paste".into(),
            subtitle: "800 x 600 px".into(),
            tops: vec![
                TopView { kind: Top::Undo, enabled: false, tip: "Undo".into() },
                TopView { kind: Top::Redo, enabled: false, tip: "Redo".into() },
                TopView { kind: Top::Reset, enabled: true, tip: "Reset all".into() },
                TopView { kind: Top::Presets, enabled: true, tip: String::new() },
            ],
            dims: (800, 600),
            name: field("Pasteroll image"),
            ext: ".png".into(),
            folder: field("C:\\Users\\me\\Desktop"),
            paste_label: "Paste Image".into(),
            paste_enabled: true,
            ..EditorView::default()
        };
        v.items = vec![
            Item { id: 1, kind: ItemKind::Header { title: "Outline".into(), summary: "4 px".into(), open: true, switch: Some(true) } },
            Item { id: 2, kind: ItemKind::Slider { label: "Thickness".into(), field: field("4"), frac: 0.2 } },
            Item { id: 3, kind: ItemKind::Color { label: "Colour".into(), color: [0, 0, 0, 255], open: false } },
            Item { id: 4, kind: ItemKind::Chips { label: "Format".into(), options: vec!["PNG".into(), "JPG".into(), "WebP".into(), "BMP".into()], selected: Some(0) } },
            Item { id: 5, kind: ItemKind::Header { title: "Shadow".into(), summary: "Off".into(), open: false, switch: Some(false) } },
        ];
        v
    }

    const SIZE: (f32, f32) = (1440.0, 810.0);

    #[test]
    fn the_regions_do_not_overlap_and_stay_inside_the_window() {
        let l = editor_layout(&view(), SIZE);
        assert!(l.panel.x > l.preview.x + l.preview.w - 1.0, "preview and panel are side by side");
        for r in [l.back, l.name, l.ext, l.folder, l.browse, l.cancel, l.paste, l.panel, l.preview] {
            assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= SIZE.0 && r.y + r.h <= SIZE.1, "{r:?}");
        }
        assert!(l.name.x + l.name.w < l.ext.x && l.ext.x + l.ext.w < l.folder.x && l.folder.x + l.folder.w < l.browse.x);
        assert!(l.browse.x + l.browse.w < l.cancel.x && l.cancel.x + l.cancel.w < l.paste.x);
        let img = l.image.unwrap();
        assert!(img.x >= l.preview.x && img.y >= l.preview.y && img.x + img.w <= l.preview.x + l.preview.w && img.y + img.h <= l.preview.y + l.preview.h);
        // 4:3 stays 4:3
        assert!((img.w / img.h - 800.0 / 600.0).abs() < 0.01);
        // the top buttons do not run into the title
        assert!(l.tops.iter().all(|t| t.x > 300.0));
    }

    #[test]
    fn small_pictures_are_not_blown_up_past_four_times() {
        let mut v = view();
        v.dims = (40, 30);
        let img = editor_layout(&v, SIZE).image.unwrap();
        assert!((img.w - 160.0).abs() < 0.1, "{img:?}");
    }

    #[test]
    fn hits_find_the_slider_chips_switch_and_buttons() {
        let v = view();
        let l = editor_layout(&v, SIZE);
        let at = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let hit = |r: Rect| {
            let (x, y) = at(r);
            editor_hit(&v, SIZE, 1.0, x, y)
        };
        assert_eq!(hit(l.back), EHit::Back);
        assert_eq!(hit(l.tops[2]), EHit::Top(Top::Reset));
        assert_eq!(hit(l.paste), EHit::Paste);
        assert_eq!(hit(l.cancel), EHit::Cancel);
        assert_eq!(hit(l.browse), EHit::Browse);
        assert_eq!(hit(l.name), EHit::Name);
        assert_eq!(hit(l.folder), EHit::Folder);
        // the outline header's switch is part 1, its title part 0
        assert_eq!(hit(l.placed[0].parts[1]), EHit::Item { id: 1, part: 1, frac: 0.0 });
        assert_eq!(hit(l.placed[0].parts[0]), EHit::Item { id: 1, part: 0, frac: 0.0 });
        // the slider: track and value box
        assert!(matches!(hit(l.placed[1].parts[0]), EHit::Item { id: 2, part: 0, .. }));
        assert_eq!(hit(l.placed[1].parts[1]), EHit::Item { id: 2, part: 1, frac: 0.0 });
        // the track's two ends map to 0 and 1
        let t = l.placed[1].parts[0];
        let y = t.y + t.h / 2.0;
        assert_eq!(editor_hit(&v, SIZE, 1.0, t.x + 1.0, y), EHit::Item { id: 2, part: 0, frac: 0.0 });
        assert_eq!(editor_hit(&v, SIZE, 1.0, t.x + t.w - 1.0, y), EHit::Item { id: 2, part: 0, frac: 1.0 });
        // chips
        let chips = &l.placed[3];
        for i in 0..4 {
            assert_eq!(hit(chips.parts[i]), EHit::Item { id: 4, part: i as u16, frac: 0.0 });
        }
        assert_eq!(hit(l.placed[2].parts[0]), EHit::Item { id: 3, part: 0, frac: 0.0 });
        // empty space in the preview is the image or nothing
        assert_eq!(editor_hit(&v, SIZE, 1.0, 5.0, 400.0), EHit::Nothing);
        let img = l.image.unwrap();
        assert_eq!(hit(img), EHit::Image);
    }

    #[test]
    fn hit_positions_follow_the_display_scale() {
        let v = view();
        let l = editor_layout(&v, SIZE);
        let r = l.paste;
        assert_eq!(editor_hit(&v, SIZE, 1.5, (r.x + 5.0) * 1.5, (r.y + 5.0) * 1.5), EHit::Paste);
    }

    #[test]
    fn scrolling_moves_the_items_and_clips_the_hits_to_the_panel() {
        let mut v = view();
        for i in 0..40 {
            v.items.push(Item { id: 100 + i, kind: ItemKind::Check { label: format!("Option {i}"), on: false } });
        }
        let top = editor_layout(&v, SIZE);
        assert!(top.max_scroll > 100.0);
        let first_y = top.placed[0].rect.y;
        v.scroll = 120.0;
        let moved = editor_layout(&v, SIZE);
        assert!((first_y - moved.placed[0].rect.y - 120.0).abs() < 0.01);
        // an item scrolled up under the header answers nothing there
        let hidden = moved.placed[0].rect;
        assert!(hidden.y + hidden.h < moved.panel.y);
        assert_eq!(editor_hit(&v, SIZE, 1.0, hidden.x + 20.0, hidden.y + 5.0), EHit::Nothing);
    }

    #[test]
    fn the_popup_sits_by_its_anchor_inside_the_window_and_is_modal() {
        let mut v = view();
        v.popup = Some(PopupView {
            anchor: PopupAnchor::Top(Top::Presets),
            title: "Presets".into(),
            w: 260.0,
            items: vec![
                Item { id: 10, kind: ItemKind::MenuRow { label: "Blog header".into(), can_delete: true, on: false } },
                Item { id: 11, kind: ItemKind::MenuRow { label: "Save current settings...".into(), can_delete: false, on: false } },
            ],
        });
        let l = editor_layout(&v, SIZE);
        let (rect, items) = l.popup.clone().unwrap();
        assert!(rect.x >= 8.0 && rect.x + rect.w <= SIZE.0 - 8.0 && rect.y + rect.h <= SIZE.1);
        assert!((rect.x + rect.w - (l.tops[3].x + l.tops[3].w)).abs() < 0.01, "right edge under the Presets button");
        let row = items[0].parts[0];
        let del = items[0].parts[1];
        assert_eq!(editor_hit(&v, SIZE, 1.0, row.x + 20.0, row.y + 10.0), EHit::Item { id: 10, part: 0, frac: 0.0 });
        assert_eq!(editor_hit(&v, SIZE, 1.0, del.x + 10.0, del.y + 10.0), EHit::Item { id: 10, part: 1, frac: 0.0 });
        assert_eq!(editor_hit(&v, SIZE, 1.0, rect.x + 3.0, rect.y + 3.0), EHit::Popup);
        // the window behind answers nothing while it is up
        let p = l.paste;
        assert_eq!(editor_hit(&v, SIZE, 1.0, p.x + 5.0, p.y + 5.0), EHit::OutsidePopup);
    }

    #[test]
    fn crop_handles_are_found_corners_first() {
        let c = Rect { x: 100.0, y: 100.0, w: 200.0, h: 120.0 };
        assert_eq!(crop_part_at(c, 101.0, 99.0), Some(CropPart::NW));
        assert_eq!(crop_part_at(c, 300.0, 220.0), Some(CropPart::SE));
        assert_eq!(crop_part_at(c, 200.0, 100.0), Some(CropPart::N));
        assert_eq!(crop_part_at(c, 300.0, 160.0), Some(CropPart::E));
        assert_eq!(crop_part_at(c, 150.0, 220.0), Some(CropPart::S), "anywhere along an edge");
        assert_eq!(crop_part_at(c, 200.0, 160.0), Some(CropPart::Move));
        assert_eq!(crop_part_at(c, 10.0, 10.0), None);
    }

    #[test]
    fn a_crop_rectangle_maps_onto_the_picture() {
        let mut v = view();
        v.crop = Some(CropView { rect: (100.0, 50.0, 400.0, 300.0), label: "400 x 300".into() });
        let l = editor_layout(&v, SIZE);
        let (img, c) = (l.image.unwrap(), l.crop.unwrap());
        let k = img.w / 800.0;
        assert!((c.x - (img.x + 100.0 * k)).abs() < 0.01 && (c.w - 400.0 * k).abs() < 0.01);
        assert!((c.y - (img.y + 50.0 * k)).abs() < 0.01 && (c.h - 300.0 * k).abs() < 0.01);
        assert_eq!(editor_hit(&v, SIZE, 1.0, c.x + c.w / 2.0, c.y + c.h / 2.0), EHit::Crop(CropPart::Move));
    }

    #[test]
    fn the_editor_draws_a_frame_of_the_right_size() {
        let mut v = view();
        let mut px = vec![0u8; 80 * 60 * 4];
        for (i, p) in px.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 80) as u8 * 3, 90, 200, if i % 80 < 10 { 0 } else { 255 }]);
        }
        v.preview = Some(Arc::new(Rgba { width: 80, height: 60, pixels: px }));
        v.dims = (80, 60);
        v.crop = Some(CropView { rect: (10.0, 10.0, 40.0, 30.0), label: "40 x 30".into() });
        v.toast = "Saved".into();
        v.busy = true;
        let mut r = Renderer::new();
        let f = r.render_editor(&v, (960.0, 600.0), 1.25);
        assert_eq!((f.width, f.height), (1200, 750));
        assert_eq!(f.bgra.len(), 1200 * 750 * 4);
        // with a popup too
        v.popup = Some(PopupView {
            anchor: PopupAnchor::Item(3),
            title: "Colour".into(),
            w: 250.0,
            items: vec![
                Item { id: 20, kind: ItemKind::Palette { colors: vec![[0, 0, 0, 0], [255, 0, 0, 255], [0, 255, 0, 255]], selected: Some(1) } },
                Item { id: 21, kind: ItemKind::Text { label: "Hex".into(), field: FieldView { text: "#FF0000".into(), focused: true, sel_all: true, ..FieldView::default() } } },
            ],
        });
        let g = r.render_editor(&v, (960.0, 600.0), 1.0);
        assert_eq!(g.width, 960);
    }

    /// Writes a sample frame to the file named by PASTEROLL_DUMP (look at it by eye).
    #[test]
    #[ignore]
    fn dump_a_sample_frame() {
        let Ok(path) = std::env::var("PASTEROLL_DUMP") else { return };
        let mut v = view();
        let mut px = vec![0u8; 400 * 260 * 4];
        for (i, p) in px.chunks_exact_mut(4).enumerate() {
            let (x, y) = ((i % 400) as u32, (i / 400) as u32);
            p.copy_from_slice(&[(x * 255 / 400) as u8, (y * 255 / 260) as u8, 140, 255]);
        }
        v.preview = Some(Arc::new(Rgba { width: 400, height: 260, pixels: px }));
        v.dims = (400, 260);
        v.items.insert(2, Item { id: 30, kind: ItemKind::Fields { label: "Size".into(), fields: vec![FieldView { label: "W".into(), text: "400".into(), suffix: "px".into(), ..FieldView::default() }, FieldView { label: "H".into(), text: "260".into(), suffix: "px".into(), focused: true, ..FieldView::default() }] } });
        v.items.push(Item { id: 31, kind: ItemKind::Buttons { buttons: vec![ButtonView { label: "Left".into(), icon: Some(Icon::RotateLeft), enabled: true, primary: false, on: false }, ButtonView { label: "Right".into(), icon: Some(Icon::RotateRight), enabled: true, primary: false, on: false }, ButtonView { label: "Flip".into(), icon: Some(Icon::FlipH), enabled: true, primary: false, on: true }, ButtonView { label: "Flip".into(), icon: Some(Icon::FlipV), enabled: true, primary: false, on: false }] } });
        v.items.push(Item { id: 32, kind: ItemKind::Header { title: "Crop".into(), summary: String::new(), open: false, switch: None } });
        v.items.push(Item { id: 33, kind: ItemKind::Note { text: "Hold Shift to keep the shape while dragging a corner.".into() } });
        v.note = "Result: 408 x 268 px".into();
        v.hover = Some(EHit::Item { id: 5, part: 0, frac: 0.0 });
        let mut r = Renderer::new();
        let f = r.render_editor(&v, (1200.0, 700.0), 1.0);
        let mut rgba = pasteroll_core::image::Rgba { width: f.width, height: f.height, pixels: f.bgra.clone() };
        for p in rgba.pixels.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        std::fs::write(&path, pasteroll_core::image::encode_png(&rgba).unwrap()).unwrap();
    }
}
