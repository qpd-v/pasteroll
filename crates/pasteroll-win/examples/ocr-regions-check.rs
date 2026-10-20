//! Runs Windows OCR on a synthetic fixture without clipboard, hooks or history.
#[path = "../src/ocr.rs"]
mod ocr;

use pasteroll_app::TextRecognizer;

fn main() {
    let bytes = std::fs::read(
        std::env::args()
            .nth(1)
            .expect("synthetic fixture PNG required"),
    )
    .unwrap();
    let source = pasteroll_core::image::decode_png(&bytes).unwrap();
    assert_eq!((source.width, source.height), (1000, 500));
    let original = source.pixels.clone();
    let ellipse = |cy: f32| {
        (0..64)
            .map(|i| {
                let t = i as f32 * std::f32::consts::TAU / 64.0;
                (300.0 + 290.0 * t.cos(), cy + 75.0 * t.sin())
            })
            .collect::<Vec<_>>()
    };
    let selected =
        pasteroll_core::ocr_regions::masked(&source, &[ellipse(110.0), ellipse(410.0)]).unwrap();
    let normalize = |s: String| {
        s.chars()
            .filter(|c| c.is_ascii_alphabetic())
            .collect::<String>()
            .to_uppercase()
    };
    let all = normalize(
        ocr::WinOcr
            .recognize(&source)
            .expect("Windows OCR unavailable"),
    );
    assert!(
        all.contains("IGNOREBETA") && all.contains("OUTSIDE"),
        "fixture must contain excluded text"
    );
    let result = normalize(ocr::WinOcr.recognize(&selected).unwrap());
    assert!(
        result.contains("KEEPALPHA") && result.contains("KEEPGAMMA"),
        "selected text must be recognized"
    );
    assert!(
        !result.contains("IGNORE") && !result.contains("OUTSIDE"),
        "unselected text must not be recognized"
    );
    assert_eq!(source.pixels, original);
    println!("PASS: native Windows OCR recognizes both circled regions, excludes other text and preserves source pixels.");
}
