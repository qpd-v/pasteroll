use super::*;

struct CaptureOcr(Arc<Mutex<Vec<Rgba>>>);
impl TextRecognizer for CaptureOcr {
    fn recognize(&self, img: &Rgba) -> Result<String, String> {
        self.0.lock().unwrap().push(img.clone());
        Ok("Selected text".into())
    }
}

fn draw(h: &mut H, points: &[(f32, f32)]) {
    let view = h.c.viewer_view().unwrap();
    let g = pasteroll_render::viewer_geometry(&view, h.c.viewer_size_dip().unwrap());
    let ViewerBody::Image {
        size, zoom, center, ..
    } = view.body
    else {
        panic!()
    };
    let z = zoom.unwrap_or_else(|| pasteroll_render::fit_zoom(&g, size.0, size.1));
    for (i, &(x, y)) in points.iter().enumerate() {
        h.send(Event::ViewerMouse(MouseEvent {
            kind: if i == 0 {
                MouseKind::Down
            } else {
                MouseKind::Move
            },
            x: g.content.x + g.content.w / 2.0 + (x - center.0) * z,
            y: g.content.y + g.content.h / 2.0 + (y - center.1) * z,
        }));
    }
    let (x, y) = points[0];
    h.send(Event::ViewerMouse(MouseEvent {
        kind: MouseKind::Up,
        x: g.content.x + g.content.w / 2.0 + (x - center.0) * z,
        y: g.content.y + g.content.h / 2.0 + (y - center.1) * z,
    }));
}

fn area(h: &mut H) {
    draw(h, &[(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]);
}

fn wait_for_text(h: &mut H) {
    for _ in 0..200 {
        h.send(Event::Wake);
        if matches!(h.c.viewer_view().unwrap().body, ViewerBody::Text { .. }) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("OCR did not complete");
}

fn recognized_image(policy: IngestPolicy, source: Option<Box<dyn ClipboardSource>>) -> (H, u64) {
    let mut h = H::with(&[Clip::Image], source, policy);
    h.c.set_ocr(Arc::new(FakeOcr));
    let generation = h.open();
    h.expand(generation);
    h.tool(Tool::More);
    let i = h.c.viewer_view().unwrap().menu.unwrap().iter()
        .position(|m| m.label.starts_with("Extract text")).unwrap();
    h.vclick(|geometry, _| geometry.menu_items[i]);
    wait_for_text(&mut h);
    assert_eq!(h.c.store().borrow().count().unwrap(), 1, "OCR alone must not record a text clip");
    (h, generation)
}

#[test]
fn copied_ocr_selection_is_recorded_even_when_own_clipboard_notification_is_skipped() {
    let state = Arc::new(Mutex::new(SourceState { seq: 0, next: None,
        app: Some("pasteroll.exe".into()) }));
    let (mut h, _) = recognized_image(IngestPolicy::default(), Some(Box::new(FakeSource(state.clone()))));
    let view = h.c.viewer_view().unwrap();
    let geometry = pasteroll_render::viewer_geometry(&view, h.c.viewer_size_dip().unwrap());
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::DoubleClick,
        x: geometry.text.x + 3.0, y: geometry.text.y + 8.0 }));
    state.lock().unwrap().seq += 1;
    h.c.clipboard_changed();
    h.tool(Tool::Copy);
    assert_eq!(h.pasted_texts(), vec!["Text"]);
    assert!(wait_for_count(&h, 2));
    let rows = h.c.store().borrow().search_rows_newest(10).unwrap();
    let newest = rows[0].id;
    assert_eq!(h.c.store().borrow().text(newest).unwrap().as_deref(), Some("Text"));
    assert!(h.c.viewer_view().unwrap().toast.contains("Copied the selection"));
}

#[test]
fn paused_recording_does_not_store_copied_ocr_but_resume_does() {
    let (mut h, generation) = recognized_image(IngestPolicy::default(), None);
    h.c.set_paused(true);
    h.tool(Tool::Copy);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(h.c.store().borrow().count().unwrap(), 1);
    assert_eq!(h.pasted_texts().len(), 1, "Copy still writes the clipboard while paused");
    h.c.set_paused(false);
    h.vkey(generation, Key::CtrlC);
    assert!(wait_for_count(&h, 2));
}

