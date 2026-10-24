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
fn a_chosen_row_is_drawn_differently_and_the_others_are_not_touched() {
    let mut r = Renderer::new();
    let plain = text_vm("x", None);
    let before = r.render(&plain, SIZE, 1.0);
    let mut vm = text_vm("x", None);
    vm.picked = vec![false, false, true, false, false, false, false];
    let after = r.render(&vm, SIZE, 1.0);
    let g = geometry(&vm, SIZE, 1.0);
    assert!(region_differs(&before, &after, g.rows[2].unwrap(), 1.0), "the chosen row is marked");
    assert!(!region_differs(&before, &after, g.rows[1].unwrap().inset(2.0), 1.0), "its neighbour is not");
    assert!(!region_differs(&before, &after, g.rows[vm.active].unwrap().inset(2.0), 1.0), "nor is the one shown");
    // a list that has been given no choice at all draws exactly as it always did
    let mut none = text_vm("x", None);
    none.picked = vec![false; 7];
    assert_eq!(r.render(&none, SIZE, 1.0).bgra, before.bgra);
}

#[test]
fn a_menu_is_drawn_inside_the_window_and_catches_clicks() {
    let mut r = Renderer::new();
    let mut vm = text_vm("x", None);
    let plain = r.render(&vm, SIZE, 1.0);
    // opened at the bottom right corner: moved inward so all of it shows
    vm.menu = Some(MenuView { items: vec!["Copy".into(), "Clear selection".into()], at: (SIZE.0 - 3.0, SIZE.1 - 3.0), hover: Some(1) });
    let g = geometry(&vm, SIZE, 1.0);
    let menu = g.menu.expect("a menu");
    assert!(menu.x >= 0.0 && menu.y >= 0.0 && menu.x + menu.w <= SIZE.0 && menu.y + menu.h <= SIZE.1, "{menu:?}");
    assert_eq!(g.menu_items.len(), 2);
    assert!(region_differs(&plain, &r.render(&vm, SIZE, 1.0), menu, 1.0), "it is drawn");
    // the entries answer clicks, the rest of the menu swallows them, what is outside it is as before
    let it = g.menu_items[0];
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, it.x + 3.0, it.y + 3.0), Hit::MenuItem(0));
    let it = g.menu_items[1];
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, it.x + 3.0, it.y + 3.0), Hit::MenuItem(1));
    assert_eq!(r.hit_test(&vm, SIZE, 1.0, menu.x + 1.0, menu.y + 1.0), Hit::Nothing);
    assert_eq!(menu_item_at(&vm, SIZE, 1.0, it.x + 3.0, it.y + 3.0), Some(1));
    assert_eq!(menu_item_at(&vm, SIZE, 1.0, 2.0, 2.0), None);
    let under = g.rows[2].unwrap();
    vm.menu = None;
    assert!(matches!(r.hit_test(&vm, SIZE, 1.0, under.x + 3.0, under.y + 3.0), Hit::Row(_)));
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

// ---------------------------------------------------------------- No Bloat

const SLIM: (f32, f32) = (SLIM_WIDTH, SLIM_HEIGHT);

/// The window of a clip in No Bloat: the active clip only, as the controller builds it.
fn slim_vm(display: &str) -> ViewModel {
    ViewModel {
        rows: vec![RowView {
            kind: RowKind::Text,
            pinned: true,
            title: "clip".into(),
            meta: "Text, 4 characters".into(),
            highlights: vec![],
        }],
        active: 0,
        body: Body::Text { display: display.to_owned(), selection: None, scroll: 0.0 },
        card_buttons: true,
        slim: true,
        ..ViewModel::default()
    }
}

#[test]
fn each_mode_has_its_own_window_size() {
    use pasteroll_core::config::UiMode;
    assert_eq!(window_dip(UiMode::Complete), (WIDTH, HEIGHT));
    assert_eq!(window_dip(UiMode::NoBloat), (SLIM_WIDTH, SLIM_HEIGHT));
}

