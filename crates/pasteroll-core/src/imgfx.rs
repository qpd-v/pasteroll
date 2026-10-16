//! Pixel operations behind Advanced Image Paste: crop, flip, rotate, resize, background removal,
//! rounded corners, outline, padding and drop shadow. Pure Rust on straight-alpha RGBA, so it can
//! run on a worker thread and is testable without a window. Every function returns a new picture
//! or changes the one it is given; none of them touches the original clip.

use crate::image::Rgba;

pub type Px = [u8; 4];

pub fn blank(w: u32, h: u32) -> Rgba {
    Rgba { width: w, height: h, pixels: vec![0; w as usize * h as usize * 4] }
}

/// `src` laid over `dst` (both straight alpha).
pub fn over(dst: &mut [u8], src: Px) {
    let sa = u32::from(src[3]);
    if sa == 0 {
        return;
    }
    let da = u32::from(dst[3]);
    if sa == 255 || da == 0 {
        dst.copy_from_slice(&src);
        return;
    }
    // out alpha = sa + da (1 - sa), in 0..255*255 scale
    let oa = sa * 255 + da * (255 - sa);
    for c in 0..3 {
        let v = u32::from(src[c]) * sa * 255 + u32::from(dst[c]) * da * (255 - sa);
        dst[c] = ((v + oa / 2) / oa.max(1)).min(255) as u8;
    }
    dst[3] = ((oa + 127) / 255).min(255) as u8;
}

/// A copy of the rectangle `(x, y, w, h)`, clamped to the picture.
pub fn crop(img: &Rgba, x: u32, y: u32, w: u32, h: u32) -> Rgba {
    let x = x.min(img.width.saturating_sub(1));
    let y = y.min(img.height.saturating_sub(1));
    let w = w.clamp(1, img.width - x);
    let h = h.clamp(1, img.height - y);
    if (x, y, w, h) == (0, 0, img.width, img.height) {
        return img.clone();
    }
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = (row as usize * img.width as usize + x as usize) * 4;
        out.extend_from_slice(&img.pixels[start..start + w as usize * 4]);
    }
    Rgba { width: w, height: h, pixels: out }
}

pub fn flip_horizontal(img: &mut Rgba) {
    let w = img.width as usize;
    for row in img.pixels.chunks_exact_mut(w * 4) {
        for x in 0..w / 2 {
            for c in 0..4 {
                row.swap(x * 4 + c, (w - 1 - x) * 4 + c);
            }
        }
    }
}

pub fn flip_vertical(img: &mut Rgba) {
    let (w, h) = (img.width as usize, img.height as usize);
    for y in 0..h / 2 {
        let (a, b) = img.pixels.split_at_mut((h - 1 - y) * w * 4);
        a[y * w * 4..(y + 1) * w * 4].swap_with_slice(&mut b[..w * 4]);
    }
}

/// Turned clockwise by `quarters` quarter turns, without losing a pixel.
pub fn rotate_quarters(img: &Rgba, quarters: u32) -> Rgba {
    let (w, h) = (img.width as usize, img.height as usize);
    let at = |x: usize, y: usize| &img.pixels[(y * w + x) * 4..(y * w + x) * 4 + 4];
    match quarters % 4 {
        0 => img.clone(),
        2 => {
            let mut out = Vec::with_capacity(img.pixels.len());
            for y in 0..h {
                for x in 0..w {
                    out.extend_from_slice(at(w - 1 - x, h - 1 - y));
                }
            }
            Rgba { width: img.width, height: img.height, pixels: out }
        }
        q => {
            let mut out = Vec::with_capacity(img.pixels.len());
            for dy in 0..w {
                for dx in 0..h {
                    // clockwise: the left column becomes the top row; counter-clockwise for 3
                    let (sx, sy) = if q == 1 { (dy, h - 1 - dx) } else { (w - 1 - dy, dx) };
                    out.extend_from_slice(at(sx, sy));
                }
            }
            Rgba { width: img.height, height: img.width, pixels: out }
        }
    }
}

