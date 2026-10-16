//! Advanced Image Paste: the image editor that opens, instead of an immediate save, when an image
//! is clicked onto the desktop with the configured modifier held.
//!
//! - The card grows into the editor exactly as it grows into the expanded viewer, and the carousel
//!   stays behind it, dimmed and inactive. Cancel shrinks it back and the carousel carries on;
//!   Paste Image saves the result and ends the session.
//! - Everything is non-destructive: the clip is only read. The adjustments are plain data
//!   ([`EditParams`]), the live preview is that data applied to a reduced copy on a worker thread,
//!   and the saved file is the same data applied to the whole picture on another.
//! - Undo and redo walk a list of earlier parameter sets; presets are named parameter sets kept by
//!   the platform in a small file.

use super::viewer::{ease_in_out, ease_out, home_rect, lerp, lerp_rect, Anim, AnimKind, ANIM_MAX_PIXELS, CLOSE_MS, DIM, OPEN_MS};
use super::*;
use crate::platform::SaveChoice;
use pasteroll_core::imgedit::{self, BgMode, Color, CropRect, EditParams, Format, PngLevel, Presets, ResizeMode};
use pasteroll_render::{
    editor_hit, editor_layout, ButtonView, Col, CropPart, CropView, EHit, EditorView, FieldView, Icon, Item, ItemKind,
    Frame, Morph, PopupAnchor, PopupView, Top, TopView,
};
use std::path::{Path, PathBuf};

const TOAST_MS: u64 = 2_400;
/// The smallest crop, in pixels.
const MIN_CROP: f32 = 8.0;
const NAME_MAX: usize = 120;
const FOLDER_MAX: usize = 400;
const HISTORY_MAX: usize = 100;
/// The editor is redrawn at most this often while the pointer drags something.
const PRESENT_EVERY_MS: u64 = 14;

type RectPx = (i32, i32, u32, u32);

// ---------------------------------------------------------------- messages from the workers

pub(crate) enum EditMsg {
    Loaded { id: i64, src: Arc<core_image::Rgba>, proxy: Arc<core_image::Rgba>, k: f32 },
    LoadFailed { id: i64 },
    Preview { gen: u64, img: Arc<core_image::Rgba>, dims: (u32, u32) },
    Exported { result: Result<PathBuf, String> },
}

struct Job {
    gen: u64,
    proxy: Arc<core_image::Rgba>,
    params: EditParams,
    dims: (u32, u32),
}

// ---------------------------------------------------------------- the editor's state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sec {
    Crop,
    Resize,
    Rotate,
    Corners,
    Outline,
    Shadow,
    Background,
    Padding,
    Output,
}

const SECS: [Sec; 9] =
    [Sec::Crop, Sec::Resize, Sec::Rotate, Sec::Corners, Sec::Outline, Sec::Shadow, Sec::Background, Sec::Padding, Sec::Output];

impl Sec {
    fn title(self) -> &'static str {
        match self {
            Sec::Crop => "Crop",
            Sec::Resize => "Resize",
            Sec::Rotate => "Rotate and flip",
            Sec::Corners => "Rounded corners",
            Sec::Outline => "Outline",
            Sec::Shadow => "Drop shadow",
            Sec::Background => "Background",
            Sec::Padding => "Padding",
            Sec::Output => "Format and quality",
        }
    }

    fn index(self) -> u32 {
        SECS.iter().position(|s| *s == self).unwrap_or(0) as u32
    }
}

/// Ids of panel items: the section, then the item within it.
fn iid(sec: Sec, n: u32) -> u32 {
    (sec.index() + 1) * 100 + n
}

fn split_id(id: u32) -> Option<(Sec, u32)> {
    let s = (id / 100).checked_sub(1)? as usize;
    Some((*SECS.get(s)?, id % 100))
}

