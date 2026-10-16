//! Line icons for the card buttons and the viewer's toolbar, drawn on a 16 x 16 grid so they stay
//! crisp at any scale. No icon font, no image files.

use super::{paint, Canvas, Rect, Rgb};
use tiny_skia::{FillRule, PathBuilder, Stroke, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Expand,
    Fullscreen,
    ExitFullscreen,
    Trash,
    TrashForever,
    Restore,
    Copy,
    Paste,
    Star,
    StarFilled,
    Save,
    Prev,
    Next,
    Fit,
    ActualSize,
    ZoomIn,
    ZoomOut,
    Search,
    Wrap,
    Info,
    ExtractText,
    Close,
    More,
    ChevronUp,
    ChevronDown,
    Back,
}

/// Maps a point of the 16-unit design grid into `r` (DIP), centred, `size` units across.
struct Grid {
    x: f32,
    y: f32,
    k: f32,
}

impl Grid {
    fn new(r: Rect, size: f32) -> Grid {
        let k = size / 16.0;
        Grid { x: r.x + (r.w - size) / 2.0, y: r.y + (r.h - size) / 2.0, k }
    }
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.k, self.y + y * self.k)
    }
}

fn poly(cv: &mut Canvas, g: &Grid, pts: &[(f32, f32)], rgb: Rgb, width: f32) {
    let s = cv.s;
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        let (px, py) = g.p(*x, *y);
        if i == 0 {
            pb.move_to(px * s, py * s);
        } else {
            pb.line_to(px * s, py * s);
        }
    }
    if let Some(path) = pb.finish() {
        let st = Stroke {
            width: width * g.k * s,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Stroke::default()
        };
        cv.pm.stroke_path(&path, &paint(rgb, 1.0), &st, Transform::identity(), None);
    }
}

fn circle(cv: &mut Canvas, g: &Grid, cx: f32, cy: f32, r: f32, rgb: Rgb, width: Option<f32>) {
    let s = cv.s;
    let (px, py) = g.p(cx, cy);
    let mut pb = PathBuilder::new();
    pb.push_circle(px * s, py * s, r * g.k * s);
    if let Some(path) = pb.finish() {
        match width {
            Some(w) => {
                let st = Stroke { width: w * g.k * s, ..Stroke::default() };
                cv.pm.stroke_path(&path, &paint(rgb, 1.0), &st, Transform::identity(), None);
            }
            None => cv.pm.fill_path(&path, &paint(rgb, 1.0), FillRule::Winding, Transform::identity(), None),
        }
    }
}

