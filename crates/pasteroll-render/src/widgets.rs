//! Small anti-aliased shapes for native windows that draw their own controls (the Windows settings
//! window): rounded boxes, check boxes, radio buttons and slider parts. Each call returns a
//! premultiplied BGRA bitmap with transparent corners, ready for an alpha blit onto whatever is
//! behind it. Sizes are in device pixels; `scale` is device pixels per DIP.

use super::{paint, Canvas, Rect};
use tiny_skia::{FillRule, Pixmap, PathBuilder, Stroke, Transform};

type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A rounded rectangle, filled and/or outlined (`radius` in DIP).
    Box { radius: f32, fill: Option<Rgb>, border: Option<Rgb> },
    /// A check box filling the bitmap.
    Checkbox { checked: bool, accent: Rgb, border: Rgb, tick: Rgb },
    /// A radio button filling the bitmap.
    Radio { checked: bool, accent: Rgb, border: Rgb, dot: Rgb },
    /// The slider channel: a pill, `filled` of it (0 to 1) in the accent colour.
    Track { filled: f32, accent: Rgb, rest: Rgb },
    /// The slider handle: a disc with a ring.
    Thumb { accent: Rgb, ring: Rgb },
}

/// The shape as a `w` x `h` pixel bitmap (premultiplied BGRA, top-down).
pub fn render(shape: Shape, w: u32, h: u32, scale: f32) -> Vec<u8> {
    let Some(pm) = Pixmap::new(w.max(1), h.max(1)) else { return vec![0; (w * h * 4) as usize] };
    let mut cv = Canvas { pm, s: scale };
    let (dw, dh) = (w as f32 / scale, h as f32 / scale);
    let whole = Rect { x: 0.0, y: 0.0, w: dw, h: dh };
    match shape {
        Shape::Box { radius, fill, border } => {
            if let Some(f) = fill {
                cv.fill(whole, radius, f, 1.0);
            }
            if let Some(b) = border {
                cv.stroke(whole.inset(0.5), (radius - 0.5).max(0.0), b, 1.0, 1.0);
            }
        }
        Shape::Checkbox { checked, accent, border, tick } => {
            let r = whole.inset(0.5);
            if checked {
                cv.fill(whole, 4.0, accent, 1.0);
                let (cx, cy) = (dw / 2.0, dh / 2.0);
                let k = dw.min(dh) / 16.0;
                cv.line(&[(cx - 3.6 * k, cy + 0.2 * k), (cx - 1.0 * k, cy + 2.8 * k), (cx + 3.8 * k, cy - 2.8 * k)], tick, 1.7);
            } else {
                cv.stroke(r, 3.5, border, 1.0, 1.0);
            }
        }
        Shape::Radio { checked, accent, border, dot } => {
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let rad = w.min(h) as f32 / 2.0;
            let s = scale;
            let mut ring = PathBuilder::new();
            ring.push_circle(cx, cy, rad - 0.5 * s);
            if let Some(p) = ring.finish() {
                if checked {
                    cv.pm.fill_path(&p, &paint(accent, 1.0), FillRule::Winding, Transform::identity(), None);
                } else {
                    let st = Stroke { width: s, ..Stroke::default() };
                    cv.pm.stroke_path(&p, &paint(border, 1.0), &st, Transform::identity(), None);
                }
            }
            if checked {
                let mut d = PathBuilder::new();
                d.push_circle(cx, cy, rad * 0.36);
                if let Some(p) = d.finish() {
                    cv.pm.fill_path(&p, &paint(dot, 1.0), FillRule::Winding, Transform::identity(), None);
                }
            }
        }
        Shape::Track { filled, accent, rest } => {
            let radius = dh / 2.0;
            cv.fill(whole, radius, rest, 1.0);
            let done = (dw * filled.clamp(0.0, 1.0)).max(0.0);
            if done > 0.5 {
                cv.fill(Rect { x: 0.0, y: 0.0, w: done.max(dh), h: dh }, radius, accent, 1.0);
            }
        }
        Shape::Thumb { accent, ring } => {
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let rad = w.min(h) as f32 / 2.0;
            let mut outer = PathBuilder::new();
            outer.push_circle(cx, cy, rad);
            if let Some(p) = outer.finish() {
                cv.pm.fill_path(&p, &paint(ring, 1.0), FillRule::Winding, Transform::identity(), None);
            }
            let mut inner = PathBuilder::new();
            inner.push_circle(cx, cy, rad * 0.62);
            if let Some(p) = inner.finish() {
                cv.pm.fill_path(&p, &paint(accent, 1.0), FillRule::Winding, Transform::identity(), None);
            }
        }
    }
    // tiny-skia is premultiplied RGBA; Windows wants BGRA
    let mut data = cv.pm.take();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENT: Rgb = (47, 174, 151);

    fn px(b: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [b[i], b[i + 1], b[i + 2], b[i + 3]]
    }

    #[test]
    fn a_box_has_clear_corners_and_a_solid_middle() {
        let b = render(Shape::Box { radius: 8.0, fill: Some((255, 0, 0)), border: None }, 40, 40, 1.0);
        assert_eq!(px(&b, 40, 0, 0)[3], 0, "the corner is transparent");
        assert_eq!(px(&b, 40, 20, 20), [0, 0, 255, 255], "BGRA red in the middle");
    }

    #[test]
    fn a_ticked_check_box_is_accent_and_an_empty_one_is_hollow() {
        let on = render(Shape::Checkbox { checked: true, accent: ACCENT, border: (120, 120, 120), tick: (255, 255, 255) }, 32, 32, 2.0);
        let off = render(Shape::Checkbox { checked: false, accent: ACCENT, border: (120, 120, 120), tick: (255, 255, 255) }, 32, 32, 2.0);
        assert_eq!(px(&on, 32, 4, 16), [151, 174, 47, 255], "filled with the accent");
        assert_eq!(px(&off, 32, 16, 16)[3], 0, "hollow inside");
        assert!(px(&off, 32, 16, 0)[3] > 0 || px(&off, 32, 16, 1)[3] > 0, "with an outline");
    }

    #[test]
    fn a_chosen_radio_has_a_dot_and_the_slider_track_fills_to_the_handle() {
        let r = render(Shape::Radio { checked: true, accent: ACCENT, border: (120, 120, 120), dot: (255, 255, 255) }, 32, 32, 2.0);
        assert_eq!(px(&r, 32, 16, 16), [255, 255, 255, 255], "white dot");
        assert_eq!(px(&r, 32, 16, 3)[..3], [151, 174, 47], "accent ring");
        let t = render(Shape::Track { filled: 0.5, accent: ACCENT, rest: (80, 80, 80) }, 100, 8, 2.0);
        assert_eq!(px(&t, 100, 25, 4)[..3], [151, 174, 47]);
        assert_eq!(px(&t, 100, 75, 4)[..3], [80, 80, 80]);
    }
}
