//! Advanced Image Paste: what the user can do to a picture before it is saved, as plain data.
//!
//! [`EditParams`] describes every adjustment; [`apply`] runs them on a picture in a fixed order
//! (crop, background removal, flip and rotate, resize, background fill, rounded corners, outline,
//! then padding and shadow on a larger canvas). The original is never changed: the same
//! parameters can be applied to a small copy for the live preview and to the full picture when the
//! result is saved. [`Presets`] keeps named parameter sets in a small text file.

use crate::image::Rgba;
use crate::imgfx;

/// Colour, straight RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const WHITE: Color = Color([255, 255, 255, 255]);
    pub const BLACK: Color = Color([0, 0, 0, 255]);
    pub const NONE: Color = Color([255, 255, 255, 0]);

    /// "#RRGGBB", or "#RRGGBBAA" when not fully opaque.
    pub fn hex(&self) -> String {
        let c = self.0;
        if c[3] == 255 {
            format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3])
        }
    }

    /// Reads "#RGB", "#RRGGBB" or "#RRGGBBAA" (the # is optional).
    pub fn parse(text: &str) -> Option<Color> {
        let t = text.trim().trim_start_matches('#');
        if !t.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
        match t.len() {
            3 => {
                let n = |i: usize| u8::from_str_radix(&t[i..i + 1], 16).ok().map(|v| v * 17);
                Some(Color([n(0)?, n(1)?, n(2)?, 255]))
            }
            6 => Some(Color([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Color([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeMode {
    /// Keep the size.
    Off,
    Pixels,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgMode {
    /// Leave the background as it is.
    Keep,
    /// Put a colour behind transparent parts.
    Fill,
    /// Make the background (the colour touching the edges) transparent.
    Remove,
    /// Remove the background, then put a colour there.
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    WebP,
    Bmp,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Png, Format::Jpeg, Format::WebP, Format::Bmp];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::WebP => "webp",
            Format::Bmp => "bmp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPG",
            Format::WebP => "WebP",
            Format::Bmp => "BMP",
        }
    }

    /// Can the file hold transparency?
    pub fn has_alpha(self) -> bool {
        matches!(self, Format::Png | Format::WebP)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngLevel {
    Fast,
    Balanced,
    Smallest,
}

/// A rectangle of the original picture, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditParams {
    pub crop: Option<CropRect>,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Clockwise degrees, -180 .. 180.
    pub angle: f32,
    pub resize_mode: ResizeMode,
    /// Pixels, or percent in [`ResizeMode::Percent`].
    pub resize_w: f32,
    pub resize_h: f32,
    pub keep_aspect: bool,
    pub bg_mode: BgMode,
    pub bg_color: Color,
    /// The colour to remove; `None` takes the colour of the picture's corners.
    pub bg_key: Option<Color>,
    /// 0 .. 100
    pub bg_tolerance: f32,
    pub round_on: bool,
    pub radius: f32,
    pub outline_on: bool,
    pub outline_color: Color,
    pub outline_width: f32,
    pub pad_on: bool,
    /// Top, right, bottom, left.
    pub pad: [f32; 4],
    pub pad_color: Color,
    pub shadow_on: bool,
    pub shadow_color: Color,
    /// 0 .. 100
    pub shadow_opacity: f32,
    pub shadow_blur: f32,
    pub shadow_distance: f32,
    /// Degrees clockwise from "to the right": 90 is straight down.
    pub shadow_angle: f32,
    pub format: Format,
    /// JPEG quality, 1 .. 100.
    pub quality: u8,
    pub png_level: PngLevel,
}

impl Default for EditParams {
    fn default() -> Self {
        EditParams {
            crop: None,
            flip_h: false,
            flip_v: false,
            angle: 0.0,
            resize_mode: ResizeMode::Off,
            resize_w: 800.0,
            resize_h: 600.0,
            keep_aspect: true,
            bg_mode: BgMode::Keep,
            bg_color: Color::WHITE,
            bg_key: None,
            bg_tolerance: 30.0,
            round_on: false,
            radius: 24.0,
            outline_on: false,
            outline_color: Color::BLACK,
            outline_width: 4.0,
            pad_on: false,
            pad: [32.0; 4],
            pad_color: Color::WHITE,
            shadow_on: false,
            shadow_color: Color::BLACK,
            shadow_opacity: 45.0,
            shadow_blur: 24.0,
            shadow_distance: 12.0,
            shadow_angle: 70.0,
            format: Format::Png,
            quality: 90,
            png_level: PngLevel::Balanced,
        }
    }
}

/// Output limits: a result larger than this is refused rather than allowed to exhaust memory.
pub const MAX_OUTPUT_PIXELS: u64 = 40_000_000;
pub const MAX_OUTPUT_SIDE: u32 = 16_384;
/// The longest side of the picture the live preview works on.
pub const PREVIEW_SIDE: u32 = 1400;

fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_nan() {
        lo
    } else {
        v.clamp(lo, hi)
    }
}

impl EditParams {
    /// Brings every value into its allowed range (after typing, loading a preset, scaling).
    pub fn sanitize(&mut self) {
        self.angle = clampf(self.angle, -180.0, 180.0);
        let max = if self.resize_mode == ResizeMode::Percent { 1000.0 } else { 40_000.0 };
        self.resize_w = clampf(self.resize_w, 1.0, max);
        self.resize_h = clampf(self.resize_h, 1.0, max);
        self.bg_tolerance = clampf(self.bg_tolerance, 0.0, 100.0);
        self.radius = clampf(self.radius, 0.0, 1.0e6);
        self.outline_width = clampf(self.outline_width, 0.0, 400.0);
        for p in &mut self.pad {
            *p = clampf(*p, 0.0, 4000.0);
        }
        self.shadow_opacity = clampf(self.shadow_opacity, 0.0, 100.0);
        self.shadow_blur = clampf(self.shadow_blur, 0.0, 400.0);
        self.shadow_distance = clampf(self.shadow_distance, 0.0, 1000.0);
        self.shadow_angle = clampf(self.shadow_angle, -360.0, 360.0);
        self.quality = self.quality.clamp(1, 100);
        if let Some(c) = &mut self.crop {
            c.x = clampf(c.x, 0.0, 1.0e6);
            c.y = clampf(c.y, 0.0, 1.0e6);
            c.w = clampf(c.w, 1.0, 1.0e6);
            c.h = clampf(c.h, 1.0, 1.0e6);
        }
    }

    /// The same adjustments for a picture `k` times as large: every length is scaled.
    pub fn scaled(&self, k: f32) -> EditParams {
        let mut p = self.clone();
        if let Some(c) = &mut p.crop {
            c.x *= k;
            c.y *= k;
            c.w *= k;
            c.h *= k;
        }
        if p.resize_mode == ResizeMode::Pixels {
            p.resize_w *= k;
            p.resize_h *= k;
        }
        p.radius *= k;
        p.outline_width = if p.outline_width > 0.0 { (p.outline_width * k).max(1.0) } else { 0.0 };
        for v in &mut p.pad {
            *v *= k;
        }
        p.shadow_blur *= k;
        p.shadow_distance *= k;
        p
    }

    fn crop_px(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        match self.crop {
            None => (0, 0, w, h),
            Some(c) => {
                let x = (c.x.round().max(0.0) as u32).min(w.saturating_sub(1));
                let y = (c.y.round().max(0.0) as u32).min(h.saturating_sub(1));
                let cw = (c.w.round().max(1.0) as u32).min(w - x).max(1);
                let ch = (c.h.round().max(1.0) as u32).min(h - y).max(1);
                (x, y, cw, ch)
            }
        }
    }

    /// Is the picture left as it is (only the file format may differ)?
    pub fn is_identity(&self) -> bool {
        self.crop.is_none()
            && !self.flip_h
            && !self.flip_v
            && self.angle == 0.0
            && self.resize_mode == ResizeMode::Off
            && self.bg_mode == BgMode::Keep
            && !self.round_on
            && !self.outline_on
            && !self.pad_on
            && !self.shadow_on
    }
}

/// Every size the pipeline passes through for a given picture and parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub crop: (u32, u32, u32, u32),
    pub rotated: (u32, u32),
    pub resized: (u32, u32),
    pub radius: f32,
    pub outline: u32,
    pub shadow: Option<imgfx::ShadowSpec>,
    /// Left, top, right, bottom.
    pub margins: [u32; 4],
    pub fill: Option<[u8; 4]>,
    pub out: (u32, u32),
}

pub fn plan(src_w: u32, src_h: u32, p: &EditParams) -> Plan {
    let crop = p.crop_px(src_w.max(1), src_h.max(1));
    let rotated = imgfx::rotated_size(crop.2, crop.3, p.angle);
    let resized = match p.resize_mode {
        ResizeMode::Off => rotated,
        ResizeMode::Pixels => {
            let w = p.resize_w.round().max(1.0);
            let h = if p.keep_aspect { (w * rotated.1 as f32 / rotated.0.max(1) as f32).round().max(1.0) } else { p.resize_h.round().max(1.0) };
            (w.min(65_536.0) as u32, h.min(65_536.0) as u32)
        }
        ResizeMode::Percent => {
            let sx = p.resize_w / 100.0;
            let sy = if p.keep_aspect { sx } else { p.resize_h / 100.0 };
            (
                (rotated.0 as f32 * sx).round().clamp(1.0, 65_536.0) as u32,
                (rotated.1 as f32 * sy).round().clamp(1.0, 65_536.0) as u32,
            )
        }
    };
    let radius = if p.round_on { p.radius.max(0.0) } else { 0.0 };
    let outline = if p.outline_on { p.outline_width.round().clamp(0.0, 400.0) as u32 } else { 0 };
    let content = (resized.0 + 2 * outline, resized.1 + 2 * outline);
    let mut need = [0u32; 4];
    let shadow = p.shadow_on.then(|| {
        let a = p.shadow_angle.to_radians();
        imgfx::ShadowSpec {
            color: p.shadow_color.0,
            opacity: p.shadow_opacity / 100.0,
            blur: p.shadow_blur.max(0.0),
            dx: (p.shadow_distance * a.cos()).round() as i32,
            dy: (p.shadow_distance * a.sin()).round() as i32,
        }
    });
    if let Some(s) = &shadow {
        let e = (1.5 * s.blur).ceil() as i32;
        need = [(e - s.dx).max(0) as u32, (e - s.dy).max(0) as u32, (e + s.dx).max(0) as u32, (e + s.dy).max(0) as u32];
    }
    let mut margins = need;
    if p.pad_on {
        let pm = [p.pad[3], p.pad[0], p.pad[1], p.pad[2]].map(|v| v.round().max(0.0) as u32);
        for i in 0..4 {
            margins[i] = margins[i].max(pm[i]);
        }
    }
    let fill = p.pad_on.then_some(p.pad_color.0);
    let out = (content.0 + margins[0] + margins[2], content.1 + margins[1] + margins[3]);
    Plan { crop, rotated, resized, radius, outline, shadow, margins, fill, out }
}

/// Size of the finished picture.
pub fn output_size(src_w: u32, src_h: u32, p: &EditParams) -> (u32, u32) {
    plan(src_w, src_h, p).out
}

/// Why a result cannot be made, if so.
pub fn too_large(out: (u32, u32)) -> Option<String> {
    if out.0 > MAX_OUTPUT_SIDE || out.1 > MAX_OUTPUT_SIDE || u64::from(out.0) * u64::from(out.1) > MAX_OUTPUT_PIXELS {
        Some(format!("{} x {} is too large (at most {} pixels a side, {} million in all)", out.0, out.1, MAX_OUTPUT_SIDE, MAX_OUTPUT_PIXELS / 1_000_000))
    } else {
        None
    }
}

/// The adjusted picture.
pub fn apply(src: &Rgba, p: &EditParams) -> Rgba {
    let pl = plan(src.width, src.height, p);
    let (x, y, w, h) = pl.crop;
    let mut img = imgfx::crop(src, x, y, w, h);
    if matches!(p.bg_mode, BgMode::Remove | BgMode::Replace) {
        imgfx::remove_background(&mut img, p.bg_key.map(|c| [c.0[0], c.0[1], c.0[2]]), p.bg_tolerance);
    }
    if p.flip_h {
        imgfx::flip_horizontal(&mut img);
    }
    if p.flip_v {
        imgfx::flip_vertical(&mut img);
    }
    if p.angle != 0.0 {
        img = imgfx::rotate(&img, p.angle);
    }
    if pl.resized != (img.width, img.height) {
        img = imgfx::resize(&img, pl.resized.0, pl.resized.1);
    }
    if matches!(p.bg_mode, BgMode::Fill | BgMode::Replace) {
        imgfx::flatten(&mut img, p.bg_color.0);
    }
    if pl.radius > 0.0 {
        imgfx::round_corners(&mut img, pl.radius);
    }
    if pl.outline > 0 {
        img = imgfx::with_outline(&img, p.outline_color.0, pl.outline, pl.radius);
    }
    if pl.margins != [0; 4] || pl.shadow.is_some() {
        img = imgfx::compose(&img, pl.margins, pl.fill, pl.shadow.as_ref());
    }
    img
}

/// A reduced copy to work on for the live preview, and the factor it was reduced by.
pub fn preview_source(src: &Rgba) -> (Rgba, f32) {
    let side = src.width.max(src.height);
    if side <= PREVIEW_SIDE {
        return (src.clone(), 1.0);
    }
    let k = PREVIEW_SIDE as f32 / side as f32;
    let nw = ((src.width as f32 * k).round() as u32).max(1);
    let nh = ((src.height as f32 * k).round() as u32).max(1);
    (imgfx::resize(src, nw, nh), nw as f32 / src.width as f32)
}

// ---------------------------------------------------------------------------- presets

/// Named sets of adjustments, and the folder last saved to, in a small text file: one preset per
/// line (`name TAB key=value ...`), `@folder TAB path` for the folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Presets {
    pub items: Vec<(String, EditParams)>,
    pub folder: Option<String>,
}

pub const PRESET_NAME_MAX: usize = 40;
pub const PRESETS_MAX: usize = 40;

fn b(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

fn pairs(p: &EditParams) -> Vec<(&'static str, String)> {
    let mode = match p.resize_mode {
        ResizeMode::Off => "off",
        ResizeMode::Pixels => "px",
        ResizeMode::Percent => "pct",
    };
    let bg = match p.bg_mode {
        BgMode::Keep => "keep",
        BgMode::Fill => "fill",
        BgMode::Remove => "remove",
        BgMode::Replace => "replace",
    };
    let level = match p.png_level {
        PngLevel::Fast => "fast",
        PngLevel::Balanced => "balanced",
        PngLevel::Smallest => "smallest",
    };
    let hex = |c: Color| c.hex().trim_start_matches('#').to_owned();
    let mut v = vec![
        ("flip_h", b(p.flip_h).to_owned()),
        ("flip_v", b(p.flip_v).to_owned()),
        ("angle", p.angle.to_string()),
        ("resize", mode.to_owned()),
        ("rw", p.resize_w.to_string()),
        ("rh", p.resize_h.to_string()),
        ("keep", b(p.keep_aspect).to_owned()),
        ("bg", bg.to_owned()),
        ("bgc", hex(p.bg_color)),
        ("tol", p.bg_tolerance.to_string()),
        ("round", b(p.round_on).to_owned()),
        ("radius", p.radius.to_string()),
        ("outline", b(p.outline_on).to_owned()),
        ("oc", hex(p.outline_color)),
        ("ow", p.outline_width.to_string()),
        ("pad", b(p.pad_on).to_owned()),
        ("pt", p.pad[0].to_string()),
        ("pr", p.pad[1].to_string()),
        ("pb", p.pad[2].to_string()),
        ("pl", p.pad[3].to_string()),
        ("pc", hex(p.pad_color)),
        ("shadow", b(p.shadow_on).to_owned()),
        ("sc", hex(p.shadow_color)),
        ("so", p.shadow_opacity.to_string()),
        ("sb", p.shadow_blur.to_string()),
        ("sd", p.shadow_distance.to_string()),
        ("sa", p.shadow_angle.to_string()),
        ("fmt", p.format.ext().to_owned()),
        ("q", p.quality.to_string()),
        ("png", level.to_owned()),
    ];
    if let Some(k) = p.bg_key {
        v.push(("key", hex(k)));
    }
    v
}

fn set_pair(p: &mut EditParams, k: &str, v: &str) {
    let num = |v: &str| v.parse::<f32>().ok();
    let flag = |v: &str| v == "1";
    match k {
        "flip_h" => p.flip_h = flag(v),
        "flip_v" => p.flip_v = flag(v),
        "angle" => p.angle = num(v).unwrap_or(0.0),
        "resize" => {
            p.resize_mode = match v {
                "px" => ResizeMode::Pixels,
                "pct" => ResizeMode::Percent,
                _ => ResizeMode::Off,
            }
        }
        "rw" => p.resize_w = num(v).unwrap_or(p.resize_w),
        "rh" => p.resize_h = num(v).unwrap_or(p.resize_h),
        "keep" => p.keep_aspect = flag(v),
        "bg" => {
            p.bg_mode = match v {
                "fill" => BgMode::Fill,
                "remove" => BgMode::Remove,
                "replace" => BgMode::Replace,
                _ => BgMode::Keep,
            }
        }
        "bgc" => p.bg_color = Color::parse(v).unwrap_or(p.bg_color),
        "key" => p.bg_key = Color::parse(v),
        "tol" => p.bg_tolerance = num(v).unwrap_or(p.bg_tolerance),
        "round" => p.round_on = flag(v),
        "radius" => p.radius = num(v).unwrap_or(p.radius),
        "outline" => p.outline_on = flag(v),
        "oc" => p.outline_color = Color::parse(v).unwrap_or(p.outline_color),
        "ow" => p.outline_width = num(v).unwrap_or(p.outline_width),
        "pad" => p.pad_on = flag(v),
        "pt" => p.pad[0] = num(v).unwrap_or(p.pad[0]),
        "pr" => p.pad[1] = num(v).unwrap_or(p.pad[1]),
        "pb" => p.pad[2] = num(v).unwrap_or(p.pad[2]),
        "pl" => p.pad[3] = num(v).unwrap_or(p.pad[3]),
        "pc" => p.pad_color = Color::parse(v).unwrap_or(p.pad_color),
        "shadow" => p.shadow_on = flag(v),
        "sc" => p.shadow_color = Color::parse(v).unwrap_or(p.shadow_color),
        "so" => p.shadow_opacity = num(v).unwrap_or(p.shadow_opacity),
        "sb" => p.shadow_blur = num(v).unwrap_or(p.shadow_blur),
        "sd" => p.shadow_distance = num(v).unwrap_or(p.shadow_distance),
        "sa" => p.shadow_angle = num(v).unwrap_or(p.shadow_angle),
        "fmt" => {
            if let Some(f) = Format::ALL.iter().find(|f| f.ext() == v) {
                p.format = *f;
            }
        }
        "q" => p.quality = v.parse().unwrap_or(p.quality),
        "png" => {
            p.png_level = match v {
                "fast" => PngLevel::Fast,
                "smallest" => PngLevel::Smallest,
                _ => PngLevel::Balanced,
            }
        }
        _ => {}
    }
}

/// Is this a usable preset name (no control characters, not blank)?
pub fn clean_preset_name(name: &str) -> Option<String> {
    let n: String = name.chars().filter(|c| !c.is_control()).collect();
    let n = n.trim().chars().take(PRESET_NAME_MAX).collect::<String>();
    (!n.trim().is_empty()).then_some(n)
}

impl Presets {
    pub fn parse(text: &str) -> Presets {
        let mut out = Presets::default();
        for line in text.lines() {
            let Some((name, rest)) = line.split_once('\t') else { continue };
            if name == "@folder" {
                let f = rest.trim();
                if !f.is_empty() {
                    out.folder = Some(f.to_owned());
                }
                continue;
            }
            let Some(name) = clean_preset_name(name) else { continue };
            if out.items.len() >= PRESETS_MAX || out.items.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let mut p = EditParams::default();
            for kv in rest.split_whitespace() {
                if let Some((k, v)) = kv.split_once('=') {
                    set_pair(&mut p, k, v);
                }
            }
            p.sanitize();
            out.items.push((name, p));
        }
        out
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        if let Some(f) = &self.folder {
            if !f.contains(['\n', '\t']) {
                s.push_str(&format!("@folder\t{f}\n"));
            }
        }
        for (name, p) in &self.items {
            let kv: Vec<String> = pairs(p).into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            s.push_str(&format!("{name}\t{}\n", kv.join(" ")));
        }
        s
    }

    /// Adds or replaces a preset. `false` when the name is unusable or the list is full.
    pub fn save(&mut self, name: &str, p: &EditParams) -> bool {
        let Some(name) = clean_preset_name(name) else { return false };
        // a preset is a way of working, not one picture: the crop is not part of it
        let mut p = p.clone();
        p.crop = None;
        if let Some(slot) = self.items.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = p;
        } else if self.items.len() < PRESETS_MAX {
            self.items.push((name, p));
        } else {
            return false;
        }
        true
    }

    pub fn remove(&mut self, name: &str) {
        self.items.retain(|(n, _)| n != name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Rgba {
        Rgba { width: w, height: h, pixels: c.iter().copied().cycle().take(w as usize * h as usize * 4).collect() }
    }

    #[test]
    fn colours_read_and_write_as_hex() {
        assert_eq!(Color::parse("#ff8000"), Some(Color([255, 128, 0, 255])));
        assert_eq!(Color::parse("f80"), Some(Color([255, 136, 0, 255])));
        assert_eq!(Color::parse("#11223380"), Some(Color([17, 34, 51, 128])));
        assert_eq!(Color::parse("#12"), None);
        assert_eq!(Color::parse("#gggggg"), None);
        assert_eq!(Color([255, 128, 0, 255]).hex(), "#FF8000");
        assert_eq!(Color([1, 2, 3, 4]).hex(), "#01020304");
    }

    #[test]
    fn with_nothing_set_the_picture_comes_back_unchanged() {
        let img = solid(7, 5, [1, 2, 3, 255]);
        let p = EditParams::default();
        assert!(p.is_identity());
        assert_eq!(apply(&img, &p), img);
        assert_eq!(output_size(7, 5, &p), (7, 5));
    }

    #[test]
    fn every_stage_changes_the_size_the_way_the_plan_says() {
        let img = solid(100, 50, [10, 20, 30, 255]);
        let mut p = EditParams::default();
        p.crop = Some(CropRect { x: 10.0, y: 5.0, w: 60.0, h: 40.0 });
        assert_eq!(apply(&img, &p).width, 60);
        p.resize_mode = ResizeMode::Pixels;
        p.resize_w = 30.0;
        p.keep_aspect = true;
        assert_eq!(output_size(100, 50, &p), (30, 20));
        p.keep_aspect = false;
        p.resize_h = 7.0;
        assert_eq!(output_size(100, 50, &p), (30, 7));
        p.resize_mode = ResizeMode::Percent;
        p.resize_w = 50.0;
        p.keep_aspect = true;
        assert_eq!(output_size(100, 50, &p), (30, 20));
        p.angle = 90.0;
        assert_eq!(output_size(100, 50, &p), (20, 30));
        p.outline_on = true;
        p.outline_width = 3.0;
        assert_eq!(output_size(100, 50, &p), (26, 36));
        p.pad_on = true;
        p.pad = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(output_size(100, 50, &p), (32, 40));
        // the picture really has the size the plan promised
        assert_eq!({ let r = apply(&img, &p); (r.width, r.height) }, (32, 40));
    }

    #[test]
    fn a_shadow_makes_room_for_itself() {
        let img = solid(40, 40, [0, 0, 255, 255]);
        let mut p = EditParams::default();
        p.shadow_on = true;
        p.shadow_blur = 0.0;
        p.shadow_distance = 10.0;
        p.shadow_angle = 0.0; // to the right
        let pl = plan(40, 40, &p);
        assert_eq!(pl.margins, [0, 0, 10, 0]);
        assert_eq!(pl.out, (50, 40));
        p.shadow_angle = 90.0; // down
        assert_eq!(plan(40, 40, &p).margins, [0, 0, 0, 10]);
        p.shadow_blur = 10.0;
        let m = plan(40, 40, &p).margins;
        assert_eq!(m, [15, 5, 15, 25]);
        let out = apply(&img, &p);
        assert_eq!((out.width, out.height), (70, 70));
        // padding bigger than the shadow needs wins
        p.pad_on = true;
        p.pad = [40.0; 4];
        assert_eq!(plan(40, 40, &p).margins, [40; 4]);
    }

    #[test]
    fn a_preview_copy_gives_the_same_picture_in_proportion() {
        let mut src = solid(2800, 1400, [200, 100, 50, 255]);
        // a mark so the picture is not flat
        for y in 0..200usize {
            for x in 0..400usize {
                src.pixels[(y * 2800 + x) * 4..][..3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let mut p = EditParams::default();
        p.outline_on = true;
        p.outline_width = 40.0;
        p.round_on = true;
        p.radius = 100.0;
        p.pad_on = true;
        p.pad = [80.0; 4];
        p.shadow_on = true;
        p.crop = Some(CropRect { x: 100.0, y: 100.0, w: 2000.0, h: 1000.0 });
        let full = output_size(2800, 1400, &p);
        let (small, k) = preview_source(&src);
        assert!(small.width <= PREVIEW_SIDE && small.height <= PREVIEW_SIDE);
        assert!((k - 0.5).abs() < 0.001);
        let scaled = apply(&small, &p.scaled(k));
        let (ew, eh) = (full.0 as f32 * k, full.1 as f32 * k);
        assert!((scaled.width as f32 - ew).abs() <= 3.0 && (scaled.height as f32 - eh).abs() <= 3.0, "{} x {} vs {ew} x {eh}", scaled.width, scaled.height);
    }

    #[test]
    fn the_original_is_never_touched() {
        let img = solid(20, 20, [5, 6, 7, 255]);
        let before = img.clone();
        let mut p = EditParams::default();
        p.flip_h = true;
        p.angle = 33.0;
        p.round_on = true;
        p.bg_mode = BgMode::Replace;
        let _ = apply(&img, &p);
        assert_eq!(img, before);
    }

    #[test]
    fn values_are_kept_in_range() {
        let mut p = EditParams::default();
        p.angle = 1000.0;
        p.resize_w = -5.0;
        p.outline_width = f32::NAN;
        p.pad = [-1.0, 1.0e9, 3.0, 4.0];
        p.quality = 0;
        p.shadow_opacity = 500.0;
        p.sanitize();
        assert_eq!(p.angle, 180.0);
        assert_eq!(p.resize_w, 1.0);
        assert_eq!(p.outline_width, 0.0);
        assert_eq!(p.pad[0], 0.0);
        assert_eq!(p.pad[1], 4000.0);
        assert_eq!(p.quality, 1);
        assert_eq!(p.shadow_opacity, 100.0);
    }

    #[test]
    fn too_large_results_are_refused() {
        assert!(too_large((1000, 1000)).is_none());
        assert!(too_large((20_000, 10)).is_some());
        assert!(too_large((10_000, 10_000)).is_some());
    }

    #[test]
    fn presets_round_trip_and_never_keep_a_crop() {
        let mut presets = Presets::default();
        let mut p = EditParams::default();
        p.outline_on = true;
        p.outline_color = Color([255, 0, 128, 255]);
        p.outline_width = 6.0;
        p.shadow_on = true;
        p.shadow_angle = 135.0;
        p.bg_mode = BgMode::Replace;
        p.bg_key = Some(Color([1, 2, 3, 255]));
        p.format = Format::Jpeg;
        p.quality = 77;
        p.png_level = PngLevel::Smallest;
        p.crop = Some(CropRect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 });
        assert!(presets.save("Blog header", &p));
        presets.folder = Some("C:\\Users\\me\\Pictures".into());
        assert!(!presets.save("   ", &p), "a blank name is refused");
        let text = presets.to_text();
        let back = Presets::parse(&text);
        assert_eq!(back.folder.as_deref(), Some("C:\\Users\\me\\Pictures"));
        assert_eq!(back.items.len(), 1);
        let (name, q) = &back.items[0];
        assert_eq!(name, "Blog header");
        assert_eq!(q.crop, None);
        p.crop = None;
        assert_eq!(*q, p);
        // saving under the same name replaces it
        let mut again = back.clone();
        p.outline_width = 9.0;
        assert!(again.save("Blog header", &p));
        assert_eq!(again.items.len(), 1);
        assert_eq!(again.items[0].1.outline_width, 9.0);
        again.remove("Blog header");
        assert!(again.items.is_empty());
    }

    #[test]
    fn a_damaged_presets_file_loads_what_it_can() {
        let text = "no tab here\nOk\toutline=1 ow=999999 junk=1 fmt=webp\n\tnoname=1\nOk\tshadow=1\n@folder\t\n";
        let p = Presets::parse(text);
        assert_eq!(p.items.len(), 1, "a repeated name is ignored");
        assert!(p.items[0].1.outline_on);
        assert_eq!(p.items[0].1.outline_width, 400.0, "out-of-range values are brought back");
        assert_eq!(p.items[0].1.format, Format::WebP);
        assert_eq!(p.folder, None);
    }

    #[test]
    fn preset_names_are_cleaned() {
        assert_eq!(clean_preset_name("  Hello\tworld\n "), Some("Helloworld".to_owned()));
        assert_eq!(clean_preset_name(""), None);
        assert_eq!(clean_preset_name(&"x".repeat(100)).map(|s| s.len()), Some(PRESET_NAME_MAX));
    }
}
