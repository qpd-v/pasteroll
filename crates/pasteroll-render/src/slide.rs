//! The slide between two rolls: the old picture goes out one side while the new one comes in from the
//! other. Only the part below `split_px` moves; the search bar and the favorite tiles above it stay
//! where they are (they are the same on every roll).

use crate::Frame;

/// How long the slide takes, in milliseconds.
pub const SLIDE_MS: u64 = 240;

/// Eases out: quick at first, settling at the end. `t` from 0 to 1.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// One frame of the slide. `progress` runs from 0 (all `old`) to 1 (all `new`). `dir` is `1` when the
/// new roll comes from the right (rolling to the next one) and `-1` when it comes from the left.
/// Both frames must be the same size, or `new` is returned as it is.
pub fn slide_frames(old: &Frame, new: &Frame, progress: f32, dir: i8, split_px: u32) -> Frame {
    if old.width != new.width || old.height != new.height || old.bgra.len() != new.bgra.len() {
        return new.clone();
    }
    let p = progress.clamp(0.0, 1.0);
    if p >= 1.0 {
        return new.clone();
    }
    let (w, h) = (new.width as usize, new.height as usize);
    let dir = if dir < 0 { -1i64 } else { 1i64 };
    let shift_old = -dir * (p * w as f32).round() as i64;
    let shift_new = dir * ((1.0 - p) * w as f32).round() as i64;
    let mut out = new.clone();
    let split = (split_px as usize).min(h);
    for y in split..h {
        let row = y * w * 4;
        for x in 0..w as i64 {
            let (src, sx) = if (0..w as i64).contains(&(x - shift_new)) {
                (&new.bgra, x - shift_new)
            } else if (0..w as i64).contains(&(x - shift_old)) {
                (&old.bgra, x - shift_old)
            } else {
                continue;
            };
            let to = row + x as usize * 4;
            let from = row + sx as usize * 4;
            out.bgra[to..to + 4].copy_from_slice(&src[from..from + 4]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, shade: u8) -> Frame {
        Frame { width: w, height: h, bgra: vec![shade; (w * h * 4) as usize] }
    }

    fn px(f: &Frame, x: u32, y: u32) -> u8 {
        f.bgra[((y * f.width + x) * 4) as usize]
    }

    #[test]
    fn it_starts_as_the_old_picture_and_ends_as_the_new_one() {
        let (old, new) = (flat(8, 4, 10), flat(8, 4, 200));
        let start = slide_frames(&old, &new, 0.0, 1, 0);
        assert!(start.bgra.iter().all(|b| *b == 10));
        let end = slide_frames(&old, &new, 1.0, 1, 0);
        assert!(end.bgra.iter().all(|b| *b == 200));
    }

    #[test]
    fn rolling_to_the_next_brings_the_new_picture_in_from_the_right() {
        let (old, new) = (flat(10, 2, 10), flat(10, 2, 200));
        let mid = slide_frames(&old, &new, 0.5, 1, 0);
        assert_eq!((px(&mid, 0, 0), px(&mid, 4, 0)), (10, 10), "the old picture is on the left");
        assert_eq!((px(&mid, 5, 0), px(&mid, 9, 0)), (200, 200), "the new one on the right");
        let back = slide_frames(&old, &new, 0.5, -1, 0);
        assert_eq!((px(&back, 0, 0), px(&back, 9, 0)), (200, 10), "the other way round when rolling back");
    }

    #[test]
    fn what_is_above_the_split_does_not_move() {
        let (old, new) = (flat(10, 4, 10), flat(10, 4, 200));
        let mid = slide_frames(&old, &new, 0.5, 1, 2);
        assert_eq!((px(&mid, 0, 0), px(&mid, 9, 1)), (200, 200), "the bar comes from the new picture whole");
        assert_eq!((px(&mid, 0, 3), px(&mid, 9, 3)), (10, 200));
    }

    #[test]
    fn frames_of_different_sizes_are_not_mixed() {
        let out = slide_frames(&flat(4, 4, 1), &flat(8, 4, 2), 0.5, 1, 0);
        assert_eq!((out.width, out.bgra[0]), (8, 2));
    }

    #[test]
    fn easing_runs_from_zero_to_one() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5);
    }
}
