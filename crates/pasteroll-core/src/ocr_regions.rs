//! Non-destructive freehand OCR masks in original image coordinates.

use crate::image::Rgba;

pub type Point = (f32, f32);
pub const MAX_REGIONS: usize = 16;
pub const MAX_POINTS: usize = 1024;

pub fn valid(region: &[Point]) -> bool {
    if region.len() < 3
        || region.len() > MAX_POINTS
        || region.iter().any(|p| !p.0.is_finite() || !p.1.is_finite())
    {
        return false;
    }
    let twice_area: f64 = region
        .iter()
        .zip(region.iter().cycle().skip(1))
        .take(region.len())
        .map(|(a, b)| f64::from(a.0) * f64::from(b.1) - f64::from(b.0) * f64::from(a.1))
        .sum();
    twice_area.abs() >= 2.0
}

/// Crop to the selected union plus white padding. Pixels outside every outline
/// become white, even inside the crop's bounding rectangle. Source pixels are never changed.
pub fn masked(image: &Rgba, regions: &[Vec<Point>]) -> Result<Rgba, String> {
    if regions.is_empty() || regions.len() > MAX_REGIONS || regions.iter().any(|r| !valid(r)) {
        return Err("Draw a closed area around the text first.".into());
    }
    let mut bounds = (image.width as f32, image.height as f32, 0.0f32, 0.0f32);
    for &(x, y) in regions.iter().flatten() {
        bounds.0 = bounds.0.min(x.max(0.0));
        bounds.1 = bounds.1.min(y.max(0.0));
        bounds.2 = bounds.2.max(x.min(image.width as f32));
        bounds.3 = bounds.3.max(y.min(image.height as f32));
    }
    let (left, top, right, bottom) = (
        bounds.0.floor() as u32,
        bounds.1.floor() as u32,
        bounds.2.ceil() as u32,
        bounds.3.ceil() as u32,
    );
    if right <= left || bottom <= top {
        return Err("The selected area is outside the picture.".into());
    }
    const PAD: u32 = 8;
    let (width, height) = (right - left + PAD * 2, bottom - top + PAD * 2);
    let mut pixels = vec![255; width as usize * height as usize * 4];
    let mut selected = false;
    for y in top..bottom {
        let scan = y as f64 + 0.5;
        for region in regions {
            let mut crossings = Vec::new();
            for (a, b) in region
                .iter()
                .zip(region.iter().cycle().skip(1))
                .take(region.len())
            {
                let (ay, by) = (f64::from(a.1), f64::from(b.1));
                if (ay <= scan && by > scan) || (by <= scan && ay > scan) {
                    crossings.push(
                        f64::from(a.0)
                            + (scan - ay) / (by - ay) * (f64::from(b.0) - f64::from(a.0)),
                    );
                }
            }
            crossings.sort_by(f64::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let start = ((pair[0] - 0.5).ceil().max(f64::from(left))) as u32;
                let end = ((pair[1] - 0.5).ceil().min(f64::from(right))).max(0.0) as u32;
                for x in start..end {
                    let src = ((y as usize * image.width as usize) + x as usize) * 4;
                    let dst = (((y - top + PAD) as usize * width as usize)
                        + (x - left + PAD) as usize)
                        * 4;
                    pixels[dst..dst + 4].copy_from_slice(&image.pixels[src..src + 4]);
                    selected = true;
                }
            }
        }
    }
    if !selected {
        return Err("The selected area is too small. Draw around the text again.".into());
    }
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black() -> Rgba {
        Rgba {
            width: 8,
            height: 8,
            pixels: [0, 0, 0, 255].repeat(64),
        }
    }
    fn pixel(img: &Rgba, x: u32, y: u32) -> &[u8] {
        let n = ((y * img.width + x) * 4) as usize;
        &img.pixels[n..n + 4]
    }

    #[test]
    fn freehand_mask_excludes_pixels_inside_bounds_but_outside_the_outline() {
        let source = black();
        let original = source.pixels.clone();
        let out = masked(&source, &[vec![(1.0, 1.0), (7.0, 1.0), (1.0, 7.0)]]).unwrap();
        assert_eq!(pixel(&out, 8, 8), &[0, 0, 0, 255]);
        assert_eq!(pixel(&out, 13, 13), &[255; 4]);
        assert_eq!(pixel(&out, 0, 0), &[255; 4]);
        assert_eq!(source.pixels, original);
    }

    #[test]
    fn separated_regions_leave_intervening_text_white_and_overlap_is_a_union() {
        let region = |x| vec![(x, 1.0), (x + 2.0, 1.0), (x + 2.0, 3.0), (x, 3.0)];
        let out = masked(&black(), &[region(0.0), region(6.0), region(0.0)]).unwrap();
        assert_eq!(pixel(&out, 8, 8), &[0, 0, 0, 255]);
        assert_eq!(pixel(&out, 11, 8), &[255; 4]);
        assert_eq!(pixel(&out, 15, 8), &[0, 0, 0, 255]);
    }

    #[test]
    fn invalid_empty_and_out_of_bounds_selections_are_rejected() {
        for regions in [
            vec![],
            vec![vec![(0.0, 0.0); 3]],
            vec![vec![(f32::NAN, 0.0), (5.0, 0.0), (0.0, 5.0)]],
            vec![vec![(10.0, 10.0), (14.0, 10.0), (10.0, 14.0)]],
        ] {
            assert!(masked(&black(), &regions).is_err());
        }
    }

    #[test]
    fn edge_crossing_polygon_is_clipped_to_source_pixels() {
        let out = masked(
            &black(),
            &[vec![(-4.0, -4.0), (12.0, -4.0), (12.0, 12.0), (-4.0, 12.0)]],
        )
        .unwrap();
        assert_eq!((out.width, out.height), (24, 24));
        assert_eq!(pixel(&out, 8, 8), &[0, 0, 0, 255]);
        assert_eq!(pixel(&out, 15, 15), &[0, 0, 0, 255]);
    }
}
