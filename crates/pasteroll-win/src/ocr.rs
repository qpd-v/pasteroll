//! Text recognition for the viewer's "Extract text", with the OCR engine built into Windows 10 and
//! 11 (Windows.Media.Ocr). It runs on the controller's worker thread and on this computer only:
//! nothing is uploaded.

use pasteroll_app::TextRecognizer;
use pasteroll_core::image::{downscale, Rgba};
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

pub struct WinOcr;

/// BGRA bytes of the picture laid on white: transparent pixels would otherwise read as black,
/// and dark text on a transparent background would vanish.
fn on_white(img: &Rgba) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.pixels.len());
    for p in img.pixels.chunks_exact(4) {
        let a = u32::from(p[3]);
        let mix = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
        out.extend_from_slice(&[mix(p[2]), mix(p[1]), mix(p[0]), 255]);
    }
    out
}

impl TextRecognizer for WinOcr {
    fn recognize(&self, img: &Rgba) -> Result<String, String> {
        unsafe {
            // a worker thread: WinRT needs the thread to be in an apartment
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
            "Text recognition needs a language with OCR support (Settings, Time & language, Language)."
                .to_owned()
        })?;
        let max = OcrEngine::MaxImageDimension().unwrap_or(10_000).max(100);
        let small;
        let img = if img.width > max || img.height > max {
            small = downscale(img, max, max);
            &small
        } else {
            img
        };
        let fail = |e: windows::core::Error| format!("Text recognition failed ({}).", e.message());
        let writer = DataWriter::new().map_err(fail)?;
        writer.WriteBytes(&on_white(img)).map_err(fail)?;
        let buffer = writer.DetachBuffer().map_err(fail)?;
        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            img.width as i32,
            img.height as i32,
            BitmapAlphaMode::Premultiplied,
        )
        .map_err(fail)?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(fail)?
            .join()
            .map_err(fail)?;
        let lines = result.Lines().map_err(fail)?;
        let mut text = String::new();
        for i in 0..lines.Size().map_err(fail)? {
            let line = lines.GetAt(i).map_err(fail)?;
            text.push_str(&line.Text().map_err(fail)?.to_string_lossy());
            text.push('\n');
        }
        Ok(text.trim_end().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparency_is_laid_on_white() {
        let img = Rgba {
            width: 2,
            height: 1,
            pixels: vec![10, 20, 30, 255, 0, 0, 0, 0],
        };
        assert_eq!(on_white(&img), vec![30, 20, 10, 255, 255, 255, 255, 255]);
    }
}