/// Points along a circular arc (angles in degrees, 0 = east, clockwise on screen).
fn arc(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    let n = 18;
    (0..=n)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / n as f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

fn star(cv: &mut Canvas, g: &Grid, rgb: Rgb, filled: bool) {
    let s = cv.s;
    let mut pb = PathBuilder::new();
    for i in 0..10 {
        let ang = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let rad = if i % 2 == 0 { 7.0 } else { 3.0 };
        let (x, y) = g.p(8.0 + rad * ang.cos(), 8.6 + rad * ang.sin());
        if i == 0 {
            pb.move_to(x * s, y * s);
        } else {
            pb.line_to(x * s, y * s);
        }
    }
    pb.close();
    if let Some(p) = pb.finish() {
        if filled {
            cv.pm.fill_path(&p, &paint(rgb, 1.0), FillRule::Winding, Transform::identity(), None);
        } else {
            let st = Stroke { width: 1.4 * g.k * s, line_join: tiny_skia::LineJoin::Round, ..Stroke::default() };
            cv.pm.stroke_path(&p, &paint(rgb, 1.0), &st, Transform::identity(), None);
        }
    }
}

fn magnifier(cv: &mut Canvas, g: &Grid, rgb: Rgb) {
    circle(cv, g, 6.8, 6.8, 4.6, rgb, Some(1.5));
    poly(cv, g, &[(10.2, 10.2), (14.0, 14.0)], rgb, 1.7);
}

/// Four corner brackets. Outward: their corners point at the box's corners (fullscreen).
/// Inward: their corners point at the middle (leave fullscreen).
fn corners(cv: &mut Canvas, g: &Grid, rgb: Rgb, inward: bool) {
    // `e` is the bracket's corner on each axis, `t` the free end of its arms
    let (e, t) = if inward { (5.5, 1.5) } else { (1.5, 5.5) };
    for (sx, sy) in [(false, false), (true, false), (false, true), (true, true)] {
        let fx = |v: f32| if sx { 16.0 - v } else { v };
        let fy = |v: f32| if sy { 16.0 - v } else { v };
        poly(cv, g, &[(fx(e), fy(t)), (fx(e), fy(e)), (fx(t), fy(e))], rgb, 1.5);
    }
}

fn trash_can(cv: &mut Canvas, g: &Grid, rgb: Rgb) {
    poly(cv, g, &[(2.0, 4.0), (14.0, 4.0)], rgb, 1.5);
    poly(cv, g, &[(6.0, 4.0), (6.5, 2.0), (9.5, 2.0), (10.0, 4.0)], rgb, 1.4);
    poly(cv, g, &[(3.5, 4.0), (4.5, 14.5), (11.5, 14.5), (12.5, 4.0)], rgb, 1.5);
}

/// Draws `icon` centred in `r` (DIP), `size` DIP across.
pub(crate) fn draw(cv: &mut Canvas, icon: Icon, r: Rect, size: f32, rgb: Rgb) {
    let g = Grid::new(r, size);
    match icon {
        Icon::Expand => {
            // four arrows from the middle out to the corners, heads at the corners
            for (sx, sy) in [(false, false), (true, false), (false, true), (true, true)] {
                let fx = |v: f32| if sx { 16.0 - v } else { v };
                let fy = |v: f32| if sy { 16.0 - v } else { v };
                poly(cv, &g, &[(fx(1.5), fy(1.5)), (fx(6.2), fy(6.2))], rgb, 1.5);
                poly(cv, &g, &[(fx(1.5), fy(5.8)), (fx(1.5), fy(1.5)), (fx(5.8), fy(1.5))], rgb, 1.5);
            }
        }
        Icon::Back => {
            poly(cv, &g, &[(14.0, 8.0), (2.5, 8.0)], rgb, 1.7);
            poly(cv, &g, &[(7.5, 3.0), (2.5, 8.0), (7.5, 13.0)], rgb, 1.7);
        }
        Icon::Fullscreen => corners(cv, &g, rgb, false),
        Icon::ExitFullscreen => corners(cv, &g, rgb, true),
        Icon::Trash => {
            trash_can(cv, &g, rgb);
            poly(cv, &g, &[(6.5, 7.0), (6.8, 12.0)], rgb, 1.3);
            poly(cv, &g, &[(9.5, 7.0), (9.2, 12.0)], rgb, 1.3);
        }
        Icon::TrashForever => {
            trash_can(cv, &g, rgb);
            poly(cv, &g, &[(6.0, 7.0), (10.0, 12.0)], rgb, 1.3);
            poly(cv, &g, &[(10.0, 7.0), (6.0, 12.0)], rgb, 1.3);
        }
        Icon::Restore => {
            poly(cv, &g, &arc(8.5, 8.5, 5.5, 200.0, 500.0), rgb, 1.5);
            // arrowhead at the arc's start (pointing back, counter-clockwise)
            poly(cv, &g, &[(0.8, 5.0), (3.3, 6.6), (5.3, 4.2)], rgb, 1.5);
        }
        Icon::Copy => {
            cv.stroke(scaled_rect(&g, 2.0, 5.0, 9.0, 9.0), 1.5 * g.k, rgb, 1.0, 1.5 * g.k);
            poly(cv, &g, &[(5.0, 3.5), (5.0, 2.0), (14.0, 2.0), (14.0, 11.0), (12.5, 11.0)], rgb, 1.5);
        }
        Icon::Paste => {
            cv.stroke(scaled_rect(&g, 2.5, 3.0, 11.0, 12.0), 1.5 * g.k, rgb, 1.0, 1.5 * g.k);
            poly(cv, &g, &[(5.5, 3.0), (6.0, 1.3), (10.0, 1.3), (10.5, 3.0)], rgb, 1.4);
            poly(cv, &g, &[(8.0, 6.0), (8.0, 12.0)], rgb, 1.5);
            poly(cv, &g, &[(5.6, 9.6), (8.0, 12.0), (10.4, 9.6)], rgb, 1.5);
        }
        Icon::Star => star(cv, &g, rgb, false),
        Icon::StarFilled => star(cv, &g, rgb, true),
        Icon::Save => {
            poly(cv, &g, &[(8.0, 1.5), (8.0, 10.0)], rgb, 1.5);
            poly(cv, &g, &[(4.5, 6.5), (8.0, 10.0), (11.5, 6.5)], rgb, 1.5);
            poly(cv, &g, &[(2.0, 10.5), (2.0, 14.5), (14.0, 14.5), (14.0, 10.5)], rgb, 1.5);
        }
        Icon::Prev => poly(cv, &g, &[(10.5, 2.5), (5.0, 8.0), (10.5, 13.5)], rgb, 1.7),
        Icon::Next => poly(cv, &g, &[(5.5, 2.5), (11.0, 8.0), (5.5, 13.5)], rgb, 1.7),
        Icon::ChevronUp => poly(cv, &g, &[(3.0, 10.5), (8.0, 5.5), (13.0, 10.5)], rgb, 1.6),
        Icon::ChevronDown => poly(cv, &g, &[(3.0, 5.5), (8.0, 10.5), (13.0, 5.5)], rgb, 1.6),
        Icon::Fit => {
            cv.stroke(scaled_rect(&g, 1.5, 2.5, 13.0, 11.0), 1.5 * g.k, rgb, 1.0, 1.5 * g.k);
            poly(cv, &g, &[(4.5, 7.0), (4.5, 5.5), (6.0, 5.5)], rgb, 1.3);
            poly(cv, &g, &[(11.5, 9.0), (11.5, 10.5), (10.0, 10.5)], rgb, 1.3);
        }
        Icon::ActualSize => {
            poly(cv, &g, &[(2.5, 5.5), (4.0, 4.0), (4.0, 12.0)], rgb, 1.5);
            circle(cv, &g, 8.0, 6.5, 0.9, rgb, None);
            circle(cv, &g, 8.0, 10.5, 0.9, rgb, None);
            poly(cv, &g, &[(10.5, 5.5), (12.0, 4.0), (12.0, 12.0)], rgb, 1.5);
        }
        Icon::ZoomIn => {
            magnifier(cv, &g, rgb);
            poly(cv, &g, &[(4.6, 6.8), (9.0, 6.8)], rgb, 1.4);
            poly(cv, &g, &[(6.8, 4.6), (6.8, 9.0)], rgb, 1.4);
        }
        Icon::ZoomOut => {
            magnifier(cv, &g, rgb);
            poly(cv, &g, &[(4.6, 6.8), (9.0, 6.8)], rgb, 1.4);
        }
        Icon::Search => magnifier(cv, &g, rgb),
        Icon::Wrap => {
            poly(cv, &g, &[(2.0, 3.5), (14.0, 3.5)], rgb, 1.5);
            let mut hook = vec![(2.0, 8.0), (11.5, 8.0)];
            hook.extend(arc(11.5, 10.25, 2.25, -90.0, 90.0));
            hook.push((8.0, 12.5));
            poly(cv, &g, &hook, rgb, 1.5);
            poly(cv, &g, &[(9.8, 10.8), (8.0, 12.5), (9.8, 14.2)], rgb, 1.4);
            poly(cv, &g, &[(2.0, 12.5), (5.0, 12.5)], rgb, 1.5);
        }
        Icon::Info => {
            circle(cv, &g, 8.0, 8.0, 6.5, rgb, Some(1.4));
            circle(cv, &g, 8.0, 4.9, 0.95, rgb, None);
            poly(cv, &g, &[(8.0, 7.3), (8.0, 11.6)], rgb, 1.6);
        }
        Icon::ExtractText => {
            corners(cv, &g, rgb, false);
            poly(cv, &g, &[(5.0, 5.5), (11.0, 5.5)], rgb, 1.5);
            poly(cv, &g, &[(8.0, 5.5), (8.0, 11.5)], rgb, 1.5);
        }
        Icon::Close => {
            poly(cv, &g, &[(3.5, 3.5), (12.5, 12.5)], rgb, 1.7);
            poly(cv, &g, &[(12.5, 3.5), (3.5, 12.5)], rgb, 1.7);
        }
        Icon::More => {
            for x in [3.0, 8.0, 13.0] {
                circle(cv, &g, x, 8.0, 1.35, rgb, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Pixmap;

    /// Which quarters of a 16 px icon are drawn near each corner: (top-left, top-right,
    /// bottom-left, bottom-right) ink in the 4 x 4 corner squares.
    fn corner_ink(icon: Icon) -> [u32; 4] {
        let mut cv = Canvas { pm: Pixmap::new(64, 64).unwrap(), s: 4.0 };
        draw(&mut cv, icon, Rect { x: 0.0, y: 0.0, w: 16.0, h: 16.0 }, 16.0, (255, 255, 255));
        let ink = |x0: u32, y0: u32| {
            let mut n = 0;
            for y in y0..y0 + 12 {
                for x in x0..x0 + 12 {
                    n += u32::from(cv.pm.pixel(x, y).unwrap().alpha() > 128);
                }
            }
            n
        };
        [ink(2, 2), ink(50, 2), ink(2, 50), ink(50, 50)]
    }

    #[test]
    fn fullscreen_and_expand_reach_all_four_corners_alike() {
        for icon in [Icon::Fullscreen, Icon::Expand] {
            let c = corner_ink(icon);
            assert!(c.iter().all(|&n| n > 10), "{icon:?}: every corner is drawn {c:?}");
            // the four corners are mirror images: the same amount of ink in each
            assert!(c.iter().all(|&n| n.abs_diff(c[0]) <= 4), "{icon:?}: {c:?}");
        }
        // leaving fullscreen points inward: nothing at the very corners
        assert!(corner_ink(Icon::ExitFullscreen).iter().all(|&n| n < 10));
    }
}

fn scaled_rect(g: &Grid, x: f32, y: f32, w: f32, h: f32) -> Rect {
    let (px, py) = g.p(x, y);
    Rect { x: px, y: py, w: w * g.k, h: h * g.k }
}