/// The ids of the popups' items.
const POP_PALETTE: u32 = 1000;
const POP_HEX: u32 = 1001;
const POP_PRESET_BASE: u32 = 1010;
const POP_SAVE_AS: u32 = 1060;
const POP_NAME: u32 = 1061;
const POP_NAME_BUTTONS: u32 = 1062;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Num {
    ResizeW,
    ResizeH,
    Angle,
    Radius,
    OutlineW,
    ShadowOpacity,
    ShadowBlur,
    ShadowDist,
    ShadowAngle,
    Tolerance,
    PadAll,
    PadTop,
    PadRight,
    PadBottom,
    PadLeft,
    Quality,
    CropW,
    CropH,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ColorTarget {
    Outline,
    Shadow,
    Fill,
    Key,
    Padding,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Num(Num),
    Name,
    Folder,
    Hex,
    PresetName,
}

/// The text box being typed into.
struct Focus {
    field: Field,
    text: String,
    sel_all: bool,
}

enum Popup {
    Color { target: ColorTarget, anchor: u32 },
    Presets { naming: bool },
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Slider { id: u32, num: Num },
    Crop { part: CropPart, start: (f32, f32), rect: (f32, f32, f32, f32) },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Then {
    /// Back to the carousel.
    Carousel,
    /// The picture was saved: the whole session is over.
    EndSession,
}

pub(crate) struct Editor {
    id: i64,
    src_dims: (u32, u32),
    /// The whole picture (once decoded) and the reduced copy the preview works on, with its factor.
    src: Option<Arc<core_image::Rgba>>,
    proxy: Option<(Arc<core_image::Rgba>, f32)>,
    failed: bool,
    params: EditParams,
    history: Vec<EditParams>,
    pos: usize,
    preview: Option<Arc<core_image::Rgba>>,
    preview_dims: (u32, u32),
    gen: u64,
    pending: bool,
    jobs: std::sync::mpsc::Sender<Job>,
    open_sec: Option<Sec>,
    scroll: f32,
    focus: Option<Focus>,
    popup: Option<Popup>,
    drag: Option<Drag>,
    hover: Option<EHit>,
    pad_linked: bool,
    crop_aspect: Option<(f32, f32)>,
    name: String,
    folder: String,
    presets: Presets,
    toast: Option<(String, u64)>,
    saving: bool,
    // window and animation
    pub(crate) rect: RectPx,
    lifted: Frame,
    dim: f32,
    anim: Option<Anim>,
    last_frame: Option<Frame>,
    last_present_ms: u64,
    then: Then,
}

impl Editor {
    fn size_dip(&self, scale: f32) -> (f32, f32) {
        let s = scale.max(0.1);
        (self.rect.2 as f32 / s, self.rect.3 as f32 / s)
    }

    fn animating(&self) -> bool {
        self.anim.is_some()
    }

    fn toast(&mut self, text: impl Into<String>, now: u64) {
        self.toast = Some((text.into(), now + TOAST_MS));
    }

    fn plan(&self) -> imgedit::Plan {
        imgedit::plan(self.src_dims.0, self.src_dims.1, &self.params)
    }

    fn out_dims(&self) -> (u32, u32) {
        self.plan().out
    }

    fn can_undo(&self) -> bool {
        self.pos > 0 || self.params != self.history[self.pos]
    }

    fn can_redo(&self) -> bool {
        self.pos + 1 < self.history.len() && self.params == self.history[self.pos]
    }

    /// Remembers the present state as a step to go back to (if it is a new state).
    fn checkpoint(&mut self) {
        if self.params == self.history[self.pos] {
            return;
        }
        self.history.truncate(self.pos + 1);
        self.history.push(self.params.clone());
        if self.history.len() > HISTORY_MAX {
            self.history.remove(0);
        }
        self.pos = self.history.len() - 1;
    }

    fn crop_mode(&self) -> bool {
        self.open_sec == Some(Sec::Crop)
    }

    /// The crop rectangle as it stands: the whole picture when nothing is cropped.
    fn crop_rect(&self) -> (f32, f32, f32, f32) {
        match self.params.crop {
            Some(c) => (c.x, c.y, c.w, c.h),
            None => (0.0, 0.0, self.src_dims.0 as f32, self.src_dims.1 as f32),
        }
    }

    fn set_crop(&mut self, r: (f32, f32, f32, f32)) {
        let (w, h) = (self.src_dims.0 as f32, self.src_dims.1 as f32);
        // whole pixels, inside the picture
        let x = r.0.round().clamp(0.0, w - 1.0);
        let y = r.1.round().clamp(0.0, h - 1.0);
        let r = (x, y, r.2.round().clamp(1.0, w - x), r.3.round().clamp(1.0, h - y));
        let whole = r.0 <= 0.5 && r.1 <= 0.5 && (r.2 - w).abs() <= 0.5 && (r.3 - h).abs() <= 0.5;
        self.params.crop = (!whole).then_some(CropRect { x: r.0, y: r.1, w: r.2, h: r.3 });
    }

    /// The most the corner radius can usefully be: half the shorter side of the resized picture.
    fn radius_max(&self) -> f32 {
        let r = self.plan().resized;
        (r.0.min(r.1) as f32 / 2.0).max(1.0)
    }

    fn num_get(&self, n: Num) -> f32 {
        let p = &self.params;
        match n {
            Num::ResizeW => match p.resize_mode {
                ResizeMode::Pixels => self.plan().resized.0 as f32,
                _ => p.resize_w,
            },
            Num::ResizeH => match (p.resize_mode, p.keep_aspect) {
                (ResizeMode::Pixels, _) => self.plan().resized.1 as f32,
                (_, true) => p.resize_w,
                _ => p.resize_h,
            },
            Num::Angle => p.angle,
            Num::Radius => p.radius.min(self.radius_max()),
            Num::OutlineW => p.outline_width,
            Num::ShadowOpacity => p.shadow_opacity,
            Num::ShadowBlur => p.shadow_blur,
            Num::ShadowDist => p.shadow_distance,
            Num::ShadowAngle => p.shadow_angle.rem_euclid(360.0),
            Num::Tolerance => p.bg_tolerance,
            Num::PadAll | Num::PadTop => p.pad[0],
            Num::PadRight => p.pad[1],
            Num::PadBottom => p.pad[2],
            Num::PadLeft => p.pad[3],
            Num::Quality => f32::from(p.quality),
            Num::CropW => self.crop_rect().2,
            Num::CropH => self.crop_rect().3,
        }
    }

    fn num_set(&mut self, n: Num, v: f32) {
        let rotated = self.plan().rotated;
        let src = self.src_dims;
        let p = &mut self.params;
        match n {
            Num::ResizeW | Num::ResizeH => {
                let w_axis = n == Num::ResizeW;
                if p.resize_mode == ResizeMode::Pixels && p.keep_aspect && !w_axis {
                    // the width that gives the typed height
                    p.resize_w = v * rotated.0 as f32 / rotated.1.max(1) as f32;
                } else if w_axis || p.keep_aspect {
                    p.resize_w = v;
                    if !p.keep_aspect && !w_axis {
                        p.resize_h = v;
                    }
                } else {
                    p.resize_h = v;
                }
                if !p.keep_aspect {
                    // both sides are independent: make sure the other one holds a real value
                    if w_axis && p.resize_h < 1.0 {
                        p.resize_h = 1.0;
                    }
                }
            }
            Num::Angle => p.angle = v,
            Num::Radius => p.radius = v,
            Num::OutlineW => p.outline_width = v,
            Num::ShadowOpacity => p.shadow_opacity = v,
            Num::ShadowBlur => p.shadow_blur = v,
            Num::ShadowDist => p.shadow_distance = v,
            Num::ShadowAngle => p.shadow_angle = v,
            Num::Tolerance => p.bg_tolerance = v,
            Num::PadAll => p.pad = [v; 4],
            Num::PadTop => p.pad[0] = v,
            Num::PadRight => p.pad[1] = v,
            Num::PadBottom => p.pad[2] = v,
            Num::PadLeft => p.pad[3] = v,
            Num::Quality => p.quality = v.round().clamp(1.0, 100.0) as u8,
            Num::CropW | Num::CropH => {
                let (x, y, w, h) = match p.crop {
                    Some(c) => (c.x, c.y, c.w, c.h),
                    None => (0.0, 0.0, src.0 as f32, src.1 as f32),
                };
                let (sw, sh) = (src.0 as f32, src.1 as f32);
                let (mut nw, mut nh) = if n == Num::CropW { (v, h) } else { (w, v) };
                if let Some((aw, ah)) = self.crop_aspect {
                    let a = aw / ah;
                    if n == Num::CropW {
                        nh = nw / a;
                    } else {
                        nw = nh * a;
                    }
                }
                nw = nw.clamp(MIN_CROP, sw);
                nh = nh.clamp(MIN_CROP, sh);
                let r = (x.min(sw - nw), y.min(sh - nh), nw, nh);
                self.set_crop(r);
                return;
            }
        }
        p.sanitize();
    }

    fn color_get(&self, t: ColorTarget) -> Col {
        let p = &self.params;
        match t {
            ColorTarget::Outline => p.outline_color.0,
            ColorTarget::Shadow => p.shadow_color.0,
            ColorTarget::Fill => p.bg_color.0,
            ColorTarget::Key => p.bg_key.map_or([255, 255, 255, 255], |c| c.0),
            ColorTarget::Padding => p.pad_color.0,
        }
    }

    fn color_set(&mut self, t: ColorTarget, c: Col) {
        let p = &mut self.params;
        match t {
            ColorTarget::Outline => p.outline_color = Color(c),
            ColorTarget::Shadow => p.shadow_color = Color(c),
            ColorTarget::Fill => p.bg_color = Color(c),
            ColorTarget::Key => p.bg_key = Some(Color([c[0], c[1], c[2], 255])),
            ColorTarget::Padding => p.pad_color = Color(c),
        }
    }

    /// Can this colour be see-through?
    fn color_allows_none(t: ColorTarget) -> bool {
        matches!(t, ColorTarget::Padding | ColorTarget::Fill | ColorTarget::Outline)
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        // the picture is clip content
        if let Some(f) = self.focus.as_mut() {
            use zeroize::Zeroize;
            f.text.zeroize();
        }
    }
}

// ---------------------------------------------------------------- small helpers

fn fmt_num(v: f32) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// A character a file name cannot hold.
fn bad_in_name(c: char) -> bool {
    c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// The file name (without extension) to use for what the person typed: no characters Windows
/// refuses, no trailing dots or spaces, not a reserved device name, not empty.
fn clean_file_stem(typed: &str, ext: &str, fallback: &str) -> String {
    let mut s: String = typed.chars().filter(|c| !bad_in_name(*c)).collect();
    // "photo.png" typed with PNG chosen is just "photo"
    let dotted = format!(".{ext}");
    if s.to_ascii_lowercase().ends_with(&dotted) {
        s.truncate(s.len() - dotted.len());
    }
    let s = s.trim().trim_end_matches(['.', ' ']).trim_start().to_owned();
    let s: String = s.chars().take(NAME_MAX).collect();
    if s.is_empty() {
        return fallback.to_owned();
    }
    let upper = s.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    let reserved = matches!(base, "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4 && (base.starts_with("COM") || base.starts_with("LPT")) && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("{s}_")
    } else {
        s
    }
}

/// Moves and resizes a crop rectangle `(x, y, w, h)` as the pointer moved by `(dx, dy)` since the
/// drag began. `aspect` (w / h) keeps the shape; the rectangle stays inside `bounds`.
fn crop_drag(
    start: (f32, f32, f32, f32),
    part: CropPart,
    dx: f32,
    dy: f32,
    bounds: (f32, f32),
    aspect: Option<f32>,
) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = start;
    let (bw, bh) = bounds;
    if part == CropPart::Move {
        return ((x + dx).clamp(0.0, bw - w), (y + dy).clamp(0.0, bh - h), w, h);
    }
    let (mut l, mut t, mut r, mut b) = (x, y, x + w, y + h);
    let moves_l = matches!(part, CropPart::W | CropPart::NW | CropPart::SW);
    let moves_r = matches!(part, CropPart::E | CropPart::NE | CropPart::SE);
    let moves_t = matches!(part, CropPart::N | CropPart::NW | CropPart::NE);
    let moves_b = matches!(part, CropPart::S | CropPart::SW | CropPart::SE);
    if moves_l {
        l = (l + dx).clamp(0.0, r - MIN_CROP);
    }
    if moves_r {
        r = (r + dx).clamp(l + MIN_CROP, bw);
    }
    if moves_t {
        t = (t + dy).clamp(0.0, b - MIN_CROP);
    }
    if moves_b {
        b = (b + dy).clamp(t + MIN_CROP, bh);
    }
    let Some(a) = aspect.filter(|a| *a > 0.0) else {
        return (l, t, r - l, b - t);
    };
    let corner = (moves_l || moves_r) && (moves_t || moves_b);
    let (mut nw, mut nh) = (r - l, b - t);
    if corner {
        // the larger of the two asked-for sizes decides
        if nw / a >= nh {
            nh = nw / a;
        } else {
            nw = nh * a;
        }
        // the opposite corner stays; the room left on the side being dragged limits the size
        let room_w = if moves_l { x + w } else { bw - x };
        let room_h = if moves_t { y + h } else { bh - y };
        let k = (room_w / nw).min(room_h / nh).min(1.0);
        nw *= k;
        nh *= k;
        let nl = if moves_l { x + w - nw } else { x };
        let nt = if moves_t { y + h - nh } else { y };
        return (nl, nt, nw, nh);
    }
    if moves_l || moves_r {
        nh = nw / a;
        // grow about the middle, as far as the picture allows
        let cy = y + h / 2.0;
        let max_h = 2.0 * cy.min(bh - cy);
        if nh > max_h {
            nh = max_h;
            nw = nh * a;
        }
        let nl = if moves_l { x + w - nw } else { x };
        return (nl, cy - nh / 2.0, nw, nh);
    }
    nw = nh * a;
    let cx = x + w / 2.0;
    let max_w = 2.0 * cx.min(bw - cx);
    if nw > max_w {
        nw = max_w;
        nh = nw / a;
    }
    let nt = if moves_t { y + h - nh } else { y };
    (cx - nw / 2.0, nt, nw, nh)
}

/// The largest rectangle of shape `aspect` inside `r`, centred on it.
fn fit_aspect(r: (f32, f32, f32, f32), aspect: f32, bounds: (f32, f32)) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = r;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (mut nw, mut nh) = if w / h > aspect { (h * aspect, h) } else { (w, w / aspect) };
    nw = nw.min(bounds.0);
    nh = nh.min(bounds.1);
    if nw / aspect > nh {
        nw = nh * aspect;
    } else {
        nh = nw / aspect;
    }
    ((cx - nw / 2.0).clamp(0.0, bounds.0 - nw), (cy - nh / 2.0).clamp(0.0, bounds.1 - nh), nw, nh)
}

const ASPECTS: [(&str, Option<(f32, f32)>); 7] = [
    ("Free", None),
    ("Original", Some((0.0, 0.0))),
    ("1:1", Some((1.0, 1.0))),
    ("4:3", Some((4.0, 3.0))),
    ("3:2", Some((3.0, 2.0))),
    ("16:9", Some((16.0, 9.0))),
    ("9:16", Some((9.0, 16.0))),
];

fn palette_colors(allow_none: bool) -> Vec<Col> {
    let mut v: Vec<Col> = Vec::new();
    if allow_none {
        v.push([0, 0, 0, 0]);
    }
    for hex in [
        "000000", "3A3A44", "7A7A86", "BDBDC8", "FFFFFF", "E86060", "F08A3C", "F2C744", "7CCB5B", "2FAE97", "3FB8D8", "4C7CF0",
        "7A5CF0", "C45CF0", "F05CA8", "8A5A3C", "FFD9D9", "FFE8CC", "FFF6BF", "DDF5D0", "CCF2EA", "D4ECFA", "DCE4FF", "E6DAFF",
    ] {
        if let Some(c) = Color::parse(hex) {
            v.push(c.0);
        }
    }
    v
}

// ---------------------------------------------------------------- building what is shown

impl Editor {
    fn field_for(&self, n: Num, label: &str, suffix: &str) -> FieldView {
        let focused = self.focus.as_ref().filter(|f| f.field == Field::Num(n));
        FieldView {
            label: label.to_owned(),
            text: focused.map_or_else(|| fmt_num(self.num_get(n)), |f| f.text.clone()),
            suffix: suffix.to_owned(),
            focused: focused.is_some(),
            sel_all: focused.is_some_and(|f| f.sel_all),
            placeholder: String::new(),
        }
    }

    fn slider(&self, id: u32, label: &str, n: Num, suffix: &str, min: f32, max: f32) -> Item {
        let v = self.num_get(n);
        let frac = if max > min { ((v - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
        Item { id, kind: ItemKind::Slider { label: label.to_owned(), field: self.field_for(n, "", suffix), frac } }
    }

    fn slider_range(&self, n: Num) -> (f32, f32) {
        match n {
            Num::Angle => (-180.0, 180.0),
            Num::Radius => (0.0, self.radius_max()),
            Num::OutlineW => (1.0, 100.0),
            Num::ShadowOpacity | Num::Tolerance | Num::Quality => (if n == Num::Quality { 1.0 } else { 0.0 }, 100.0),
            Num::ShadowBlur | Num::ShadowDist => (0.0, 100.0),
            Num::ShadowAngle => (0.0, 360.0),
            Num::PadAll | Num::PadTop | Num::PadRight | Num::PadBottom | Num::PadLeft => (0.0, 200.0),
            _ => (0.0, 100.0),
        }
    }

    fn color_item(&self, id: u32, label: &str, t: ColorTarget) -> Item {
        let open = matches!(&self.popup, Some(Popup::Color { anchor, .. }) if *anchor == id);
        Item { id, kind: ItemKind::Color { label: label.to_owned(), color: self.color_get(t), open } }
    }

    fn summary(&self, s: Sec) -> String {
        let p = &self.params;
        match s {
            Sec::Crop => match p.crop {
                Some(c) => format!("{} x {}", c.w.round(), c.h.round()),
                None => String::new(),
            },
            Sec::Resize => match p.resize_mode {
                ResizeMode::Off => String::new(),
                ResizeMode::Pixels => {
                    let r = self.plan().resized;
                    format!("{} x {}", r.0, r.1)
                }
                ResizeMode::Percent => format!("{}%", fmt_num(p.resize_w)),
            },
            Sec::Rotate => {
                let mut v = Vec::new();
                if p.angle != 0.0 {
                    v.push(format!("{}\u{b0}", fmt_num(p.angle)));
                }
                if p.flip_h {
                    v.push("flipped".to_owned());
                }
                if p.flip_v {
                    v.push("upside down".to_owned());
                }
                v.join(", ")
            }
            Sec::Corners => {
                if p.round_on {
                    format!("{} px", fmt_num(p.radius.min(self.radius_max())))
                } else {
                    String::new()
                }
            }
            Sec::Outline => {
                if p.outline_on {
                    format!("{} px", fmt_num(p.outline_width))
                } else {
                    String::new()
                }
            }
            Sec::Shadow => String::new(),
            Sec::Background => match p.bg_mode {
                BgMode::Keep => String::new(),
                BgMode::Fill => "Fill".into(),
                BgMode::Remove => "Remove".into(),
                BgMode::Replace => "Replace".into(),
            },
            Sec::Padding => {
                if p.pad_on {
                    if p.pad.iter().all(|v| *v == p.pad[0]) {
                        format!("{} px", fmt_num(p.pad[0]))
                    } else {
                        "custom".into()
                    }
                } else {
                    String::new()
                }
            }
            Sec::Output => p.format.label().to_owned(),
        }
    }

    fn switch_of(&self, s: Sec) -> Option<bool> {
        let p = &self.params;
        match s {
            Sec::Corners => Some(p.round_on),
            Sec::Outline => Some(p.outline_on),
            Sec::Shadow => Some(p.shadow_on),
            Sec::Padding => Some(p.pad_on),
            Sec::Background => Some(p.bg_mode != BgMode::Keep),
            _ => None,
        }
    }

    fn section_items(&self, s: Sec) -> Vec<Item> {
        let p = &self.params;
        let chips = |id: u32, label: &str, options: &[&str], selected: Option<usize>| Item {
            id,
            kind: ItemKind::Chips { label: label.to_owned(), options: options.iter().map(|s| (*s).to_owned()).collect(), selected },
        };
        let check = |id: u32, label: &str, on: bool| Item { id, kind: ItemKind::Check { label: label.to_owned(), on } };
        let note = |id: u32, text: &str| Item { id, kind: ItemKind::Note { text: text.to_owned() } };
        let button = |label: &str, icon: Option<Icon>, enabled: bool| ButtonView { label: label.to_owned(), icon, enabled, primary: false, on: false };
        let mut v = Vec::new();
        match s {
            Sec::Crop => {
                let sel = match self.crop_aspect {
                    None => Some(0),
                    Some(a) => {
                        let orig = self.src_dims.0 as f32 / self.src_dims.1.max(1) as f32;
                        ASPECTS.iter().position(|(_, asp)| match asp {
                            Some((0.0, _)) => (a.0 / a.1 - orig).abs() < 0.0005,
                            Some(x) => (x.0 / x.1 - a.0 / a.1).abs() < 0.0005,
                            None => false,
                        })
                    }
                };
                let names: Vec<&str> = ASPECTS.iter().map(|(n, _)| *n).collect();
                v.push(chips(iid(s, 1), "Shape", &names, sel));
                v.push(Item {
                    id: iid(s, 2),
                    kind: ItemKind::Fields {
                        label: "Size".into(),
                        fields: vec![self.field_for(Num::CropW, "W", "px"), self.field_for(Num::CropH, "H", "px")],
                    },
                });
                v.push(Item {
                    id: iid(s, 3),
                    kind: ItemKind::Buttons { buttons: vec![button("Whole picture", Some(Icon::Fit), p.crop.is_some())] },
                });
                v.push(note(iid(s, 4), "Drag the handles on the picture. Drag inside to move the frame."));
            }
            Sec::Resize => {
                let sel = match p.resize_mode {
                    ResizeMode::Off => 0,
                    ResizeMode::Pixels => 1,
                    ResizeMode::Percent => 2,
                };
                v.push(chips(iid(s, 1), "Size by", &["Original", "Pixels", "Percent"], Some(sel)));
                if p.resize_mode != ResizeMode::Off {
                    let unit = if p.resize_mode == ResizeMode::Percent { "%" } else { "px" };
                    v.push(Item {
                        id: iid(s, 2),
                        kind: ItemKind::Fields {
                            label: String::new(),
                            fields: vec![self.field_for(Num::ResizeW, "W", unit), self.field_for(Num::ResizeH, "H", unit)],
                        },
                    });
                    v.push(check(iid(s, 3), "Keep the proportions", p.keep_aspect));
                }
            }
            Sec::Rotate => {
                v.push(self.slider(iid(s, 1), "Angle", Num::Angle, "\u{b0}", -180.0, 180.0));
                v.push(Item {
                    id: iid(s, 2),
                    kind: ItemKind::Buttons {
                        buttons: vec![
                            button("Left", Some(Icon::RotateLeft), true),
                            button("Right", Some(Icon::RotateRight), true),
                            ButtonView { on: p.flip_h, ..button("Flip", Some(Icon::FlipH), true) },
                            ButtonView { on: p.flip_v, ..button("Flip", Some(Icon::FlipV), true) },
                        ],
                    },
                });
                v.push(note(iid(s, 3), "Left and Right turn by 90\u{b0}. The other two flip the picture sideways and upside down."));
            }
            Sec::Corners => {
                let (lo, hi) = self.slider_range(Num::Radius);
                v.push(self.slider(iid(s, 1), "Radius", Num::Radius, "px", lo, hi));
                v.push(Item {
                    id: iid(s, 2),
                    kind: ItemKind::Buttons { buttons: vec![button("Square", None, true), button("Fully round", None, true)] },
                });
            }
            Sec::Outline => {
                v.push(self.slider(iid(s, 1), "Thickness", Num::OutlineW, "px", 1.0, 100.0));
                v.push(self.color_item(iid(s, 2), "Colour", ColorTarget::Outline));
            }
            Sec::Shadow => {
                v.push(self.color_item(iid(s, 1), "Colour", ColorTarget::Shadow));
                v.push(self.slider(iid(s, 2), "Opacity", Num::ShadowOpacity, "%", 0.0, 100.0));
                v.push(self.slider(iid(s, 3), "Blur", Num::ShadowBlur, "px", 0.0, 100.0));
                v.push(self.slider(iid(s, 4), "Distance", Num::ShadowDist, "px", 0.0, 100.0));
                v.push(self.slider(iid(s, 5), "Direction", Num::ShadowAngle, "\u{b0}", 0.0, 360.0));
                v.push(note(iid(s, 6), "Direction 90\u{b0} is straight down, 0\u{b0} to the right."));
            }
            Sec::Background => {
                let sel = match p.bg_mode {
                    BgMode::Keep => 0,
                    BgMode::Fill => 1,
                    BgMode::Remove => 2,
                    BgMode::Replace => 3,
                };
                v.push(chips(iid(s, 1), "Background", &["Keep", "Fill", "Remove", "Replace"], Some(sel)));
                if matches!(p.bg_mode, BgMode::Fill | BgMode::Replace) {
                    v.push(self.color_item(iid(s, 2), "Colour", ColorTarget::Fill));
                }
                if matches!(p.bg_mode, BgMode::Remove | BgMode::Replace) {
                    v.push(check(iid(s, 3), "Use the colour of the corners", p.bg_key.is_none()));
                    if p.bg_key.is_some() {
                        v.push(self.color_item(iid(s, 4), "Remove", ColorTarget::Key));
                    }
                    v.push(self.slider(iid(s, 5), "Tolerance", Num::Tolerance, "%", 0.0, 100.0));
                }
                let text = match p.bg_mode {
                    BgMode::Keep => "The picture's own background stays.",
                    BgMode::Fill => "Fills see-through parts of the picture with the colour.",
                    BgMode::Remove => "Makes the background that touches the edges see-through. Works best on a plain colour.",
                    BgMode::Replace => "Removes the background, then puts the colour in its place.",
                };
                v.push(note(iid(s, 6), text));
            }
            Sec::Padding => {
                v.push(check(iid(s, 1), "Same on every side", self.pad_linked));
                if self.pad_linked {
                    v.push(self.slider(iid(s, 2), "Space", Num::PadAll, "px", 0.0, 200.0));
                } else {
                    v.push(Item {
                        id: iid(s, 2),
                        kind: ItemKind::Fields {
                            label: String::new(),
                            fields: vec![self.field_for(Num::PadTop, "Top", "px"), self.field_for(Num::PadBottom, "Bottom", "px")],
                        },
                    });
                    v.push(Item {
                        id: iid(s, 3),
                        kind: ItemKind::Fields {
                            label: String::new(),
                            fields: vec![self.field_for(Num::PadLeft, "Left", "px"), self.field_for(Num::PadRight, "Right", "px")],
                        },
                    });
                }
                v.push(self.color_item(iid(s, 4), "Colour", ColorTarget::Padding));
            }
            Sec::Output => {
                let fmts: Vec<&str> = Format::ALL.iter().map(|f| f.label()).collect();
                let sel = Format::ALL.iter().position(|f| *f == p.format);
                v.push(chips(iid(s, 1), "Format", &fmts, sel));
                match p.format {
                    Format::Jpeg => v.push(self.slider(iid(s, 2), "Quality", Num::Quality, "", 1.0, 100.0)),
                    Format::Png => {
                        let sel = match p.png_level {
                            PngLevel::Fast => 0,
                            PngLevel::Balanced => 1,
                            PngLevel::Smallest => 2,
                        };
                        v.push(chips(iid(s, 3), "Compression", &["Fast", "Balanced", "Smallest"], Some(sel)));
                    }
                    _ => {}
                }
                let text = match p.format {
                    Format::Png => "Lossless, keeps transparency.",
                    Format::Jpeg => "Smaller files, no transparency: see-through parts become white.",
                    Format::WebP => "Lossless, keeps transparency. Every pixel is kept exactly.",
                    Format::Bmp => "Large files, no transparency: see-through parts become white.",
                };
                v.push(note(iid(s, 4), text));
            }
        }
        v
    }

    fn popup_view(&self) -> Option<PopupView> {
        let popup = self.popup.as_ref()?;
        match popup {
            Popup::Color { target, anchor } => {
                let colors = palette_colors(Self::color_allows_none(*target));
                let cur = self.color_get(*target);
                let selected = colors.iter().position(|c| *c == cur);
                let hex = match self.focus.as_ref().filter(|f| f.field == Field::Hex) {
                    Some(f) => FieldView { text: f.text.clone(), focused: true, sel_all: f.sel_all, placeholder: "#RRGGBB".into(), ..FieldView::default() },
                    None => FieldView { text: Color(cur).hex(), placeholder: "#RRGGBB".into(), ..FieldView::default() },
                };
                Some(PopupView {
                    anchor: PopupAnchor::Item(*anchor),
                    title: "Colour".into(),
                    w: 268.0,
                    items: vec![
                        Item { id: POP_PALETTE, kind: ItemKind::Palette { colors, selected } },
                        Item { id: POP_HEX, kind: ItemKind::Text { label: "Hex".into(), field: hex } },
                        Item { id: POP_HEX + 1, kind: ItemKind::Note { text: "Type a code like #2FAE97 and press Enter.".into() } },
                    ],
                })
            }
            Popup::Presets { naming } => {
                let mut items = Vec::new();
                if *naming {
                    let name = match self.focus.as_ref().filter(|f| f.field == Field::PresetName) {
                        Some(f) => FieldView { text: f.text.clone(), focused: true, sel_all: f.sel_all, placeholder: "Preset name".into(), ..FieldView::default() },
                        None => FieldView { placeholder: "Preset name".into(), ..FieldView::default() },
                    };
                    items.push(Item { id: POP_NAME, kind: ItemKind::Text { label: "Name".into(), field: name } });
                    items.push(Item {
                        id: POP_NAME_BUTTONS,
                        kind: ItemKind::Buttons {
                            buttons: vec![
                                ButtonView { label: "Save preset".into(), icon: Some(Icon::Check), enabled: true, primary: true, on: false },
                                ButtonView { label: "Back".into(), icon: None, enabled: true, primary: false, on: false },
                            ],
                        },
                    });
                    items.push(Item {
                        id: POP_NAME + 5,
                        kind: ItemKind::Note { text: "Saves every setting except the crop. Using an existing name replaces that preset.".into() },
                    });
                } else {
                    if self.presets.items.is_empty() {
                        items.push(Item { id: POP_NAME + 6, kind: ItemKind::Note { text: "No presets yet. Set the picture up as you like it, then save the settings here to use them again.".into() } });
                    }
                    for (i, (name, _)) in self.presets.items.iter().enumerate() {
                        items.push(Item { id: POP_PRESET_BASE + i as u32, kind: ItemKind::MenuRow { label: name.clone(), can_delete: true, on: false } });
                    }
                    items.push(Item { id: POP_NAME + 7, kind: ItemKind::Separator });
                    items.push(Item { id: POP_SAVE_AS, kind: ItemKind::MenuRow { label: "Save current settings as a preset...".into(), can_delete: false, on: false } });
                }
                Some(PopupView { anchor: PopupAnchor::Top(Top::Presets), title: "Presets".into(), w: 330.0, items })
            }
        }
    }

    fn view(&self, scale: f32) -> EditorView {
        let _ = scale;
        let out = self.out_dims();
        let too_large = imgedit::too_large(out);
        let crop_mode = self.crop_mode();
        let mut items = Vec::new();
        for s in SECS {
            let open = self.open_sec == Some(s);
            items.push(Item {
                id: iid(s, 0),
                kind: ItemKind::Header { title: s.title().to_owned(), summary: self.summary(s), open, switch: self.switch_of(s) },
            });
            if open {
                items.extend(self.section_items(s));
                items.push(Item { id: iid(s, 99), kind: ItemKind::Gap(6.0) });
            }
        }
        let (preview, dims) = if crop_mode {
            (self.proxy.as_ref().map(|(p, _)| p.clone()).or_else(|| self.preview.clone()), self.src_dims)
        } else {
            (self.preview.clone(), self.preview_dims)
        };
        let crop = crop_mode.then(|| {
            let r = self.crop_rect();
            CropView { rect: r, label: format!("{} x {}", r.2.round(), r.3.round()) }
        });
        let ext = format!(".{}", self.params.format.ext());
        let name = match self.focus.as_ref().filter(|f| f.field == Field::Name) {
            Some(f) => FieldView { text: f.text.clone(), focused: true, sel_all: f.sel_all, ..FieldView::default() },
            None => FieldView { text: self.name.clone(), placeholder: "File name".into(), ..FieldView::default() },
        };
        let folder = match self.focus.as_ref().filter(|f| f.field == Field::Folder) {
            Some(f) => FieldView { text: f.text.clone(), focused: true, sel_all: f.sel_all, placeholder: "Desktop".into(), ..FieldView::default() },
            None => FieldView { text: self.folder.clone(), placeholder: "Desktop".into(), ..FieldView::default() },
        };
        let note = match (&too_large, crop_mode) {
            (Some(why), _) => why.clone(),
            (None, true) => "Showing the original while you crop".to_owned(),
            (None, false) => format!("Result: {} x {} px", out.0, out.1),
        };
        let tip = |s: &str| s.to_owned();
        EditorView {
            title: "Advanced Image Paste".into(),
            subtitle: format!("Original {} x {} px", self.src_dims.0, self.src_dims.1),
            tops: vec![
                TopView { kind: Top::Undo, enabled: self.can_undo(), tip: tip("Undo (Ctrl+Z)") },
                TopView { kind: Top::Redo, enabled: self.can_redo(), tip: tip("Redo (Ctrl+Y)") },
                TopView { kind: Top::Reset, enabled: self.params != EditParams::default(), tip: tip("Reset all adjustments") },
                TopView { kind: Top::Presets, enabled: true, tip: String::new() },
            ],
            preview,
            dims,
            busy: self.pending && !crop_mode,
            crop,
            items,
            scroll: self.scroll,
            name,
            ext,
            folder,
            paste_label: if self.saving { "Saving...".into() } else { "Paste Image".into() },
            paste_enabled: !self.saving && self.src.is_some() && too_large.is_none(),
            popup: self.popup_view(),
            toast: self.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default(),
            note,
            hover: self.hover,
        }
    }
}

// ---------------------------------------------------------------- the controller's side

impl Controller {
    // ------------------------------------------------------------------ queries (tests, platforms)

    pub fn editor_is_open(&self) -> bool {
        self.editor.is_some()
    }

    pub fn editor_animating(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.animating())
    }

    pub fn editor_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.editor.as_ref().map(|e| e.rect)
    }

    pub fn editor_view(&self) -> Option<EditorView> {
        self.editor.as_ref().map(|e| e.view(self.overlay.scale))
    }

    /// The editor's size in DIP.
    pub fn editor_size_dip(&self) -> Option<(f32, f32)> {
        self.editor.as_ref().map(|e| e.size_dip(self.overlay.scale))
    }

    pub fn editor_hit(&self, px: f32, py: f32) -> Option<EHit> {
        let view = self.editor_view()?;
        let size = self.editor_size_dip()?;
        Some(editor_hit(&view, size, self.overlay.scale, px, py))
    }

    /// The editor's current adjustments (tests).
    pub fn editor_params(&self) -> Option<EditParams> {
        self.editor.as_ref().map(|e| e.params.clone())
    }

    /// The Ctrl, Shift or Alt that opens the editor when held while clicking the desktop.
    pub fn set_advanced_modifier(&mut self, m: pasteroll_core::config::ClickModifier) {
        self.advanced_modifier = m;
    }

    // ------------------------------------------------------------------ open and close

    /// The picture under the carousel's pointer grows into the editor.
    pub(super) fn open_editor(&mut self, p: &mut dyn Platform, now: u64) {
        if self.editor.is_some() || self.viewer.is_some() {
            return;
        }
        let Some(id) = self.open.as_ref().and_then(Open::current_id) else { return };
        let info = self.store.borrow().info(id).ok().flatten();
        let Some(info) = info.filter(|i| i.kind == Kind::Image && i.width > 0 && i.height > 0) else {
            self.cancel(p, "the candidate is not an image");
            return;
        };
        let png = self.store.borrow().image_png(id);
        let png = match png {
            Ok(Some(b)) => b,
            _ => {
                self.cancel(p, "the image could not be read");
                return;
            }
        };
        self.withdraw_warning();
        if let Some(o) = self.open.as_mut() {
            // the click was on the desktop: that is where the carousel stands now
            o.ui.click = None;
            o.ui.dropdown_open = false;
            o.ui.hover = None;
            if o.ui.confirm_delete.take().is_some() {
                self.status = None;
            }
            match p.foreground() {
                Some(t) => o.target = t,
                None => o.target.desktop = true,
            }
        }
        let Some((card_img, card_rect, lifted)) = self.card_pictures() else {
            self.cancel(p, "no card to open");
            return;
        };
        let home = home_rect(&self.overlay);
        let presets = Presets::parse(&p.load_presets());
        let folder = presets.folder.clone().filter(|f| Path::new(f).is_dir()).unwrap_or_default();
        let name = format!("Pasteroll image {}", p.local_timestamp());
        let thumb = self.cached_thumb(id);

        // the decoding of the whole picture, and the worker that redraws the preview
        let tx = self.edit_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let mut png = png;
            let msg = match core_image::decode_png(&png) {
                Some(src) => {
                    let (proxy, k) = imgedit::preview_source(&src);
                    EditMsg::Loaded { id, src: Arc::new(src), proxy: Arc::new(proxy), k }
                }
                None => EditMsg::LoadFailed { id },
            };
            {
                use zeroize::Zeroize;
                png.zeroize();
            }
            let _ = tx.send(msg);
            waker.wake();
        });
        let (jobs, rx) = std::sync::mpsc::channel::<Job>();
        let tx = self.edit_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            while let Ok(mut job) = rx.recv() {
                // only the newest request matters
                while let Ok(newer) = rx.try_recv() {
                    job = newer;
                }
                let img = imgedit::apply(&job.proxy, &job.params);
                if tx.send(EditMsg::Preview { gen: job.gen, img: Arc::new(img), dims: job.dims }).is_err() {
                    break;
                }
                waker.wake();
            }
        });
        let params = EditParams::default();
        let src_dims = (info.width, info.height);
        self.editor = Some(Editor {
            id,
            src_dims,
            src: None,
            proxy: None,
            failed: false,
            history: vec![params.clone()],
            pos: 0,
            params,
            preview: thumb,
            preview_dims: src_dims,
            gen: 0,
            pending: false,
            jobs,
            open_sec: None,
            scroll: 0.0,
            focus: None,
            popup: None,
            drag: None,
            hover: None,
            pad_linked: true,
            crop_aspect: None,
            name,
            folder,
            presets,
            toast: None,
            saving: false,
            rect: card_rect,
            lifted,
            dim: 1.0,
            anim: None,
            last_frame: None,
            last_present_ms: 0,
            then: Then::Carousel,
        });
        let to = self.render_editor_at(home, true);
        let Some(morph) = Morph::new(&card_img, &to) else {
            self.editor = None;
            return;
        };
        let now = p.now_ms().max(now);
        if let Some(e) = self.editor.as_mut() {
            e.anim = Some(Anim { kind: AnimKind::Open, start_ms: now, dur_ms: OPEN_MS, from: card_rect, to: home, morph, dim: (1.0, DIM) });
        }
        p.viewer_mode(true);
        p.modal_clicks(true);
        p.animating(true);
        p.log("image editor opened");
        self.step_editor_anim(p, now);
    }

    fn render_editor_at(&mut self, rect: RectPx, draft: bool) -> Frame {
        let scale = self.overlay.scale;
        let res = if draft && rect.2 > 1600 { 0.5 } else { 1.0 };
        let Some(e) = self.editor.as_mut() else {
            return Frame { width: 1, height: 1, bgra: vec![0, 0, 0, 255] };
        };
        let saved = e.rect;
        e.rect = rect;
        let size = e.size_dip(scale);
        let view = e.view(scale);
        e.rect = saved;
        self.renderer.render_editor(&view, size, scale * res)
    }

    /// Shrinks the editor back into the card's place. `then` says what follows: the carousel again
    /// (Cancel), or the end of the session (the picture was saved).
    pub(super) fn close_editor(&mut self, p: &mut dyn Platform, now: u64, then: Then) {
        let Some(e) = self.editor.as_mut() else { return };
        if e.anim.as_ref().is_some_and(|a| a.kind == AnimKind::Close) {
            return;
        }
        e.then = then;
        e.popup = None;
        e.focus = None;
        e.drag = None;
        e.hover = None;
        let Some((card_img, card_rect, lifted)) = self.card_pictures() else {
            self.finish_editor(p);
            return;
        };
        let from_img = match self.editor.as_mut().and_then(|e| e.last_frame.take()) {
            Some(f) => f,
            None => {
                let r = self.editor.as_ref().map(|e| e.rect).unwrap_or(card_rect);
                self.render_editor_at(r, true)
            }
        };
        let now = p.now_ms().max(now);
        let Some(e) = self.editor.as_mut() else { return };
        let Some(morph) = Morph::new(&from_img, &card_img) else {
            self.finish_editor(p);
            return;
        };
        e.lifted = lifted;
        e.anim = Some(Anim { kind: AnimKind::Close, start_ms: now, dur_ms: CLOSE_MS, from: e.rect, to: card_rect, morph, dim: (e.dim, 1.0) });
        e.last_frame = Some(from_img);
        p.animating(true);
        self.step_editor_anim(p, now);
    }

    /// Draws the animation's frame for `now`; finishes it when its time is up.
    fn step_editor_anim(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(e) = self.editor.as_mut() else { return };
        let Some(a) = e.anim.as_ref() else { return };
        let t = (now.saturating_sub(a.start_ms) as f32 / a.dur_ms.max(1) as f32).clamp(0.0, 1.0);
        let ease = match a.kind {
            AnimKind::Open => ease_out(t),
            _ => ease_in_out(t),
        };
        let rect = lerp_rect(a.from, a.to, ease);
        let shrink = (ANIM_MAX_PIXELS / (rect.2 as f32 * rect.3 as f32)).sqrt().min(1.0);
        let (fw, fh) = ((rect.2 as f32 * shrink).round().max(1.0) as u32, (rect.3 as f32 * shrink).round().max(1.0) as u32);
        let frame = a.morph.frame(ease, fw, fh);
        let dim = lerp(a.dim.0, a.dim.1, ease);
        let kind = a.kind;
        p.viewer_show(rect, true);
        p.viewer_present(&frame);
        e.rect = rect;
        e.last_frame = Some(frame);
        if (dim - e.dim).abs() > 0.004 || t >= 1.0 {
            e.dim = dim;
            p.overlay_present(&e.lifted.dimmed(dim));
        }
        if t < 1.0 {
            return;
        }
        match kind {
            AnimKind::Open | AnimKind::Resize => {
                e.anim = None;
                e.last_frame = None;
                p.animating(false);
                self.dirty = true;
            }
            AnimKind::Close => self.finish_editor(p),
        }
    }

    /// The editor is gone; what follows depends on how it was closed.
    fn finish_editor(&mut self, p: &mut dyn Platform) {
        let then = self.editor.as_ref().map_or(Then::Carousel, |e| e.then);
        self.drop_editor(p);
        match then {
            Then::EndSession => {
                self.end_session(p);
                p.log("image editor closed");
            }
            Then::Carousel => {
                self.dirty = true;
                p.log("image editor closed, back to the carousel");
            }
        }
    }

    /// Removes the editor at once (no animation).
    pub(super) fn drop_editor(&mut self, p: &mut dyn Platform) {
        if self.editor.take().is_some() {
            p.viewer_hide();
            p.viewer_mode(false);
            p.modal_clicks(false);
            p.animating(false);
            let now = p.now_ms();
            if let Some(o) = self.open.as_mut() {
                // back in the carousel: its usual time limit starts again, in step with the hooks
                o.opened_ms = now;
                self.last_activity_ms = now;
                p.input_keepalive();
            }
        }
    }

    pub(super) fn present_editor(&mut self, p: &mut dyn Platform) {
        let scale = self.overlay.scale;
        let now = p.now_ms();
        let Some(e) = self.editor.as_mut() else { return };
        if e.animating() {
            return; // the animation draws itself
        }
        if now.saturating_sub(e.last_present_ms) < PRESENT_EVERY_MS && e.last_present_ms != 0 {
            self.dirty = true; // the next tick draws it
            return;
        }
        e.last_present_ms = now.max(1);
        let size = e.size_dip(scale);
        let view = e.view(scale);
        let frame = self.renderer.render_editor(&view, size, scale);
        p.viewer_show(e.rect, true);
        p.viewer_present(&frame);
        e.last_frame = Some(frame);
    }

    pub(super) fn editor_tick(&mut self, p: &mut dyn Platform, now: u64) {
        if self.editor.as_ref().is_some_and(|e| e.animating()) {
            self.step_editor_anim(p, now);
            return;
        }
        if let Some(e) = self.editor.as_mut() {
            if e.toast.as_ref().is_some_and(|t| now >= t.1) {
                e.toast = None;
                self.dirty = true;
            }
        }
    }

    // ------------------------------------------------------------------ results from the workers

    pub(super) fn drain_edit(&mut self, p: &mut dyn Platform, now: u64) {
        while let Ok(msg) = self.edit_rx.try_recv() {
            match msg {
                EditMsg::Loaded { id, src, proxy, k } => {
                    let Some(e) = self.editor.as_mut().filter(|e| e.id == id) else { continue };
                    e.src_dims = (src.width, src.height);
                    e.src = Some(src);
                    e.proxy = Some((proxy, k));
                    self.request_preview();
                    self.dirty = true;
                }
                EditMsg::LoadFailed { id } => {
                    let Some(e) = self.editor.as_mut().filter(|e| e.id == id) else { continue };
                    e.failed = true;
                    e.toast("The picture could not be read.", now);
                    self.dirty = true;
                }
                EditMsg::Preview { gen, img, dims } => {
                    let Some(e) = self.editor.as_mut() else { continue };
                    if gen == e.gen {
                        e.preview = Some(img);
                        e.preview_dims = dims;
                        e.pending = false;
                        self.dirty = true;
                    }
                }
                EditMsg::Exported { result } => self.on_exported(p, result, now),
            }
        }
    }

    /// Asks for the preview of the current adjustments to be redrawn.
    fn request_preview(&mut self) {
        let Some(e) = self.editor.as_mut() else { return };
        let Some((proxy, k)) = e.proxy.clone() else { return };
        if e.crop_mode() {
            return; // the original is shown; the result is drawn when the crop is done
        }
        e.gen += 1;
        e.pending = true;
        let job = Job { gen: e.gen, proxy, params: e.params.scaled(k), dims: e.out_dims() };
        let _ = e.jobs.send(job);
    }

    /// The parameters changed: redraw the preview.
    fn edited(&mut self) {
        self.request_preview();
        self.dirty = true;
    }

    // ------------------------------------------------------------------ saving

    fn effective_folder(&self, p: &dyn Platform) -> Option<PathBuf> {
        let e = self.editor.as_ref()?;
        let typed = e.folder.trim();
        if typed.is_empty() {
            p.desktop_folder()
        } else {
            Some(PathBuf::from(typed))
        }
    }

    fn editor_paste(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(e) = self.editor.as_ref() else { return };
        if e.saving || e.animating() {
            return;
        }
        let Some(src) = e.src.clone() else {
            let msg = if e.failed { "The picture could not be read." } else { "Still reading the picture..." };
            self.editor_toast(msg, now);
            return;
        };
        let out = e.out_dims();
        if let Some(why) = imgedit::too_large(out) {
            self.editor_toast(why, now);
            return;
        }
        let Some(folder) = self.effective_folder(p).filter(|f| f.is_dir()) else {
            self.editor_toast("That folder does not exist. Choose another with the folder button.", now);
            return;
        };
        let Some(e) = self.editor.as_mut() else { return };
        let ext = e.params.format.ext();
        let fallback = format!("Pasteroll image {}", p.local_timestamp());
        let stem = clean_file_stem(&e.name, ext, &fallback);
        let params = e.params.clone();
        e.saving = true;
        e.focus = None;
        e.popup = None;
        self.dirty = true;
        let tx = self.edit_tx.clone();
        let waker = self.waker.clone();
        p.log("saving the edited image");
        std::thread::spawn(move || {
            let result = (|| {
                let img = imgedit::apply(&src, &params);
                let bytes = pasteroll_core::imgout::encode(&img, params.format, params.quality, params.png_level)?;
                crate::files::save_unique(&folder, &stem, ext, &bytes).map_err(|e| format!("Could not save the file: {e}"))
            })();
            let _ = tx.send(EditMsg::Exported { result });
            waker.wake();
        });
    }

    fn on_exported(&mut self, p: &mut dyn Platform, result: Result<PathBuf, String>, now: u64) {
        let Some(e) = self.editor.as_mut() else { return };
        e.saving = false;
        match result {
            Ok(path) => {
                p.file_created(&path);
                p.log(&format!("saved image as {}", path.file_name().unwrap_or_default().to_string_lossy()));
                // remember where it went (the Desktop is the default and is not stored)
                let dir = path.parent().map(|d| d.to_string_lossy().into_owned());
                let desktop = p.desktop_folder().map(|d| d.to_string_lossy().into_owned());
                let e = self.editor.as_mut().expect("editor");
                let remembered = if dir == desktop { None } else { dir };
                if e.presets.folder != remembered {
                    e.presets.folder = remembered;
                    let text = e.presets.to_text();
                    if let Err(err) = p.save_presets(&text) {
                        p.log(&format!("could not save the presets file: {err}"));
                    }
                }
                self.close_editor(p, now, Then::EndSession);
            }
            Err(msg) => {
                p.log(&format!("saving the edited image failed: {msg}"));
                self.editor_toast(msg, now);
            }
        }
        self.dirty = true;
    }

    fn editor_toast(&mut self, text: impl Into<String>, now: u64) {
        if let Some(e) = self.editor.as_mut() {
            e.toast(text, now);
            self.dirty = true;
        }
    }

    // ------------------------------------------------------------------ input

    /// Routes an event while the editor is open. `true` = handled.
    pub(super) fn editor_event(&mut self, p: &mut dyn Platform, ev: Event, now: u64) -> bool {
        match ev {
            Event::Key(key, gen) => {
                if self.live(p, gen) {
                    self.touch(p, now);
                    self.editor_key(p, key, now);
                }
                true
            }
            Event::Step(step, gen) => {
                if self.live(p, gen) {
                    self.touch(p, now);
                    self.editor_step(step == pasteroll_core::Step::Older, now);
                }
                true
            }
            Event::ViewerMouse(m) => {
                self.touch(p, now);
                self.editor_mouse(p, m, now);
                true
            }
            Event::Wheel(w, gen) => {
                if self.live(p, gen) {
                    self.touch(p, now);
                    self.editor_wheel(w);
                }
                true
            }
            // the editor is modal: nothing else reaches the carousel or the program underneath
            Event::ClickPaste(_) | Event::ClickCancel(_) | Event::ClickOutside(_) | Event::Mouse(_) | Event::Activate => true,
            Event::Tick | Event::Wake => false,
        }
    }

    fn editor_step(&mut self, older: bool, now: u64) {
        let _ = now;
        let Some(e) = self.editor.as_ref() else { return };
        if e.animating() {
            return;
        }
        // Up and Down: nudge the number being typed, or scroll the panel
        if let Some(Field::Num(n)) = e.focus.as_ref().map(|f| f.field) {
            let cur = e.focus.as_ref().and_then(|f| parse_num(&f.text)).unwrap_or_else(|| e.num_get(n));
            let v = cur + if older { -1.0 } else { 1.0 };
            self.apply_num(n, v);
            if let Some(e) = self.editor.as_mut() {
                let text = fmt_num(e.num_get(n));
                if let Some(f) = e.focus.as_mut() {
                    f.text = text;
                    f.sel_all = true;
                }
            }
            return;
        }
        self.editor_scroll_by(if older { 48.0 } else { -48.0 });
    }

    fn editor_scroll_by(&mut self, dy: f32) {
        let scale = self.overlay.scale;
        let Some(e) = self.editor.as_ref() else { return };
        let view = e.view(scale);
        let size = e.size_dip(scale);
        let max = editor_layout(&view, size).max_scroll;
        if let Some(e) = self.editor.as_mut() {
            e.scroll = (e.scroll + dy).clamp(0.0, max);
            self.dirty = true;
        }
    }

    fn editor_wheel(&mut self, w: crate::event::Wheel) {
        let scale = self.overlay.scale;
        let Some(e) = self.editor.as_ref() else { return };
        if e.animating() || e.popup.is_some() {
            return;
        }
        let r = e.rect;
        let (x, y) = (((self.last_pointer.0 - r.0) as f32) / scale, ((self.last_pointer.1 - r.1) as f32) / scale);
        let view = e.view(scale);
        let l = editor_layout(&view, e.size_dip(scale));
        if l.panel.contains(x, y) {
            let notches = w.delta as f32 / 120.0;
            self.editor_scroll_by(-notches * 54.0);
        }
    }

    fn editor_key(&mut self, p: &mut dyn Platform, key: Key, now: u64) {
        let Some(e) = self.editor.as_mut() else { return };
        if e.animating() {
            if key == Key::Escape {
                self.close_editor(p, now, Then::Carousel);
            }
            return;
        }
        if e.saving {
            return;
        }
        // typing into a box
        if e.focus.is_some() {
            match key {
                Key::Char(c) => {
                    self.focus_type(c);
                    return;
                }
                Key::Backspace => {
                    if let Some(f) = e.focus.as_mut() {
                        if f.sel_all {
                            f.text.clear();
                            f.sel_all = false;
                        } else {
                            f.text.pop();
                        }
                    }
                    self.live_commit();
                    self.dirty = true;
                    return;
                }
                Key::CtrlBackspace | Key::Delete | Key::CtrlDelete => {
                    if let Some(f) = e.focus.as_mut() {
                        f.text.clear();
                        f.sel_all = false;
                    }
                    self.live_commit();
                    self.dirty = true;
                    return;
                }
                Key::CtrlA => {
                    if let Some(f) = e.focus.as_mut() {
                        f.sel_all = true;
                    }
                    self.dirty = true;
                    return;
                }
                Key::Enter | Key::Tab => {
                    let was_popup_text = matches!(self.editor.as_ref().and_then(|e| e.focus.as_ref().map(|f| f.field)), Some(Field::Hex | Field::PresetName));
                    self.commit_focus(p, now);
                    if was_popup_text {
                        return;
                    }
                    return;
                }
                Key::Escape => {
                    // give up the edit
                    let field = e.focus.as_ref().map(|f| f.field);
                    e.focus = None;
                    if matches!(field, Some(Field::Hex | Field::PresetName)) {
                        e.popup = None;
                    }
                    self.dirty = true;
                    return;
                }
                _ => {}
            }
        }
        let popup = e.popup.is_some();
        match key {
            Key::Escape => {
                if popup {
                    e.popup = None;
                    self.dirty = true;
                } else if e.open_sec.is_some() && e.crop_mode() {
                    // leaving the crop view shows the result again
                    e.open_sec = None;
                    self.edited();
                } else {
                    self.close_editor(p, now, Then::Carousel);
                }
            }
            Key::Enter if !popup => self.editor_paste(p, now),
            Key::CtrlZ => self.editor_undo(),
            Key::CtrlY => self.editor_redo(),
            _ => {}
        }
    }

    fn focus_type(&mut self, c: char) {
        let Some(e) = self.editor.as_mut() else { return };
        let Some(f) = e.focus.as_mut() else { return };
        let ok = match f.field {
            Field::Num(_) => c.is_ascii_digit() || matches!(c, '.' | '-' | ','),
            Field::Hex => c.is_ascii_hexdigit() || c == '#',
            Field::Name | Field::PresetName => !bad_in_name(c),
            Field::Folder => !c.is_control(),
        };
        if !ok {
            return;
        }
        let max = match f.field {
            Field::Num(_) => 9,
            Field::Hex => 9,
            Field::Name => NAME_MAX,
            Field::PresetName => imgedit::PRESET_NAME_MAX,
            Field::Folder => FOLDER_MAX,
        };
        if f.sel_all {
            f.text.clear();
            f.sel_all = false;
        }
        if f.text.chars().count() < max {
            f.text.push(if c == ',' { '.' } else { c });
        }
        self.live_commit();
        self.dirty = true;
    }

    /// A number typed into a box takes effect as it is typed (a half-typed value is ignored).
    fn live_commit(&mut self) {
        let Some(e) = self.editor.as_ref() else { return };
        let Some(f) = e.focus.as_ref() else { return };
        if let Field::Num(n) = f.field {
            if let Some(v) = parse_num(&f.text) {
                self.apply_num(n, v);
            }
        }
    }

    /// Sets a number and redraws, without making a step to undo yet.
    fn apply_num(&mut self, n: Num, v: f32) {
        let Some(e) = self.editor.as_mut() else { return };
        e.num_set(n, v);
        self.edited();
    }

    /// Ends the typing: the value is kept, and is one step to undo.
    fn commit_focus(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(e) = self.editor.as_mut() else { return };
        let Some(f) = e.focus.take() else { return };
        self.dirty = true;
        match f.field {
            Field::Num(n) => {
                if let Some(v) = parse_num(&f.text) {
                    self.apply_num(n, v);
                }
                if let Some(e) = self.editor.as_mut() {
                    e.checkpoint();
                }
            }
            Field::Name => {
                if let Some(e) = self.editor.as_mut() {
                    e.name = f.text.clone();
                }
            }
            Field::Folder => {
                if let Some(e) = self.editor.as_mut() {
                    e.folder = f.text.trim().to_owned();
                }
            }
            Field::Hex => {
                let Some(c) = Color::parse(&f.text) else {
                    self.editor_toast("That is not a colour code. Use a code like #2FAE97.", now);
                    // stay in the popup
                    if let Some(e) = self.editor.as_mut() {
                        e.focus = Some(Focus { field: Field::Hex, text: f.text.clone(), sel_all: false });
                    }
                    return;
                };
                let target = match self.editor.as_ref().and_then(|e| e.popup.as_ref()) {
                    Some(Popup::Color { target, .. }) => Some(*target),
                    _ => None,
                };
                if let (Some(t), Some(e)) = (target, self.editor.as_mut()) {
                    e.color_set(t, c.0);
                    e.checkpoint();
                    e.popup = None;
                    self.edited();
                }
            }
            Field::PresetName => self.save_preset(p, &f.text, now),
        }
    }

    fn focus_field(&mut self, field: Field, select_all: bool) {
        let Some(e) = self.editor.as_mut() else { return };
        let text = match field {
            Field::Num(n) => fmt_num(e.num_get(n)),
            Field::Name => e.name.clone(),
            Field::Folder => e.folder.clone(),
            Field::Hex => match &e.popup {
                Some(Popup::Color { target, .. }) => Color(e.color_get(*target)).hex(),
                _ => String::new(),
            },
            Field::PresetName => String::new(),
        };
        e.focus = Some(Focus { field, text, sel_all: select_all });
        self.dirty = true;
    }

    // ------------------------------------------------------------------ the mouse

    fn editor_hit_at(&self, x: f32, y: f32) -> Option<EHit> {
        let e = self.editor.as_ref()?;
        let scale = self.overlay.scale;
        Some(editor_hit(&e.view(scale), e.size_dip(scale), scale, x, y))
    }

    fn editor_mouse(&mut self, p: &mut dyn Platform, m: MouseEvent, now: u64) {
        let Some(e) = self.editor.as_ref() else { return };
        if e.animating() {
            return;
        }
        let r = e.rect;
        self.last_pointer = (r.0 + m.x as i32, r.1 + m.y as i32);
        match m.kind {
            MouseKind::Leave => {
                if let Some(e) = self.editor.as_mut() {
                    if e.hover.take().is_some() {
                        self.dirty = true;
                    }
                }
            }
            MouseKind::Up => self.editor_release(),
            MouseKind::Move => {
                if self.editor.as_ref().is_some_and(|e| e.drag.is_some()) {
                    self.editor_drag(m.x, m.y);
                } else if let Some(hit) = self.editor_hit_at(m.x, m.y) {
                    let hit = Some(hit.plain());
                    if let Some(e) = self.editor.as_mut() {
                        if e.hover != hit {
                            e.hover = hit;
                            self.dirty = true;
                        }
                    }
                }
            }
            MouseKind::Down | MouseKind::DoubleClick => {
                let Some(hit) = self.editor_hit_at(m.x, m.y) else { return };
                self.editor_press(p, hit, m, now);
            }
        }
    }

    fn editor_release(&mut self) {
        let Some(e) = self.editor.as_mut() else { return };
        if e.drag.take().is_some() {
            e.checkpoint();
            self.dirty = true;
        }
    }

    /// Source-picture pixels under a window position (px), for dragging the crop.
    fn crop_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let e = self.editor.as_ref()?;
        let scale = self.overlay.scale;
        let view = e.view(scale);
        let l = editor_layout(&view, e.size_dip(scale));
        let img = l.image?;
        let k = img.w / e.src_dims.0.max(1) as f32;
        Some(((x / scale - img.x) / k, (y / scale - img.y) / k))
    }

    fn editor_drag(&mut self, x: f32, y: f32) {
        let Some(drag) = self.editor.as_ref().and_then(|e| e.drag) else { return };
        match drag {
            Drag::Slider { id, num } => {
                let scale = self.overlay.scale;
                let Some(e) = self.editor.as_ref() else { return };
                let view = e.view(scale);
                let l = editor_layout(&view, e.size_dip(scale));
                let Some(pl) = l.placed.iter().find(|pl| pl.id == id) else { return };
                let t = pl.parts[0];
                let frac = (((x / scale) - t.x - 7.0) / (t.w - 14.0).max(1.0)).clamp(0.0, 1.0);
                self.slider_to(num, frac);
            }
            Drag::Crop { part, start, rect } => {
                let Some((px, py)) = self.crop_point(x, y) else { return };
                let Some(e) = self.editor.as_ref() else { return };
                let bounds = (e.src_dims.0 as f32, e.src_dims.1 as f32);
                let aspect = e.crop_aspect.map(|(w, h)| w / h);
                let r = crop_drag(rect, part, px - start.0, py - start.1, bounds, aspect);
                if let Some(e) = self.editor.as_mut() {
                    e.set_crop(r);
                    self.dirty = true;
                }
            }
        }
    }

    fn slider_to(&mut self, num: Num, frac: f32) {
        let Some(e) = self.editor.as_ref() else { return };
        let (lo, hi) = e.slider_range(num);
        let mut v = lo + (hi - lo) * frac;
        // whole numbers, except where the range is tiny
        v = if hi - lo >= 10.0 { v.round() } else { (v * 10.0).round() / 10.0 };
        if e.num_get(num) != v {
            self.apply_num(num, v);
        }
    }

    fn editor_press(&mut self, p: &mut dyn Platform, hit: EHit, m: MouseEvent, now: u64) {
        let Some(e) = self.editor.as_mut() else { return };
        if e.saving {
            return;
        }
        // a popup is modal
        if e.popup.is_some() {
            match hit {
                EHit::OutsidePopup => {
                    e.popup = None;
                    e.focus = None;
                    self.dirty = true;
                }
                EHit::Item { id, part, .. } => self.popup_press(p, id, part, now),
                _ => {}
            }
            return;
        }
        // clicking anywhere but the box being typed in ends the typing
        let keeps_focus = match (hit, e.focus.as_ref().map(|f| f.field)) {
            (EHit::Name, Some(Field::Name)) | (EHit::Folder, Some(Field::Folder)) => true,
            (EHit::Item { id, part, .. }, Some(Field::Num(n))) => self.item_field(id, part) == Some(n),
            _ => false,
        };
        if !keeps_focus && self.editor.as_ref().is_some_and(|e| e.focus.is_some()) {
            self.commit_focus(p, now);
        }
        match hit {
            EHit::Back | EHit::Cancel => self.close_editor(p, now, Then::Carousel),
            EHit::Paste => self.editor_paste(p, now),
            EHit::Top(t) => self.top_press(p, t, now),
            EHit::Name => {
                if !keeps_focus {
                    self.focus_field(Field::Name, false);
                }
            }
            EHit::Folder => {
                if !keeps_focus {
                    self.focus_field(Field::Folder, false);
                }
            }
            EHit::Browse => self.browse_folder(p, now),
            EHit::Item { id, part, frac } => self.item_press(p, id, part, frac, keeps_focus, now),
            EHit::Crop(part) => {
                if let (Some(pt), Some(e)) = (self.crop_point(m.x, m.y), self.editor.as_mut()) {
                    let rect = e.crop_rect();
                    e.drag = Some(Drag::Crop { part, start: pt, rect });
                }
            }
            _ => {}
        }
    }

    /// The number box an item's part stands for.
    fn item_field(&self, id: u32, part: u16) -> Option<Num> {
        let (sec, n) = split_id(id)?;
        let part = usize::from(part);
        Some(match (sec, n) {
            (Sec::Crop, 2) => [Num::CropW, Num::CropH].get(part).copied()?,
            (Sec::Resize, 2) => [Num::ResizeW, Num::ResizeH].get(part).copied()?,
            (Sec::Rotate, 1) => Num::Angle,
            (Sec::Corners, 1) => Num::Radius,
            (Sec::Outline, 1) => Num::OutlineW,
            (Sec::Shadow, 2) => Num::ShadowOpacity,
            (Sec::Shadow, 3) => Num::ShadowBlur,
            (Sec::Shadow, 4) => Num::ShadowDist,
            (Sec::Shadow, 5) => Num::ShadowAngle,
            (Sec::Background, 5) => Num::Tolerance,
            (Sec::Padding, 2) => {
                if self.editor.as_ref().is_some_and(|e| e.pad_linked) {
                    Num::PadAll
                } else {
                    [Num::PadTop, Num::PadBottom].get(part).copied()?
                }
            }
            (Sec::Padding, 3) => [Num::PadLeft, Num::PadRight].get(part).copied()?,
            (Sec::Output, 2) => Num::Quality,
            _ => return None,
        })
    }

    fn item_press(&mut self, p: &mut dyn Platform, id: u32, part: u16, frac: f32, keeps_focus: bool, now: u64) {
        let Some((sec, n)) = split_id(id) else { return };
        // headers: the title opens the section (one at a time), the switch turns the effect on or off
        if n == 0 {
            if part == 1 {
                self.toggle_switch(sec);
            } else {
                self.toggle_section(sec);
            }
            return;
        }
        // a number box or a slider
        if let Some(num) = self.item_field(id, part) {
            let is_slider_track = matches!((sec, n), (Sec::Rotate, 1) | (Sec::Corners, 1) | (Sec::Outline, 1) | (Sec::Shadow, 2..=5) | (Sec::Background, 5) | (Sec::Output, 2))
                || (sec == Sec::Padding && n == 2 && self.editor.as_ref().is_some_and(|e| e.pad_linked));
            if is_slider_track && part == 0 {
                if let Some(e) = self.editor.as_mut() {
                    e.drag = Some(Drag::Slider { id, num });
                }
                self.slider_to(num, frac);
                return;
            }
            if !keeps_focus {
                self.focus_field(Field::Num(num), true);
            }
            return;
        }
        let Some(e) = self.editor.as_mut() else { return };
        let part = usize::from(part);
        match (sec, n) {
            (Sec::Crop, 1) => {
                let Some((_, aspect)) = ASPECTS.get(part).copied() else { return };
                let (sw, sh) = (e.src_dims.0 as f32, e.src_dims.1 as f32);
                match aspect {
                    None => e.crop_aspect = None,
                    Some(a) => {
                        let a = if a.0 == 0.0 { (sw, sh) } else { a };
                        e.crop_aspect = Some(a);
                        let r = fit_aspect(e.crop_rect(), a.0 / a.1, (sw, sh));
                        e.set_crop(r);
                    }
                }
                e.checkpoint();
                self.dirty = true;
            }
            (Sec::Crop, 3) => {
                e.params.crop = None;
                e.checkpoint();
                self.dirty = true;
            }
            (Sec::Resize, 1) => {
                let rotated = e.plan().rotated;
                match part {
                    0 => e.params.resize_mode = ResizeMode::Off,
                    1 => {
                        // start from the size the picture has now
                        e.params.resize_mode = ResizeMode::Pixels;
                        e.params.resize_w = rotated.0 as f32;
                        e.params.resize_h = rotated.1 as f32;
                    }
                    _ => {
                        e.params.resize_mode = ResizeMode::Percent;
                        e.params.resize_w = 100.0;
                        e.params.resize_h = 100.0;
                    }
                }
                e.checkpoint();
                self.edited();
            }
            (Sec::Resize, 3) => {
                e.params.keep_aspect = !e.params.keep_aspect;
                e.checkpoint();
                self.edited();
            }
            (Sec::Rotate, 2) => {
                match part {
                    0 | 1 => {
                        let turned = e.params.angle + if part == 0 { -90.0 } else { 90.0 };
                        e.params.angle = if turned > 180.0 { turned - 360.0 } else if turned < -180.0 { turned + 360.0 } else { turned };
                    }
                    2 => e.params.flip_h = !e.params.flip_h,
                    _ => e.params.flip_v = !e.params.flip_v,
                }
                e.checkpoint();
                self.edited();
            }
            (Sec::Corners, 2) => {
                e.params.round_on = part == 1 || e.params.round_on;
                e.params.radius = if part == 0 { 0.0 } else { 1.0e6 };
                if part == 0 {
                    e.params.round_on = false;
                }
                e.checkpoint();
                self.edited();
            }
            (Sec::Outline, 2) => self.open_color(ColorTarget::Outline, id),
            (Sec::Shadow, 1) => self.open_color(ColorTarget::Shadow, id),
            (Sec::Padding, 4) => self.open_color(ColorTarget::Padding, id),
            (Sec::Background, 2) => self.open_color(ColorTarget::Fill, id),
            (Sec::Background, 4) => self.open_color(ColorTarget::Key, id),
            (Sec::Background, 1) => {
                e.params.bg_mode = [BgMode::Keep, BgMode::Fill, BgMode::Remove, BgMode::Replace].get(part).copied().unwrap_or(BgMode::Keep);
                e.checkpoint();
                self.edited();
            }
            (Sec::Background, 3) => {
                e.params.bg_key = if e.params.bg_key.is_some() { None } else { Some(Color([255, 255, 255, 255])) };
                e.checkpoint();
                self.edited();
            }
            (Sec::Padding, 1) => {
                e.pad_linked = !e.pad_linked;
                if e.pad_linked {
                    e.params.pad = [e.params.pad[0]; 4];
                }
                e.checkpoint();
                self.edited();
            }
            (Sec::Output, 1) => {
                e.params.format = Format::ALL.get(part).copied().unwrap_or(Format::Png);
                e.checkpoint();
                self.dirty = true;
            }
            (Sec::Output, 3) => {
                e.params.png_level = [PngLevel::Fast, PngLevel::Balanced, PngLevel::Smallest].get(part).copied().unwrap_or(PngLevel::Balanced);
                e.checkpoint();
                self.dirty = true;
            }
            _ => {}
        }
        let _ = (p, now);
    }

    fn toggle_section(&mut self, sec: Sec) {
        let Some(e) = self.editor.as_mut() else { return };
        let was_crop = e.crop_mode();
        e.open_sec = if e.open_sec == Some(sec) { None } else { Some(sec) };
        e.scroll = 0.0;
        e.focus = None;
        let crop_now = e.crop_mode();
        if was_crop != crop_now {
            // the crop view shows the original, the others show the result
            if crop_now {
                self.dirty = true;
            } else {
                self.edited();
            }
        }
        self.dirty = true;
    }

    fn toggle_switch(&mut self, sec: Sec) {
        let Some(e) = self.editor.as_mut() else { return };
        let p = &mut e.params;
        match sec {
            Sec::Corners => p.round_on = !p.round_on,
            Sec::Outline => p.outline_on = !p.outline_on,
            Sec::Shadow => p.shadow_on = !p.shadow_on,
            Sec::Padding => p.pad_on = !p.pad_on,
            Sec::Background => p.bg_mode = if p.bg_mode == BgMode::Keep { BgMode::Remove } else { BgMode::Keep },
            _ => return,
        }
        let on = e.switch_of(sec).unwrap_or(false);
        e.checkpoint();
        // turning an effect on shows its settings
        if on && e.open_sec != Some(sec) {
            let was_crop = e.crop_mode();
            e.open_sec = Some(sec);
            e.scroll = 0.0;
            if was_crop {
                e.focus = None;
            }
        }
        self.edited();
    }

    fn top_press(&mut self, p: &mut dyn Platform, t: Top, now: u64) {
        match t {
            Top::Undo => self.editor_undo(),
            Top::Redo => self.editor_redo(),
            Top::Reset => {
                let Some(e) = self.editor.as_mut() else { return };
                if e.params != EditParams::default() {
                    e.params = EditParams::default();
                    e.pad_linked = true;
                    e.crop_aspect = None;
                    e.checkpoint();
                    self.edited();
                    self.editor_toast("Everything is back to the original.", now);
                }
            }
            Top::Presets => {
                if let Some(e) = self.editor.as_mut() {
                    e.popup = Some(Popup::Presets { naming: false });
                    e.focus = None;
                    self.dirty = true;
                }
            }
        }
        let _ = p;
    }

    fn editor_undo(&mut self) {
        let Some(e) = self.editor.as_mut() else { return };
        e.focus = None;
        if e.params != e.history[e.pos] {
            e.params = e.history[e.pos].clone();
        } else if e.pos > 0 {
            e.pos -= 1;
            e.params = e.history[e.pos].clone();
        } else {
            return;
        }
        self.edited();
    }

    fn editor_redo(&mut self) {
        let Some(e) = self.editor.as_mut() else { return };
        e.focus = None;
        if e.can_redo() {
            e.pos += 1;
            e.params = e.history[e.pos].clone();
            self.edited();
        }
    }

    fn open_color(&mut self, target: ColorTarget, anchor: u32) {
        let Some(e) = self.editor.as_mut() else { return };
        e.popup = Some(Popup::Color { target, anchor });
        self.focus_field(Field::Hex, true);
    }

    fn popup_press(&mut self, p: &mut dyn Platform, id: u32, part: u16, now: u64) {
        let Some(e) = self.editor.as_mut() else { return };
        let part = usize::from(part);
        match (id, e.popup.is_some()) {
            (POP_PALETTE, true) => {
                let Some(Popup::Color { target, .. }) = e.popup.as_ref() else { return };
                let target = *target;
                let colors = palette_colors(Editor::color_allows_none(target));
                if let Some(c) = colors.get(part) {
                    e.color_set(target, *c);
                    e.checkpoint();
                    e.popup = None;
                    e.focus = None;
                    self.edited();
                }
            }
            (POP_HEX, true) => {
                if e.focus.as_ref().map(|f| f.field) != Some(Field::Hex) {
                    self.focus_field(Field::Hex, true);
                }
            }
            (POP_SAVE_AS, true) => {
                e.popup = Some(Popup::Presets { naming: true });
                self.focus_field(Field::PresetName, false);
            }
            (POP_NAME, true) => {
                if e.focus.as_ref().map(|f| f.field) != Some(Field::PresetName) {
                    self.focus_field(Field::PresetName, false);
                }
            }
            (POP_NAME_BUTTONS, true) => {
                if part == 0 {
                    let text = e.focus.as_ref().filter(|f| f.field == Field::PresetName).map(|f| f.text.clone()).unwrap_or_default();
                    self.save_preset(p, &text, now);
                } else {
                    e.popup = Some(Popup::Presets { naming: false });
                    e.focus = None;
                    self.dirty = true;
                }
            }
            (i, true) if (POP_PRESET_BASE..POP_SAVE_AS).contains(&i) => {
                let idx = (i - POP_PRESET_BASE) as usize;
                let Some((name, preset)) = e.presets.items.get(idx).cloned() else { return };
                if part == 1 {
                    e.presets.remove(&name);
                    let text = e.presets.to_text();
                    if let Err(err) = p.save_presets(&text) {
                        p.log(&format!("could not save the presets file: {err}"));
                    }
                    self.dirty = true;
                } else {
                    // a preset is a way of working: this picture's crop stays
                    let crop = e.params.crop;
                    e.params = preset;
                    e.params.crop = crop;
                    e.params.sanitize();
                    e.pad_linked = e.params.pad.iter().all(|v| *v == e.params.pad[0]);
                    e.checkpoint();
                    e.popup = None;
                    self.edited();
                    self.editor_toast(format!("Applied \"{name}\""), now);
                }
            }
            _ => {}
        }
    }

    fn save_preset(&mut self, p: &mut dyn Platform, name: &str, now: u64) {
        let Some(e) = self.editor.as_mut() else { return };
        if imgedit::clean_preset_name(name).is_none() {
            e.toast("Give the preset a name first.", now);
            self.dirty = true;
            return;
        }
        let params = e.params.clone();
        if !e.presets.save(name, &params) {
            e.toast("There are too many presets: delete one first.", now);
            self.dirty = true;
            return;
        }
        let text = e.presets.to_text();
        match p.save_presets(&text) {
            Ok(()) => e.toast(format!("Saved the preset \"{}\"", imgedit::clean_preset_name(name).unwrap_or_default()), now),
            Err(err) => e.toast(format!("Could not save the preset: {err}"), now),
        }
        e.popup = None;
        e.focus = None;
        self.dirty = true;
    }

    fn browse_folder(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(e) = self.editor.as_ref() else { return };
        let Some(target) = self.open.as_ref().map(|o| o.target.clone()) else { return };
        let start = if e.folder.trim().is_empty() { p.desktop_folder() } else { Some(PathBuf::from(e.folder.trim())) };
        match p.folder_dialog(start.as_deref(), &target) {
            SaveChoice::Path(dir) => {
                if let Some(e) = self.editor.as_mut() {
                    e.folder = dir.to_string_lossy().into_owned();
                    e.focus = None;
                    self.dirty = true;
                }
            }
            SaveChoice::Cancelled => {}
            SaveChoice::Unavailable => {
                self.editor_toast("This system has no folder chooser: type the folder's path instead.", now);
            }
        }
    }
}

