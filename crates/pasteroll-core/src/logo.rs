//! The Pasteroll logo as pixels, for small icons (tray icon, window icons). One 256 px source,
//! scaled with an area average on premultiplied colour so transparent edges do not darken.

/// The approved source picture, resized to a 256 x 256 RGBA PNG.
pub const LOGO_PNG: &[u8] = include_bytes!("../assets/logo-256.png");

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

/// The approved logo at `size` x `size`, straight (non-premultiplied) RGBA.
/// `paused` greys and dims it, so a paused app is visibly
/// different in the tray.
pub fn rgba(size: u32, paused: bool) -> Vec<u8> {
    let size = size.max(1);
    let mut out = scaled(size);
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
    fn the_approved_art_is_used_at_small_and_large_sizes() {
        assert_eq!(rgba(256, false), crate::image::decode_png(LOGO_PNG).unwrap().pixels);
        for n in [16u32, 24, 64] {
            let px = rgba(n, false);
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

    #[test]
    fn the_roll_has_transparent_corners_instead_of_a_square_background() {
        for n in [16u32, 24, 32, 48, 64, 128, 256] {
            let px = rgba(n, false);
            for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
                assert_eq!(px[((y * n + x) * 4 + 3) as usize], 0);
            }
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
