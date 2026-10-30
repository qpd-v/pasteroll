//! No Bloat is a look, not a smaller program: the carousel shows only the clip, and every feature
//! still works from the keyboard. What it hides (the search bar, the filter menu, the card's
//! buttons, the list) comes up by itself when it is used.

use super::*;
use pasteroll_core::config::{ClickModifier, UiMode};
use pasteroll_render::Hit;

impl H {
    /// A harness whose carousel starts in No Bloat, like the program does from its config.
    pub(super) fn no_bloat(clips: &[Clip<'_>]) -> H {
        let mut h = H::new(clips);
        h.c.init_ui_mode(UiMode::NoBloat);
        h.p.mode = UiMode::NoBloat;
        h
    }

    fn is_pinned(&self, id: i64) -> bool {
        self.c.store().borrow().info(id).unwrap().unwrap().pinned
    }

    fn is_trashed(&self, id: i64) -> bool {
        self.c.store().borrow().is_trashed(id).unwrap()
    }

    /// The clip is in the history and not in the recycle bin.
    fn is_kept(&self, id: i64) -> bool {
        self.c.store().borrow().info(id).unwrap().is_some() && !self.is_trashed(id)
    }
}

/// Where the full window puts its controls, as pixel points (they are not drawn in No Bloat).
struct Complete {
    filter_button: (f32, f32),
    search_box: (f32, f32),
    expand: (f32, f32),
    delete: (f32, f32),
    neighbour: (f32, f32),
}

fn complete_layout() -> Complete {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    h.open();
    let vm = h.vm();
    let g = geometry(&vm, (pasteroll_render::WIDTH, pasteroll_render::HEIGHT), 1.0);
    let mid = |r: pasteroll_render::Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let neighbour = g.rows.iter().enumerate().find(|(i, r)| *i != vm.active && r.is_some()).unwrap().1.unwrap();
    Complete {
        filter_button: mid(g.filter_button),
        search_box: mid(g.query_box),
        expand: mid(g.expand.unwrap()),
        delete: mid(g.delete.unwrap()),
        neighbour: mid(neighbour),
    }
}

/// Opens the filter menu with Tab and picks the item at `index` with the keyboard (the menu
/// opens on the filter in use, so Down walks on from there).
fn pick_filter(h: &mut H, g: u64, index: usize) {
    let labels = crate::filters::labels(&[]);
    let from = labels.iter().position(|l| Some(l.as_str()) == h.c.filter_label().as_deref()).unwrap_or(0);
    key(h, g, Key::Tab);
    for _ in 0..(index + labels.len() - from) % labels.len() {
        key(h, g, Key::Down);
    }
    key(h, g, Key::Enter);
}

// ---------------------------------------------------------------- what it looks like

#[test]
fn it_shows_the_one_clip_in_its_own_small_window() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    h.open();
    let vm = h.vm();
    assert!(vm.slim && !vm.slim_bar, "no search bar until one is needed");
    assert_eq!(vm.rows.len(), 1, "no list of clips");
    assert!(!vm.card_buttons && vm.dropdown.is_none());
    assert!(vm.footer.is_empty(), "no count or other furniture");
    assert!(matches!(vm.body, Body::Text { ref display, .. } if display == "c"));
    assert_eq!(h.p.window(), (480, 300));
    assert!(h.p.frames >= 1, "a frame of the small window was presented (the fake checks its size)");
}

#[test]
fn the_wheel_and_arrows_scroll_through_the_clips_and_wrap() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    h.step_older(g);
    assert_eq!(newest_text(&mut h), "b");
    h.send(Event::Key(Key::Down, g));
    assert_eq!(newest_text(&mut h), "a");
    h.step_older(g);
    assert_eq!(newest_text(&mut h), "c", "wraps to the newest");
    h.send(Event::Key(Key::Up, g));
    assert_eq!(newest_text(&mut h), "a");
    assert_eq!(h.vm().rows.len(), 1, "still only the one clip");
}

#[test]
fn the_card_has_no_buttons_and_the_unseen_search_bar_cannot_be_clicked() {
    let at = complete_layout();
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    h.open();
    let id = h.c.current_id().unwrap();
    for point in [at.filter_button, at.search_box, at.expand, at.delete, at.neighbour] {
        h.mouse(MouseKind::Move, point);
        assert_eq!(h.vm().hover, None, "{point:?}: nothing lights up");
        h.mouse(MouseKind::Down, point);
        h.mouse(MouseKind::Up, point);
        assert_eq!(h.c.current_id(), Some(id), "{point:?}: the clip did not change");
        assert!(h.c.is_open());
    }
    assert!(h.vm().dropdown.is_none());
    assert!(!h.c.viewer_is_open());
    assert!(!h.is_trashed(id));
    // not even a shift-click or double click, which delete for good in the other mode
    h.p.shift = true;
    h.mouse(MouseKind::DoubleClick, at.delete);
    assert!(h.is_kept(id));
}