#[test]
fn no_bloat_has_no_list_buttons_or_idle_search_bar_to_hit() {
    let vm = slim_vm("hello");
    let g = geometry(&vm, SLIM, 1.0);
    let body = g.body.expect("the clip has a body");
    assert!(g.expand.is_none() && g.delete.is_none() && g.restore.is_none(), "no card buttons");
    assert!(g.dropdown.is_none() && g.dropdown_items.is_empty());
    // the clip fills the window inside the margins
    assert_eq!((body.x, body.y), (OUTER + INNER, OUTER + INNER));
    assert_eq!((body.w, body.h), (SLIM_WIDTH - 2.0 * (OUTER + INNER), SLIM_HEIGHT - 2.0 * (OUTER + INNER)));
    let mut r = Renderer::new();
    // wherever the search bar, filter button and card buttons are in the full window
    for (x, y) in [(40.0, 30.0), (SLIM_WIDTH - 60.0, 30.0), (SLIM_WIDTH - 24.0, 40.0), (SLIM_WIDTH - 24.0, SLIM_HEIGHT - 40.0), (4.0, 4.0)] {
        let hit = r.hit_test(&vm, SLIM, 1.0, x, y);
        assert!(
            !matches!(hit, Hit::Query | Hit::FilterButton | Hit::DropdownItem(_) | Hit::Expand | Hit::Delete | Hit::Restore),
            "({x}, {y}) hit {hit:?}"
        );
        assert!(button_at(&vm, SLIM, 1.0, x, y).is_none());
    }
    // the clip itself is still there to select from
    let centre = r.hit_test(&vm, SLIM, 1.0, SLIM_WIDTH / 2.0, SLIM_HEIGHT / 2.0);
    assert!(matches!(centre, Hit::Body { .. }), "{centre:?}");
}

#[test]
fn no_bloat_draws_the_clip_without_the_chrome() {
    let mut r = Renderer::new();
    let slim = r.render(&slim_vm("hello world"), SLIM, 1.0);
    assert_eq!((slim.width, slim.height), (480, 300));
    assert!(slim.bgra.chunks_exact(4).all(|p| p[3] == 255));
    // where the full window has its filter button the clip's own text does not reach: plain surface
    for (x, y) in [(400, 24), (440, 30), (440, 250), (300, 270)] {
        assert_eq!(slim.pixel(x, y), SURFACE, "({x}, {y})");
    }
    // the text is drawn on the surface
    let blank = r.render(&slim_vm(""), SLIM, 1.0);
    assert!(region_differs(&slim, &blank, geometry(&slim_vm("x"), SLIM, 1.0).body.unwrap(), 1.0));
}

#[test]
fn no_bloat_message_line_takes_room_only_while_there_is_a_message() {
    let quiet = geometry(&slim_vm("x"), SLIM, 1.0);
    let mut warned = slim_vm("x");
    warned.footer = "Press Enter again to paste 3 lines".into();
    let warned = geometry(&warned, SLIM, 1.0);
    assert!(warned.body.unwrap().h < quiet.body.unwrap().h);
    assert!(warned.body.unwrap().y + warned.body.unwrap().h <= warned.footer.y);
}

#[test]
fn no_bloat_shows_an_image_and_a_message() {
    let mut r = Renderer::new();
    let img = Arc::new(Rgba { width: 8, height: 4, pixels: vec![200; 8 * 4 * 4] });
    let mut vm = slim_vm("");
    vm.body = Body::Image(Some(img));
    let f = r.render(&vm, SLIM, 1.0);
    let b = geometry(&vm, SLIM, 1.0).body.unwrap();
    assert_ne!(f.pixel((b.x + b.w / 2.0) as u32, (b.y + b.h / 2.0) as u32), SURFACE, "the picture is drawn");
    let msg = ViewModel { body: Body::Message("Clipboard history is empty.".into()), slim: true, ..ViewModel::default() };
    let f = r.render(&msg, SLIM, 1.0);
    let blank = r.render(&ViewModel { slim: true, ..ViewModel::default() }, SLIM, 1.0);
    assert!(region_differs(&f, &blank, geometry(&msg, SLIM, 1.0).list, 1.0), "the message is drawn");
}

