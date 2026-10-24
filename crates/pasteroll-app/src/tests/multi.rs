//! Choosing several clips with Ctrl+click and pasting or copying them together (Complete mode).

use super::*;
use pasteroll_core::config::UiMode;

const CTRL: Modifiers = Modifiers { ctrl: true, shift: false, alt: false };
const CTRL_SHIFT: Modifiers = Modifiers { ctrl: true, shift: true, alt: false };

impl H {
    /// The middle of row `i` of the carousel, in pixels.
    fn row_point(&mut self, i: usize) -> (f32, f32) {
        let vm = self.vm();
        let size = self.p.window();
        let g = pasteroll_render::geometry(&vm, (size.0 as f32, size.1 as f32), 1.0);
        let r = g.rows[i].expect("that row is on show");
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    /// A click on row `i` with `mods` held.
    fn click_row(&mut self, i: usize, mods: Modifiers) {
        let at = self.row_point(i);
        self.p.mods = mods;
        self.mouse(MouseKind::Down, at);
        self.mouse(MouseKind::Up, at);
        self.p.mods = Modifiers::default();
    }

    fn right_click_row(&mut self, i: usize) {
        let at = self.row_point(i);
        self.mouse(MouseKind::RightDown, at);
    }

    /// Which rows are chosen.
    fn chosen(&mut self) -> Vec<usize> {
        let vm = self.vm();
        (0..vm.rows.len()).filter(|i| vm.picked.get(*i).copied().unwrap_or(false)).collect()
    }

    /// Clicks the window in front, which pastes.
    fn click_to_paste(&mut self, g: u64) {
        self.click_window(g, Modifiers::default());
    }

    fn footer(&mut self) -> String {
        self.vm().footer
    }
}

fn four() -> H {
    // d is the newest, so row 0 is d, then c, b and a
    H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c"), Clip::Text("d")])
}

#[test]
fn ctrl_click_chooses_a_clip_and_again_takes_it_out() {
    let mut h = four();
    h.open();
    assert!(h.chosen().is_empty());
    h.click_row(2, CTRL);
    assert_eq!(h.chosen(), vec![2]);
    assert!(h.footer().starts_with("1 chosen"), "{}", h.footer());
    h.click_row(1, CTRL);
    assert_eq!(h.chosen(), vec![1, 2]);
    assert!(h.footer().starts_with("2 chosen"), "{}", h.footer());
    h.click_row(2, CTRL);
    assert_eq!(h.chosen(), vec![1]);
    h.click_row(1, CTRL);
    assert!(h.chosen().is_empty());
    assert!(!h.footer().contains("chosen"), "{}", h.footer());
}

#[test]
fn ctrl_clicking_the_clip_that_is_shown_chooses_it_too() {
    let mut h = four();
    h.open();
    h.click_row(0, CTRL);
    assert_eq!(h.chosen(), vec![0]);
}

#[test]
fn a_plain_click_still_moves_to_the_clip_and_keeps_the_choice() {
    let mut h = four();
    h.open();
    h.click_row(1, CTRL);
    h.click_row(2, Modifiers::default());
    assert_eq!(newest_text(&mut h), "b", "the click moved there");
    assert_eq!(h.p.pastes.len(), 0);
    let vm = h.vm();
    assert_eq!(vm.picked.iter().filter(|p| **p).count(), 1, "the choice is kept");
}

#[test]
fn ctrl_shift_click_chooses_everything_from_the_last_choice_to_the_click() {
    let mut h = four();
    let g = h.open();
    h.click_row(1, CTRL); // c
    h.click_row(3, CTRL_SHIFT); // down to a
    assert_eq!(h.chosen(), vec![1, 2, 3]);
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["c\nb\na"], "in the order walked from the first");
}

#[test]
fn a_range_walked_upwards_is_in_the_order_walked() {
    let mut h = four();
    let g = h.open();
    h.click_row(3, CTRL); // a
    h.click_row(1, CTRL_SHIFT); // up to c
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["a\nb\nc"]);
}

#[test]
fn a_range_with_no_start_is_just_one_click() {
    let mut h = four();
    h.open();
    h.click_row(2, CTRL_SHIFT);
    assert_eq!(h.chosen(), vec![2]);
}

#[test]
fn the_clips_are_pasted_one_per_line_in_the_order_they_were_chosen() {
    let mut h = four();
    let g = h.open();
    h.click_row(3, CTRL); // a
    h.click_row(0, CTRL); // d
    h.click_row(2, CTRL); // b
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["a\nd\nb"]);
    assert_eq!(h.p.pastes.len(), 1, "one paste, not three");
    assert!(!h.c.is_open());
}

#[test]
fn enter_pastes_the_chosen_clips_too() {
    let mut h = four();
    let g = h.open();
    h.click_row(1, CTRL);
    h.click_row(2, CTRL);
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["c\nb"]);
}

