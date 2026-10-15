//! The Pasteroll logo as pixels, for small icons (tray icon, window icons). One 256 px source,
//! scaled with an area average on premultiplied colour so transparent edges do not darken.

/// The source picture: 256 x 256 RGBA PNG with a transparent background.
pub const LOGO_PNG: &[u8] = include_bytes!("../assets/logo-256.png");

/// Sizes up to this are drawn as a simplified logo: the full picture turns to mud in 24 pixels.
pub const SIMPLE_MAX: u32 = 24;

/// Signed distance to a rounded rectangle (negative inside), in unit-square coordinates.
fn rrect(u: f32, v: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
    let (dx, dy) = ((u - cx).abs() - hx, (v - cy).abs() - hy);
    (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r
}

/// The logo reduced to its idea: three coloured cards rolled up into a green core, with a dark
/// outline, drawn with 4x4 supersampling. Straight RGBA.
fn simple(n: u32) -> Vec<u8> {
    const OUTLINE: [u8; 3] = [24, 24, 40];
    let n = n as usize;
    let t = (1.3 / n as f32).max(0.06);
    let mut out = vec![0u8; n * n * 4];
    let sample = |u: f32, v: f32| -> Option<([u8; 3], bool)> {
        // fills, topmost first: hole, cap, purple, yellow, red
        let hole = ((u - 0.74).powi(2) + (v - 0.24).powi(2)).sqrt() - 0.075;
        let cap = ((u - 0.74).powi(2) + (v - 0.24).powi(2)).sqrt() - 0.21;
        let red = rrect(u, v, 0.07, 0.07, 0.66, 0.36, 0.11);
        let yellow = rrect(u, v, 0.07, 0.40, 0.66, 0.66, 0.03);
        let purple = rrect(u, v, 0.07, 0.70, 0.66, 0.95, 0.09);
        if hole <= 0.0 {
            return Some(([30, 70, 60], false));
        }
        if cap <= 0.0 {
            return Some(([120, 230, 165], false));
        }
        if purple <= 0.0 {
            return Some(([150, 95, 240], false));
        }
        if yellow <= 0.0 {
            return Some(([250, 200, 45], false));
        }
        if red <= 0.0 {
            return Some(([240, 82, 48], false));
        }
        let near = hole.min(cap).min(red).min(yellow).min(purple);
        (near <= t).then_some((OUTLINE, true))
    };
    for y in 0..n {
        for x in 0..n {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let u = (x as f32 + (sx as f32 + 0.5) / 4.0) / n as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / 4.0) / n as f32;
                    if let Some((c, _)) = sample(u, v) {
                        r += u32::from(c[0]);
                        g += u32::from(c[1]);
                        b += u32::from(c[2]);
                        a += 1;
                    }
                }
            }
            if a > 0 {
                let o = (y * n + x) * 4;
                out[o] = (r / a) as u8;
                out[o + 1] = (g / a) as u8;
                out[o + 2] = (b / a) as u8;
                out[o + 3] = (a * 255 / 16) as u8;
            }
        }
    }
    out
}

/// The full logo scaled to `size`, straight RGBA (area average on premultiplied colour).
fn scaled(size: u32) -> Vec<u8> {
    let Some(src) = crate::image::decode_png(LOGO_PNG) else {
        return vec![0; (size * size * 4) as usize];
    };
    let (sw, sh) = (src.width as usize, src.height as usize);
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        let y0 = y * sh / n;
        let y1 = ((y + 1) * sh / n).max(y0 + 1).min(sh);
        for x in 0..n {
            let x0 = x * sw / n;
            let x1 = ((x + 1) * sw / n).max(x0 + 1).min(sw);
            let (mut r, mut g, mut b, mut a, mut cnt) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let p = &src.pixels[(yy * sw + xx) * 4..][..4];
                    let pa = u64::from(p[3]);
                    r += u64::from(p[0]) * pa;
                    g += u64::from(p[1]) * pa;
                    b += u64::from(p[2]) * pa;
                    a += pa;
                    cnt += 1;
                }
            }
            let o = (y * n + x) * 4;
            if a > 0 {
                out[o] = (r / a) as u8;
                out[o + 1] = (g / a) as u8;
                out[o + 2] = (b / a) as u8;
                out[o + 3] = (a / cnt.max(1)) as u8;
            }
        }
    }
    out
}

/// The logo at `size` x `size`, straight (non-premultiplied) RGBA: the simplified drawing for
/// small sizes, the real logo otherwise. `paused` greys and dims it, so a paused app is visibly
/// different in the tray.
pub fn rgba(size: u32, paused: bool) -> Vec<u8> {
    let size = size.max(1);
    let mut out = if size <= SIMPLE_MAX { simple(size) } else { scaled(size) };
    if paused {
        for p in out.chunks_exact_mut(4).filter(|p| p[3] > 0) {
            let l = (u32::from(p[0]) * 30 + u32::from(p[1]) * 59 + u32::from(p[2]) * 11) / 100;
            let l = (l * 72 / 100) as u8;
            p[0] = l;
            p[1] = l;
            p[2] = l;
            p[3] = (u32::from(p[3]) * 85 / 100) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_scales_to_the_requested_size() {
        for n in [16u32, 24, 32, 48, 64, 256] {
            assert_eq!(rgba(n, false).len(), (n * n * 4) as usize);
        }
    }

    #[test]
    fn the_background_is_transparent_and_the_middle_is_not() {
        for n in [16u32, 24, 64] {
            let px = rgba(n, false);
            assert_eq!(px[px.len() - 1], 0, "bottom-right corner is transparent at {n}");
            let covered = px.chunks_exact(4).filter(|p| p[3] > 200).count();
            assert!(covered > (n * n / 4) as usize, "a good part of the icon is drawn at {n}");
        }
    }

    #[test]
    fn paused_is_grey_and_dimmer() {
        let a = rgba(32, false);
        let b = rgba(32, true);
        assert_ne!(a, b);
        for p in b.chunks_exact(4).filter(|p| p[3] > 0) {
            assert!(p[0] == p[1] && p[1] == p[2], "no colour left when paused");
        }
    }

    /// `PASTEROLL_ICON_DUMP=<dir> cargo test -p pasteroll-core --lib dump_small_icons -- --ignored`
    #[test]
    #[ignore]
    fn dump_small_icons() {
        let Ok(dir) = std::env::var("PASTEROLL_ICON_DUMP") else { return };
        for (n, paused) in [(16u32, false), (24, false), (32, false), (16, true)] {
            let px = rgba(n, paused);
            let k = 10usize;
            let big = n as usize * k;
            let mut out = Vec::with_capacity(big * big * 4);
            for y in 0..big {
                for x in 0..big {
                    let o = ((y / k) * n as usize + x / k) * 4;
                    // over a light grey so the shape is visible
                    let a = u32::from(px[o + 3]);
                    for c in 0..3 {
                        out.push(((u32::from(px[o + c]) * a + 190 * (255 - a)) / 255) as u8);
                    }
                    out.push(255);
                }
            }
            let img = crate::image::Rgba { width: big as u32, height: big as u32, pixels: out };
            let png = crate::image::encode_png(&img).unwrap();
            std::fs::write(std::path::Path::new(&dir).join(format!("icon-{n}-{}.png", if paused { "paused" } else { "normal" })), png).unwrap();
        }
    }
}