/// The size of the picture after turning it by `degrees` (clockwise) on a canvas that fits it.
pub fn rotated_size(w: u32, h: u32, degrees: f32) -> (u32, u32) {
    let q = (degrees / 90.0).round();
    if (degrees - q * 90.0).abs() < 0.01 {
        return if (q as i64).rem_euclid(2) == 1 { (h, w) } else { (w, h) };
    }
    let t = degrees.to_radians();
    let (c, s) = (t.cos().abs(), t.sin().abs());
    let nw = (w as f32 * c + h as f32 * s - 0.001).ceil().max(1.0);
    let nh = (w as f32 * s + h as f32 * c - 0.001).ceil().max(1.0);
    (nw as u32, nh as u32)
}

/// Turned by any angle, clockwise. The corners that appear are transparent.
pub fn rotate(img: &Rgba, degrees: f32) -> Rgba {
    let q = (degrees / 90.0).round();
    if (degrees - q * 90.0).abs() < 0.01 {
        return rotate_quarters(img, (q as i64).rem_euclid(4) as u32);
    }
    let (nw, nh) = rotated_size(img.width, img.height, degrees);
    let t = degrees.to_radians();
    let (c, s) = (t.cos(), t.sin());
    let (sw, sh) = (img.width as i64, img.height as i64);
    let (cx, cy) = (img.width as f32 / 2.0, img.height as f32 / 2.0);
    let (dcx, dcy) = (nw as f32 / 2.0, nh as f32 / 2.0);
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for dy in 0..nh as usize {
        for dx in 0..nw as usize {
            let x = dx as f32 + 0.5 - dcx;
            let y = dy as f32 + 0.5 - dcy;
            let sx = x * c + y * s + cx - 0.5;
            let sy = -x * s + y * c + cy - 0.5;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - x0, sy - y0);
            let mut acc = [0f32; 4];
            for (ox, oy, wgt) in [
                (0, 0, (1.0 - fx) * (1.0 - fy)),
                (1, 0, fx * (1.0 - fy)),
                (0, 1, (1.0 - fx) * fy),
                (1, 1, fx * fy),
            ] {
                let (px, py) = (x0 as i64 + ox, y0 as i64 + oy);
                if px < 0 || py < 0 || px >= sw || py >= sh || wgt <= 0.0 {
                    continue;
                }
                let p = &img.pixels[(py as usize * img.width as usize + px as usize) * 4..][..4];
                let a = f32::from(p[3]) / 255.0 * wgt;
                acc[0] += f32::from(p[0]) * a;
                acc[1] += f32::from(p[1]) * a;
                acc[2] += f32::from(p[2]) * a;
                acc[3] += a;
            }
            if acc[3] > 0.002 {
                let o = (dy * nw as usize + dx) * 4;
                out[o] = (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 1] = (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 2] = (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 3] = (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgba { width: nw, height: nh, pixels: out }
}

/// For every output index, the source indexes that feed it and their weights (sum 1): the area
/// under the output pixel when shrinking, a linear blend of the two nearest when growing.
fn taps(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = src as f32 / dst as f32;
    (0..dst)
        .map(|i| {
            if scale > 1.0 {
                let (start, end) = (i as f32 * scale, (i as f32 + 1.0) * scale);
                let first = start.floor() as usize;
                let last = (end.ceil() as usize).min(src);
                (first..last)
                    .filter_map(|sx| {
                        let cover = end.min(sx as f32 + 1.0) - start.max(sx as f32);
                        (cover > 0.0).then_some((sx, cover / scale))
                    })
                    .collect()
            } else {
                let c = (i as f32 + 0.5) * scale - 0.5;
                let x0 = c.floor();
                let f = c - x0;
                let a = (x0.max(0.0) as usize).min(src - 1);
                let b = ((x0 + 1.0).max(0.0) as usize).min(src - 1);
                if a == b {
                    vec![(a, 1.0)]
                } else {
                    vec![(a, 1.0 - f), (b, f)]
                }
            }
        })
        .collect()
}

/// Scaled to `nw x nh`: area averaging when smaller, bilinear when larger, with the colours of
/// transparent pixels never bleeding into the edges.
pub fn resize(img: &Rgba, nw: u32, nh: u32) -> Rgba {
    let (nw, nh) = (nw.max(1), nh.max(1));
    if nw == img.width && nh == img.height {
        return img.clone();
    }
    let (sw, sh) = (img.width as usize, img.height as usize);
    let (tx, ty) = (taps(sw, nw as usize), taps(sh, nh as usize));
    let mut tmp = vec![0f32; nw as usize * sh * 4];
    for y in 0..sh {
        let row = &img.pixels[y * sw * 4..(y + 1) * sw * 4];
        for (ox, t) in tx.iter().enumerate() {
            let mut acc = [0f32; 4];
            for &(sx, wgt) in t {
                let p = &row[sx * 4..sx * 4 + 4];
                let a = f32::from(p[3]) / 255.0 * wgt;
                acc[0] += f32::from(p[0]) * a;
                acc[1] += f32::from(p[1]) * a;
                acc[2] += f32::from(p[2]) * a;
                acc[3] += a;
            }
            tmp[(y * nw as usize + ox) * 4..][..4].copy_from_slice(&acc);
        }
    }
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for (oy, t) in ty.iter().enumerate() {
        for ox in 0..nw as usize {
            let mut acc = [0f32; 4];
            for &(sy, wgt) in t {
                let p = &tmp[(sy * nw as usize + ox) * 4..][..4];
                for c in 0..4 {
                    acc[c] += p[c] * wgt;
                }
            }
            if acc[3] > 0.0005 {
                let o = (oy * nw as usize + ox) * 4;
                out[o] = (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 1] = (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 2] = (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 3] = (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgba { width: nw, height: nh, pixels: out }
}

/// The picture laid over a solid colour.
pub fn flatten(img: &mut Rgba, color: Px) {
    for p in img.pixels.chunks_exact_mut(4) {
        let mut base = color;
        over(&mut base, [p[0], p[1], p[2], p[3]]);
        p.copy_from_slice(&base);
    }
}

/// The colour most of the four corners agree on (opaque corner pixels only).
pub fn corner_colour(img: &Rgba) -> Option<[u8; 3]> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut corners: Vec<[u8; 3]> = Vec::new();
    for (cx, cy) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        // a small patch averages out noise; tiny pictures use their corner pixel alone
        let patch = (w.min(h) / 8).clamp(1, 3);
        for dy in 0..patch {
            for dx in 0..patch {
                let x = if cx == 0 { dx } else { w - 1 - dx };
                let y = if cy == 0 { dy } else { h - 1 - dy };
                let p = &img.pixels[(y * w + x) * 4..][..4];
                if p[3] >= 200 {
                    r += u32::from(p[0]);
                    g += u32::from(p[1]);
                    b += u32::from(p[2]);
                    n += 1;
                }
            }
        }
        if n > 0 {
            corners.push([(r / n) as u8, (g / n) as u8, (b / n) as u8]);
        }
    }
    // the corner colour that most others are close to
    corners
        .iter()
        .max_by_key(|c| corners.iter().filter(|d| colour_distance(**c, **d) < 40.0).count())
        .copied()
}

fn colour_distance(a: [u8; 3], b: [u8; 3]) -> f32 {
    let d = |i: usize| f32::from(a[i]) - f32::from(b[i]);
    (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt()
}

/// Makes the background see-through: the area that touches the picture's edge and is close to
/// `key` in colour (`tolerance` 0..100). The edge of what remains is softened by a pixel.
/// `key` `None` takes the colour of the corners. Pixels already transparent count as background.
pub fn remove_background(img: &mut Rgba, key: Option<[u8; 3]>, tolerance: f32) {
    let (w, h) = (img.width as usize, img.height as usize);
    let key = key.or_else(|| corner_colour(img));
    let thr = tolerance.clamp(0.0, 100.0) * 2.2;
    let close = |img: &Rgba, i: usize| {
        let p = &img.pixels[i * 4..i * 4 + 4];
        p[3] < 8 || key.is_some_and(|k| colour_distance([p[0], p[1], p[2]], k) <= thr)
    };
    let mut region = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let seed = |i: usize, region: &mut Vec<bool>, stack: &mut Vec<usize>, img: &Rgba| {
        if !region[i] && close(img, i) {
            region[i] = true;
            stack.push(i);
        }
    };
    for x in 0..w {
        seed(x, &mut region, &mut stack, img);
        seed((h - 1) * w + x, &mut region, &mut stack, img);
    }
    for y in 0..h {
        seed(y * w, &mut region, &mut stack, img);
        seed(y * w + w - 1, &mut region, &mut stack, img);
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        if x > 0 {
            seed(i - 1, &mut region, &mut stack, img);
        }
        if x + 1 < w {
            seed(i + 1, &mut region, &mut stack, img);
        }
        if y > 0 {
            seed(i - w, &mut region, &mut stack, img);
        }
        if y + 1 < h {
            seed(i + w, &mut region, &mut stack, img);
        }
    }
    // a one-pixel feather on the side that stays
    let mut feather = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if region[i] {
                continue;
            }
            'near: for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && region[ny as usize * w + nx as usize] {
                        feather[i] = true;
                        break 'near;
                    }
                }
            }
        }
    }
    for i in 0..w * h {
        let a = &mut img.pixels[i * 4 + 3];
        if region[i] {
            *a = 0;
        } else if feather[i] {
            *a = ((u32::from(*a) + 1) / 2) as u8;
        }
    }
}

/// How much of the pixel at `(x, y)` (its centre at +0.5) lies inside a rounded rectangle.
fn rounded_cover(x: f32, y: f32, rect: (f32, f32, f32, f32), radius: f32) -> f32 {
    let (rx, ry, rw, rh) = rect;
    let (px, py) = (x + 0.5, y + 0.5);
    let r = radius.min(rw / 2.0).min(rh / 2.0).max(0.0);
    // distance outside the straight edges
    let cx = px.clamp(rx + r, rx + rw - r);
    let cy = py.clamp(ry + r, ry + rh - r);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    if r <= 0.0 {
        let inside = px >= rx && px <= rx + rw && py >= ry && py <= ry + rh;
        return f32::from(u8::from(inside));
    }
    (r - d + 0.5).clamp(0.0, 1.0)
}

/// Cuts the corners round. `radius` is in pixels and is limited to half the shorter side.
pub fn round_corners(img: &mut Rgba, radius: f32) {
    let (w, h) = (img.width as usize, img.height as usize);
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    if r < 0.5 {
        return;
    }
    let span = r.ceil() as usize + 1;
    let rect = (0.0, 0.0, w as f32, h as f32);
    for ys in [(0..span.min(h)), (h.saturating_sub(span).max(span.min(h))..h)] {
        for y in ys {
            for xs in [(0..span.min(w)), (w.saturating_sub(span).max(span.min(w))..w)] {
                for x in xs {
                    let k = rounded_cover(x as f32, y as f32, rect, r);
                    if k < 1.0 {
                        let a = &mut img.pixels[(y * w + x) * 4 + 3];
                        *a = (f32::from(*a) * k).round() as u8;
                    }
                }
            }
        }
    }
}

/// The picture with a frame of `width` pixels around it. The frame follows the corner radius.
pub fn with_outline(img: &Rgba, color: Px, width: u32, radius: f32) -> Rgba {
    if width == 0 || color[3] == 0 {
        return img.clone();
    }
    let t = width as usize;
    let (w, h) = (img.width as usize, img.height as usize);
    let (cw, ch) = (w + 2 * t, h + 2 * t);
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0).max(0.0);
    let inner = (t as f32, t as f32, w as f32, h as f32);
    let outer = (0.0, 0.0, cw as f32, ch as f32);
    let outer_r = if r > 0.0 { r + t as f32 } else { 0.0 };
    let mut out = blank(cw as u32, ch as u32);
    let ring = |x: usize, y: usize| {
        let o = rounded_cover(x as f32, y as f32, outer, outer_r);
        let i = rounded_cover(x as f32, y as f32, inner, r);
        (o * (1.0 - i)).clamp(0.0, 1.0)
    };
    let paint = |out: &mut Rgba, x: usize, y: usize| {
        let k = ring(x, y);
        if k > 0.0 {
            let a = (f32::from(color[3]) * k).round() as u8;
            over(&mut out.pixels[(y * cw + x) * 4..][..4], [color[0], color[1], color[2], a]);
        }
    };
    // the frame: everything outside the picture, plus the picture's own rounded corners
    for y in 0..ch {
        let band = y < t || y >= t + h;
        if band {
            for x in 0..cw {
                paint(&mut out, x, y);
            }
        } else {
            for x in 0..t {
                paint(&mut out, x, y);
            }
            for x in t + w..cw {
                paint(&mut out, x, y);
            }
            let span = r.ceil() as usize + 1;
            if r > 0.0 && (y < t + span || y >= t + h - span.min(h)) {
                for x in (t..t + span.min(w)).chain((t + w).saturating_sub(span)..t + w) {
                    paint(&mut out, x, y);
                }
            }
        }
    }
    // the picture on top
    for y in 0..h {
        for x in 0..w {
            let p = &img.pixels[(y * w + x) * 4..][..4];
            over(&mut out.pixels[((y + t) * cw + x + t) * 4..][..4], [p[0], p[1], p[2], p[3]]);
        }
    }
    out
}