#[test]
fn failed_clipboard_copy_does_not_record_ocr_text() {
    let (mut h, _) = recognized_image(IngestPolicy::default(), None);
    h.p.write_ok = false;
    h.tool(Tool::Copy);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(h.c.store().borrow().count().unwrap(), 1);
    h.p.write_ok = true;
    h.tool(Tool::Copy);
    assert!(wait_for_count(&h, 2));
}

#[test]
fn excluded_pasteroll_copies_are_not_recorded() {
    let source = if cfg!(target_os = "windows") { "pasteroll.exe" } else { "pasteroll" };
    let policy = IngestPolicy { exclude_apps: [source.to_owned()].into_iter().collect(), ..Default::default() };
    let (mut h, _) = recognized_image(policy, None);
    h.tool(Tool::Copy);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(h.c.store().borrow().count().unwrap(), 1);
    assert_eq!(h.pasted_texts().len(), 1);
}

#[test]
fn region_ocr_masks_pixels_and_preserves_original_clip_and_copy_flow() {
    let mut h = H::new(&[Clip::Image]);
    let received = Arc::new(Mutex::new(Vec::new()));
    h.c.set_ocr(Arc::new(CaptureOcr(received.clone())));
    let generation = h.open();
    h.expand(generation);
    let id = h.c.viewer_clip().unwrap();
    let original = h.c.store().borrow().image_png(id).unwrap().unwrap();
    h.tool(Tool::OcrSelect);
    area(&mut h);
    h.tool(Tool::OcrRead);
    wait_for_text(&mut h);
    let images = received.lock().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height), (18, 18));
    assert_eq!(
        images[0]
            .pixels
            .chunks_exact(4)
            .filter(|p| *p == [10, 200, 30, 255])
            .count(),
        4
    );
    assert_eq!(
        h.c.store().borrow().image_png(id).unwrap().unwrap(),
        original
    );
    h.vkey(generation, Key::CtrlC);
    assert_eq!(h.pasted_texts(), vec!["Selected text"]);
    assert!(wait_for_count(&h, 2), "copied OCR text must become a history clip");
    // Repeated copies must deduplicate, rather than generating identical rows.
    h.tool(Tool::Copy);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(h.c.store().borrow().count().unwrap(), 2);
    h.vkey(generation, Key::Escape); // Return from OCR text to image.
    h.vkey(generation, Key::Escape); // Return to carousel.
    h.settle();
    assert!(h.c.is_open() && !h.c.viewer_is_open(), "same carousel session stays open");
    let newest = h.c.current_id().unwrap();
    assert_eq!(h.c.store().borrow().text(newest).unwrap().as_deref(), Some("Selected text"));
    assert!(matches!(h.vm().body, Body::Text { ref display, .. } if display == "Selected text"));
    assert!(
        h.p.pastes.is_empty(),
        "extracting does not automatically paste"
    );
}

#[test]
fn an_ocr_copy_completing_after_viewer_close_refreshes_the_open_carousel() {
    let (mut h, generation) = recognized_image(IngestPolicy::default(), None);
    h.tool(Tool::Copy);
    h.vkey(generation, Key::Escape);
    h.vkey(generation, Key::Escape);
    h.settle();
    assert!(h.c.is_open() && !h.c.viewer_is_open());
    assert!(wait_for_count(&h, 2));
    h.send(Event::Wake);
    let newest = h.c.current_id().unwrap();
    assert_eq!(h.c.store().borrow().text(newest).unwrap().as_deref(), Some("Text in a 6x4 picture"));
    assert!(matches!(h.vm().body, Body::Text { ref display, .. } if display == "Text in a 6x4 picture"));
}

