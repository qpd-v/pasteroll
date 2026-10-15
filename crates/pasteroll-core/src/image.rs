//! Image helpers: DIB (what Windows puts on the clipboard) <-> RGBA <-> PNG, and thumbnails.
//! Pure Rust, no Win32, so it is testable and can run on a worker thread.

#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    /// Straight (non-premultiplied) RGBA, row-major, top-down.
    pub pixels: Vec<u8>,
}

/// Preview pictures are kept at most this big (the overlay shows them inside a ~500 x 260 area).
pub const THUMB_W: u32 = 720;
pub const THUMB_H: u32 = 420;

/// Largest image we will decode (pixels). Bounds memory for hostile or accidental giants.
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Decodes a packed DIB (`BITMAPINFOHEADER`, `BITMAPV4HEADER` or `BITMAPV5HEADER` followed by pixel
/// data, as in `CF_DIB`). Supports 24- and 32-bit uncompressed images, the common clipboard cases.
/// Anything else returns `None` and the clip is simply not captured as an image.
pub fn dib_to_rgba(dib: &[u8]) -> Option<Rgba> {
    let header = u32le(dib, 0)? as usize;
    if !matches!(header, 40 | 108 | 124) {
        return None;
    }
    let width = u32le(dib, 4)? as i32;
    let height = u32le(dib, 8)? as i32;
    let planes = u16le(dib, 12)?;
    let bpp = u16le(dib, 14)?;
    let compression = u32le(dib, 16)?;
    let clr_used = u32le(dib, 32)? as usize;
    if planes != 1 || width <= 0 || height == 0 || !matches!(bpp, 24 | 32) {
        return None;
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let mut offset = header;
    match compression {
        0 => {}
        3 => {
            // BI_BITFIELDS: only the standard BGRA layout is supported.
            let (r, g, b) = if header == 40 {
                offset += 12;
                (u32le(dib, 40)?, u32le(dib, 44)?, u32le(dib, 48)?)
            } else {
                (u32le(dib, 40)?, u32le(dib, 44)?, u32le(dib, 48)?)
            };
            if bpp != 32 || (r, g, b) != (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) {
                return None;
            }
        }
        _ => return None,
    }
    offset = offset.checked_add(clr_used.checked_mul(4)?)?;
    let stride = ((w as usize * bpp as usize + 31) / 32) * 4;
    let need = offset.checked_add(stride.checked_mul(h as usize)?)?;
    if dib.len() < need {
        return None;
    }
    let bottom_up = height > 0;
    let bytes = (bpp / 8) as usize;
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let mut any_alpha = false;
    for y in 0..h as usize {
        let src_row = if bottom_up { h as usize - 1 - y } else { y };
        let row = &dib[offset + src_row * stride..offset + src_row * stride + w as usize * bytes];
        for x in 0..w as usize {
            let px = &row[x * bytes..x * bytes + bytes];
            let o = (y * w as usize + x) * 4;
            out[o] = px[2];
            out[o + 1] = px[1];
            out[o + 2] = px[0];
            let a = if bytes == 4 { px[3] } else { 255 };
            any_alpha |= a != 0;
            out[o + 3] = a;
        }
    }
    // 32-bit BI_RGB images routinely carry an unused (zero) alpha byte: treat as opaque.
    if bpp == 32 && !any_alpha {
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
    Some(Rgba { width: w, height: h, pixels: out })
}

/// A 32-bit bottom-up `BITMAPINFOHEADER` DIB (what we put on the clipboard as `CF_DIB`).
pub fn rgba_to_dib(img: &Rgba) -> Vec<u8> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = Vec::with_capacity(40 + w * h * 4);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(img.width as i32).to_le_bytes());
    out.extend_from_slice(&(img.height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // resolution, colours used/important
    for y in (0..h).rev() {
        for x in 0..w {
            let p = &img.pixels[(y * w + x) * 4..(y * w + x) * 4 + 4];
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    out
}

pub fn encode_png(img: &Rgba) -> Option<Vec<u8>> {
    encode_png_with(img, png::Compression::Balanced)
}

/// Quick, larger PNG: for pictures that are small anyway (previews).
pub fn encode_png_fast(img: &Rgba) -> Option<Vec<u8>> {
    encode_png_with(img, png::Compression::Fast)
}

fn encode_png_with(img: &Rgba, level: png::Compression) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(level);
        let mut w = enc.write_header().ok()?;
        w.write_image_data(&img.pixels).ok()?;
    }
    Some(out)
}

pub fn decode_png(bytes: &[u8]) -> Option<Rgba> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let info = reader.info();
    if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (frame.width, frame.height);
    let data = &buf[..frame.buffer_size()];
    let pixels = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => {
            data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
        }
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None, // EXPAND converts these; defensive
    };
    Some(Rgba { width: w, height: h, pixels })
}

/// Box-filter downscale to fit inside `max_w x max_h`, keeping aspect. Never upscales.
pub fn downscale(img: &Rgba, max_w: u32, max_h: u32) -> Rgba {
    let scale = (f64::from(max_w) / f64::from(img.width))
        .min(f64::from(max_h) / f64::from(img.height))
        .min(1.0);
    if scale >= 1.0 {
        return img.clone();
    }
    let nw = ((f64::from(img.width) * scale) as u32).max(1);
    let nh = ((f64::from(img.height) * scale) as u32).max(1);
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for ny in 0..nh {
        let y0 = (u64::from(ny) * u64::from(img.height) / u64::from(nh)) as usize;
        let y1 = (((u64::from(ny) + 1) * u64::from(img.height) / u64::from(nh)) as usize).max(y0 + 1);
        for nx in 0..nw {
            let x0 = (u64::from(nx) * u64::from(img.width) / u64::from(nw)) as usize;
            let x1 = (((u64::from(nx) + 1) * u64::from(img.width) / u64::from(nw)) as usize).max(x0 + 1);
            let mut acc = [0u64; 4];
            let mut n = 0u64;
            for y in y0..y1.min(img.height as usize) {
                for x in x0..x1.min(img.width as usize) {
                    let p = &img.pixels[(y * img.width as usize + x) * 4..][..4];
                    for c in 0..4 {
                        acc[c] += u64::from(p[c]);
                    }
                    n += 1;
                }
            }
            let o = (ny as usize * nw as usize + nx as usize) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    Rgba { width: nw, height: nh, pixels: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dib24(w: i32, h: i32, rows_bgr: &[&[[u8; 3]]]) -> Vec<u8> {
        let stride = ((w as usize * 24 + 31) / 32) * 4;
        let mut v = Vec::new();
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes());
        v.extend_from_slice(&[0u8; 24]);
        for row in rows_bgr {
            let mut r: Vec<u8> = row.iter().flatten().copied().collect();
            r.resize(stride, 0);
            v.extend_from_slice(&r);
        }
        v
    }

    #[test]
    fn decodes_a_bottom_up_24_bit_dib_with_row_padding() {
        // 1 pixel wide -> 3 bytes + 1 pad per row. Bottom-up: the first stored row is the bottom.
        let dib = dib24(1, 2, &[&[[0, 0, 255]], &[[255, 0, 0]]]); // bottom red(BGR 0,0,255), top blue
        let img = dib_to_rgba(&dib).unwrap();
        assert_eq!((img.width, img.height), (1, 2));
        assert_eq!(&img.pixels[0..4], &[0, 0, 255, 255]); // top row is blue (RGB 0,0,255)
        assert_eq!(&img.pixels[4..8], &[255, 0, 0, 255]); // bottom row is red
    }

    #[test]
    fn top_down_dib_is_not_flipped() {
        let dib = dib24(1, -2, &[&[[0, 0, 255]], &[[255, 0, 0]]]);
        let img = dib_to_rgba(&dib).unwrap();
        assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]); // first stored row is the top
    }

    #[test]
    fn rejects_unsupported_formats() {
        assert!(dib_to_rgba(&[]).is_none());
        assert!(dib_to_rgba(&[0; 40]).is_none());
        let mut d = dib24(1, 1, &[&[[1, 2, 3]]]);
        d[14] = 8; // 8 bpp (palette): unsupported
        assert!(dib_to_rgba(&d).is_none());
        let mut d = dib24(1, 1, &[&[[1, 2, 3]]]);
        d[16] = 1; // BI_RLE8
        assert!(dib_to_rgba(&d).is_none());
    }

    #[test]
    fn truncated_pixel_data_is_rejected() {
        let mut d = dib24(2, 2, &[&[[1, 2, 3], [4, 5, 6]], &[[7, 8, 9], [1, 1, 1]]]);
        d.truncate(d.len() - 3);
        assert!(dib_to_rgba(&d).is_none());
    }

    #[test]
    fn absurd_dimensions_are_rejected_without_allocating() {
        let mut d = dib24(1, 1, &[&[[1, 2, 3]]]);
        d[4..8].copy_from_slice(&0x7FFF_FFFFi32.to_le_bytes());
        d[8..12].copy_from_slice(&0x7FFF_FFFFi32.to_le_bytes());
        assert!(dib_to_rgba(&d).is_none());
    }

    #[test]
    fn rgba_dib_roundtrip_preserves_pixels_and_alpha() {
        let img = Rgba {
            width: 2,
            height: 2,
            pixels: vec![10, 20, 30, 255, 40, 50, 60, 128, 70, 80, 90, 0, 1, 2, 3, 77],
        };
        let back = dib_to_rgba(&rgba_to_dib(&img)).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn zero_alpha_32_bit_dib_is_treated_as_opaque() {
        let img = Rgba { width: 1, height: 1, pixels: vec![9, 8, 7, 0] };
        let back = dib_to_rgba(&rgba_to_dib(&img)).unwrap();
        assert_eq!(back.pixels, vec![9, 8, 7, 255]);
    }

    #[test]
    fn png_roundtrip() {
        let img = Rgba {
            width: 3,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 200, 0, 0, 255, 10],
        };
        let png = encode_png(&img).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(decode_png(&png).unwrap(), img);
    }

    #[test]
    fn decode_png_rejects_garbage() {
        assert!(decode_png(b"not a png").is_none());
        assert!(decode_png(&[]).is_none());
    }

    #[test]
    fn downscale_keeps_aspect_and_never_upscales() {
        let img = Rgba { width: 1000, height: 500, pixels: vec![128; 1000 * 500 * 4] };
        let s = downscale(&img, 100, 100);
        assert_eq!((s.width, s.height), (100, 50));
        assert!(s.pixels.iter().all(|&p| p == 128));
        let small = Rgba { width: 10, height: 10, pixels: vec![1; 400] };
        assert_eq!(downscale(&small, 720, 420), small);
    }

    #[test]
    fn downscale_averages_pixels() {
        // 2x1 black + white -> 1x1 grey
        let img = Rgba { width: 2, height: 1, pixels: vec![0, 0, 0, 255, 254, 254, 254, 255] };
        let s = downscale(&img, 1, 1);
        assert_eq!((s.width, s.height), (1, 1));
        assert_eq!(&s.pixels, &[127, 127, 127, 255]);
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// A screenshot-like picture: smooth areas, text-like detail and a little noise.
    fn picture(w: u32, h: u32) -> Rgba {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        let mut seed = 12345u32;
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = ((seed >> 24) & 7) as u8;
                let band = ((x / 40 + y / 40) % 2) as u8 * 60;
                let edge = if x % 17 < 2 || y % 23 < 2 { 90 } else { 0 };
                px.extend_from_slice(&[
                    (x * 255 / w) as u8 / 2 + band + noise,
                    (y * 255 / h) as u8 / 2 + edge,
                    120 + noise,
                    255,
                ]);
            }
        }
        Rgba { width: w, height: h, pixels: px }
    }

    #[test]
    #[ignore]
    fn time_the_preview_pipeline() {
        for (w, h) in [(1280u32, 720u32), (1920, 1080), (3840, 2160)] {
            let img = picture(w, h);
            let t = std::time::Instant::now();
            let png = encode_png(&img).unwrap();
            let enc = t.elapsed();
            let t = std::time::Instant::now();
            let dec = decode_png(&png).unwrap();
            let dect = t.elapsed();
            let t = std::time::Instant::now();
            let small = downscale(&dec, 720, 420);
            let down = t.elapsed();
            eprintln!(
                "{w}x{h}: png {:.1} MB; encode {:?}; decode {:?}; downscale to {}x{} {:?}",
                png.len() as f64 / 1e6,
                enc,
                dect,
                small.width,
                small.height,
                down
            );
        }
    }
}