#[test]
fn nothing_in_the_window_is_a_button() {
    let mut h = H::no_bloat(&[Clip::Image]);
    h.open();
    let vm = h.vm();
    let mut r = pasteroll_render::Renderer::new();
    let size = pasteroll_render::window_dip(UiMode::NoBloat);
    for y in (4..300).step_by(16) {
        for x in (4..480).step_by(16) {
            let hit = r.hit_test(&vm, size, 1.0, x as f32, y as f32);
            assert!(matches!(hit, Hit::Nothing | Hit::Body { .. } | Hit::Row(0)), "({x}, {y}) is {hit:?}");
        }
    }
}

// ---------------------------------------------------------------- pasting

#[test]
fn enter_pastes_and_escape_closes() {
    let mut h = H::no_bloat(&[Clip::Text("older"), Clip::Text("newest")]);
    let g = h.open();
    h.step_older(g);
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["older"]);
    assert!(!h.c.is_open());

    let mut h = H::no_bloat(&[Clip::Text("x")]);
    let g = h.open();
    h.send(Event::Key(Key::Escape, g));
    assert!(!h.c.is_open());
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn clicking_the_destination_pastes_there() {
    let mut h = H::no_bloat(&[Clip::Text("hello")]);
    let g = h.open();
    let dest = target(2, "code.exe");
    h.p.current = dest.id;
    h.p.click_polls = VecDeque::from([ClickPoll::Target(dest)]);
    h.send(Event::ClickPaste(g));
    h.send(Event::Tick);
    assert_eq!(h.pasted_texts(), vec!["hello"]);
    assert!(!h.c.is_open());
}

#[test]
fn part_of_a_clip_can_still_be_selected_and_pasted() {
    let mut h = H::no_bloat(&[Clip::Text("The quick brown fox jumps over the lazy dog")]);
    let g = h.open();
    h.drag(4, 15);
    assert_eq!(h.c.selected_text().as_deref(), Some("quick brown"));
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["quick brown"]);
}

#[test]
fn long_text_still_scrolls_with_the_page_keys() {
    let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let mut h = H::no_bloat(&[Clip::Text(&long)]);
    let g = h.open();
    h.send(Event::Key(Key::PageDown, g));
    assert!(matches!(h.vm().body, Body::Text { scroll, .. } if scroll > 0.0));
}

#[test]
fn the_terminal_warning_still_protects_a_multi_line_paste() {
    let mut h = H::no_bloat(&[Clip::Text("echo one\necho two")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty(), "the first Enter does not paste");
    assert!(h.vm().footer.contains("terminal"), "the warning is shown: {}", h.vm().footer);
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["echo one\necho two"]);
}

#[test]
fn a_destination_hint_is_still_shown_when_browsing_from_the_tray() {
    let mut h = H::no_bloat(&[Clip::Text("tray history")]);
    h.p.foreground = None;
    h.send(Event::BrowseAt(1800, 1040));
    assert!(h.c.is_open());
    assert!(h.vm().footer.contains("destination"), "{}", h.vm().footer);
    let g = h.p.input_generation();
    h.enter(g);
    assert!(h.p.clipboard.is_empty(), "no destination yet, so Enter pastes nothing");
}

// ---------------------------------------------------------------- search and filters appear when used

#[test]
fn typing_searches_and_brings_up_the_search_bar() {
    let mut h = H::no_bloat(&[Clip::Text("alpha one"), Clip::Text("beta two"), Clip::Text("alpha three")]);
    let g = h.open();
    assert!(!h.vm().slim_bar);
    type_text(&mut h, g, "alp");
    assert_eq!(h.c.query(), Some("alp"));
    assert_eq!(h.c.match_counts(), Some((2, 3)));
    let vm = h.vm();
    assert!(vm.slim_bar, "the bar appears as you type");
    assert_eq!(vm.query, "alp");
    assert_eq!(vm.footer, "2 of 3 clips");
    assert_eq!(vm.rows.len(), 1, "still one clip at a time");
    assert_eq!(newest_text(&mut h), "alpha three");
    h.step_older(g);
    assert_eq!(newest_text(&mut h), "alpha one", "scrolling walks the matches");
    // Escape clears the search (and the bar goes away), the next one closes
    key(&mut h, g, Key::Escape);
    assert!(h.c.is_open());
    let vm = h.vm();
    assert!(!vm.slim_bar && vm.query.is_empty() && vm.footer.is_empty());
    key(&mut h, g, Key::Escape);
    assert!(!h.c.is_open());
}

#[test]
fn enter_pastes_the_clip_the_search_found() {
    let mut h = H::no_bloat(&[Clip::Text("alpha one"), Clip::Text("beta two")]);
    let g = h.open();
    type_text(&mut h, g, "alpha");
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["alpha one"]);
}

