use super::*;
use std::sync::Arc;

const SIZE: (f32, f32) = (WIDTH, HEIGHT);

fn text_vm(display: &str, selection: Option<(usize, usize)>) -> ViewModel {
    ViewModel {
        query: String::new(),
        rows: (0..7)
            .map(|i| RowView {
                kind: RowKind::Text,
                pinned: i == 5,
                title: format!("clip number {i}"),
                meta: format!("Text, {i} characters"),
                highlights: vec![],
            })
            .collect(),
        active: 3,
        body: Body::Text { display: display.to_owned(), selection, scroll: 0.0 },
        footer: "4 of 7".into(),
        ..ViewModel::default()
    }
}

fn region_differs(a: &Frame, b: &Frame, r: Rect, scale: f32) -> bool {
    let (x0, y0) = ((r.x * scale) as u32, (r.y * scale) as u32);
    let (x1, y1) = (((r.x + r.w) * scale) as u32, ((r.y + r.h) * scale) as u32);
    for y in y0..y1.min(a.height) {
        for x in x0..x1.min(a.width) {
            if a.pixel(x, y) != b.pixel(x, y) {
                return true;
            }
        }
    }
    false
}

#[test]
fn frame_has_the_requested_size_and_is_opaque() {
    let mut r = Renderer::new();
    let f = r.render(&text_vm("hello", None), SIZE, 1.0);
    assert_eq!((f.width, f.height), (520, 440));
    assert_eq!(f.bgra.len(), 520 * 440 * 4);
    assert!(f.bgra.chunks_exact(4).all(|p| p[3] == 255), "every pixel must be opaque");
    let f2 = r.render(&text_vm("hello", None), SIZE, 2.0);
    assert_eq!((f2.width, f2.height), (1040, 880));
}

#[test]
fn background_and_surface_colours_are_where_expected() {
    let mut r = Renderer::new();
    let f = r.render(&text_vm("x", None), SIZE, 1.0);
    assert_eq!(f.pixel(1, 1), BACKGROUND);
    assert_eq!(f.pixel(12, 200), SURFACE); // inside the surface margin, outside the list
}

#[test]
fn geometry_places_the_active_row_larger_than_the_rest_without_overlap() {
    let vm = text_vm("x", None);
    let g = geometry(&vm, SIZE, 1.0);
    let rects: Vec<Rect> = g.rows.iter().flatten().copied().collect();
    assert!(rects.len() >= 3, "expected several rows, got {}", rects.len());
    let active = g.rows[vm.active].unwrap();
    for (i, r) in g.rows.iter().enumerate() {
        if let Some(r) = r {
            if i != vm.active {
                assert!(active.h > r.h * 2.0, "active {} vs row {}", active.h, r.h);
            }
        }
    }
    // rows are ordered top to bottom and do not overlap
    let mut last_bottom = g.list.y - 0.01;
    for r in &rects {
        assert!(r.y >= last_bottom - 0.01, "overlap at y={}", r.y);
        last_bottom = r.y + r.h;
    }
    assert!(last_bottom <= g.list.y + g.list.h + 0.5, "rows overflow the list area");
}

#[test]
fn with_one_row_it_is_the_active_row_alone() {
    let mut vm = text_vm("only", None);
    vm.rows.truncate(1);
    vm.active = 0;
    let g = geometry(&vm, SIZE, 1.0);
    assert!(g.rows[0].is_some());
    assert!(g.body.is_some());
}

#[test]
fn empty_state_renders_a_message_without_rows() {
    let mut r = Renderer::new();
    let vm = ViewModel { body: Body::Message("Nothing matches".into()), ..ViewModel::default() };
    let empty = r.render(&vm, SIZE, 1.0);
    let blank = r.render(&ViewModel::default(), SIZE, 1.0);
    let g = geometry(&vm, SIZE, 1.0);
    assert!(region_differs(&empty, &blank, g.list, 1.0), "message should be drawn");
}

#[test]
fn typing_a_query_changes_the_search_bar() {
    let mut r = Renderer::new();
    let a = r.render(&text_vm("x", None), SIZE, 1.0);
    let mut vm = text_vm("x", None);
    vm.query = "invoice".into();
    let b = r.render(&vm, SIZE, 1.0);
    let g = geometry(&vm, SIZE, 1.0);
    assert!(region_differs(&a, &b, g.query_box, 1.0));
    // and nothing outside the bar and the (unchanged) rest of the window moved
    assert!(!region_differs(&a, &b, g.list, 1.0));
}