#[test]
fn empty_or_invalid_area_does_not_fall_back_to_whole_image_ocr_or_paste() {
    let mut h = H::new(&[Clip::Image]);
    let received = Arc::new(Mutex::new(Vec::new()));
    h.c.set_ocr(Arc::new(CaptureOcr(received.clone())));
    let generation = h.open();
    h.expand(generation);
    h.tool(Tool::OcrSelect);
    draw(&mut h, &[(1.0, 1.0), (2.0, 2.0)]);
    h.vkey(generation, Key::Enter);
    assert!(received.lock().unwrap().is_empty());
    assert!(h.p.pastes.is_empty());
    assert!(h
        .c
        .viewer_view()
        .unwrap()
        .ocr_selection
        .unwrap()
        .regions
        .is_empty());
}

#[test]
fn multiple_outlines_support_undo_clear_and_cancel_without_deleting_clip() {
    let mut h = H::new(&[Clip::Image]);
    h.c.set_ocr(Arc::new(FakeOcr));
    let generation = h.open();
    h.expand(generation);
    let id = h.c.viewer_clip().unwrap();
    h.tool(Tool::OcrSelect);
    area(&mut h);
    area(&mut h);
    assert_eq!(
        h.c.viewer_view()
            .unwrap()
            .ocr_selection
            .unwrap()
            .regions
            .len(),
        2
    );
    h.vkey(generation, Key::CtrlZ);
    assert_eq!(
        h.c.viewer_view()
            .unwrap()
            .ocr_selection
            .unwrap()
            .regions
            .len(),
        1
    );
    h.vkey(generation, Key::Delete);
    assert!(h
        .c
        .viewer_view()
        .unwrap()
        .ocr_selection
        .unwrap()
        .regions
        .is_empty());
    assert!(!h.c.store().borrow().is_trashed(id).unwrap());
    h.vkey(generation, Key::Escape);
    assert!(h.c.viewer_is_open());
    assert!(h.c.viewer_view().unwrap().ocr_selection.is_none());
}

#[test]
fn zooming_keeps_outlines_in_source_coordinates() {
    let mut h = H::new(&[Clip::Image]);
    h.c.set_ocr(Arc::new(FakeOcr));
    let generation = h.open();
    h.expand(generation);
    h.tool(Tool::OcrSelect);
    area(&mut h);
    let before = h.c.viewer_view().unwrap().ocr_selection.unwrap().regions;
    h.tool(Tool::ZoomIn);
    let after = h.c.viewer_view().unwrap().ocr_selection.unwrap().regions;
    assert_eq!(before, after);
    area(&mut h);
    let regions = h.c.viewer_view().unwrap().ocr_selection.unwrap().regions;
    for (a, b) in regions[0].iter().zip(&regions[1]) {
        assert!((a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001);
    }
}

#[test]
fn cancelling_an_old_request_cannot_replace_a_new_ocr_result() {
    use std::sync::{atomic::AtomicUsize, mpsc};
    struct Delayed {
        calls: AtomicUsize,
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl TextRecognizer for Delayed {
        fn recognize(&self, _: &Rgba) -> Result<String, String> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.started.send(()).unwrap();
                let _ = self.release.lock().unwrap().recv();
                Ok("Old result".into())
            } else {
                Ok("New result".into())
            }
        }
    }
    let mut h = H::new(&[Clip::Image]);
    let (release, rx) = mpsc::channel();
    let (started, started_rx) = mpsc::channel();
    h.c.set_ocr(Arc::new(Delayed {
        calls: AtomicUsize::new(0),
        started,
        release: Mutex::new(rx),
    }));
    let generation = h.open();
    h.expand(generation);
    h.tool(Tool::OcrSelect);
    area(&mut h);
    h.vkey(generation, Key::Enter);
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    h.vkey(generation, Key::Escape);
    h.tool(Tool::OcrSelect);
    area(&mut h);
    h.vkey(generation, Key::Enter);
    wait_for_text(&mut h);
    assert_eq!(h.viewer_text(), "New result");
    release.send(()).unwrap();
    for _ in 0..20 {
        h.send(Event::Wake);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(h.viewer_text(), "New result");
}