#[test]
fn a_clips_own_line_breaks_do_not_leave_blank_lines() {
    let mut h = H::new(&[Clip::Text("one\r\n"), Clip::Text("two\n\n"), Clip::Text("three")]);
    let g = h.open();
    h.click_row(2, CTRL);
    h.click_row(1, CTRL);
    h.click_row(0, CTRL);
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["one\ntwo\nthree"]);
}

#[test]
fn a_files_clip_gives_one_path_per_line() {
    let mut h = H::new(&[Clip::Files(&["C:\\a\\x.txt", "C:\\a\\y.txt"]), Clip::Text("note")]);
    let g = h.open();
    h.click_row(1, CTRL);
    h.click_row(0, CTRL);
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["C:\\a\\x.txt\nC:\\a\\y.txt\nnote"]);
}

#[test]
fn pictures_are_left_out_of_the_text() {
    let mut h = H::new(&[Clip::Text("first"), Clip::Image, Clip::Text("last")]);
    let g = h.open();
    for i in [2, 1, 0] {
        h.click_row(i, CTRL);
    }
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["first\nlast"]);
    assert!(h.p.clipboard.iter().all(|p| matches!(p, Payload::Text(_))));
}

#[test]
fn only_pictures_chosen_pastes_nothing_and_says_why() {
    let mut h = H::new(&[Clip::Color(10), Clip::Color(200), Clip::Text("t")]);
    let g = h.open();
    h.click_row(1, CTRL);
    h.click_row(2, CTRL);
    h.enter(g);
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty());
    assert!(h.c.is_open(), "still there to choose again");
    assert!(h.footer().contains("Pictures cannot be joined"), "{}", h.footer());
}

#[test]
fn one_chosen_picture_is_pasted_as_a_picture() {
    let mut h = H::new(&[Clip::Text("t"), Clip::Color(10)]);
    let g = h.open();
    h.click_row(0, CTRL);
    h.click_to_paste(g);
    assert!(matches!(h.p.clipboard.as_slice(), [Payload::Image(_)]));
    assert_eq!(h.p.pastes.len(), 1);
}

#[test]
fn one_chosen_clip_that_is_not_the_one_shown_is_the_one_pasted() {
    let mut h = four();
    let g = h.open();
    h.click_row(3, CTRL); // a, while d is shown
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["a"]);
}

#[test]
fn ctrl_c_copies_the_chosen_clips_and_pastes_nothing() {
    let mut h = four();
    let g = h.open();
    h.click_row(1, CTRL);
    h.click_row(3, CTRL);
    h.send(Event::Key(Key::CtrlC, g));
    assert_eq!(h.pasted_texts(), vec!["c\na"]);
    assert!(h.p.pastes.is_empty(), "nothing is pasted anywhere");
    assert!(!h.c.is_open(), "the carousel closes");
    // the person is told, where the carousel was
    assert!(matches!(h.vm().body, pasteroll_render::Body::Message(ref m) if m == "Copied 2 clips"));
    assert_eq!(h.c.store().borrow().count().unwrap(), 4, "and the copy is not recorded as a new clip");
}

#[test]
fn ctrl_c_with_nothing_chosen_copies_the_clip_that_is_shown() {
    let mut h = four();
    let g = h.open();
    h.send(Event::Key(Key::CtrlC, g));
    assert_eq!(h.pasted_texts(), vec!["d"]);
    assert!(h.p.pastes.is_empty() && !h.c.is_open());
}

#[test]
fn ctrl_c_on_a_picture_copies_the_picture() {
    let mut h = H::new(&[Clip::Color(10)]);
    let g = h.open();
    h.send(Event::Key(Key::CtrlC, g));
    assert!(matches!(h.p.clipboard.as_slice(), [Payload::Image(_)]));
}

#[test]
fn escape_forgets_the_choice_first_and_closes_the_next_time() {
    let mut h = four();
    let g = h.open();
    h.click_row(1, CTRL);
    h.send(Event::Key(Key::Escape, g));
    assert!(h.c.is_open());
    assert!(h.chosen().is_empty());
    h.send(Event::Key(Key::Escape, g));
    assert!(!h.c.is_open());
}

#[test]
fn a_deleted_clip_leaves_the_choice() {
    let mut h = four();
    let g = h.open();
    h.click_row(0, CTRL); // d, the one shown
    h.click_row(2, CTRL);
    h.send(Event::Key(Key::Delete, g));
    let vm = h.vm();
    assert_eq!(vm.picked.iter().filter(|p| **p).count(), 1, "only b is still chosen");
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["b"]);
}

#[test]
fn many_lines_into_a_terminal_ask_first() {
    let mut h = four();
    h.p.foreground = Some(target(1, "windowsterminal.exe"));
    let g = h.open();
    h.click_row(0, CTRL);
    h.click_row(1, CTRL);
    h.enter(g);
    assert!(h.p.pastes.is_empty(), "a second Enter is needed");
    assert!(h.footer().contains("terminal"), "{}", h.footer());
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["d\nc"]);
}

