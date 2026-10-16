//! Writing a finished picture as a file: PNG, JPG, WebP (lossless) or BMP.

use crate::image::Rgba;
use crate::imgedit::{Format, PngLevel};
use crate::imgfx;

/// The picture's pixels as RGB, laid on white (JPG and BMP cannot hold transparency).
fn rgb_on_white(img: &Rgba) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.width as usize * img.height as usize * 3);
    for p in img.pixels.chunks_exact(4) {
        let mut base = [255, 255, 255, 255];
        imgfx::over(&mut base, [p[0], p[1], p[2], p[3]]);
        out.extend_from_slice(&base[..3]);
    }
    out
}

fn bmp(img: &Rgba) -> Vec<u8> {
    let (w, h) = (img.width as usize, img.height as usize);
    let stride = (w * 3).div_ceil(4) * 4;
    let rgb = rgb_on_white(img);
    let data = stride * h;
    let mut out = Vec::with_capacity(54 + data);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // BI_RGB
    out.extend_from_slice(&(data as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for y in (0..h).rev() {
        let row = &rgb[y * w * 3..(y + 1) * w * 3];
        for px in row.chunks_exact(3) {
            out.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        out.resize(out.len() + (stride - w * 3), 0);
    }
    out
}

/// The picture as the bytes of a file in `format`. `quality` (1..100) is for JPG; `level` for PNG.
pub fn encode(img: &Rgba, format: Format, quality: u8, level: PngLevel) -> Result<Vec<u8>, String> {
    if img.width == 0 || img.height == 0 {
        return Err("the picture is empty".into());
    }
    match format {
        Format::Png => {
            let compression = match level {
                PngLevel::Fast => png::Compression::Fast,
                PngLevel::Balanced => png::Compression::Balanced,
                PngLevel::Smallest => png::Compression::High,
            };
            let mut out = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut out, img.width, img.height);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.set_compression(compression);
                let mut w = enc.write_header().map_err(|e| format!("PNG: {e}"))?;
                w.write_image_data(&img.pixels).map_err(|e| format!("PNG: {e}"))?;
            }
            Ok(out)
        }
        Format::Jpeg => {
            let (w, h) = (u16::try_from(img.width), u16::try_from(img.height));
            let (Ok(w), Ok(h)) = (w, h) else { return Err("a JPG can be at most 65,535 pixels a side".into()) };
            let mut out = Vec::new();
            let enc = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
            enc.encode(&rgb_on_white(img), w, h, jpeg_encoder::ColorType::Rgb).map_err(|e| format!("JPG: {e}"))?;
            Ok(out)
        }
        Format::WebP => {
            let mut out = Vec::new();
            image_webp::WebPEncoder::new(&mut out)
                .encode(&img.pixels, img.width, img.height, image_webp::ColorType::Rgba8)
                .map_err(|e| format!("WebP: {e}"))?;
            Ok(out)
        }
        Format::Bmp => Ok(bmp(img)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Rgba {
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 90, 255]);
            }
        }
        Rgba { width: w, height: h, pixels }
    }

    #[test]
    fn png_round_trips_with_every_level() {
        let img = gradient(40, 30);
        for level in [PngLevel::Fast, PngLevel::Balanced, PngLevel::Smallest] {
            let bytes = encode(&img, Format::Png, 90, level).unwrap();
            assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
            assert_eq!(crate::image::decode_png(&bytes).unwrap(), img);
        }
    }

    #[test]
    fn jpg_is_smaller_at_lower_quality_and_flattens_transparency() {
        let img = gradient(64, 64);
        let high = encode(&img, Format::Jpeg, 95, PngLevel::Balanced).unwrap();
        let low = encode(&img, Format::Jpeg, 10, PngLevel::Balanced).unwrap();
        assert_eq!(&high[..2], &[0xFF, 0xD8]);
        assert!(low.len() < high.len(), "{} !< {}", low.len(), high.len());
        let clear = Rgba { width: 8, height: 8, pixels: vec![0; 8 * 8 * 4] };
        assert!(encode(&clear, Format::Jpeg, 90, PngLevel::Fast).is_ok());
        assert_eq!(rgb_on_white(&clear)[..3], [255, 255, 255]);
    }

    #[test]
    fn webp_and_bmp_have_the_right_signatures_and_sizes() {
        let img = gradient(17, 9);
        let w = encode(&img, Format::WebP, 90, PngLevel::Fast).unwrap();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..12], b"WEBP");
        let b = encode(&img, Format::Bmp, 90, PngLevel::Fast).unwrap();
        assert_eq!(&b[..2], b"BM");
        // 17 px * 3 = 51 bytes, padded to 52 per row
        assert_eq!(b.len(), 54 + 52 * 9);
        assert_eq!(u32::from_le_bytes(b[2..6].try_into().unwrap()) as usize, b.len());
        // the first stored row is the bottom row; BGR order
        let bottom_left = &b[54..57];
        assert_eq!(bottom_left, &[90, (8 * 255 / 9) as u8, 0]);
    }

    #[test]
    fn webp_keeps_every_pixel() {
        // lossless: decode it again with the same library
        let img = gradient(10, 6);
        let bytes = encode(&img, Format::WebP, 90, PngLevel::Fast).unwrap();
        let mut dec = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(dec.dimensions(), (10, 6));
        let mut buf = vec![0u8; dec.output_buffer_size().unwrap()];
        dec.read_image(&mut buf).unwrap();
        let rgba: Vec<u8> = if dec.has_alpha() { buf } else { buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
        assert_eq!(rgba, img.pixels);
    }
}