fn parse_num(text: &str) -> Option<f32> {
    let t = text.trim().trim_end_matches(['%', ' ']).trim_end_matches("px").trim();
    let v: f32 = t.parse().ok()?;
    v.is_finite().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: (f32, f32) = (1000.0, 800.0);

    #[test]
    fn dragging_a_free_corner_changes_both_sides_and_stays_inside() {
        let r = crop_drag((100.0, 100.0, 400.0, 300.0), CropPart::SE, 50.0, -20.0, B, None);
        assert_eq!(r, (100.0, 100.0, 450.0, 280.0));
        let r = crop_drag((100.0, 100.0, 400.0, 300.0), CropPart::NW, -500.0, -500.0, B, None);
        assert_eq!((r.0, r.1), (0.0, 0.0), "cannot go outside the picture");
        assert_eq!((r.2, r.3), (500.0, 400.0));
        let r = crop_drag((100.0, 100.0, 400.0, 300.0), CropPart::SE, 5000.0, 5000.0, B, None);
        assert_eq!((r.0 + r.2, r.1 + r.3), (1000.0, 800.0));
        // it never collapses
        let r = crop_drag((100.0, 100.0, 400.0, 300.0), CropPart::E, -5000.0, 0.0, B, None);
        assert_eq!(r.2, MIN_CROP);
    }

    #[test]
    fn moving_keeps_the_size_and_stops_at_the_edges() {
        let r = crop_drag((100.0, 100.0, 400.0, 300.0), CropPart::Move, 900.0, -900.0, B, None);
        assert_eq!(r, (600.0, 0.0, 400.0, 300.0));
    }

    #[test]
    fn a_locked_shape_is_kept_on_corners_and_edges() {
        let a = Some(16.0 / 9.0);
        let start = (100.0, 100.0, 320.0, 180.0);
        let r = crop_drag(start, CropPart::SE, 160.0, 0.0, B, a);
        assert!((r.2 / r.3 - 16.0 / 9.0).abs() < 0.01, "{r:?}");
        assert_eq!((r.0, r.1), (100.0, 100.0), "the opposite corner stays");
        assert!(r.2 > 320.0);
        let r = crop_drag(start, CropPart::NW, -80.0, -80.0, B, a);
        assert!((r.2 / r.3 - 16.0 / 9.0).abs() < 0.01);
        assert!((r.0 + r.2 - 420.0).abs() < 0.01 && (r.1 + r.3 - 280.0).abs() < 0.01, "the far corner stays {r:?}");
        // an edge drag grows the other side about the middle
        let r = crop_drag(start, CropPart::E, 160.0, 0.0, B, a);
        assert!((r.2 / r.3 - 16.0 / 9.0).abs() < 0.01);
        assert_eq!(r.0, 100.0);
        assert!((r.1 + r.3 / 2.0 - 190.0).abs() < 0.01);
        let r = crop_drag(start, CropPart::S, 0.0, 90.0, B, a);
        assert!((r.2 / r.3 - 16.0 / 9.0).abs() < 0.01);
        assert!((r.0 + r.2 / 2.0 - 260.0).abs() < 0.01);
        // a huge drag is limited by the picture and keeps the shape
        let r = crop_drag(start, CropPart::SE, 9000.0, 9000.0, B, a);
        assert!((r.2 / r.3 - 16.0 / 9.0).abs() < 0.01);
        assert!(r.0 + r.2 <= 1000.01 && r.1 + r.3 <= 800.01);
    }

    #[test]
    fn a_shape_is_fitted_inside_the_current_crop() {
        let r = fit_aspect((0.0, 0.0, 1000.0, 800.0), 1.0, B);
        assert_eq!((r.2, r.3), (800.0, 800.0));
        assert_eq!(r.0, 100.0);
        let r = fit_aspect((0.0, 0.0, 1000.0, 800.0), 16.0 / 9.0, B);
        assert!((r.2 - 1000.0).abs() < 0.01 && (r.3 - 562.5).abs() < 0.01);
        let r = fit_aspect((0.0, 0.0, 1000.0, 800.0), 9.0 / 16.0, B);
        assert!((r.3 - 800.0).abs() < 0.01 && (r.2 - 450.0).abs() < 0.01);
    }

    #[test]
    fn typed_file_names_are_made_safe() {
        assert_eq!(clean_file_stem("My picture", "png", "fallback"), "My picture");
        assert_eq!(clean_file_stem("a/b\\c:d*e?f\"g<h>i|j", "png", "fallback"), "abcdefghij");
        assert_eq!(clean_file_stem("photo.PNG", "png", "fallback"), "photo");
        assert_eq!(clean_file_stem("photo.png", "jpg", "fallback"), "photo.png");
        assert_eq!(clean_file_stem("  ..  ", "png", "fallback"), "fallback");
        assert_eq!(clean_file_stem("", "png", "fallback"), "fallback");
        assert_eq!(clean_file_stem("CON", "png", "fallback"), "CON_");
        assert_eq!(clean_file_stem("lpt1", "png", "fallback"), "lpt1_");
        assert_eq!(clean_file_stem("report.", "png", "fallback"), "report");
        assert_eq!(clean_file_stem(&"x".repeat(300), "png", "f").len(), NAME_MAX);
    }

    #[test]
    fn numbers_are_read_leniently() {
        assert_eq!(parse_num("12"), Some(12.0));
        assert_eq!(parse_num(" 12.5 px"), Some(12.5));
        assert_eq!(parse_num("50%"), Some(50.0));
        assert_eq!(parse_num("-3"), Some(-3.0));
        assert_eq!(parse_num(""), None);
        assert_eq!(parse_num("-"), None);
        assert_eq!(parse_num("1e999"), None);
        assert_eq!(fmt_num(4.0), "4");
        assert_eq!(fmt_num(4.26), "4.3");
    }

    #[test]
    fn item_ids_round_trip() {
        for s in SECS {
            for n in [0, 1, 6, 99] {
                assert_eq!(split_id(iid(s, n)), Some((s, n)));
            }
        }
        assert_eq!(split_id(POP_PALETTE), None, "popup items are not panel items");
    }
}