/// Blurs a plane of values 0..1 (three box passes, close to a Gaussian of deviation `sigma`).
pub fn blur_plane(plane: &mut [f32], w: usize, h: usize, sigma: f32) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    let n = (4.0 * sigma * sigma + 1.0).sqrt();
    let r = (((n - 1.0) / 2.0).round() as usize).max(1);
    let mut tmp = vec![0f32; plane.len()];
    for _ in 0..3 {
        // rows
        for y in 0..h {
            box_line(&plane[y * w..(y + 1) * w], &mut tmp[y * w..(y + 1) * w], r);
        }
        // columns
        let mut col = vec![0f32; h];
        let mut res = vec![0f32; h];
        for x in 0..w {
            for y in 0..h {
                col[y] = tmp[y * w + x];
            }
            box_line(&col, &mut res, r);
            for y in 0..h {
                plane[y * w + x] = res[y];
            }
        }
    }
}

/// Running-sum box blur of one line; outside the line counts as zero.
fn box_line(src: &[f32], dst: &mut [f32], r: usize) {
    let n = src.len();
    let k = 1.0 / (2 * r + 1) as f32;
    let mut sum: f32 = src.iter().take(r.min(n)).sum();
    for i in 0..n {
        if i + r < n {
            sum += src[i + r];
        }
        if i > r {
            sum -= src[i - r - 1];
        }
        dst[i] = sum * k;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    pub color: Px,
    /// 0..1, on top of the colour's own alpha.
    pub opacity: f32,
    pub blur: f32,
    pub dx: i32,
    pub dy: i32,
}

/// The picture on a larger canvas: `margins` = left, top, right, bottom. The canvas is filled with
/// `fill` (or left transparent); a drop shadow falls under the picture.
pub fn compose(content: &Rgba, margins: [u32; 4], fill: Option<Px>, shadow: Option<&ShadowSpec>) -> Rgba {
    let [ml, mt, mr, mb] = margins;
    let (cw, ch) = (content.width as usize + (ml + mr) as usize, content.height as usize + (mt + mb) as usize);
    let mut out = blank(cw as u32, ch as u32);
    if let Some(f) = fill {
        for p in out.pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&f);
        }
    }
    let (w, h) = (content.width as usize, content.height as usize);
    if let Some(s) = shadow.filter(|s| s.opacity > 0.0 && s.color[3] > 0) {
        let mut plane = vec![0f32; cw * ch];
        for y in 0..h {
            let ty = y as i64 + i64::from(mt) + i64::from(s.dy);
            if ty < 0 || ty >= ch as i64 {
                continue;
            }
            for x in 0..w {
                let tx = x as i64 + i64::from(ml) + i64::from(s.dx);
                if tx < 0 || tx >= cw as i64 {
                    continue;
                }
                plane[ty as usize * cw + tx as usize] = f32::from(content.pixels[(y * w + x) * 4 + 3]) / 255.0;
            }
        }
        blur_plane(&mut plane, cw, ch, s.blur / 2.0);
        let k = s.opacity.clamp(0.0, 1.0) * f32::from(s.color[3]) / 255.0;
        for (i, a) in plane.iter().enumerate() {
            let alpha = (a * k * 255.0).round().clamp(0.0, 255.0) as u8;
            if alpha > 0 {
                over(&mut out.pixels[i * 4..i * 4 + 4], [s.color[0], s.color[1], s.color[2], alpha]);
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let p = &content.pixels[(y * w + x) * 4..][..4];
            if p[3] > 0 {
                over(&mut out.pixels[((y + mt as usize) * cw + x + ml as usize) * 4..][..4], [p[0], p[1], p[2], p[3]]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: Px) -> Rgba {
        Rgba { width: w, height: h, pixels: c.iter().copied().cycle().take(w as usize * h as usize * 4).collect() }
    }

    fn at(img: &Rgba, x: u32, y: u32) -> Px {
        let i = ((y * img.width + x) * 4) as usize;
        [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2], img.pixels[i + 3]]
    }

    /// 3 x 2: top row red, green, blue; bottom row black, white, grey.
    fn sample() -> Rgba {
        let px: [Px; 6] =
            [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [0, 0, 0, 255], [255, 255, 255, 255], [128, 128, 128, 255]];
        Rgba { width: 3, height: 2, pixels: px.iter().flatten().copied().collect() }
    }

    #[test]
    fn cropping_takes_the_rectangle_and_clamps_it() {
        let c = crop(&sample(), 1, 0, 2, 1);
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(at(&c, 0, 0), [0, 255, 0, 255]);
        assert_eq!(at(&c, 1, 0), [0, 0, 255, 255]);
        let big = crop(&sample(), 2, 1, 50, 50);
        assert_eq!((big.width, big.height), (1, 1));
        assert_eq!(at(&big, 0, 0), [128, 128, 128, 255]);
    }

    #[test]
    fn flips_mirror_the_picture() {
        let mut a = sample();
        flip_horizontal(&mut a);
        assert_eq!(at(&a, 0, 0), [0, 0, 255, 255]);
        assert_eq!(at(&a, 2, 0), [255, 0, 0, 255]);
        let mut b = sample();
        flip_vertical(&mut b);
        assert_eq!(at(&b, 0, 0), [0, 0, 0, 255]);
        assert_eq!(at(&b, 0, 1), [255, 0, 0, 255]);
    }

    #[test]
    fn quarter_turns_go_clockwise_and_lose_nothing() {
        let r = rotate_quarters(&sample(), 1);
        assert_eq!((r.width, r.height), (2, 3));
        // the old left column (red above black) becomes the top row, black first
        assert_eq!(at(&r, 0, 0), [0, 0, 0, 255]);
        assert_eq!(at(&r, 1, 0), [255, 0, 0, 255]);
        let back = rotate_quarters(&r, 3);
        assert_eq!(back, sample());
        let half = rotate_quarters(&sample(), 2);
        assert_eq!(at(&half, 0, 0), [128, 128, 128, 255]);
        assert_eq!(rotate_quarters(&rotate_quarters(&sample(), 2), 2), sample());
    }

    #[test]
    fn any_angle_grows_the_canvas_and_leaves_transparent_corners() {
        let img = solid(40, 20, [200, 30, 30, 255]);
        let r = rotate(&img, 45.0);
        assert_eq!((r.width, r.height), rotated_size(40, 20, 45.0));
        assert!(r.width > 40 && r.height > 20);
        assert_eq!(at(&r, 0, 0)[3], 0, "the corner is empty");
        assert_eq!(at(&r, r.width / 2, r.height / 2), [200, 30, 30, 255]);
        // exact angles are lossless
        assert_eq!(rotate(&sample(), 90.0), rotate_quarters(&sample(), 1));
        assert_eq!(rotate(&sample(), -90.0), rotate_quarters(&sample(), 3));
        assert_eq!(rotated_size(30, 10, 90.0), (10, 30));
        assert_eq!(rotated_size(30, 10, 0.0), (30, 10));
    }

    #[test]
    fn resizing_keeps_flat_colours_and_averages_when_shrinking() {
        let img = solid(8, 8, [10, 200, 90, 255]);
        let big = resize(&img, 20, 13);
        assert_eq!((big.width, big.height), (20, 13));
        assert!(big.pixels.chunks_exact(4).all(|p| p == [10, 200, 90, 255]));
        // a black and white checker shrunk to one pixel is grey
        let mut checker = solid(2, 2, [0, 0, 0, 255]);
        checker.pixels[0..4].copy_from_slice(&[255, 255, 255, 255]);
        checker.pixels[12..16].copy_from_slice(&[255, 255, 255, 255]);
        let one = resize(&checker, 1, 1);
        assert!((i32::from(one.pixels[0]) - 128).abs() <= 1);
        assert_eq!(resize(&img, 8, 8), img);
    }

    #[test]
    fn resizing_does_not_bleed_the_colour_of_transparent_pixels() {
        let mut img = solid(2, 1, [0, 0, 0, 0]);
        img.pixels[0..4].copy_from_slice(&[255, 0, 0, 255]); // red, then fully transparent black
        let r = resize(&img, 1, 1);
        assert_eq!(&r.pixels[0..3], &[255, 0, 0], "still red, not dark red");
        assert!((i32::from(r.pixels[3]) - 128).abs() <= 1);
    }

    #[test]
    fn flattening_lays_the_picture_on_a_colour() {
        let mut img = solid(1, 1, [255, 0, 0, 0]);
        flatten(&mut img, [0, 0, 255, 255]);
        assert_eq!(at(&img, 0, 0), [0, 0, 255, 255]);
        let mut half = solid(1, 1, [255, 0, 0, 128]);
        flatten(&mut half, [0, 0, 0, 255]);
        assert_eq!(at(&half, 0, 0)[3], 255);
        assert!((i32::from(half.pixels[0]) - 128).abs() <= 1);
    }

    #[test]
    fn the_background_connected_to_the_edge_goes_but_an_enclosed_island_stays() {
        // white page, a black ring, and white inside the ring
        let mut img = solid(9, 9, [255, 255, 255, 255]);
        for i in 2..7 {
            for (x, y) in [(i, 2), (i, 6), (2, i), (6, i)] {
                img.pixels[(y * 9 + x) * 4..][..3].copy_from_slice(&[0, 0, 0]);
            }
        }
        remove_background(&mut img, None, 20.0);
        assert_eq!(at(&img, 0, 0)[3], 0, "the outside is gone");
        assert_eq!(at(&img, 4, 4)[3], 255, "the inside of the ring is kept");
        assert_eq!(at(&img, 2, 2)[3], 128, "the ring's edge is feathered");
    }

    #[test]
    fn a_picked_colour_is_removed_wherever_it_touches_the_edge() {
        let mut img = solid(4, 1, [255, 255, 255, 255]);
        img.pixels[0..4].copy_from_slice(&[10, 200, 10, 255]);
        img.pixels[4..8].copy_from_slice(&[12, 198, 12, 255]);
        remove_background(&mut img, Some([10, 200, 10]), 10.0);
        assert_eq!(at(&img, 0, 0)[3], 0);
        assert_eq!(at(&img, 1, 0)[3], 0, "close enough counts");
        assert_ne!(at(&img, 3, 0)[3], 0);
    }

    #[test]
    fn rounding_cuts_the_corners_and_leaves_the_middle() {
        let mut img = solid(20, 20, [9, 9, 9, 255]);
        round_corners(&mut img, 8.0);
        assert_eq!(at(&img, 0, 0)[3], 0);
        assert_eq!(at(&img, 19, 19)[3], 0);
        assert_eq!(at(&img, 10, 10)[3], 255);
        assert_eq!(at(&img, 10, 0)[3], 255, "the straight edge is untouched");
        let mut sq = solid(20, 20, [9, 9, 9, 255]);
        round_corners(&mut sq, 0.0);
        assert_eq!(at(&sq, 0, 0)[3], 255, "radius 0 is square");
        // a huge radius makes a circle
        let mut disc = solid(20, 20, [9, 9, 9, 255]);
        round_corners(&mut disc, 1.0e6);
        assert_eq!(at(&disc, 0, 10)[3] < 255, true);
        assert_eq!(at(&disc, 10, 10)[3], 255);
        assert_eq!(at(&disc, 1, 1)[3], 0);
    }

    #[test]
    fn the_outline_surrounds_the_picture() {
        let img = solid(10, 6, [255, 255, 255, 255]);
        let o = with_outline(&img, [0, 0, 0, 255], 3, 0.0);
        assert_eq!((o.width, o.height), (16, 12));
        assert_eq!(at(&o, 0, 0), [0, 0, 0, 255]);
        assert_eq!(at(&o, 2, 5), [0, 0, 0, 255]);
        assert_eq!(at(&o, 3, 3), [255, 255, 255, 255]);
        assert_eq!(at(&o, 8, 6), [255, 255, 255, 255]);
        assert_eq!(at(&o, 15, 11), [0, 0, 0, 255]);
    }

    #[test]
    fn a_rounded_picture_gets_a_rounded_outline_without_gaps() {
        let mut img = solid(30, 30, [255, 255, 255, 255]);
        round_corners(&mut img, 10.0);
        let o = with_outline(&img, [255, 0, 0, 255], 4, 10.0);
        assert_eq!((o.width, o.height), (38, 38));
        assert_eq!(at(&o, 0, 0)[3], 0, "outside the rounded frame");
        // just inside the picture's rounded corner the frame shows
        let p = at(&o, 4, 4);
        assert!(p[3] > 200 && p[0] > p[1], "frame colour shows in the corner: {p:?}");
        assert_eq!(at(&o, 19, 19), [255, 255, 255, 255]);
        for y in 0..38 {
            for x in 0..38 {
                // no hole between frame and picture: along the straight middle edge every pixel is opaque
                if (x == 19 && (y < 4 || y >= 34)) || (y == 19 && (x < 4 || x >= 34)) {
                    assert_eq!(at(&o, x, y)[3], 255, "gap at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn compose_adds_margins_a_fill_and_a_shadow() {
        let img = solid(10, 10, [255, 255, 255, 255]);
        let padded = compose(&img, [5, 6, 7, 8], Some([10, 20, 30, 255]), None);
        assert_eq!((padded.width, padded.height), (22, 24));
        assert_eq!(at(&padded, 0, 0), [10, 20, 30, 255]);
        assert_eq!(at(&padded, 5, 6), [255, 255, 255, 255]);
        assert_eq!(at(&padded, 14, 15), [255, 255, 255, 255]);
        assert_eq!(at(&padded, 15, 16), [10, 20, 30, 255]);
        let sh = ShadowSpec { color: [0, 0, 0, 255], opacity: 0.5, blur: 0.0, dx: 4, dy: 4 };
        let s = compose(&img, [0, 0, 8, 8], None, Some(&sh));
        assert_eq!((s.width, s.height), (18, 18));
        let p = at(&s, 12, 12); // inside the shadow, outside the picture
        assert_eq!(p[3], 128);
        assert_eq!(&p[..3], &[0, 0, 0]);
        assert_eq!(at(&s, 16, 16)[3], 0, "past the shadow");
        assert_eq!(at(&s, 5, 5), [255, 255, 255, 255], "the picture covers its shadow");
    }

    #[test]
    fn blur_spreads_the_shadow_and_keeps_its_total() {
        let mut plane = vec![0f32; 21 * 21];
        plane[10 * 21 + 10] = 1.0;
        let before: f32 = plane.iter().sum();
        blur_plane(&mut plane, 21, 21, 3.0);
        let after: f32 = plane.iter().sum();
        assert!((after - before).abs() < 0.02, "{after}");
        assert!(plane[10 * 21 + 10] < 0.2 && plane[10 * 21 + 10] > plane[10 * 21 + 14]);
        assert!(plane[10 * 21 + 14] > 0.0);
    }
}