#[test]
fn no_bloat_has_no_choosing() {
    let mut h = four();
    h.c.init_ui_mode(UiMode::NoBloat);
    h.p.mode = UiMode::NoBloat;
    let g = h.open();
    h.p.mods = CTRL;
    let at = (200.0, 120.0);
    h.mouse(MouseKind::Down, at);
    h.mouse(MouseKind::Up, at);
    h.p.mods = Modifiers::default();
    assert!(h.chosen().is_empty());
    h.mouse(MouseKind::RightDown, at);
    assert!(h.vm().menu.is_none());
    h.click_to_paste(g);
    assert_eq!(h.pasted_texts(), vec!["d"], "the plain paste is as it always was");
}

// ---------------------------------------------------------------- the right-click menu

#[test]
fn a_right_click_chooses_the_clip_and_opens_the_copy_menu() {
    let mut h = four();
    h.open();
    h.right_click_row(2);
    assert_eq!(h.chosen(), vec![2]);
    let menu = h.vm().menu.expect("a menu");
    assert_eq!(menu.items, vec!["Copy", "Clear selection"]);
}

#[test]
fn a_right_click_on_a_chosen_clip_keeps_the_whole_choice() {
    let mut h = four();
    h.open();
    h.click_row(1, CTRL);
    h.click_row(2, CTRL);
    h.right_click_row(2);
    assert_eq!(h.chosen(), vec![1, 2]);
    assert_eq!(h.vm().menu.expect("a menu").items[0], "Copy 2 clips");
}

#[test]
fn choosing_copy_in_the_menu_copies_and_closes() {
    let mut h = four();
    h.open();
    h.click_row(1, CTRL);
    h.click_row(2, CTRL);
    h.right_click_row(2);
    let vm = h.vm();
    let size = h.p.window();
    let g = pasteroll_render::geometry(&vm, (size.0 as f32, size.1 as f32), 1.0);
    let item = g.menu_items[0];
    h.mouse(MouseKind::Down, (item.x + 5.0, item.y + 5.0));
    assert_eq!(h.pasted_texts(), vec!["c\nb"]);
    assert!(h.p.pastes.is_empty() && !h.c.is_open());
}

#[test]
fn clear_selection_in_the_menu_forgets_the_choice() {
    let mut h = four();
    h.open();
    h.click_row(1, CTRL);
    h.right_click_row(1);
    let vm = h.vm();
    let size = h.p.window();
    let g = pasteroll_render::geometry(&vm, (size.0 as f32, size.1 as f32), 1.0);
    let item = g.menu_items[1];
    h.mouse(MouseKind::Down, (item.x + 5.0, item.y + 5.0));
    assert!(h.chosen().is_empty());
    assert!(h.vm().menu.is_none());
    assert!(h.c.is_open());
}

#[test]
fn a_click_elsewhere_only_closes_the_menu() {
    let mut h = four();
    h.open();
    h.right_click_row(1);
    assert!(h.vm().menu.is_some());
    let at = h.row_point(3);
    h.mouse(MouseKind::Down, at);
    assert!(h.vm().menu.is_none(), "closed");
    assert_eq!(newest_text(&mut h), "d", "and the click did nothing else");
    assert_eq!(h.chosen(), vec![1], "the choice stays");
}

#[test]
fn escape_closes_the_menu_without_forgetting_the_choice() {
    let mut h = four();
    let g = h.open();
    h.right_click_row(1);
    h.send(Event::Key(Key::Escape, g));
    assert!(h.vm().menu.is_none());
    assert_eq!(h.chosen(), vec![1]);
    assert!(h.c.is_open());
}

#[test]
fn the_menu_lights_up_the_entry_under_the_pointer() {
    let mut h = four();
    h.open();
    h.right_click_row(1);
    let vm = h.vm();
    let size = h.p.window();
    let g = pasteroll_render::geometry(&vm, (size.0 as f32, size.1 as f32), 1.0);
    let item = g.menu_items[1];
    h.mouse(MouseKind::Move, (item.x + 5.0, item.y + 5.0));
    assert_eq!(h.vm().menu.unwrap().hover, Some(1));
    h.mouse(MouseKind::Move, (2.0, 2.0));
    assert_eq!(h.vm().menu.unwrap().hover, None);
}

#[test]
fn the_choice_does_not_survive_the_session() {
    let mut h = four();
    let g = h.open();
    h.click_row(1, CTRL);
    h.send(Event::Key(Key::Escape, g));
    h.send(Event::Key(Key::Escape, g));
    assert!(!h.c.is_open());
    h.open();
    assert!(h.chosen().is_empty());
}