#[test]
fn tab_brings_up_the_filter_menu_and_a_filter_keeps_the_bar_visible() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b"), Clip::Image]);
    let g = h.open();
    key(&mut h, g, Key::Tab);
    let vm = h.vm();
    assert!(vm.slim_bar && vm.dropdown.is_some(), "the menu needs its button, so the bar comes with it");
    // the whole menu fits inside the small window
    let geo = geometry(&vm, (480.0, 300.0), 1.0);
    let menu = geo.dropdown.unwrap();
    assert!(menu.x + menu.w <= 480.0 && menu.y + menu.h <= 300.0, "{menu:?}");
    key(&mut h, g, Key::Tab);
    assert!(!h.vm().slim_bar, "closing the menu with nothing chosen hides the bar again");
    // Image is the fourth filter
    pick_filter(&mut h, g, 3);
    assert_eq!(h.c.filter_label().as_deref(), Some("Image"));
    let vm = h.vm();
    assert!(vm.slim_bar, "a filter in use stays visible");
    assert_eq!(vm.footer, "1 of 3 clips");
    assert!(matches!(vm.body, Body::Image(_) | Body::Message(_)));
    // back to Everything: the bar goes
    pick_filter(&mut h, g, 0);
    assert!(!h.vm().slim_bar);
}

// ---------------------------------------------------------------- pin, delete, recycle bin

#[test]
fn ctrl_p_pins_and_the_star_shows_on_the_card() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    let id = h.c.current_id().unwrap();
    assert!(!h.vm().rows[0].pinned);
    key(&mut h, g, Key::CtrlF);
    assert!(h.is_pinned(id));
    assert!(h.vm().rows[0].pinned, "the card shows its pin");
    key(&mut h, g, Key::CtrlF);
    assert!(!h.is_pinned(id) && !h.vm().rows[0].pinned);
}

#[test]
fn delete_uses_the_recycle_bin_and_ctrl_z_brings_it_back() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    let id = h.c.current_id().unwrap();
    key(&mut h, g, Key::Delete);
    assert!(h.is_trashed(id), "moved to the recycle bin");
    assert_ne!(h.c.current_id(), Some(id), "on to the next clip");
    assert!(h.vm().footer.contains("recycle bin"), "it says so: {}", h.vm().footer);
    key(&mut h, g, Key::CtrlZ);
    assert!(h.is_kept(id), "and Ctrl+Z brings it back");
}

#[test]
fn the_recycle_bin_filter_and_deleting_for_good_work_from_the_keyboard() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    let id = h.c.current_id().unwrap();
    key(&mut h, g, Key::Delete);
    assert!(h.is_trashed(id));
    // "Recycle bin" is the ninth filter
    pick_filter(&mut h, g, 8);
    assert_eq!(h.c.filter_label().as_deref(), Some("Recycle bin"));
    assert_eq!(h.c.current_id(), Some(id), "the binned clip is what the bin shows");
    assert!(h.vm().slim_bar);
    // Shift+Delete asks first, in the message line
    key(&mut h, g, Key::ShiftDelete);
    assert!(h.vm().footer.contains("permanently"), "{}", h.vm().footer);
    key(&mut h, g, Key::Escape);
    assert!(h.is_trashed(id), "Escape keeps it");
    key(&mut h, g, Key::ShiftDelete);
    h.enter(g);
    assert!(!h.is_kept(id) && h.c.store().borrow().info(id).unwrap().is_none(), "deleted for good");
    assert!(h.p.clipboard.is_empty(), "that Enter confirmed the question, it pasted nothing");
}

// ---------------------------------------------------------------- the expanded viewer and the image editor

#[test]
fn ctrl_e_opens_the_expanded_viewer_and_escape_returns_to_the_card() {
    let mut h = H::no_bloat(&[Clip::Text("older"), Clip::Text("hello viewer")]);
    let g = h.open();
    h.expand(g);
    assert_eq!(h.viewer_text(), "hello viewer");
    // the card's place is empty behind it, and the carousel behind is dimmed
    assert!(h.p.last_overlay.is_some());
    h.vkey(g, Key::Left);
    h.settle();
    assert_eq!(h.viewer_text(), "older", "the viewer browses like in the other mode");
    h.send(Event::Key(Key::Escape, g));
    h.settle();
    assert!(!h.c.viewer_is_open());
    assert!(h.c.is_open(), "back at the carousel");
    let vm = h.vm();
    assert!(vm.slim && !vm.lifted);
}