#[test]
fn selection_paints_over_the_selected_text_only() {
    let mut r = Renderer::new();
    let none = r.render(&text_vm("one two three", None), SIZE, 1.0);
    let sel = r.render(&text_vm("one two three", Some((4, 7))), SIZE, 1.0);
    let g = geometry(&text_vm("x", None), SIZE, 1.0);
    let body = g.body.unwrap();
    assert!(region_differs(&none, &sel, body, 1.0));
    // the search bar is untouched by a selection
    assert!(!region_differs(&none, &sel, g.query_box, 1.0));
}

#[test]
fn reversed_selection_ranges_draw_the_same() {
    let mut r = Renderer::new();
    let a = r.render(&text_vm("one two three", Some((4, 7))), SIZE, 1.0);
    let b = r.render(&text_vm("one two three", Some((7, 4))), SIZE, 1.0);
    assert_eq!(a.bgra, b.bgra);
}

#[test]
fn hit_testing_routes_to_the_right_targets() {
    let mut r = Renderer::new();
    let vm = text_vm("hello world", None);
    let g = geometry(&vm, SIZE, 1.0);
    let c = |rect: Rect| (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let (x, y) = c(g.query_box);
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, x, y), Hit::Query);
    let (x, y) = c(g.filter_button);
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, x, y), Hit::FilterButton);
    let above = g.rows.iter().enumerate().find(|(i, r)| r.is_some() && *i < vm.active).unwrap();
    let (x, y) = c(above.1.unwrap());
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, x, y), Hit::Row(above.0));
    let b = g.body.unwrap();
    let (x, y) = c(b);
    assert!(matches!(r.hit_test(&vm, SIZE, 1.0, x, y), Hit::Body { .. }));
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, 1.0, 1.0), Hit::Nothing);
}

#[test]
fn text_positions_increase_left_to_right_and_clamp() {
    let mut r = Renderer::new();
    let vm = text_vm("abcdefghij", None);
    let g = geometry(&vm, SIZE, 1.0);
    let b = g.body.unwrap();
    let y = b.y + 8.0;
    let mut last = 0usize;
    for k in 0..20 {
        let x = b.x + 2.0 + k as f32 * 6.0;
        let p = r.text_position(&vm, SIZE, 1.0, x, y).unwrap();
        assert!(p >= last, "position went backwards at x={x}: {p} < {last}");
        last = p;
    }
    assert_eq!(r.text_position(&vm, SIZE, 1.0, b.x + b.w - 1.0, y).unwrap(), 10);
    // far above the body clamps to the start
    assert_eq!(r.text_position(&vm, SIZE, 1.0, b.x + 50.0, -500.0).unwrap(), 0);
}

#[test]
fn char_centre_and_hit_agree() {
    let mut r = Renderer::new();
    let vm = text_vm("The quick brown fox", None);
    let (x, y) = r.char_centre(&vm, SIZE, 1.0, 4).expect("centre of 'q'");
    let p = r.text_position(&vm, SIZE, 1.0, x, y).unwrap();
    assert!(p == 4 || p == 5, "centre of char 4 hit-tested to {p}");
}

#[test]
fn multi_line_text_maps_lines_correctly() {
    let mut r = Renderer::new();
    let vm = text_vm("first line\r\nsecond line\r\nthird", None);
    let g = geometry(&vm, SIZE, 1.0);
    let b = g.body.unwrap();
    // click in the second text line (one BODY_LINE below the first)
    let y = b.y + BODY_LINE * 1.5;
    let p = r.text_position(&vm, SIZE, 1.0, b.x + 0.5, y).unwrap();
    assert_eq!(p, "first line\r\n".encode_utf16().count(), "start of line two");
}

#[test]
fn long_text_can_scroll_and_short_text_cannot() {
    let mut r = Renderer::new();
    let long = (0..80).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    assert!(r.max_scroll(&text_vm(&long, None), SIZE, 1.0) > 100.0);
    assert_eq!(r.max_scroll(&text_vm("short", None), SIZE, 1.0), 0.0);
}

