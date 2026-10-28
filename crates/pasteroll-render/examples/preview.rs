//! Writes sample frames to PNG files so the layout can be inspected by eye.
//! `cargo run -p pasteroll-render --example preview -- <output-dir>`
use pasteroll_core::image::{encode_png, Rgba};
use pasteroll_render::*;
use std::sync::Arc;

fn save(r: &mut Renderer, vm: &ViewModel, scale: f32, path: &std::path::Path) {
    let size = if vm.slim { (SLIM_WIDTH, SLIM_HEIGHT) } else { (WIDTH, HEIGHT) };
    let f = r.render(vm, size, scale);
    let mut rgba = Vec::with_capacity(f.bgra.len());
    for p in f.bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    let png = encode_png(&Rgba { width: f.width, height: f.height, pixels: rgba }).unwrap();
    std::fs::write(path, png).unwrap();
}

fn row(kind: RowKind, title: &str, pinned: bool, hl: Vec<(usize, usize)>) -> RowView {
    RowView { kind, pinned, title: title.into(), meta: "Text, 142 characters".into(), highlights: hl, multi: 0 }
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("output dir"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::new();
    let rows = vec![
        row(RowKind::Text, "Meeting notes: ship the spike, measure paste reliability", false, vec![]),
        row(RowKind::Files, "invoice-2026-10.pdf", true, vec![(0, 7)]),
        row(RowKind::Text, "https://example.com/docs/getting-started?ref=carousel", false, vec![]),
        row(RowKind::Text, "The quick brown fox jumps over the lazy dog.", false, vec![]),
        row(RowKind::Image, "Image 200 x 120", false, vec![]),
        row(RowKind::Text, "fn greet(name: &str) -> String", false, vec![]),
        row(RowKind::Files, "holiday photos.zip", false, vec![]),
    ];
    let long = "The quick brown fox jumps over the lazy dog. A second sentence follows it, \
        long enough that the preview has to wrap onto several lines so we can see how the active \
        item behaves when the clip is longer than the space available for it.\r\n\r\n\
        Second paragraph with <U+200B> a visible token.\r\n... 3,210 more characters not shown";
    let mut vm = ViewModel {
        query: "invoice".into(),
        filter_label: "Document".into(),
        rows: rows.clone(),
        active: 3,
        body: Body::Text { display: long.into(), selection: Some((16, 25)), scroll: 0.0 },
        footer: "4 of 120".into(),
        ..ViewModel::default()
    };
    save(&mut r, &vm, 1.0, &dir.join("1-text.png"));
    // a multi-paste in the list (badge), with Multi-paste switched on (icon lit, tooltip off)
    let mut multi = vm.clone();
    multi.rows[0].multi = 3;
    multi.rows[2].multi = 12;
    multi.rows[2].pinned = true;
    multi.multi_mode = true;
    multi.picked = vec![false, true, true];
    save(&mut r, &multi, 1.0, &dir.join("1b-multi.png"));
    // the options card over the carousel
    let mut card = multi.clone();
    card.panel = Some(PanelView {
        title: "Paste 3 clips together".into(),
        groups: vec![
            PanelGroup { label: "Join with".into(), options: ["New line", "Space", "Comma", "Tab", "None"].map(String::from).to_vec(), selected: 2 },
            PanelGroup { label: "List style".into(), options: ["Plain", "Numbered", "Bulleted"].map(String::from).to_vec(), selected: 1 },
            PanelGroup { label: "Order".into(), options: ["As chosen", "Newest first", "Oldest first"].map(String::from).to_vec(), selected: 0 },
        ],
        preview: "1. Meeting notes: ship the spike, 2. invoice-2026-10.pdf, 3. https://example.com/docs".into(),
        note: String::new(),
        focus: 1,
        hover: Some(PanelHit::Option(0, 3)),
    });
    save(&mut r, &card, 2.0, &dir.join("1d-options-card-2x.png"));
    multi.multi_mode = false;
    multi.active = 0;
    multi.rows[0].meta = "Multi-paste, 3 clips, 211 characters".into();
    save(&mut r, &multi, 2.0, &dir.join("1c-multi-active-2x.png"));
    vm.query.clear();
    vm.dropdown = Some(Dropdown {
        items: ["Everything", "Text", "Document", "Image", "Video", "Audio", "Compressed", "Pinned", "Custom..."]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        selected: 2,
        hover: Some(3),
    });
    save(&mut r, &vm, 1.0, &dir.join("2-dropdown.png"));
    vm.dropdown = None;
    let mut px = Vec::new();
    for y in 0..120u32 {
        for x in 0..200u32 {
            px.extend_from_slice(&[(x * 255 / 200) as u8, (y * 255 / 120) as u8, 160, 255]);
        }
    }
    vm.active = 4;
    vm.body = Body::Image(Some(Arc::new(Rgba { width: 200, height: 120, pixels: px })));
    save(&mut r, &vm, 1.0, &dir.join("3-image.png"));
    let empty = ViewModel { query: "zzzz".into(), body: Body::Message("No clips match \"zzzz\"".into()), ..ViewModel::default() };
    save(&mut r, &empty, 1.0, &dir.join("4-empty.png"));
    save(&mut r, &vm, 2.0, &dir.join("5-image-2x.png"));
    // No Bloat: the clip alone, in its smaller window
    let slim = |body: Body, footer: &str| ViewModel {
        rows: vec![row(RowKind::Text, "", false, vec![])],
        active: 0,
        body,
        footer: footer.into(),
        slim: true,
        ..ViewModel::default()
    };
    save(&mut r, &slim(Body::Text { display: long.into(), selection: Some((16, 25)), scroll: 0.0 }, ""), 1.0, &dir.join("6-no-bloat-text.png"));
    save(&mut r, &slim(vm.body.clone(), ""), 1.0, &dir.join("7-no-bloat-image.png"));
    save(
        &mut r,
        &slim(Body::Text { display: "echo one\r\necho two".into(), selection: None, scroll: 0.0 }, "2 line(s) into a terminal may run commands. Enter pastes, Esc cancels."),
        2.0,
        &dir.join("8-no-bloat-warning-2x.png"),
    );
    println!("wrote 8 frames to {}", dir.display());
}
