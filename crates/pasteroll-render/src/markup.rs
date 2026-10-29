//! Drawing the marks of the image editor (pen, brush, circle, arrow, text) onto a picture.
//!
//! The marks are plain data ([`Mark`], in pixels of the picture); [`Renderer::burn_marks`] paints
//! them onto a copy of the picture. The editor shows that copy while marking, and the saved
//! picture is made from the same call, so what is seen is what is saved. A text being typed is
//! drawn with its selection and caret ([`MarkEdit`]).

use super::{premultiplied, Renderer};
use cosmic_text::{Attrs, Buffer, Color as TextColor, Family, Metrics, Shaping, Wrap};
use pasteroll_core::image::Rgba;
use pasteroll_core::imgedit::{Mark, MarkKind};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// The text being typed: which mark, where the caret is and where the selection started (both are
/// character positions; equal means nothing is selected).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkEdit {
    pub index: usize,
    pub caret: usize,
    pub anchor: usize,
}

/// Teal, like the rest of the program: the selection and the caret.
const EDIT: (u8, u8, u8) = (47, 174, 151);

fn sk_paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn stroke_of(width: f32) -> Stroke {
    Stroke { width: width.max(0.5), line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() }
}

/// A smooth line through the points (each point is a control point between the midpoints).
fn smooth_path(pts: &[(f32, f32)]) -> Option<tiny_skia::Path> {
    let first = *pts.first()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.0, first.1);
    match pts.len() {
        1 => pb.line_to(first.0 + 0.01, first.1),
        2 => pb.line_to(pts[1].0, pts[1].1),
        n => {
            for i in 1..n - 1 {
                let mid = ((pts[i].0 + pts[i + 1].0) / 2.0, (pts[i].1 + pts[i + 1].1) / 2.0);
                pb.quad_to(pts[i].0, pts[i].1, mid.0, mid.1);
            }
            pb.line_to(pts[n - 1].0, pts[n - 1].1);
        }
    }
    pb.finish()
}

/// Source-over blend of a straight-alpha colour into a premultiplied pixmap.
fn blend_over(pm: &mut Pixmap, x: i32, y: i32, w: u32, h: u32, c: TextColor) {
    let a = u32::from(c.a());
    if a == 0 {
        return;
    }
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let inv = 255 - a;
    let data = pm.data_mut();
    for yy in y.max(0)..(y + h as i32).min(ph) {
        for xx in x.max(0)..(x + w as i32).min(pw) {
            let i = ((yy * pw + xx) * 4) as usize;
            let src = [u32::from(c.r()), u32::from(c.g()), u32::from(c.b())];
            for k in 0..3 {
                data[i + k] = (((src[k] * a + 127) / 255) + (u32::from(data[i + k]) * inv + 127) / 255).min(255) as u8;
            }
            data[i + 3] = (a + (u32::from(data[i + 3]) * inv + 127) / 255).min(255) as u8;
        }
    }
}

/// Which line of `text` the character `idx` is on, and its byte offset within that line.
fn locate(text: &str, idx: usize) -> (usize, usize) {
    let (mut line, mut start) = (0, 0);
    for (ci, (bi, ch)) in text.char_indices().enumerate() {
        if ci == idx {
            return (line, bi - start);
        }
        if ch == '\n' {
            line += 1;
            start = bi + 1;
        }
    }
    (line, text.len() - start)
}

