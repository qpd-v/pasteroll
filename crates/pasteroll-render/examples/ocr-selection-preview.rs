//! Render the real viewer UI with synthetic OCR regions at several DPI scales.
use pasteroll_core::image::{decode_png, encode_png, Rgba};
use pasteroll_render::{
    OcrSelectionView, Renderer, RowKind, Tool, ToolView, ViewerBody, ViewerView,
};
use std::{path::PathBuf, sync::Arc};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let image = Arc::new(decode_png(&std::fs::read(&args[1]).unwrap()).unwrap());
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).unwrap();
    let ellipse = |cy: f32| {
        (0..64)
            .map(|i| {
                let t = i as f32 * std::f32::consts::TAU / 64.0;
                (300.0 + 290.0 * t.cos(), cy + 75.0 * t.sin())
            })
            .collect::<Vec<_>>()
    };
    let button = |tool, on, tip: &str| ToolView {
        tool,
        on,
        enabled: true,
        tip: tip.into(),
    };
    let view = ViewerView {
        kind: RowKind::Image,
        title: "Image, 1000 x 500".into(),
        subtitle: "Draw around text; Enter extracts; Ctrl+Z undoes; Esc cancels. 2 area(s)".into(),
        body: ViewerBody::Image {
            img: Some(image),
            size: (1000, 500),
            zoom: None,
            center: (500.0, 250.0),
        },
        tools: vec![
            button(Tool::ZoomOut, false, "Zoom out"),
            button(Tool::FitActual, false, "Actual size"),
            button(Tool::ZoomIn, false, "Zoom in"),
            button(Tool::OcrSelect, true, "Circle text for OCR"),
            button(Tool::OcrRead, false, "Extract selected text (Enter)"),
            button(Tool::OcrUndo, false, "Undo area"),
            button(Tool::More, false, "More"),
            button(Tool::Close, false, "Close"),
        ],
        ocr_selection: Some(OcrSelectionView {
            regions: vec![ellipse(110.0), ellipse(410.0)],
            drawing: vec![],
        }),
        ..Default::default()
    };
    let mut renderer = Renderer::new();
    for (label, scale) in [("100", 1.0), ("150", 1.5), ("200", 2.0)] {
        let frame = renderer.render_viewer(&view, (1050.0, 680.0), scale);
        let mut pixels = frame.bgra;
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let png = encode_png(&Rgba {
            width: frame.width,
            height: frame.height,
            pixels,
        })
        .unwrap();
        std::fs::write(out.join(format!("ocr-selection-{label}.png")), png).unwrap();
    }
    println!("Rendered OCR selection viewer at 100%, 150% and 200% scaling.");
}