#[test]
fn the_expanded_card_grows_out_of_the_small_card() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("hello viewer")]);
    let g = h.open();
    h.send(Event::Key(Key::CtrlE, g));
    assert!(h.c.viewer_is_open());
    let first = h.p.viewer_rect.unwrap();
    let vm = h.vm();
    let card = geometry(&vm, (480.0, 300.0), 1.0).rows[vm.active].unwrap();
    assert_eq!(first, (316 + card.x.round() as i32, 320 + card.y.round() as i32, card.w.round() as u32, card.h.round() as u32));
    h.settle();
    assert_eq!(h.p.viewer_rect, Some((240, 115, 1440, 810)));
}

#[test]
fn an_image_clicked_onto_the_desktop_can_still_be_edited_first() {
    let (mut h, _folder) = H::desktop_session(&[Clip::Gradient(40, 30)]);
    h.c.init_ui_mode(UiMode::NoBloat);
    h.p.mode = UiMode::NoBloat;
    let g = h.open();
    // Ctrl is the key that opens the editor (a setting)
    h.click_desktop(g, Modifiers { ctrl: true, ..Modifiers::default() });
    assert!(h.c.editor_is_open(), "the editor opens in No Bloat too");
    assert!(h.p.created.is_empty(), "nothing is saved until it is told to");
    h.settle_editor();
    h.wait_for(|h| h.c.editor_view().is_some_and(|v| v.paste_enabled && !v.busy));
}

#[test]
fn the_plain_desktop_click_still_saves_the_picture_at_once() {
    let (mut h, folder) = H::desktop_session(&[Clip::Gradient(40, 30)]);
    h.c.init_ui_mode(UiMode::NoBloat);
    h.p.mode = UiMode::NoBloat;
    let g = h.open();
    h.click_desktop(g, Modifiers::default());
    assert!(!h.c.editor_is_open() && !h.c.is_open());
    assert_eq!(h.p.created.len(), 1);
    assert!(h.p.created[0].starts_with(&folder));
}

#[test]
fn the_editor_key_can_be_changed_in_no_bloat_too() {
    let (mut h, _folder) = H::desktop_session(&[Clip::Gradient(40, 30)]);
    h.c.init_ui_mode(UiMode::NoBloat);
    h.p.mode = UiMode::NoBloat;
    h.c.set_advanced_modifier(ClickModifier::Shift);
    let g = h.open();
    h.click_desktop(g, Modifiers { shift: true, ..Modifiers::default() });
    assert!(h.c.editor_is_open());
}

// ---------------------------------------------------------------- switching

#[test]
fn switching_modes_closes_an_open_carousel_and_resizes_the_next() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    assert_eq!(h.c.ui_mode(), UiMode::Complete);
    h.open();
    assert!(!h.vm().slim);
    assert_eq!(h.p.window(), (520, 440));

    h.c.set_ui_mode(&mut h.p, UiMode::NoBloat);
    assert!(!h.c.is_open(), "the open carousel is closed, not reshaped under the pointer");
    assert_eq!(h.c.ui_mode(), UiMode::NoBloat);
    assert_eq!(h.p.mode_changes, vec![UiMode::NoBloat]);
    h.open();
    assert!(h.vm().slim);
    assert_eq!(h.p.window(), (480, 300));
    assert!(h.p.clipboard.is_empty());

    h.c.set_ui_mode(&mut h.p, UiMode::Complete);
    h.open();
    let vm = h.vm();
    assert!(!vm.slim && vm.card_buttons, "everything is back");
    assert_eq!(h.p.window(), (520, 440));
    assert_eq!(h.p.mode_changes, vec![UiMode::NoBloat, UiMode::Complete]);
}

#[test]
fn choosing_the_mode_you_are_in_changes_nothing() {
    let mut h = H::no_bloat(&[Clip::Text("a")]);
    h.open();
    h.c.set_ui_mode(&mut h.p, UiMode::NoBloat);
    assert!(h.c.is_open(), "the carousel stays up");
    assert!(h.p.mode_changes.is_empty());
}

#[test]
fn the_history_is_the_same_in_both_modes() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    key(&mut h, g, Key::CtrlF);
    let id = h.c.current_id().unwrap();
    assert!(h.is_pinned(id));
    h.send(Event::Key(Key::Escape, g));
    h.c.set_ui_mode(&mut h.p, UiMode::NoBloat);
    h.open();
    assert_eq!(h.c.current_id(), Some(id));
    assert!(h.is_pinned(id), "a pin made in Complete survives in No Bloat");
}

#[test]
fn a_clip_copied_while_it_is_open_keeps_the_slim_look() {
    let mut h = H::no_bloat(&[Clip::Text("a"), Clip::Text("b")]);
    h.open();
    h.c.store().borrow_mut().add_text("fresh", 1_700_000_100).unwrap();
    h.p.now_ms += 5;
    h.send(Event::Wake);
    h.send(Event::Tick);
    let vm = h.vm();
    assert!(vm.slim && !vm.slim_bar && vm.rows.len() == 1 && vm.dropdown.is_none());
}