#[test]
fn scrolling_moves_the_text() {
    let mut r = Renderer::new();
    let long = (0..80).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let mut vm = text_vm(&long, None);
    let a = r.render(&vm, SIZE, 1.0);
    vm.body = Body::Text { display: long.clone(), selection: None, scroll: 60.0 };
    let b = r.render(&vm, SIZE, 1.0);
    let g = geometry(&vm, SIZE, 1.0);
    assert!(region_differs(&a, &b, g.body.unwrap(), 1.0));
}

#[test]
fn image_body_draws_the_image_centred() {
    let mut r = Renderer::new();
    let red = Rgba { width: 40, height: 30, pixels: [255u8, 0, 0, 255].repeat(40 * 30) };
    let mut vm = text_vm("x", None);
    vm.body = Body::Image(Some(Arc::new(red)));
    let f = r.render(&vm, SIZE, 1.0);
    let b = geometry(&vm, SIZE, 1.0).body.unwrap();
    let p = f.pixel((b.x + b.w / 2.0) as u32, (b.y + b.h / 2.0) as u32);
    assert!(p.0 > 200 && p.1 < 60 && p.2 < 60, "centre pixel {p:?}");
}

#[test]
fn image_still_loading_draws_an_empty_frame() {
    let mut r = Renderer::new();
    let mut vm = text_vm("x", None);
    vm.body = Body::Image(None);
    let f = r.render(&vm, SIZE, 1.0);
    let b = geometry(&vm, SIZE, 1.0).body.unwrap();
    assert_eq!(f.pixel((b.x + b.w / 2.0) as u32, (b.y + b.h / 2.0) as u32), ACTIVE_ROW);
}

#[test]
fn transparent_image_pixels_show_the_row_colour() {
    let mut r = Renderer::new();
    let clear = Rgba { width: 8, height: 8, pixels: [255u8, 0, 0, 0].repeat(64) };
    let mut vm = text_vm("x", None);
    vm.body = Body::Image(Some(Arc::new(clear)));
    let f = r.render(&vm, SIZE, 1.0);
    let b = geometry(&vm, SIZE, 1.0).body.unwrap();
    assert_eq!(f.pixel((b.x + b.w / 2.0) as u32, (b.y + b.h / 2.0) as u32), ACTIVE_ROW);
}

#[test]
fn dropdown_draws_and_is_hit_testable() {
    let mut r = Renderer::new();
    let mut vm = text_vm("x", None);
    let closed = r.render(&vm, SIZE, 1.0);
    vm.dropdown = Some(Dropdown {
        items: ["Everything", "Text", "Document", "Image"].iter().map(|s| s.to_string()).collect(),
        selected: 0,
        hover: Some(1),
    });
    let open = r.render(&vm, SIZE, 1.0);
    let g = geometry(&vm, SIZE, 1.0);
    assert!(region_differs(&closed, &open, g.dropdown.unwrap(), 1.0));
    let it = g.dropdown_items[2];
    assert_eq!(
        r.hit_test(&vm, SIZE, 1.0, it.x + it.w / 2.0, it.y + it.h / 2.0),
        Hit::DropdownItem(2)
    );
    // the open dropdown captures clicks that would otherwise reach the rows beneath it
    let d = g.dropdown.unwrap();
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, d.x + 1.0, d.y + d.h - 1.0), Hit::Nothing);
}

#[test]
fn query_highlight_marks_the_matching_characters() {
    let mut r = Renderer::new();
    let plain = r.render(&text_vm("x", None), SIZE, 1.0);
    let mut vm = text_vm("x", None);
    vm.rows[1].highlights = vec![(5, 11)];
    let hl = r.render(&vm, SIZE, 1.0);
    let g = geometry(&vm, SIZE, 1.0);
    let row = g.rows[1].expect("row 1 visible");
    assert!(region_differs(&plain, &hl, row, 1.0));
}

#[test]
fn pinned_rows_show_a_star() {
    let mut r = Renderer::new();
    let mut vm = text_vm("x", None);
    vm.rows[4].pinned = false;
    let off = r.render(&vm, SIZE, 1.0);
    vm.rows[4].pinned = true;
    let on = r.render(&vm, SIZE, 1.0);
    let row = geometry(&vm, SIZE, 1.0).rows[4].expect("row 4 visible");
    assert!(region_differs(&off, &on, row, 1.0));
}