impl Renderer {
    /// The font families, from `wanted`, that are installed (in the order of `wanted`).
    pub fn installed_fonts(&self, wanted: &[&str]) -> Vec<String> {
        let db = self.fs.db();
        wanted
            .iter()
            .filter(|w| db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(w))))
            .map(|w| (*w).to_owned())
            .collect()
    }

    fn text_buffer(&mut self, m: &Mark) -> Buffer {
        let mut buf = Buffer::new(&mut self.fs, Metrics::new(m.size.max(1.0), m.size.max(1.0) * 1.25));
        buf.set_wrap(Wrap::None);
        buf.set_size(None, None);
        let fam = if m.font.is_empty() { self.family.clone() } else { m.font.clone() };
        let base = if fam.is_empty() { Attrs::new() } else { Attrs::new().family(Family::Name(&fam)) };
        // one span for each stretch of characters of one colour
        let mut spans: Vec<(String, [u8; 4])> = Vec::new();
        for (i, ch) in m.text.chars().enumerate() {
            let c = m.colors.get(i).copied().unwrap_or(m.color);
            match spans.last_mut() {
                Some((s, sc)) if *sc == c => s.push(ch),
                _ => spans.push((ch.to_string(), c)),
            }
        }
        let attrs = |c: [u8; 4]| base.clone().color(TextColor::rgba(c[0], c[1], c[2], c[3]));
        buf.set_rich_text(spans.iter().map(|(s, c)| (s.as_str(), attrs(*c))), &base, Shaping::Advanced, None);
        buf.shape_until_scroll(&mut self.fs, false);
        buf
    }

    /// The size, in pixels of the picture, that the text of `m` takes.
    pub fn mark_text_size(&mut self, m: &Mark) -> (f32, f32) {
        let buf = self.text_buffer(m);
        let lh = m.size.max(1.0) * 1.25;
        let mut bottom = lh;
        let mut w = 0.0f32;
        for run in buf.layout_runs() {
            bottom = bottom.max(run.line_top + run.line_height);
            w = w.max(run.line_w);
        }
        let lines = m.text.split('\n').count() as f32;
        (w, bottom.max(lines * lh))
    }

    /// The character nearest to `(x, y)`, which is relative to the text's top left.
    pub fn mark_text_hit(&mut self, m: &Mark, x: f32, y: f32) -> usize {
        let buf = self.text_buffer(m);
        let total = m.text.chars().count();
        let Some(cur) = buf.hit(x, y) else {
            return if y < 0.0 { 0 } else { total };
        };
        let mut before = 0;
        for (i, line) in m.text.split('\n').enumerate() {
            if i == cur.line {
                let end = cur.index.min(line.len());
                let end = (0..=end).rev().find(|e| line.is_char_boundary(*e)).unwrap_or(0);
                return (before + line[..end].chars().count()).min(total);
            }
            before += line.chars().count() + 1;
        }
        total
    }

    /// Paints `marks` onto `img`.
    pub fn burn_marks(&mut self, img: &mut Rgba, marks: &[Mark], edit: Option<MarkEdit>) {
        if marks.is_empty() {
            return;
        }
        let Some(mut pm) = premultiplied(img) else { return };
        for (i, m) in marks.iter().enumerate() {
            self.draw_mark(&mut pm, m, edit.filter(|e| e.index == i));
        }
        // back to straight alpha
        for (px, out) in pm.data().chunks_exact(4).zip(img.pixels.chunks_exact_mut(4)) {
            let a = u32::from(px[3]);
            match a {
                0 => out.copy_from_slice(&[0, 0, 0, 0]),
                255 => out.copy_from_slice(px),
                _ => {
                    for k in 0..3 {
                        out[k] = ((u32::from(px[k]) * 255 + a / 2) / a).min(255) as u8;
                    }
                    out[3] = a as u8;
                }
            }
        }
    }

    fn draw_mark(&mut self, pm: &mut Pixmap, m: &Mark, edit: Option<MarkEdit>) {
        match m.kind {
            MarkKind::Pen => {
                if let Some(path) = smooth_path(&m.pts) {
                    pm.stroke_path(&path, &sk_paint(m.color), &stroke_of(m.size), Transform::identity(), None);
                }
            }
            MarkKind::Brush => {
                // soft: wide faint passes under narrower stronger ones
                let Some(path) = smooth_path(&m.pts) else { return };
                for (grow, alpha) in [(3.0f32, 0.16f32), (2.4, 0.2), (1.8, 0.28), (1.3, 0.45)] {
                    let mut c = m.color;
                    c[3] = (f32::from(c[3]) * alpha).round() as u8;
                    pm.stroke_path(&path, &sk_paint(c), &stroke_of(m.size * grow), Transform::identity(), None);
                }
            }
            MarkKind::Ellipse => {
                let (Some(a), Some(b)) = (m.pts.first(), m.pts.get(1)) else { return };
                let (l, r) = (a.0.min(b.0), a.0.max(b.0));
                let (t, bo) = (a.1.min(b.1), a.1.max(b.1));
                if r - l < 1.0 || bo - t < 1.0 {
                    return;
                }
                if let Some(rect) = tiny_skia::Rect::from_ltrb(l, t, r, bo) {
                    if let Some(path) = PathBuilder::from_oval(rect) {
                        pm.stroke_path(&path, &sk_paint(m.color), &stroke_of(m.size), Transform::identity(), None);
                    }
                }
            }
            MarkKind::Arrow => {
                let (Some(&(x0, y0)), Some(&(x1, y1))) = (m.pts.first(), m.pts.get(1)) else { return };
                let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
                if len < 1.0 {
                    return;
                }
                let (dx, dy) = ((x1 - x0) / len, (y1 - y0) / len);
                let head = (m.size * 4.5).max(10.0).min(len * 0.7);
                // the shaft stops inside the head so its round end does not poke past the tip
                let (sx, sy) = (x1 - dx * head * 0.5, y1 - dy * head * 0.5);
                let mut pb = PathBuilder::new();
                pb.move_to(x0, y0);
                pb.line_to(sx, sy);
                if let Some(path) = pb.finish() {
                    pm.stroke_path(&path, &sk_paint(m.color), &stroke_of(m.size), Transform::identity(), None);
                }
                let (px, py) = (-dy, dx);
                let (bx, by) = (x1 - dx * head, y1 - dy * head);
                let mut pb = PathBuilder::new();
                pb.move_to(x1, y1);
                pb.line_to(bx + px * head * 0.5, by + py * head * 0.5);
                pb.line_to(bx - px * head * 0.5, by - py * head * 0.5);
                pb.close();
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &sk_paint(m.color), FillRule::Winding, Transform::identity(), None);
                }
            }
            MarkKind::Text => self.draw_text_mark(pm, m, edit),
        }
    }

    fn draw_text_mark(&mut self, pm: &mut Pixmap, m: &Mark, edit: Option<MarkEdit>) {
        let Some(&(ox, oy)) = m.pts.first() else { return };
        let (ox, oy) = (ox.round() as i32, oy.round() as i32);
        let mut buf = self.text_buffer(m);
        let lh = m.size.max(1.0) * 1.25;
        if let Some(e) = edit {
            let (w, h) = self.mark_text_size(m);
            let pad = (m.size * 0.2).max(2.0);
            let line = (m.size / 18.0).max(1.0);
            // a thin frame, so an empty text can still be seen
            if let Some(rect) = tiny_skia::Rect::from_xywh(ox as f32 - pad, oy as f32 - pad * 0.5, (w + 2.0 * pad).max(lh * 0.6), h + pad) {
                let mut pb = PathBuilder::new();
                pb.push_rect(rect);
                if let Some(path) = pb.finish() {
                    let c = [EDIT.0, EDIT.1, EDIT.2, 200];
                    pm.stroke_path(&path, &sk_paint(c), &Stroke { width: line, ..Stroke::default() }, Transform::identity(), None);
                }
            }
            // the selection
            let (a, b) = (e.caret.min(e.anchor), e.caret.max(e.anchor));
            if a != b {
                let ((la, oa), (lb, ob)) = (locate(&m.text, a), locate(&m.text, b));
                for run in buf.layout_runs() {
                    if run.line_i < la || run.line_i > lb {
                        continue;
                    }
                    let from = if run.line_i == la { oa } else { 0 };
                    let to = if run.line_i == lb { ob } else { usize::MAX };
                    let (mut x0, mut x1) = (f32::MAX, f32::MIN);
                    for g in run.glyphs {
                        if g.start >= from && g.end <= to {
                            x0 = x0.min(g.x);
                            x1 = x1.max(g.x + g.w);
                        }
                    }
                    if run.line_i < lb {
                        // the line break itself
                        if x0 > x1 {
                            x0 = run.line_w;
                            x1 = run.line_w;
                        }
                        x1 += m.size * 0.3;
                    }
                    if x0 <= x1 {
                        if let Some(rect) = tiny_skia::Rect::from_xywh(ox as f32 + x0, oy as f32 + run.line_top, x1 - x0, run.line_height) {
                            let mut pb = PathBuilder::new();
                            pb.push_rect(rect);
                            if let Some(path) = pb.finish() {
                                let c = [EDIT.0, EDIT.1, EDIT.2, 110];
                                pm.fill_path(&path, &sk_paint(c), FillRule::Winding, Transform::identity(), None);
                            }
                        }
                    }
                }
            }
        }
        let default = TextColor::rgba(m.color[0], m.color[1], m.color[2], m.color[3]);
        buf.draw(&mut self.fs, &mut self.cache, default, |gx, gy, gw, gh, c| {
            blend_over(pm, ox + gx, oy + gy, gw, gh, c);
        });
        if let Some(e) = edit {
            let (line, off) = locate(&m.text, e.caret);
            let (mut x, mut top, mut h) = (0.0f32, line as f32 * lh, lh);
            for run in buf.layout_runs() {
                if run.line_i != line {
                    continue;
                }
                top = run.line_top;
                h = run.line_height;
                x = 0.0;
                let mut placed = false;
                for g in run.glyphs {
                    if off <= g.start {
                        x = g.x;
                        placed = true;
                        break;
                    }
                    x = g.x + g.w;
                }
                let _ = placed;
                break;
            }
            let width = (m.size / 14.0).max(1.5);
            if let Some(rect) = tiny_skia::Rect::from_xywh(ox as f32 + x - width / 2.0, oy as f32 + top, width, h) {
                let mut pb = PathBuilder::new();
                pb.push_rect(rect);
                if let Some(path) = pb.finish() {
                    let c = [EDIT.0, EDIT.1, EDIT.2, 255];
                    pm.fill_path(&path, &sk_paint(c), FillRule::Winding, Transform::identity(), None);
                }
            }
        }
    }
}