#[test]
fn no_bloat_shows_the_search_bar_only_while_it_is_in_use() {
    let quiet = slim_vm("hello");
    let mut searching = slim_vm("hello");
    searching.slim_bar = true;
    searching.query = "hel".into();
    let (a, b) = (geometry(&quiet, SLIM, 1.0), geometry(&searching, SLIM, 1.0));
    assert_eq!((a.query_box.w, a.filter_button.w), (0.0, 0.0), "no bar until it is needed");
    assert!(b.query_box.w > 0.0 && b.filter_button.w > 0.0);
    // the clip moves down to make room for it and keeps its width
    assert!(b.body.unwrap().y >= b.query_box.y + b.query_box.h);
    assert!(b.body.unwrap().h < a.body.unwrap().h);
    assert_eq!(b.body.unwrap().w, a.body.unwrap().w);
    let mut r = Renderer::new();
    assert_eq!(r.hit_test(&searching, SLIM, 1.0, b.query_box.x + 10.0, b.query_box.y + 10.0), Hit::Query);
    assert_eq!(r.hit_test(&searching, SLIM, 1.0, b.filter_button.x + 10.0, b.filter_button.y + 10.0), Hit::FilterButton);
    // the bar is drawn: the filter button is a lighter field, not plain surface
    let (fa, fb) = (r.render(&quiet, SLIM, 1.0), r.render(&searching, SLIM, 1.0));
    assert!(region_differs(&fa, &fb, b.filter_button, 1.0));
}

#[test]
fn no_bloat_filter_menu_wraps_into_columns_that_fit_the_small_window() {
    let items: Vec<String> = (0..14).map(|i| format!("Filter {i}")).collect();
    let mut vm = slim_vm("x");
    vm.slim_bar = true;
    vm.dropdown = Some(Dropdown { items: items.clone(), selected: 0, hover: None });
    let g = geometry(&vm, SLIM, 1.0);
    let menu = g.dropdown.expect("menu");
    assert!(menu.x >= 0.0 && menu.y >= 0.0 && menu.x + menu.w <= SLIM_WIDTH && menu.y + menu.h <= SLIM_HEIGHT, "{menu:?} inside the window");
    assert_eq!(g.dropdown_items.len(), 14);
    let cols = g.dropdown_items.iter().map(|r| r.x.to_bits()).collect::<std::collections::BTreeSet<_>>().len();
    assert!(cols > 1, "too tall for one column, so it wraps");
    // items fill each column top to bottom, left to right
    assert!(g.dropdown_items[1].y > g.dropdown_items[0].y && g.dropdown_items[1].x == g.dropdown_items[0].x);
    for w in g.dropdown_items.windows(2) {
        assert!(w[1].y > w[0].y || w[1].x > w[0].x, "in order");
    }
    for it in &g.dropdown_items {
        assert!(menu.contains(it.x + 1.0, it.y + 1.0) && menu.contains(it.x + it.w - 1.0, it.y + it.h - 1.0));
    }
    // every item can be hit, and nothing overlaps another
    let mut r = Renderer::new();
    for (i, it) in g.dropdown_items.iter().enumerate() {
        assert_eq!(r.hit_test(&vm, SLIM, 1.0, it.x + it.w / 2.0, it.y + it.h / 2.0), Hit::DropdownItem(i));
    }
    // the full window keeps its single column
    let mut full = text_vm("x", None);
    full.dropdown = Some(Dropdown { items, selected: 0, hover: None });
    let g = geometry(&full, SIZE, 1.0);
    assert_eq!(g.dropdown.unwrap().w, 170.0);
    assert!(g.dropdown_items.iter().all(|r| r.x == g.dropdown_items[0].x));
}

#[test]
fn no_bloat_marks_a_pinned_clip_and_leaves_a_lifted_card_empty() {
    let mut r = Renderer::new();
    let mut plain = slim_vm("hello");
    plain.rows[0].pinned = false;
    let mut pinned = slim_vm("hello");
    pinned.rows[0].pinned = true;
    let body = geometry(&plain, SLIM, 1.0).body.unwrap();
    let corner = Rect { x: body.x + body.w - 20.0, y: body.y, w: 20.0, h: 20.0 };
    assert!(region_differs(&r.render(&plain, SLIM, 1.0), &r.render(&pinned, SLIM, 1.0), corner, 1.0), "the star");
    let mut lifted = slim_vm("hello");
    lifted.lifted = true;
    let (a, b) = (r.render(&plain, SLIM, 1.0), r.render(&lifted, SLIM, 1.0));
    assert!(region_differs(&a, &b, body, 1.0));
    assert_ne!(b.pixel(body.x as u32 + 40, body.y as u32 + 40), SURFACE, "the card's place is drawn dark");
}