#[test]
fn first_line_collapses_whitespace_and_cuts_at_newlines() {
    assert_eq!(first_line("  hello \t  world \r\nnext"), "hello world");
    assert_eq!(first_line(""), "");
}

#[test]
fn rendering_is_deterministic() {
    let mut r = Renderer::new();
    let vm = text_vm("The quick brown fox", Some((4, 9)));
    let a = r.render(&vm, SIZE, 1.0);
    let b = r.render(&vm, SIZE, 1.0);
    assert_eq!(a.bgra, b.bgra);
}

#[test]
fn hidpi_hit_testing_scales() {
    let mut r = Renderer::new();
    let vm = text_vm("hello", None);
    let g = geometry(&vm, SIZE, 2.0);
    let q = g.query_box;
    assert_eq!(
        r.hit_test(&vm, SIZE, 2.0, (q.x + 10.0) * 2.0, (q.y + 5.0) * 2.0),
        Hit::Query
    );
}

#[test]
fn a_selection_on_one_line_does_not_highlight_other_lines() {
    let mut r = Renderer::new();
    let text = "alpha beta
gamma delta
epsilon zeta
eta theta
iota kappa";
    let none = r.render(&text_vm(text, None), SIZE, 1.0);
    let sel = r.render(&text_vm(text, Some((2, 6))), SIZE, 1.0);
    let g = geometry(&text_vm("x", None), SIZE, 1.0);
    let b = g.body.unwrap();
    // lines 3 and 4 (0-based) are well below the selected first line
    let later = Rect { x: b.x, y: b.y + BODY_LINE * 3.0, w: b.w, h: BODY_LINE * 2.0 };
    assert!(!region_differs(&none, &sel, later, 1.0), "selection leaked onto later lines");
    let first = Rect { x: b.x, y: b.y, w: b.w, h: BODY_LINE };
    assert!(region_differs(&none, &sel, first, 1.0), "selection missing on its own line");
}

#[test]
fn a_multi_line_selection_highlights_every_spanned_line() {
    let mut r = Renderer::new();
    let text = "alpha beta
gamma delta
epsilon zeta
eta theta";
    let none = r.render(&text_vm(text, None), SIZE, 1.0);
    // from inside line 0 to inside line 2
    let sel = r.render(&text_vm(text, Some((3, 28))), SIZE, 1.0);
    let b = geometry(&text_vm("x", None), SIZE, 1.0).body.unwrap();
    for line in 0..3 {
        let rect = Rect { x: b.x, y: b.y + BODY_LINE * line as f32, w: b.w, h: BODY_LINE };
        assert!(region_differs(&none, &sel, rect, 1.0), "line {line} should be highlighted");
    }
    let last = Rect { x: b.x, y: b.y + BODY_LINE * 3.0, w: b.w, h: BODY_LINE };
    assert!(!region_differs(&none, &sel, last, 1.0), "line 3 is outside the selection");
}

#[test]
fn the_active_row_header_shows_meta_not_a_repeat_of_the_text() {
    let mut r = Renderer::new();
    let mut a = text_vm("body", None);
    a.rows[3].meta = "Text, 4 characters".into();
    let mut b = a.clone();
    b.rows[3].meta = "Image, 200 x 120".into();
    let (fa, fb) = (r.render(&a, SIZE, 1.0), r.render(&b, SIZE, 1.0));
    let g = geometry(&a, SIZE, 1.0);
    let header = g.rows[3].unwrap();
    let header = Rect { x: header.x, y: header.y, w: header.w, h: 28.0 };
    assert!(region_differs(&fa, &fb, header, 1.0), "meta text should be what the active header draws");
}

#[test]
fn wiping_a_view_model_clears_every_clip_string() {
    let mut vm = text_vm("secret clip text", Some((0, 3)));
    vm.query = "secret query".into();
    vm.footer = "secret footer".into();
    vm.wipe();
    assert!(vm.query.is_empty() && vm.footer.is_empty());
    assert!(vm.rows.iter().all(|r| r.title.is_empty() && r.meta.is_empty()));
    match &vm.body {
        Body::Text { display, .. } => assert!(display.is_empty()),
        _ => panic!("text body"),
    }
}
