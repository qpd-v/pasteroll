//! Controller tests against a scripted fake platform: no OS, no window, no clipboard.

use crate::*;
use pasteroll_core::image::{encode_png, Rgba};
use pasteroll_core::vault::{Vault, MASTER_LEN};
use pasteroll_core::{Step, Store};
use pasteroll_render::{geometry, Body, Hit, ViewModel};
use pasteroll_core::Limits;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------- fake platform

struct Fake {
    foreground: Option<Target>,
    current: u64,
    privileged: bool,
    cursor: (i32, i32),
    shown: u32,
    hidden: u32,
    frames: u32,
    input_active: bool,
    generation: u64,
    mods_released: bool,
    click_polls: VecDeque<ClickPoll>,
    clipboard: Vec<Payload>,
    write_ok: bool,
    /// After the clipboard is written the foreground moves to this window (simulates the user
    /// switching away mid-paste).
    switch_after_write: Option<u64>,
    pastes: Vec<PasteKey>,
    paste_ok: bool,
    desktop: Option<PathBuf>,
    created: Vec<PathBuf>,
    now_ms: u64,
    secs: i64,
    heartbeats: u32,
    /// Custom filters the user has saved through the overlay.
    saved_filters: Vec<pasteroll_core::config::CustomFilter>,
    /// The expanded viewer's window: where it is (while shown) and what it was given.
    viewer_rect: Option<(i32, i32, u32, u32)>,
    viewer_frames: u32,
    viewer_mode: bool,
    animating: bool,
    keepalives: u32,
    shift: bool,
    /// What the save dialog answers.
    save_choice: SaveChoice,
    /// The last carousel frame presented (to see it dimmed behind the viewer).
    last_overlay: Option<pasteroll_render::Frame>,
}

fn target(id: u64, exe: &str) -> Target {
    Target { id, pid: 1000 + id as u32, exe: exe.to_owned(), class: String::new(), desktop: false }
}

impl Fake {
    fn new() -> Fake {
        Fake {
            foreground: Some(target(1, "notepad.exe")),
            current: 1,
            privileged: false,
            cursor: (300, 300),
            shown: 0,
            hidden: 0,
            frames: 0,
            input_active: false,
            generation: 0,
            mods_released: true,
            click_polls: VecDeque::new(),
            clipboard: Vec::new(),
            write_ok: true,
            switch_after_write: None,
            pastes: Vec::new(),
            paste_ok: true,
            desktop: None,
            created: Vec::new(),
            now_ms: 10_000,
            secs: 1_700_000_000,
            heartbeats: 0,
            saved_filters: Vec::new(),
            viewer_rect: None,
            viewer_frames: 0,
            viewer_mode: false,
            animating: false,
            keepalives: 0,
            shift: false,
            save_choice: SaveChoice::Unavailable,
            last_overlay: None,
        }
    }
}

impl Platform for Fake {
    fn foreground(&mut self) -> Option<Target> {
        self.foreground.clone()
    }
    fn target_is_current(&mut self, t: &Target) -> bool {
        self.current == t.id
    }
    fn target_privileged(&mut self, _t: &Target) -> bool {
        self.privileged
    }
    fn cursor(&self) -> (i32, i32) {
        self.cursor
    }
    fn overlay_show(&mut self, _at: (i32, i32)) -> OverlayInfo {
        self.shown += 1;
        let mut info = OverlayInfo::new(1.0, (520, 440));
        info.origin_px = (316, 320);
        info
    }
    fn overlay_present(&mut self, frame: &pasteroll_render::Frame) {
        assert_eq!((frame.width, frame.height), (520, 440));
        self.frames += 1;
        self.last_overlay = Some(frame.clone());
    }
    fn viewer_show(&mut self, rect: (i32, i32, u32, u32), _rounded: bool) {
        self.viewer_rect = Some(rect);
    }
    fn viewer_present(&mut self, frame: &pasteroll_render::Frame) {
        let r = self.viewer_rect.expect("the viewer is shown before it is drawn");
        // full size, or a smaller animation frame of the same shape
        assert!(frame.width <= r.2 && frame.height <= r.3);
        let (a, b) = (frame.width as f32 / frame.height as f32, r.2 as f32 / r.3 as f32);
        assert!((a - b).abs() / b < 0.05, "{}x{} in {:?}", frame.width, frame.height, r);
        if !self.animating {
            assert_eq!((frame.width, frame.height), (r.2, r.3), "a still frame fills its window");
        }
        self.viewer_frames += 1;
    }
    fn viewer_hide(&mut self) {
        self.viewer_rect = None;
    }
    fn viewer_mode(&mut self, on: bool) {
        self.viewer_mode = on;
    }
    fn animating(&mut self, on: bool) {
        self.animating = on;
    }
    fn input_keepalive(&mut self) {
        self.keepalives += 1;
    }
    fn shift_held(&self) -> bool {
        self.shift
    }
    fn save_dialog(&mut self, _name: &str, _ext: &str, _target: &Target) -> SaveChoice {
        self.save_choice.clone()
    }
    fn overlay_hide(&mut self) {
        self.hidden += 1;
    }
    fn input_active(&mut self, active: bool) {
        self.generation += 1;
        self.input_active = active;
    }
    fn input_generation(&self) -> u64 {
        self.generation
    }
    fn heartbeat(&mut self) {
        self.heartbeats += 1;
    }
    fn modifiers_released(&mut self, _timeout: Duration) -> bool {
        self.mods_released
    }
    fn poll_click(&mut self, _first: bool) -> ClickPoll {
        self.click_polls.pop_front().unwrap_or(ClickPoll::Pending)
    }
    fn write_clipboard(&mut self, payload: &Payload) -> Result<(), String> {
        if !self.write_ok {
            return Err("clipboard busy".into());
        }
        self.clipboard.push(payload.clone());
        if let Some(id) = self.switch_after_write {
            self.current = id;
        }
        Ok(())
    }
    fn send_paste(&mut self, key: PasteKey) -> bool {
        if self.paste_ok {
            self.pastes.push(key);
        }
        self.paste_ok
    }
    fn desktop_folder(&self) -> Option<PathBuf> {
        self.desktop.clone()
    }
    fn local_timestamp(&self) -> String {
        "2026-10-13 12-00-00".to_owned()
    }
    fn file_created(&mut self, path: &std::path::Path) {
        self.created.push(path.to_path_buf());
    }
    fn now_secs(&self) -> i64 {
        self.secs
    }
    fn now_ms(&self) -> u64 {
        self.now_ms
    }
    fn log(&self, _msg: &str) {}
    fn add_custom_filter(
        &mut self,
        name: &str,
        extensions: &[String],
    ) -> Result<Vec<pasteroll_core::config::CustomFilter>, String> {
        let text = pasteroll_core::config::add_custom_filter("", name, extensions).map_err(|e| e.join("; "))?;
        if self.saved_filters.iter().any(|f| f.name.eq_ignore_ascii_case(name)) {
            return Err(format!("there is already a filter called {name}"));
        }
        let parsed = pasteroll_core::config::parse(&text).map_err(|e| e.join("; "))?;
        self.saved_filters.extend(parsed.custom_filters);
        Ok(self.saved_filters.clone())
    }
}

struct NoWake;
impl Waker for NoWake {
    fn wake(&self) {}
}

// ---------------------------------------------------------------- harness

enum Clip<'a> {
    Text(&'a str),
    Image,
    /// A picture of one colour: different values are different clips.
    Color(u8),
    Files(&'a [&'a str]),
}

struct H {
    c: Controller,
    p: Fake,
    dir: PathBuf,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn vault() -> Vault {
    Vault::new(&[3; MASTER_LEN])
}

fn png() -> Vec<u8> {
    let img = Rgba { width: 6, height: 4, pixels: [10u8, 200, 30, 255].repeat(24) };
    encode_png(&img).unwrap()
}

impl H {
    /// Clips are added oldest first, so the last one is the newest (the carousel opens on it).
    fn new(clips: &[Clip<'_>]) -> H {
        H::with(clips, None, IngestPolicy::default())
    }

    fn with(clips: &[Clip<'_>], source: Option<Box<dyn ClipboardSource>>, policy: IngestPolicy) -> H {
        let dir = std::env::temp_dir().join(format!(
            "pasteroll-app-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("h.db");
        let mut store = Store::open(&db, vault()).unwrap();
        for (i, c) in clips.iter().enumerate() {
            // the fake platform's clock; clips must look recent or start-up maintenance expires them
            let now = 1_700_000_000 + i as i64;
            match c {
                Clip::Text(t) => store.add_text(t, now).unwrap(),
                Clip::Image => store.add_image_png(&png(), now).unwrap(),
                Clip::Color(c) => {
                    let img = Rgba { width: 8, height: 8, pixels: [*c, 255 - *c, 90, 255].repeat(64) };
                    store.add_image_png(&encode_png(&img).unwrap(), now).unwrap()
                }
                Clip::Files(f) => {
                    store.add_files(&f.iter().map(|s| s.to_string()).collect::<Vec<_>>(), now).unwrap()
                }
            };
        }
        let db2 = db.clone();
        let factory: StoreFactory =
            Arc::new(move || Store::open(&db2, vault()).map_err(|e| e.to_string()));
        let log: Logger = Arc::new(|_| {});
        let c = Controller::new(store, factory, PasteKeys::builtin(), Arc::new(NoWake), source, policy, log);
        H { c, p: Fake::new(), dir }
    }

    fn send(&mut self, ev: Event) {
        self.c.handle(&mut self.p, ev);
    }

    fn open(&mut self) -> u64 {
        self.send(Event::Activate);
        assert!(self.c.is_open(), "session should be open");
        self.p.input_generation()
    }

    fn step_older(&mut self, gen: u64) {
        self.send(Event::Step(Step::Older, gen));
    }

    fn enter(&mut self, gen: u64) {
        self.send(Event::Key(Key::Enter, gen));
    }

    fn vm(&mut self) -> ViewModel {
        self.c.view_model()
    }

    fn mouse(&mut self, kind: MouseKind, at: (f32, f32)) {
        self.send(Event::Mouse(MouseEvent { kind, x: at.0, y: at.1 }));
    }

    /// The boundary in front of char `i`, as a pixel point: halfway between the centres of the
    /// previous char and this one (so it does not depend on the font's glyph widths), or half a
    /// char before the first one.
    fn boundary_px(&mut self, i: usize) -> (f32, f32) {
        let c = self.c.char_centre_px(i).expect("char centre");
        if i > 0 {
            let prev = self.c.char_centre_px(i - 1).expect("previous char centre");
            if (prev.1 - c.1).abs() < 1.0 {
                return ((prev.0 + c.0) / 2.0, c.1);
            }
        }
        let next = self.c.char_centre_px(i + 1).expect("next char centre");
        // inside the left half of the first glyph (the very edge may fall in the text padding)
        (c.0 - (next.0 - c.0).abs() / 4.0, c.1)
    }

    /// Drag from the boundary in front of char `a` to the boundary in front of char `b`.
    fn drag(&mut self, a: usize, b: usize) {
        let pa = self.boundary_px(a);
        let pb = self.boundary_px(b);
        self.mouse(MouseKind::Down, pa);
        self.mouse(MouseKind::Move, pb);
        self.mouse(MouseKind::Up, pb);
    }

    fn pasted_texts(&self) -> Vec<String> {
        self.p
            .clipboard
            .iter()
            .filter_map(|p| match p {
                Payload::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }
}

impl Drop for H {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn newest_text(h: &mut H) -> String {
    let id = h.c.current_id().unwrap();
    h.c.store().borrow().text(id).unwrap().unwrap()
}

// ---------------------------------------------------------------- opening and navigating

#[test]
fn activating_opens_on_the_newest_clip_and_shows_the_overlay() {
    let mut h = H::new(&[Clip::Text("oldest"), Clip::Text("middle"), Clip::Text("newest")]);
    assert!(!h.p.input_active);
    h.open();
    assert!(h.p.input_active, "wheel/keys must be swallowed while open");
    assert_eq!(newest_text(&mut h), "newest");
    assert_eq!(h.p.shown, 1);
    assert!(h.p.frames >= 1, "a frame must be presented");
}

#[test]
fn wheel_steps_walk_older_then_newer_and_wrap() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    h.step_older(g);
    assert_eq!(newest_text(&mut h), "b");
    h.step_older(g);
    assert_eq!(newest_text(&mut h), "a");
    h.step_older(g); // wraps to the newest
    assert_eq!(newest_text(&mut h), "c");
    h.send(Event::Step(Step::Newer, g)); // wraps to the oldest
    assert_eq!(newest_text(&mut h), "a");
}

#[test]
fn arrow_keys_step_like_the_wheel() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    h.send(Event::Key(Key::Down, g));
    assert_eq!(newest_text(&mut h), "a");
    h.send(Event::Key(Key::Up, g));
    assert_eq!(newest_text(&mut h), "b");
}

#[test]
fn the_view_shows_neighbours_and_a_large_active_row() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c"), Clip::Text("d")]);
    let g = h.open();
    h.step_older(g);
    let vm = h.vm();
    assert_eq!(vm.rows.len(), 4);
    assert_eq!(vm.active, 1, "one newer clip above the active one");
    assert!(matches!(vm.body, Body::Text { .. }));
    let geo = geometry(&vm, (pasteroll_render::WIDTH, pasteroll_render::HEIGHT), 1.0);
    let active = geo.rows[vm.active].unwrap();
    let other = geo.rows.iter().enumerate().find(|(i, r)| *i != vm.active && r.is_some()).unwrap().1.unwrap();
    assert!(active.h > other.h * 2.0);
}

#[test]
fn stale_generation_events_are_ignored() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let old = h.open();
    h.send(Event::Key(Key::Escape, old));
    assert!(!h.c.is_open());
    let new = h.open();
    assert_ne!(old, new);
    h.step_older(old); // queued by the previous session
    assert_eq!(newest_text(&mut h), "b", "stale step must not move the carousel");
    h.enter(old);
    assert!(h.p.clipboard.is_empty(), "stale Enter must not paste");
}

#[test]
fn empty_history_shows_a_notice_that_expires() {
    let mut h = H::new(&[]);
    h.send(Event::Activate);
    assert!(!h.c.is_open());
    assert_eq!(h.c.notice_text(), Some("Clipboard history is empty."));
    assert!(!h.p.input_active);
    h.p.now_ms += 2_000;
    h.send(Event::Tick);
    assert_eq!(h.c.notice_text(), None);
    assert!(h.p.hidden >= 1);
}

#[test]
fn no_suitable_target_does_nothing() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.p.foreground = None;
    h.send(Event::Activate);
    assert!(!h.c.is_open());
    assert_eq!(h.p.shown, 0);
}

#[test]
fn a_privileged_target_is_refused_before_anything_is_touched() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.p.privileged = true;
    h.send(Event::Activate);
    assert!(!h.c.is_open());
    assert!(h.c.notice_text().unwrap().contains("administrator"));
    assert!(!h.p.input_active, "no input may be swallowed for a refused target");
    assert!(h.p.clipboard.is_empty());
}

// ---------------------------------------------------------------- pasting

#[test]
fn enter_pastes_the_whole_clip_and_closes() {
    let mut h = H::new(&[Clip::Text("older clip"), Clip::Text("the newest clip")]);
    let g = h.open();
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["the newest clip"]);
    assert_eq!(h.p.pastes, vec![PasteKey::CtrlV]);
    assert!(!h.c.is_open());
    assert!(!h.p.input_active);
    assert!(h.p.hidden >= 1);
}

#[test]
fn pasting_an_older_clip_moves_it_to_the_top_of_history() {
    let mut h = H::new(&[Clip::Text("first"), Clip::Text("second"), Clip::Text("third")]);
    let g = h.open();
    h.step_older(g);
    h.step_older(g);
    assert_eq!(newest_text(&mut h), "first");
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["first"]);
    h.open();
    assert_eq!(newest_text(&mut h), "first", "the pasted clip is now the newest");
}

#[test]
fn escape_closes_without_touching_the_clipboard() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.send(Event::Key(Key::Escape, g));
    assert!(!h.c.is_open());
    assert!(h.p.clipboard.is_empty());
    assert!(h.p.pastes.is_empty());
    assert!(!h.p.input_active);
}

#[test]
fn the_per_app_paste_key_is_used() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.p.foreground = Some(target(1, "mintty.exe"));
    let g = h.open();
    h.enter(g);
    assert_eq!(h.p.pastes, vec![PasteKey::ShiftInsert]);
}

#[test]
fn a_selection_pastes_only_the_selected_part() {
    let mut h = H::new(&[Clip::Text("The quick brown fox jumps over the lazy dog")]);
    let g = h.open();
    h.drag(4, 15); // "quick brown "?  4..15 is "quick brown"
    assert_eq!(h.c.selected_text().as_deref(), Some("quick brown"));
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["quick brown"]);
}

#[test]
fn selection_does_not_reorder_history() {
    let mut h = H::new(&[Clip::Text("first clip"), Clip::Text("second clip")]);
    let g = h.open();
    h.step_older(g);
    h.drag(0, 5); // "first"
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["first"]);
    h.open();
    assert_eq!(newest_text(&mut h), "second clip", "a partial paste must not bump the source clip");
}

#[test]
fn double_click_selects_a_word_and_triple_click_a_line() {
    let mut h = H::new(&[Clip::Text("alpha beta\r\ngamma delta")]);
    let g = h.open();
    let p = h.c.char_centre_px(7).unwrap(); // inside "beta"
    h.mouse(MouseKind::DoubleClick, p);
    assert_eq!(h.c.selected_text().as_deref(), Some("beta"));
    h.p.now_ms += 100;
    h.mouse(MouseKind::Down, p); // third click in quick succession
    assert_eq!(h.c.selected_text().as_deref(), Some("alpha beta"));
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["alpha beta"]);
}

#[test]
fn a_new_click_clears_the_selection() {
    let mut h = H::new(&[Clip::Text("one two three")]);
    h.open();
    h.drag(0, 3);
    assert!(h.c.selected_text().is_some());
    let p = h.c.char_centre_px(8).unwrap();
    h.mouse(MouseKind::Down, p);
    h.mouse(MouseKind::Up, p);
    assert_eq!(h.c.selected_text(), None);
}

#[test]
fn hidden_characters_are_shown_as_tokens_but_the_real_text_is_pasted() {
    let text = "pay\u{202E}fdp.exe\u{200B}now";
    let mut h = H::new(&[Clip::Text(text)]);
    let g = h.open();
    let shown = h.c.displayed_text().unwrap();
    assert!(shown.contains("<U+202E>") && shown.contains("<U+200B>"), "{shown}");
    // whole clip: the real text
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec![text]);
}

#[test]
fn selecting_across_a_hidden_character_pastes_that_character() {
    let text = "ab\u{200B}cd";
    let mut h = H::new(&[Clip::Text(text)]);
    let g = h.open();
    // select the whole display by dragging from its start to far past its end
    let first = h.c.char_centre_px(0).unwrap();
    let last = (first.0 + 400.0, first.1);
    h.mouse(MouseKind::Down, (first.0 - 2.0, first.1));
    h.mouse(MouseKind::Move, last);
    h.mouse(MouseKind::Up, last);
    assert_eq!(h.c.selected_text().as_deref(), Some(text));
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec![text]);
}

#[test]
fn a_long_clip_is_truncated_in_the_preview_but_pasted_in_full() {
    let long = "x".repeat(10_000);
    let mut h = H::new(&[Clip::Text(&long)]);
    let g = h.open();
    let shown = h.c.displayed_text().unwrap();
    assert!(shown.contains("more characters not shown"), "preview must say it is cut");
    assert!(shown.contains("5,904"), "and say how much: {}", &shown[shown.len().saturating_sub(60)..]);
    h.enter(g);
    assert_eq!(h.pasted_texts()[0].len(), 10_000);
}

#[test]
fn the_truncation_footer_is_never_pasted() {
    let long = "y".repeat(5_000);
    let mut h = H::new(&[Clip::Text(&long)]);
    h.open();
    // drag from the start to far below the end of the shown text
    let a = h.c.char_centre_px(0).unwrap();
    h.mouse(MouseKind::Down, (a.0 - 2.0, a.1));
    h.mouse(MouseKind::Move, (a.0 + 450.0, a.1 + 400.0));
    h.mouse(MouseKind::Up, (a.0 + 450.0, a.1 + 400.0));
    let sel = h.c.selected_text().expect("something selected");
    assert!(!sel.contains("more characters"), "footer leaked into the selection");
    assert!(sel.chars().all(|c| c == 'y'));
}

#[test]
fn files_are_pasted_as_files() {
    let mut h = H::new(&[Clip::Files(&["C:\\a.txt", "C:\\b c.txt"])]);
    let g = h.open();
    h.enter(g);
    assert_eq!(h.p.clipboard, vec![Payload::Files(vec!["C:\\a.txt".into(), "C:\\b c.txt".into()])]);
}

#[test]
fn images_are_pasted_as_the_stored_png() {
    let mut h = H::new(&[Clip::Image]);
    let g = h.open();
    h.enter(g);
    assert_eq!(h.p.clipboard, vec![Payload::Image(png())]);
}

#[test]
fn enter_on_the_desktop_never_pastes() {
    let mut h = H::new(&[Clip::Image]);
    h.p.foreground = Some(Target { desktop: true, ..target(7, "explorer.exe") });
    h.p.current = 7;
    let g = h.open();
    h.enter(g);
    assert!(h.p.clipboard.is_empty());
    assert!(h.p.pastes.is_empty());
    assert!(!h.c.is_open());
}

// ---------------------------------------------------------------- safety

#[test]
fn leaving_the_target_cancels_the_session_on_the_next_tick() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.open();
    h.p.current = 99; // the user alt-tabbed
    h.send(Event::Tick);
    assert!(!h.c.is_open());
    assert!(!h.p.input_active, "input must be released as soon as the target is left");
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn nothing_is_pasted_if_the_foreground_changed_before_enter() {
    let mut h = H::new(&[Clip::Text("secret")]);
    let g = h.open();
    h.p.current = 99;
    h.enter(g);
    assert!(h.p.clipboard.is_empty(), "the clip must not even reach the clipboard");
    assert!(h.p.pastes.is_empty());
}

#[test]
fn nothing_is_typed_if_the_foreground_changes_while_the_clipboard_is_written() {
    let mut h = H::new(&[Clip::Text("secret")]);
    let g = h.open();
    h.p.switch_after_write = Some(99);
    h.enter(g);
    assert_eq!(h.p.clipboard.len(), 1, "the clipboard write happened first");
    assert!(h.p.pastes.is_empty(), "but the keystroke must not be sent to the new window");
}

#[test]
fn held_alt_or_win_blocks_the_paste() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.p.mods_released = false;
    h.enter(g);
    assert!(h.p.clipboard.is_empty());
    assert!(h.p.pastes.is_empty());
}

#[test]
fn a_blocked_keystroke_does_not_reorder_history() {
    let mut h = H::new(&[Clip::Text("first"), Clip::Text("second")]);
    let g = h.open();
    h.step_older(g);
    h.p.paste_ok = false;
    h.enter(g);
    h.p.paste_ok = true;
    h.open();
    assert_eq!(newest_text(&mut h), "second");
}

#[test]
fn a_clipboard_write_failure_sends_no_keystroke() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.p.write_ok = false;
    h.enter(g);
    assert!(h.p.pastes.is_empty());
}

#[test]
fn an_idle_session_closes_itself() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.open();
    h.p.now_ms += 61_000;
    h.send(Event::Tick);
    assert!(!h.c.is_open());
    assert!(!h.p.input_active);
}

#[test]
fn activity_keeps_the_session_alive() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    // 50 s in total, never idle for more than 25 s (the 55 s session cap is tested separately)
    for _ in 0..2 {
        h.p.now_ms += 25_000;
        h.step_older(g);
        h.send(Event::Tick);
    }
    assert!(h.c.is_open());
}

#[test]
fn ticks_send_a_heartbeat() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.send(Event::Tick);
    h.send(Event::Tick);
    assert_eq!(h.p.heartbeats, 2);
}

// ---------------------------------------------------------------- paste at click

#[test]
fn a_click_elsewhere_pastes_there_once_the_platform_says_it_settled() {
    let mut h = H::new(&[Clip::Text("clip body")]);
    let g = h.open();
    let other = target(2, "winword.exe");
    h.p.click_polls = VecDeque::from([ClickPoll::Pending, ClickPoll::Target(other.clone())]);
    h.p.current = other.id; // the click activated the other window
    h.send(Event::ClickPaste(g));
    assert!(h.c.is_open(), "still waiting for the click to settle");
    h.p.now_ms += 30;
    h.send(Event::Tick);
    assert!(h.p.clipboard.is_empty(), "first poll is still pending");
    h.p.now_ms += 30;
    h.send(Event::Tick);
    assert_eq!(h.pasted_texts(), vec!["clip body"]);
    assert!(!h.c.is_open());
}

#[test]
fn a_rejected_click_closes_without_pasting() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.p.click_polls = VecDeque::from([ClickPoll::Rejected("click was on a control")]);
    h.send(Event::ClickPaste(g));
    h.send(Event::Tick);
    assert!(!h.c.is_open());
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn a_click_that_never_settles_times_out() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.send(Event::ClickPaste(g));
    h.p.now_ms += 1_500;
    h.send(Event::Tick);
    assert!(!h.c.is_open());
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn stepping_is_ignored_while_a_click_paste_is_pending() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    h.send(Event::ClickPaste(g));
    h.step_older(g);
    assert_eq!(newest_text(&mut h), "b", "the clip to be pasted must not change after the click");
}

#[test]
fn clicking_the_desktop_saves_an_image_and_leaves_the_clipboard_alone() {
    let mut h = H::new(&[Clip::Image]);
    let folder = h.dir.join("Desktop");
    std::fs::create_dir_all(&folder).unwrap();
    h.p.desktop = Some(folder.clone());
    let g = h.open();
    h.p.click_polls = VecDeque::from([ClickPoll::Desktop]);
    h.send(Event::ClickPaste(g));
    h.send(Event::Tick);
    assert!(!h.c.is_open());
    assert!(h.p.clipboard.is_empty(), "image-to-desktop is a file save, not a paste");
    assert_eq!(h.p.created.len(), 1);
    let saved = std::fs::read(&h.p.created[0]).unwrap();
    assert_eq!(&saved[1..4], b"PNG");
    assert!(h.p.created[0].file_name().unwrap().to_string_lossy().starts_with("Pasteroll image 2026-10-13"));
}

#[test]
fn clicking_the_desktop_with_a_text_clip_does_nothing() {
    let mut h = H::new(&[Clip::Text("a")]);
    let folder = h.dir.join("Desktop");
    std::fs::create_dir_all(&folder).unwrap();
    h.p.desktop = Some(folder.clone());
    let g = h.open();
    h.p.click_polls = VecDeque::from([ClickPoll::Desktop]);
    h.send(Event::ClickPaste(g));
    h.send(Event::Tick);
    assert!(h.p.created.is_empty());
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
}

// ---------------------------------------------------------------- overlay interaction

#[test]
fn clicking_a_neighbouring_row_makes_it_active() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    h.step_older(g); // on b, c is above
    assert_eq!(newest_text(&mut h), "b");
    let vm = h.vm();
    let geo = geometry(&vm, (pasteroll_render::WIDTH, pasteroll_render::HEIGHT), 1.0);
    let above = geo.rows[0].unwrap(); // the newer neighbour (c)
    let at = (above.x + above.w / 2.0, above.y + above.h / 2.0);
    h.mouse(MouseKind::Down, at);
    assert_eq!(newest_text(&mut h), "c");
}

#[test]
fn the_filter_button_toggles_the_dropdown() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.open();
    let vm = h.vm();
    let geo = geometry(&vm, (pasteroll_render::WIDTH, pasteroll_render::HEIGHT), 1.0);
    let b = geo.filter_button;
    let at = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    assert!(h.vm().dropdown.is_none());
    h.mouse(MouseKind::Down, at);
    assert!(h.vm().dropdown.is_some());
    h.mouse(MouseKind::Down, at);
    assert!(h.vm().dropdown.is_none());
    let _ = Hit::Nothing;
}

#[test]
fn long_text_scrolls_with_page_keys_and_clamps() {
    let long = (0..120).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let mut h = H::new(&[Clip::Text(&long)]);
    let g = h.open();
    let top = match h.vm().body {
        Body::Text { scroll, .. } => scroll,
        _ => panic!("text body"),
    };
    assert_eq!(top, 0.0);
    h.send(Event::Key(Key::PageDown, g));
    let moved = match h.vm().body {
        Body::Text { scroll, .. } => scroll,
        _ => unreachable!(),
    };
    assert!(moved > 50.0);
    h.send(Event::Key(Key::PageUp, g));
    h.send(Event::Key(Key::PageUp, g));
    let back = match h.vm().body {
        Body::Text { scroll, .. } => scroll,
        _ => unreachable!(),
    };
    assert_eq!(back, 0.0, "scroll clamps at the top");
}

// ---------------------------------------------------------------- previews

#[test]
fn an_image_preview_arrives_from_the_worker() {
    let mut h = H::new(&[Clip::Text("text"), Clip::Image]);
    h.open();
    assert!(matches!(h.vm().body, Body::Image(None)), "empty frame while decoding");
    let mut got = false;
    for _ in 0..200 {
        h.send(Event::Wake);
        if matches!(h.vm().body, Body::Image(Some(_))) {
            got = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(got, "thumbnail never arrived");
}

#[test]
fn a_stale_preview_is_dropped_when_the_user_has_scrolled_on() {
    let mut h = H::new(&[Clip::Text("older text"), Clip::Image]);
    let g = h.open();
    h.step_older(g); // now on the text clip; the image preview (if it arrives) is stale
    for _ in 0..50 {
        h.send(Event::Wake);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(h.vm().body, Body::Text { .. }));
}

// ---------------------------------------------------------------- pins, delete, rows

#[test]
fn ctrl_p_pins_and_unpins_the_candidate() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    assert!(!h.vm().rows[h.vm().active].pinned);
    h.send(Event::Key(Key::CtrlP, g));
    let vm = h.vm();
    assert!(vm.rows[vm.active].pinned, "the star must show");
    assert!(vm.footer.contains("Pinned"), "{}", vm.footer);
    let id = h.c.current_id().unwrap();
    assert!(h.c.store().borrow().info(id).unwrap().unwrap().pinned);
    h.send(Event::Key(Key::CtrlP, g));
    assert!(!h.vm().rows[h.vm().active].pinned);
    assert!(!h.c.store().borrow().info(id).unwrap().unwrap().pinned);
}

#[test]
fn the_pin_limit_is_reported_in_the_footer() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    h.c.store().borrow_mut().set_limits(Limits { max_pins: 1, ..Limits::default() }).unwrap();
    let g = h.open();
    h.send(Event::Key(Key::CtrlP, g));
    h.step_older(g);
    h.send(Event::Key(Key::CtrlP, g));
    assert!(h.vm().footer.contains("Pin limit reached"), "{}", h.vm().footer);
    let id = h.c.current_id().unwrap();
    assert!(!h.c.store().borrow().info(id).unwrap().unwrap().pinned);
}

#[test]
fn the_footer_message_goes_away_by_itself() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.send(Event::Key(Key::CtrlP, g));
    assert!(!h.vm().footer.is_empty());
    h.p.now_ms += 2_000;
    h.send(Event::Tick);
    assert!(h.vm().footer.is_empty());
}

#[test]
fn delete_removes_the_clip_and_moves_on() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    h.send(Event::Key(Key::Delete, g));
    assert_eq!(h.c.store().borrow().count().unwrap(), 2);
    assert_eq!(newest_text(&mut h), "b");
    assert!(h.c.is_open());
}

#[test]
fn deleting_the_last_clip_closes_the_session() {
    let mut h = H::new(&[Clip::Text("only")]);
    let g = h.open();
    h.send(Event::Key(Key::Delete, g));
    assert!(!h.c.is_open());
    assert_eq!(h.c.store().borrow().count().unwrap(), 0);
    assert!(!h.p.input_active);
}

#[test]
fn rows_describe_each_kind_from_the_sealed_record() {
    let mut h = H::new(&[Clip::Text("some text"), Clip::Files(&["C:\\docs\\invoice.pdf"]), Clip::Image]);
    h.open();
    let vm = h.vm();
    assert_eq!(vm.rows[0].title, "Image 6 x 4");
    assert_eq!(vm.rows[1].title, "invoice.pdf");
    assert_eq!(vm.rows[2].title, "some text");
    assert_eq!(vm.rows[2].meta, "Text, 9 characters");
}

// ---------------------------------------------------------------- capture policy

struct SourceState {
    seq: u64,
    next: Option<Captured>,
    app: Option<String>,
}

struct FakeSource(Arc<Mutex<SourceState>>);

impl ClipboardSource for FakeSource {
    fn sequence(&mut self) -> u64 {
        self.0.lock().unwrap().seq
    }
    fn capture(&mut self) -> Result<Captured, Skip> {
        self.0.lock().unwrap().next.take().ok_or(Skip::Unsupported)
    }
    fn source_app(&mut self) -> Option<String> {
        self.0.lock().unwrap().app.clone()
    }
}

fn copy(h: &H, state: &Arc<Mutex<SourceState>>, text: &str, app: Option<&str>) {
    {
        let mut s = state.lock().unwrap();
        s.seq += 1;
        s.next = Some(Captured::Text(text.to_owned()));
        s.app = app.map(str::to_owned);
    }
    h.c.clipboard_changed();
}

fn wait_for_count(h: &H, n: i64) -> bool {
    for _ in 0..200 {
        if h.c.store().borrow().count().unwrap() == n {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn ingest_harness(exclude: &[&str]) -> (H, Arc<Mutex<SourceState>>) {
    let state = Arc::new(Mutex::new(SourceState { seq: 0, next: None, app: None }));
    let policy = IngestPolicy {
        exclude_apps: exclude.iter().map(|s| s.to_string()).collect::<HashSet<_>>(),
        paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let h = H::with(&[], Some(Box::new(FakeSource(state.clone()))), policy);
    (h, state)
}

#[test]
fn a_normal_copy_is_recorded() {
    let (h, state) = ingest_harness(&[]);
    copy(&h, &state, "hello from notepad", Some("notepad.exe"));
    assert!(wait_for_count(&h, 1), "copy was not recorded");
}

#[test]
fn copies_from_excluded_apps_are_never_read() {
    let (h, state) = ingest_harness(&["keepass.exe"]);
    copy(&h, &state, "hunter2", Some("keepass.exe"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(h.c.store().borrow().count().unwrap(), 0, "excluded copy must not be stored");
    assert!(state.lock().unwrap().next.is_some(), "excluded copy must not even be read from the clipboard");
    copy(&h, &state, "fine", Some("notepad.exe"));
    assert!(wait_for_count(&h, 1));
}

#[test]
fn pausing_stops_recording_and_resuming_restarts_it() {
    let (h, state) = ingest_harness(&[]);
    h.c.set_paused(true);
    assert!(h.c.is_paused());
    copy(&h, &state, "while paused", None);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(h.c.store().borrow().count().unwrap(), 0);
    h.c.set_paused(false);
    copy(&h, &state, "after resume", None);
    assert!(wait_for_count(&h, 1));
}

#[test]
fn an_unknown_source_app_is_still_recorded() {
    let (h, state) = ingest_harness(&["keepass.exe"]);
    copy(&h, &state, "anonymous copy", None);
    assert!(wait_for_count(&h, 1));
}

#[test]
fn old_clips_expire_on_the_first_tick_without_any_new_copy() {
    let mut h = H::new(&[Clip::Text("ancient"), Clip::Text("recent")]);
    {
        let store = h.c.store();
        let mut st = store.borrow_mut();
        // make the first clip 200 days old
        let id = st.search_rows().unwrap().last().unwrap().id;
        st.mark_pasted(id, 1_700_000_000 - 200 * 86_400, false).unwrap();
    }
    assert_eq!(h.c.store().borrow().count().unwrap(), 2);
    h.send(Event::Tick);
    assert_eq!(h.c.store().borrow().count().unwrap(), 1, "the 200-day-old clip must be gone");
}

// ---------------------------------------------------------------- terminal paste warning

fn in_terminal(h: &mut H) {
    h.p.foreground = Some(target(1, "WindowsTerminal.exe"));
}

#[test]
fn multi_line_text_into_a_terminal_needs_a_second_enter() {
    let mut h = H::new(&[Clip::Text("echo one\necho two")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty(), "first Enter must not paste");
    assert!(h.c.is_open(), "the overlay stays up");
    assert!(h.vm().footer.contains("terminal"), "{}", h.vm().footer);
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["echo one\necho two"]);
    assert!(!h.c.is_open());
}

#[test]
fn escape_after_the_warning_pastes_nothing() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    h.send(Event::Key(Key::Escape, g));
    assert!(!h.c.is_open());
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn a_single_line_into_a_terminal_pastes_immediately() {
    let mut h = H::new(&[Clip::Text("git status")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["git status"]);
}

#[test]
fn a_trailing_newline_still_warns() {
    let mut h = H::new(&[Clip::Text("rm -rf build\n")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert!(h.p.clipboard.is_empty(), "a trailing newline would run the command");
    assert!(h.c.is_open());
}

#[test]
fn multi_line_text_into_a_normal_app_pastes_immediately() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    let g = h.open();
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["a\nb"]);
}

#[test]
fn the_warning_can_be_switched_off() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    h.c.set_paste_policy(PastePolicy::new(false, &[]));
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["a\nb"]);
}

#[test]
fn moving_to_another_clip_withdraws_the_announcement() {
    let mut h = H::new(&[Clip::Text("x\ny"), Clip::Text("p\nq")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g); // warned for "p\nq"
    h.step_older(g); // now on "x\ny"
    assert!(h.vm().footer.is_empty());
    h.enter(g);
    assert!(h.p.clipboard.is_empty(), "the new candidate must be announced afresh");
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["x\ny"]);
}

#[test]
fn changing_the_selection_withdraws_the_announcement() {
    let mut h = H::new(&[Clip::Text("first line\nsecond line")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert!(h.c.is_open());
    h.drag(0, 5); // select "first": single line now
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["first"], "the single-line selection needs no warning");
}

#[test]
fn a_user_listed_terminal_is_covered() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    h.c.set_paste_policy(PastePolicy::new(true, &["MyShell.exe".to_owned()]));
    h.p.foreground = Some(target(1, "myshell.exe"));
    let g = h.open();
    h.enter(g);
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn a_click_paste_into_a_terminal_also_waits_for_enter() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    let g = h.open();
    let term = target(2, "cmd.exe");
    h.p.current = term.id;
    h.p.click_polls = VecDeque::from([ClickPoll::Target(term)]);
    h.send(Event::ClickPaste(g));
    h.send(Event::Tick);
    assert!(h.p.clipboard.is_empty(), "the click alone must not paste a multi-line clip into a terminal");
    assert!(h.c.is_open());
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["a\nb"]);
}

// ---------------------------------------------------------------- wiping

#[test]
fn a_payload_can_be_wiped_in_place() {
    let mut p = Payload::Text("hunter2 is my password".into());
    p.wipe();
    assert_eq!(p, Payload::Text(String::new()));
    let mut f = Payload::Files(vec!["C:\\secret\\a.txt".into(), "C:\\secret\\b.txt".into()]);
    f.wipe();
    assert_eq!(f, Payload::Files(vec![String::new(), String::new()]));
    let mut i = Payload::Image(vec![1, 2, 3]);
    i.wipe();
    assert_eq!(i, Payload::Image(Vec::new()), "overwritten, then cleared");
}

#[test]
fn closing_the_overlay_forgets_the_cached_row_text() {
    let mut h = H::new(&[Clip::Text("some secret text"), Clip::Text("another secret")]);
    let g = h.open();
    let _ = h.vm();
    h.send(Event::Key(Key::Escape, g));
    assert!(!h.c.is_open());
    // reopening rebuilds rows from the store rather than from a leftover cache
    h.open();
    assert_eq!(h.vm().rows.len(), 2);
}

#[test]
fn a_terminal_window_class_is_recognised_whatever_program_runs_in_it() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    h.p.foreground = Some(Target { class: "CASCADIA_HOSTING_WINDOW_CLASS".into(), ..target(1, "python.exe") });
    let g = h.open();
    h.enter(g);
    assert!(h.p.clipboard.is_empty(), "python in a console window is still a terminal");
}

#[test]
fn the_terminal_warning_does_not_leak_into_the_next_session() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert!(h.vm().footer.contains("terminal"));
    h.send(Event::Key(Key::Escape, g)); // user backs out
    h.p.foreground = Some(target(1, "notepad.exe"));
    h.open();
    assert!(h.vm().footer.is_empty(), "the old warning must not greet the next session");
}

#[test]
fn a_click_paste_into_a_different_terminal_is_announced_afresh() {
    let mut h = H::new(&[Clip::Text("a\nb")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g); // warned for the first terminal
    let second = target(2, "cmd.exe");
    h.p.current = second.id;
    h.p.click_polls = VecDeque::from([ClickPoll::Target(second)]);
    h.send(Event::ClickPaste(g));
    h.send(Event::Tick);
    assert!(h.p.clipboard.is_empty(), "the warning given for window 1 must not apply to window 2");
    assert!(h.c.is_open());
}

// ---------------------------------------------------------------- search and filters

fn type_text(h: &mut H, g: u64, s: &str) {
    for c in s.chars() {
        h.send(Event::Key(Key::Char(c), g));
    }
}

fn key(h: &mut H, g: u64, k: Key) {
    h.send(Event::Key(k, g));
}

#[test]
fn typing_narrows_the_carousel_to_the_matching_clips() {
    let mut h = H::new(&[Clip::Text("alpha one"), Clip::Text("beta two"), Clip::Text("alpha three")]);
    let g = h.open();
    type_text(&mut h, g, "alpha");
    assert_eq!(h.c.query(), Some("alpha"));
    assert_eq!(h.c.displayed_text().as_deref(), Some("alpha three"), "the newest match is shown");
    h.step_older(g);
    assert_eq!(h.c.displayed_text().as_deref(), Some("alpha one"));
    h.step_older(g);
    assert_eq!(h.c.displayed_text().as_deref(), Some("alpha three"), "wraps within the matches only");
    assert_eq!(h.c.match_counts(), Some((2, 3)));
    assert!(h.vm().footer.contains("2 of 3"), "{}", h.vm().footer);
    assert_eq!(h.vm().query, "alpha");
}

#[test]
fn matches_are_highlighted_in_the_rows() {
    let mut h = H::new(&[Clip::Text("alpha one"), Clip::Text("an ALPHA two")]);
    let g = h.open();
    type_text(&mut h, g, "alpha");
    let vm = h.vm();
    assert_eq!(vm.rows.len(), 2);
    assert!(
        vm.rows.iter().all(|r| r.highlights.len() == 1),
        "{:?}",
        vm.rows.iter().map(|r| &r.highlights).collect::<Vec<_>>()
    );
}

#[test]
fn a_search_with_no_match_says_so_and_enter_pastes_nothing() {
    let mut h = H::new(&[Clip::Text("alpha")]);
    let g = h.open();
    type_text(&mut h, g, "zzz");
    assert_eq!(h.c.current_id(), None);
    assert!(matches!(h.vm().body, Body::Message(ref m) if m.contains("No matching")));
    assert!(h.vm().rows.is_empty());
    h.enter(g);
    assert!(h.c.is_open(), "nothing to paste: the overlay stays");
    assert!(h.p.clipboard.is_empty());
    // wheel steps with nothing to show are harmless
    h.step_older(g);
    assert!(h.c.is_open());
}

#[test]
fn backspace_widens_the_search_again() {
    let mut h = H::new(&[Clip::Text("alpha one"), Clip::Text("alphabet"), Clip::Text("beta")]);
    let g = h.open();
    type_text(&mut h, g, "alphab");
    assert_eq!(h.c.match_counts(), Some((1, 3)));
    key(&mut h, g, Key::Backspace);
    key(&mut h, g, Key::Backspace);
    assert_eq!(h.c.query(), Some("alph"));
    assert_eq!(h.c.match_counts(), Some((2, 3)));
}

#[test]
fn ctrl_backspace_removes_a_word() {
    let mut h = H::new(&[Clip::Text("a b c")]);
    let g = h.open();
    type_text(&mut h, g, "one two  ");
    key(&mut h, g, Key::CtrlBackspace);
    assert_eq!(h.c.query(), Some("one "));
    key(&mut h, g, Key::CtrlBackspace);
    assert_eq!(h.c.query(), Some(""));
}

#[test]
fn escape_clears_the_search_before_it_closes_the_overlay() {
    let mut h = H::new(&[Clip::Text("alpha"), Clip::Text("beta")]);
    let g = h.open();
    type_text(&mut h, g, "alp");
    key(&mut h, g, Key::Escape);
    assert!(h.c.is_open());
    assert_eq!(h.c.query(), Some(""));
    assert_eq!(h.c.displayed_text().as_deref(), Some("beta"), "back to everything, newest first");
    key(&mut h, g, Key::Escape);
    assert!(!h.c.is_open());
}

#[test]
fn enter_pastes_the_clip_the_search_found() {
    let mut h = H::new(&[Clip::Text("alpha one"), Clip::Text("beta two"), Clip::Text("gamma three")]);
    let g = h.open();
    type_text(&mut h, g, "beta");
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["beta two".to_string()]);
    assert!(!h.c.is_open());
}

#[test]
fn a_selection_inside_a_search_result_pastes_only_that_part() {
    let mut h = H::new(&[Clip::Text("alpha one"), Clip::Text("hello brave new world")]);
    let g = h.open();
    type_text(&mut h, g, "brave");
    h.drag(6, 11); // "brave"
    h.enter(g);
    assert_eq!(h.pasted_texts(), vec!["brave".to_string()]);
}

#[test]
fn searching_finds_file_names_and_extensions() {
    let mut h = H::new(&[
        Clip::Files(&["C:\\Users\\me\\Documents\\Invoice-2026.pdf"]),
        Clip::Text("unrelated"),
        Clip::Files(&["C:\\Users\\me\\Music\\song.mp3"]),
    ]);
    let g = h.open();
    type_text(&mut h, g, "invoice");
    assert_eq!(h.c.match_counts(), Some((1, 3)));
    assert!(h.vm().rows[0].title.contains("Invoice-2026.pdf"));
    for _ in 0..7 {
        key(&mut h, g, Key::Backspace);
    }
    type_text(&mut h, g, "mp3");
    assert_eq!(h.c.match_counts(), Some((1, 3)));
}

#[test]
fn the_filter_dropdown_lists_the_categories_and_filters_by_click() {
    let mut h = H::new(&[Clip::Text("note"), Clip::Image, Clip::Files(&["C:\\x\\report.pdf"])]);
    h.open();
    let vm = h.vm();
    let size = (pasteroll_render::WIDTH, pasteroll_render::HEIGHT);
    let b = geometry(&vm, size, 1.0).filter_button;
    h.mouse(MouseKind::Down, (b.x + b.w / 2.0, b.y + b.h / 2.0));
    let vm = h.vm();
    let items = vm.dropdown.as_ref().expect("open").items.clone();
    assert_eq!(items.first().map(String::as_str), Some("Everything"));
    assert!(items.iter().any(|i| i == "Document") && items.iter().any(|i| i == "Pinned"));
    assert_eq!(items.last().map(String::as_str), Some("Add custom filter..."));
    let doc = items.iter().position(|i| i == "Document").unwrap();
    let r = geometry(&vm, size, 1.0).dropdown_items[doc];
    h.mouse(MouseKind::Down, (r.x + r.w / 2.0, r.y + r.h / 2.0));
    assert!(h.vm().dropdown.is_none(), "closes after picking");
    assert_eq!(h.c.filter_label().as_deref(), Some("Document"));
    assert_eq!(h.vm().filter_label, "Document");
    assert_eq!(h.c.match_counts(), Some((1, 3)));
    assert!(h.vm().rows[0].title.contains("report.pdf"));
}

#[test]
fn the_filter_can_be_chosen_from_the_keyboard() {
    let mut h = H::new(&[Clip::Text("note"), Clip::Image]);
    let g = h.open();
    key(&mut h, g, Key::Tab);
    assert!(h.vm().dropdown.is_some());
    key(&mut h, g, Key::Down); // Text
    key(&mut h, g, Key::Down); // Document
    key(&mut h, g, Key::Down); // Image
    key(&mut h, g, Key::Enter);
    assert!(h.vm().dropdown.is_none());
    assert_eq!(h.c.filter_label().as_deref(), Some("Image"));
    assert_eq!(h.c.match_counts(), Some((1, 2)));
    assert!(h.c.is_open(), "Enter picked the filter, it did not paste");
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn an_open_dropdown_swallows_typing_and_escape_only_closes_it() {
    let mut h = H::new(&[Clip::Text("alpha")]);
    let g = h.open();
    key(&mut h, g, Key::Tab);
    type_text(&mut h, g, "x");
    assert_eq!(h.c.query(), Some(""), "typing does nothing while the dropdown is open");
    key(&mut h, g, Key::Escape);
    assert!(h.c.is_open());
    assert!(h.vm().dropdown.is_none());
}

#[test]
fn filter_and_query_combine() {
    let mut h = H::new(&[
        Clip::Text("report text"),
        Clip::Files(&["C:\\x\\report.pdf"]),
        Clip::Files(&["C:\\x\\other.pdf"]),
    ]);
    let g = h.open();
    key(&mut h, g, Key::Tab);
    key(&mut h, g, Key::Down);
    key(&mut h, g, Key::Down); // Document
    key(&mut h, g, Key::Enter);
    assert_eq!(h.c.match_counts(), Some((2, 3)));
    type_text(&mut h, g, "report");
    assert_eq!(h.c.match_counts(), Some((1, 3)));
    assert!(h.vm().rows[0].title.contains("report.pdf"));
}

#[test]
fn pinning_in_a_search_updates_the_pinned_filter() {
    let mut h = H::new(&[Clip::Text("keep this"), Clip::Text("other")]);
    let g = h.open();
    type_text(&mut h, g, "keep");
    key(&mut h, g, Key::CtrlP);
    key(&mut h, g, Key::Escape); // clears the query
    key(&mut h, g, Key::Tab);
    for _ in 0..7 {
        key(&mut h, g, Key::Down);
    }
    key(&mut h, g, Key::Enter);
    assert_eq!(h.c.filter_label().as_deref(), Some("Pinned"));
    assert_eq!(h.c.match_counts(), Some((1, 2)));
}

#[test]
fn deleting_in_a_search_removes_only_that_clip_and_stays_in_the_results() {
    let mut h = H::new(&[Clip::Text("alpha one"), Clip::Text("beta"), Clip::Text("alpha two")]);
    let g = h.open();
    type_text(&mut h, g, "alpha");
    key(&mut h, g, Key::CtrlDelete);
    assert!(h.c.is_open());
    assert_eq!(h.c.displayed_text().as_deref(), Some("alpha one"));
    assert_eq!(h.c.match_counts(), Some((1, 2)));
    key(&mut h, g, Key::CtrlDelete);
    assert!(h.c.is_open(), "the last match went: the overlay says so instead of closing");
    assert_eq!(h.c.current_id(), None);
    key(&mut h, g, Key::Escape);
    assert_eq!(h.c.displayed_text().as_deref(), Some("beta"), "the unrelated clip is untouched");
}

#[test]
fn a_new_session_starts_with_an_empty_search_and_everything() {
    let mut h = H::new(&[Clip::Text("alpha"), Clip::Text("beta")]);
    let g = h.open();
    type_text(&mut h, g, "alp");
    key(&mut h, g, Key::Tab);
    key(&mut h, g, Key::Down);
    key(&mut h, g, Key::Enter);
    key(&mut h, g, Key::Escape);
    key(&mut h, g, Key::Escape);
    assert!(!h.c.is_open());
    h.open();
    assert_eq!(h.c.query(), Some(""));
    assert_eq!(h.c.filter_label().as_deref(), Some("Everything"));
    assert_eq!(h.vm().query, "");
}

#[test]
fn a_custom_filter_is_added_from_the_dropdown() {
    let mut h = H::new(&[Clip::Files(&["C:\\x\\logo.psd"]), Clip::Files(&["C:\\x\\a.pdf"])]);
    let g = h.open();
    key(&mut h, g, Key::Tab);
    key(&mut h, g, Key::Up); // wraps to the "Add custom filter..." entry
    key(&mut h, g, Key::Enter);
    assert!(h.vm().placeholder.contains("then extensions"));
    type_text(&mut h, g, "Design psd .ai");
    assert_eq!(h.c.match_counts(), Some((0, 0)), "typing the definition must not search");
    key(&mut h, g, Key::Enter);
    assert_eq!(h.c.filter_label().as_deref(), Some("Design"));
    assert_eq!(h.c.match_counts(), Some((1, 2)));
    assert!(h.vm().rows[0].title.contains("logo.psd"));
    assert_eq!(h.p.saved_filters.len(), 1);
    // it is now part of the dropdown
    key(&mut h, g, Key::Tab);
    let items = h.vm().dropdown.expect("open").items;
    assert!(items.iter().any(|i| i == "Design"));
}

#[test]
fn a_bad_custom_filter_is_refused_with_a_message_and_nothing_is_saved() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    key(&mut h, g, Key::Tab);
    key(&mut h, g, Key::Up);
    key(&mut h, g, Key::Enter);
    type_text(&mut h, g, "Image png"); // a built-in name
    key(&mut h, g, Key::Enter);
    assert!(h.vm().footer.contains("built-in"), "{}", h.vm().footer);
    assert!(h.p.saved_filters.is_empty());
    key(&mut h, g, Key::Escape);
    assert!(h.c.is_open(), "Esc leaves the add-filter mode first");
}

#[test]
fn the_search_box_is_capped_and_ignores_control_characters() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.send(Event::Key(Key::Char('\u{7}'), g));
    h.send(Event::Key(Key::Char('\n'), g));
    assert_eq!(h.c.query(), Some(""));
    for _ in 0..500 {
        h.send(Event::Key(Key::Char('x'), g));
    }
    assert_eq!(h.c.query().map(|q| q.chars().count()), Some(120));
}

#[test]
fn typing_after_the_session_ended_is_ignored() {
    let mut h = H::new(&[Clip::Text("alpha")]);
    let g = h.open();
    key(&mut h, g, Key::Escape);
    assert!(!h.c.is_open());
    h.send(Event::Key(Key::Char('a'), g));
    assert_eq!(h.c.query(), None);
}

/// Writes what the user would see for a few searches to PNGs, for looking at by eye.
/// `PASTEROLL_DUMP_DIR=<dir> cargo test -p pasteroll-app --lib dump_search_screens -- --ignored`
#[test]
#[ignore]
fn dump_search_screens() {
    let Ok(dir) = std::env::var("PASTEROLL_DUMP_DIR") else { return };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    fn save(h: &mut H, dir: &std::path::Path, name: &str, r: &mut pasteroll_render::Renderer) {
        let vm = h.c.view_model();
        let f = r.render(&vm, (pasteroll_render::WIDTH, pasteroll_render::HEIGHT), 1.0);
        let mut rgba = Vec::with_capacity(f.bgra.len());
        for p in f.bgra.chunks_exact(4) {
            rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        let png = encode_png(&Rgba { width: f.width, height: f.height, pixels: rgba }).unwrap();
        std::fs::write(dir.join(name), &png).unwrap();
        let _ = png;
    }
    let mut r = pasteroll_render::Renderer::new();
    let mut h = H::new(&[
        Clip::Text("Meeting notes: ship the spike, then write the invoice reminder for Friday"),
        Clip::Files(&["C:\\Users\\me\\Documents\\Invoice-2026.pdf"]),
        Clip::Text("https://example.com/Invoice/pay?ref=123"),
        Clip::Image,
        Clip::Files(&["C:\\Users\\me\\Music\\song.mp3", "C:\\Users\\me\\Music\\cover.jpg"]),
        Clip::Text("hello world, nothing special here"),
    ]);
    let g = h.open();
    save(&mut h, &dir, "s1-all.png", &mut r);
    type_text(&mut h, g, "invoice");
    save(&mut h, &dir, "s2-query.png", &mut r);
    key(&mut h, g, Key::Tab);
    save(&mut h, &dir, "s3-dropdown.png", &mut r);
    key(&mut h, g, Key::Escape);
    key(&mut h, g, Key::Escape); // clear the query
    key(&mut h, g, Key::Tab);
    key(&mut h, g, Key::Down);
    key(&mut h, g, Key::Down);
    key(&mut h, g, Key::Down);
    key(&mut h, g, Key::Enter);
    save(&mut h, &dir, "s4-image-filter.png", &mut r);
    key(&mut h, g, Key::Tab);
    for _ in 0..5 {
        key(&mut h, g, Key::Down);
    }
    key(&mut h, g, Key::Enter);
    save(&mut h, &dir, "s5a-add-mode-empty.png", &mut r);
    type_text(&mut h, g, "Design psd");
    save(&mut h, &dir, "s5-add-filter.png", &mut r);
    key(&mut h, g, Key::Escape);
    type_text(&mut h, g, "zzzz");
    save(&mut h, &dir, "s6-no-match.png", &mut r);
    let _ = h.dir.clone();
}

#[test]
#[ignore]
fn timing_the_search_index_build() {
    let dir = std::env::temp_dir().join(format!("pasteroll-idx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::open(&dir.join("h.db"), vault()).unwrap();
    store.set_limits(Limits { max_clips: 1_000_000, ..Limits::default() }).unwrap();
    let t = std::time::Instant::now();
    for i in 0..20_000 {
        store.add_text(&format!("clip {i}: some ordinary copied text, long enough to look real {}", i * 7), 1_700_000_000 + i).unwrap();
    }
    eprintln!("inserted 20000 in {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let rows = store.search_rows().unwrap();
    eprintln!("search_rows for {} clips: {:?}", rows.len(), t.elapsed());
    let t = std::time::Instant::now();
    let idx = crate::search::SearchIndex::from_rows(rows);
    eprintln!("index build: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let r = idx.search("clip 19999", &crate::filters::Filter::Everything, &[]);
    eprintln!("search: {:?} ({} hits)", t.elapsed(), r.len());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- review follow-ups

#[test]
fn plain_delete_does_nothing_while_the_search_box_has_text() {
    let mut h = H::new(&[Clip::Text("alpha one"), Clip::Text("alpha two")]);
    let g = h.open();
    type_text(&mut h, g, "alpha");
    key(&mut h, g, Key::Delete);
    assert_eq!(h.c.match_counts(), Some((2, 2)), "nothing was deleted");
    key(&mut h, g, Key::CtrlDelete);
    assert_eq!(h.c.match_counts(), Some((1, 1)), "Ctrl+Delete still deletes");
}

#[test]
fn plain_delete_still_deletes_when_the_box_is_empty() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    key(&mut h, g, Key::Delete);
    assert_eq!(h.c.displayed_text().as_deref(), Some("a"));
}

#[test]
fn a_pending_click_paste_is_modal() {
    let mut h = H::new(&[Clip::Text("alpha"), Clip::Text("beta")]);
    let g = h.open();
    h.send(Event::ClickPaste(g));
    type_text(&mut h, g, "alp");
    key(&mut h, g, Key::Tab);
    key(&mut h, g, Key::CtrlP);
    key(&mut h, g, Key::CtrlDelete);
    assert_eq!(h.c.query(), Some(""), "typing must not change the clip a click is about to paste");
    assert!(h.vm().dropdown.is_none());
    assert_eq!(h.c.match_counts(), Some((0, 0)));
    key(&mut h, g, Key::Escape);
    assert!(!h.c.is_open(), "Esc still cancels");
}

#[test]
fn changing_the_filter_withdraws_a_terminal_warning() {
    let mut h = H::new(&[Clip::Text("a\nb"), Clip::Text("c\nd")]);
    in_terminal(&mut h);
    let g = h.open();
    h.enter(g);
    assert!(h.vm().footer.contains("terminal"));
    // pick a filter with the mouse: no key event clears the flag on this path
    let vm = h.vm();
    let size = (pasteroll_render::WIDTH, pasteroll_render::HEIGHT);
    let b = geometry(&vm, size, 1.0).filter_button;
    h.mouse(MouseKind::Down, (b.x + b.w / 2.0, b.y + b.h / 2.0));
    let vm = h.vm();
    let r = geometry(&vm, size, 1.0).dropdown_items[1]; // Text
    h.mouse(MouseKind::Down, (r.x + r.w / 2.0, r.y + r.h / 2.0));
    assert!(!h.vm().footer.contains("terminal"), "{}", h.vm().footer);
    h.enter(g);
    assert!(h.p.clipboard.is_empty(), "the candidate changed, so the warning must be given again");
    assert!(h.vm().footer.contains("terminal"));
}

#[test]
fn a_session_closes_before_the_input_hook_gives_up() {
    let mut h = H::new(&[Clip::Text("a")]);
    h.open();
    h.p.now_ms += 40_000;
    h.send(Event::Key(Key::Down, h.p.input_generation()));
    h.send(Event::Tick);
    assert!(h.c.is_open());
    h.p.now_ms += 16_000; // 56 s after opening, activity 16 s ago: only the session cap applies
    h.send(Event::Key(Key::Down, h.p.input_generation()));
    h.send(Event::Tick);
    assert!(!h.c.is_open(), "the hook stops swallowing at 60 s; the overlay must be gone by then");
}

#[test]
fn processing_input_counts_as_a_heartbeat() {
    let mut h = H::new(&[Clip::Text("alpha")]);
    let g = h.open();
    let before = h.p.heartbeats;
    type_text(&mut h, g, "abc");
    assert!(h.p.heartbeats >= before + 3, "typing under key repeat must keep the hook alive");
}

#[test]
fn a_pathological_clip_does_not_make_a_search_slow() {
    let mut h = H::new(&[Clip::Text(&"e ".repeat(1000)), Clip::Text("plain")]);
    let g = h.open();
    let start = std::time::Instant::now();
    type_text(&mut h, g, "e");
    let _ = h.vm();
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
}

// ---------------------------------------------------------------- ultrareview follow-ups

#[test]
fn unpinning_in_the_pinned_filter_removes_the_clip_from_the_list() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    key(&mut h, g, Key::CtrlP); // pins "b"
    key(&mut h, g, Key::Tab);
    for _ in 0..7 {
        key(&mut h, g, Key::Down);
    }
    key(&mut h, g, Key::Enter);
    assert_eq!(h.c.filter_label().as_deref(), Some("Pinned"));
    assert_eq!(h.c.match_counts(), Some((1, 2)));
    key(&mut h, g, Key::CtrlP); // unpin it again
    assert_eq!(h.c.match_counts(), Some((0, 2)), "the count must follow the pin");
    assert_eq!(h.c.current_id(), None, "the unpinned clip must leave the Pinned list");
}

#[test]
fn leaving_add_filter_mode_restores_the_full_list() {
    let mut h = H::new(&[Clip::Text("alpha"), Clip::Text("beta")]);
    let g = h.open();
    type_text(&mut h, g, "alp");
    assert_eq!(h.c.displayed_text().as_deref(), Some("alpha"));
    key(&mut h, g, Key::Tab);
    key(&mut h, g, Key::Up); // "Add custom filter..."
    key(&mut h, g, Key::Enter);
    assert_eq!(h.c.query(), Some(""));
    assert_eq!(h.c.displayed_text().as_deref(), Some("beta"), "an empty box shows everything");
    type_text(&mut h, g, "Design psd");
    assert_eq!(h.c.displayed_text().as_deref(), Some("beta"), "a filter definition is not a search");
    key(&mut h, g, Key::Escape);
    assert_eq!(h.c.query(), Some(""));
    assert_eq!(h.c.displayed_text().as_deref(), Some("beta"));
    assert_eq!(h.c.match_counts(), Some((0, 0)));
    assert!(h.c.is_open());
}

#[test]
fn search_matches_are_marked_in_the_big_preview() {
    let mut h = H::new(&[Clip::Text("say hello world, hello again, héllo")]);
    let g = h.open();
    type_text(&mut h, g, "hello");
    // "hello" at chars 4..9 and 17..22 (the accented one is a different word)
    assert_eq!(h.vm().body_matches, vec![(4, 9), (17, 22)]);
    key(&mut h, g, Key::Escape);
    assert!(h.vm().body_matches.is_empty(), "no query, no marks");
}

#[test]
fn a_clip_copied_while_searching_joins_the_results() {
    let (mut h, state) = ingest_harness(&[]);
    copy(&h, &state, "hello from notepad", Some("notepad.exe"));
    assert!(wait_for_count(&h, 1));
    let g = h.open();
    type_text(&mut h, g, "hello");
    assert_eq!(h.c.match_counts(), Some((1, 1)));
    copy(&h, &state, "hello again", Some("notepad.exe"));
    assert!(wait_for_count(&h, 2));
    std::thread::sleep(Duration::from_millis(100));
    h.send(Event::Wake); // the platform delivers this when the worker has stored a clip
    assert_eq!(h.c.match_counts(), Some((2, 2)), "the new clip must be searchable at once");
}

#[test]
fn a_picture_that_was_fetched_ahead_shows_at_once() {
    let mut h = H::new(&[Clip::Color(10), Clip::Color(60), Clip::Color(120), Clip::Color(200)]);
    let g = h.open();
    // the newest picture arrives from the worker
    for _ in 0..200 {
        h.send(Event::Wake);
        if matches!(h.vm().body, Body::Image(Some(_))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(h.vm().body, Body::Image(Some(_))), "the candidate's picture never arrived");
    // give the worker a moment to fetch the neighbours, then deliver them
    for _ in 0..100 {
        h.send(Event::Wake);
        std::thread::sleep(Duration::from_millis(10));
    }
    h.step_older(g);
    assert!(
        matches!(h.vm().body, Body::Image(Some(_))),
        "the next picture should already be in the cache when its turn comes"
    );
}

#[test]
fn closing_the_overlay_forgets_the_pictures() {
    let mut h = H::new(&[Clip::Color(10), Clip::Color(60)]);
    let g = h.open();
    for _ in 0..200 {
        h.send(Event::Wake);
        std::thread::sleep(Duration::from_millis(5));
    }
    key(&mut h, g, Key::Escape);
    let g = h.open();
    assert!(
        matches!(h.vm().body, Body::Image(None)),
        "a new session starts without cached pictures (privacy)"
    );
    let _ = g;
}

// ---------------------------------------------------------------- expanded viewer

impl H {
    /// Runs the timer until the viewer's animation is over.
    fn settle(&mut self) {
        for _ in 0..60 {
            if !self.c.viewer_animating() {
                break;
            }
            self.p.now_ms += 16;
            self.send(Event::Tick);
        }
        assert!(!self.c.viewer_animating(), "the animation must end");
    }

    fn expand(&mut self, g: u64) {
        self.send(Event::Key(Key::CtrlE, g));
        assert!(self.c.viewer_is_open());
        self.settle();
    }

    fn vkey(&mut self, g: u64, k: Key) {
        self.send(Event::Key(k, g));
    }

    /// Clicks the centre of a viewer element found with `pick`.
    fn vclick(&mut self, pick: impl Fn(&pasteroll_render::VGeom, &ViewerView) -> pasteroll_render::Rect) {
        let view = self.c.viewer_view().unwrap();
        let size = self.c.viewer_size_dip().unwrap();
        let g = pasteroll_render::viewer_geometry(&view, size);
        let r = pick(&g, &view);
        let at = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        self.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Down, x: at.0, y: at.1 }));
        self.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x: at.0, y: at.1 }));
    }

    fn tool(&mut self, t: Tool) {
        self.vclick(|g, v| g.tools[v.tools.iter().position(|x| x.tool == t).expect("tool shown")]);
    }

    fn viewer_text(&self) -> String {
        match self.c.viewer_view().unwrap().body {
            ViewerBody::Text { display, .. } => display,
            other => panic!("not text: {other:?}"),
        }
    }
}

use pasteroll_render::{Tool, ViewerBody, ViewerView, VHit};

#[test]
fn expanding_grows_the_card_into_three_quarters_of_the_monitor() {
    let mut h = H::new(&[Clip::Text("older"), Clip::Text("hello viewer")]);
    let g = h.open();
    h.send(Event::Key(Key::CtrlE, g));
    assert!(h.c.viewer_is_open() && h.p.viewer_mode && h.p.animating);
    // the first frame sits exactly on the card
    let first = h.p.viewer_rect.unwrap();
    let vm = h.vm();
    let card = geometry(&vm, (520.0, 440.0), 1.0).rows[vm.active].unwrap();
    assert_eq!(first, (316 + card.x.round() as i32, 320 + card.y.round() as i32, card.w.round() as u32, card.h.round() as u32));
    h.p.now_ms += 100;
    h.send(Event::Tick);
    let mid = h.p.viewer_rect.unwrap();
    assert!(mid.2 > first.2 && mid.2 < 1440, "growing: {mid:?}");
    h.settle();
    // 75% of 1920 x 1080, centred in the free area (0, 0, 1920, 1040)
    assert_eq!(h.p.viewer_rect, Some((240, 115, 1440, 810)));
    assert!(!h.p.animating);
    assert_eq!(h.viewer_text(), "hello viewer");
    // the carousel behind it is dimmed
    let dim = h.p.last_overlay.clone().unwrap();
    let (r, gch, b) = dim.pixel(5, 5);
    assert!(u32::from(r) + u32::from(gch) + u32::from(b) < 28 + 28 + 32, "dimmed: {:?}", (r, gch, b));
}

#[test]
fn the_card_button_expands_too() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    h.open();
    let vm = h.vm();
    let g = geometry(&vm, (520.0, 440.0), 1.0);
    let e = g.expand.expect("the active card has an expand button");
    h.mouse(MouseKind::Move, (e.x + 5.0, e.y + 5.0));
    assert_eq!(h.vm().hover, Some(Hit::Expand));
    h.mouse(MouseKind::Down, (e.x + 5.0, e.y + 5.0));
    assert!(h.c.viewer_is_open());
}

#[test]
fn browsing_in_the_viewer_leaves_the_carousel_where_it_was() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c"), Clip::Text("d")]);
    let g = h.open();
    h.step_older(g); // carousel on "c"
    let before = h.c.current_id();
    h.expand(g);
    h.vkey(g, Key::Right);
    assert_eq!(h.viewer_text(), "b");
    h.vkey(g, Key::Right);
    assert_eq!(h.viewer_text(), "a");
    h.vkey(g, Key::Left);
    assert_eq!(h.viewer_text(), "b");
    // the wheel and the arrow keys never reach the carousel
    h.send(Event::Step(Step::Older, g));
    h.send(Event::Wheel(Wheel { delta: -120, horizontal: false, ctrl: false, shift: false }, g));
    // typing searches inside the clip, not history
    h.vkey(g, Key::Char('b'));
    assert_eq!(h.c.query(), Some(""));
    assert_eq!(h.c.current_id(), before);
    h.vkey(g, Key::Escape); // closes the find bar
    h.vkey(g, Key::Escape); // collapses
    assert!(h.c.viewer_animating());
    h.settle();
    assert!(!h.c.viewer_is_open() && !h.p.viewer_mode);
    assert_eq!(h.p.viewer_rect, None);
    assert_eq!(h.c.current_id(), before, "back exactly where it was");
    assert!(h.c.is_open(), "the carousel stays open");
}

#[test]
fn the_back_button_returns_to_the_carousel() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    let before = h.c.current_id();
    h.expand(g);
    h.vkey(g, Key::Right);
    h.vclick(|g, _| g.back);
    assert!(h.c.viewer_animating(), "it shrinks back");
    h.settle();
    assert!(!h.c.viewer_is_open() && h.c.is_open());
    assert_eq!(h.c.current_id(), before);
    // from fullscreen too
    h.expand(g);
    h.vkey(g, Key::F11);
    h.settle();
    h.vclick(|g, _| g.back);
    h.settle();
    assert!(!h.c.viewer_is_open() && h.c.is_open());
}

#[test]
fn the_carousel_takes_no_input_under_the_viewer() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b")]);
    let g = h.open();
    h.expand(g);
    let before = h.c.current_id();
    let vm = h.vm();
    let row = geometry(&vm, (520.0, 440.0), 1.0).rows.iter().flatten().next().copied().unwrap();
    h.mouse(MouseKind::Down, (row.x + 5.0, row.y + 5.0));
    h.send(Event::ClickPaste(g));
    assert_eq!(h.c.current_id(), before);
    assert!(h.p.pastes.is_empty());
    // a click elsewhere closes the viewer instead of pasting there
    h.send(Event::ClickOutside(g));
    h.settle();
    assert!(!h.c.viewer_is_open() && h.c.is_open());
}

#[test]
fn fullscreen_comes_first_and_escape_leaves_it_first() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.expand(g);
    h.vkey(g, Key::F11);
    h.settle();
    assert!(h.c.viewer_fullscreen());
    assert_eq!(h.p.viewer_rect, Some((0, 0, 1920, 1080)));
    h.vkey(g, Key::Escape);
    h.settle();
    assert!(!h.c.viewer_fullscreen() && h.c.viewer_is_open());
    assert_eq!(h.p.viewer_rect, Some((240, 115, 1440, 810)));
    h.vclick(|g, _| g.fullscreen);
    h.settle();
    assert!(h.c.viewer_fullscreen());
    h.vkey(g, Key::Escape);
    h.settle();
    h.vkey(g, Key::Escape);
    h.settle();
    assert!(!h.c.viewer_is_open());
}

#[test]
fn copy_takes_the_whole_clip_or_the_selection() {
    let mut h = H::new(&[Clip::Text("alpha beta gamma")]);
    let g = h.open();
    h.expand(g);
    h.vkey(g, Key::CtrlC);
    assert_eq!(h.pasted_texts(), vec!["alpha beta gamma".to_owned()]);
    assert!(h.p.pastes.is_empty(), "copy does not paste");
    // a double-click selects a word, Ctrl+C copies just that
    let view = h.c.viewer_view().unwrap();
    let size = h.c.viewer_size_dip().unwrap();
    let gm = pasteroll_render::viewer_geometry(&view, size);
    let at = (gm.text.x + 3.0, gm.text.y + 8.0);
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::DoubleClick, x: at.0, y: at.1 }));
    h.tool(Tool::Copy);
    assert_eq!(h.pasted_texts().last().unwrap(), "alpha");
    assert!(h.c.viewer_is_open() && h.c.is_open());
}

#[test]
fn paste_from_the_viewer_pastes_the_clip_it_shows() {
    let mut h = H::new(&[Clip::Text("first"), Clip::Text("second")]);
    let g = h.open();
    h.expand(g);
    h.vkey(g, Key::Right);
    assert_eq!(h.viewer_text(), "first");
    h.tool(Tool::Paste);
    assert_eq!(h.pasted_texts(), vec!["first".to_owned()]);
    assert_eq!(h.p.pastes.len(), 1);
    assert!(!h.c.is_open() && !h.c.viewer_is_open() && h.p.viewer_rect.is_none());
}

#[test]
fn deleting_in_the_viewer_uses_the_bin_and_ctrl_z_brings_it_back() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    h.expand(g);
    let c_id = h.c.viewer_clip().unwrap();
    h.tool(Tool::Delete);
    assert_eq!(h.viewer_text(), "b", "the next clip is shown");
    assert!(h.c.store().borrow().is_trashed(c_id).unwrap());
    h.vkey(g, Key::CtrlZ);
    assert_eq!(h.c.viewer_clip(), Some(c_id));
    assert!(!h.c.store().borrow().is_trashed(c_id).unwrap());
    // Shift+Delete asks first, then deletes for good
    h.vkey(g, Key::ShiftDelete);
    assert!(h.c.viewer_view().unwrap().confirm.is_some());
    h.vkey(g, Key::Escape);
    assert!(h.c.viewer_view().unwrap().confirm.is_none());
    assert_eq!(h.c.viewer_clip(), Some(c_id), "Esc keeps it");
    h.vkey(g, Key::ShiftDelete);
    h.vclick(|g, _| g.confirm_ok.unwrap());
    assert!(h.c.store().borrow().info(c_id).unwrap().is_none(), "gone for good");
    assert_eq!(h.viewer_text(), "b");
    // the carousel was on the deleted clip: closing lands it on the next one
    h.vkey(g, Key::Escape);
    h.settle();
    assert_eq!(newest_text(&mut h), "b");
}

#[test]
fn the_carousel_delete_key_and_button_use_the_bin() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    let c_id = h.c.current_id().unwrap();
    h.send(Event::Key(Key::Delete, g));
    assert_eq!(newest_text(&mut h), "b");
    assert!(h.c.store().borrow().is_trashed(c_id).unwrap());
    h.send(Event::Key(Key::CtrlZ, g));
    assert_eq!(h.c.current_id(), Some(c_id), "undo shows it again");
    // the bin button; Shift+click asks to delete for good, a second Shift+click does it
    let vm = h.vm();
    let d = geometry(&vm, (520.0, 440.0), 1.0).delete.unwrap();
    let at = (d.x + 5.0, d.y + 5.0);
    h.p.shift = true;
    h.mouse(MouseKind::Down, at);
    assert!(h.vm().footer.contains("permanently"));
    assert!(!h.c.store().borrow().is_trashed(c_id).unwrap());
    h.mouse(MouseKind::Down, at);
    assert!(h.c.store().borrow().info(c_id).unwrap().is_none());
    h.p.shift = false;
    // a plain click bins it (the list is shorter now: the button has moved)
    let b_id = h.c.current_id().unwrap();
    let vm = h.vm();
    let d = geometry(&vm, (520.0, 440.0), 1.0).delete.unwrap();
    h.mouse(MouseKind::Down, (d.x + 5.0, d.y + 5.0));
    assert!(h.c.store().borrow().is_trashed(b_id).unwrap());
}

#[test]
fn the_recycle_bin_filter_lists_binned_clips_and_restores_them() {
    let mut h = H::new(&[Clip::Text("a"), Clip::Text("b"), Clip::Text("c")]);
    let g = h.open();
    let c_id = h.c.current_id().unwrap();
    h.send(Event::Key(Key::Delete, g));
    let bin = filters::labels(&[]).iter().position(|l| l == "Recycle bin").unwrap();
    h.send(Event::Key(Key::Tab, g));
    for _ in 0..bin {
        h.send(Event::Step(Step::Older, g));
    }
    h.send(Event::Key(Key::Down, g));
    // the dropdown is driven by Up/Down; pick with Enter
    let _ = bin;
    h.send(Event::Key(Key::Escape, g));
    // choose the bin through the controller's public API instead of counting dropdown rows
    h.send(Event::Key(Key::Tab, g));
    for _ in 0..bin {
        h.send(Event::Key(Key::Down, g));
    }
    h.send(Event::Key(Key::Enter, g));
    assert_eq!(h.c.filter_label().as_deref(), Some("Recycle bin"));
    assert_eq!(h.c.current_id(), Some(c_id));
    let vm = h.vm();
    assert!(vm.in_trash);
    let gm = geometry(&vm, (520.0, 440.0), 1.0);
    let r = gm.restore.expect("a restore button in the bin");
    h.mouse(MouseKind::Down, (r.x + 5.0, r.y + 5.0));
    assert!(!h.c.store().borrow().is_trashed(c_id).unwrap());
    assert!(h.vm().rows.is_empty(), "the bin is empty again");
}

#[test]
fn pictures_zoom_and_fit() {
    let mut h = H::new(&[Clip::Image]);
    let g = h.open();
    h.expand(g);
    let zoom = |h: &H| match h.c.viewer_view().unwrap().body {
        ViewerBody::Image { zoom, .. } => zoom,
        _ => panic!("not a picture"),
    };
    assert_eq!(zoom(&h), None, "opens fitted");
    h.vkey(g, Key::Char('+'));
    let z1 = zoom(&h).unwrap();
    h.send(Event::Wheel(Wheel { delta: 120, horizontal: false, ctrl: true, shift: false }, g));
    assert!(zoom(&h).unwrap() > z1);
    h.vkey(g, Key::Char('0'));
    assert_eq!(zoom(&h), None);
    h.tool(Tool::FitActual);
    assert_eq!(zoom(&h), Some(1.0), "actual size at scale 1");
    assert!(h.c.viewer_view().unwrap().zoom_label.contains("100%"));
    h.vkey(g, Key::CtrlC);
    assert!(matches!(h.p.clipboard.last(), Some(Payload::Image(_))));
}

struct FakeOcr;
impl TextRecognizer for FakeOcr {
    fn recognize(&self, img: &pasteroll_core::image::Rgba) -> Result<String, String> {
        Ok(format!("Text in a {}x{} picture", img.width, img.height))
    }
}

#[test]
fn extract_text_shows_the_recognised_text_and_copy_takes_it() {
    let mut h = H::new(&[Clip::Image]);
    h.c.set_ocr(Arc::new(FakeOcr));
    let g = h.open();
    h.expand(g);
    h.tool(Tool::More);
    let view = h.c.viewer_view().unwrap();
    let i = view.menu.as_ref().unwrap().iter().position(|m| m.label.starts_with("Extract text")).unwrap();
    h.vclick(|g, _| g.menu_items[i]);
    for _ in 0..200 {
        h.send(Event::Wake);
        if matches!(h.c.viewer_view().unwrap().body, ViewerBody::Text { .. }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(h.viewer_text(), "Text in a 6x4 picture");
    h.vkey(g, Key::CtrlC);
    assert_eq!(h.pasted_texts(), vec!["Text in a 6x4 picture".to_owned()]);
    h.vkey(g, Key::Escape); // back to the picture
    assert!(matches!(h.c.viewer_view().unwrap().body, ViewerBody::Image { .. }));
    assert!(h.c.viewer_is_open());
}

#[test]
fn find_counts_matches_and_steps_through_them() {
    let mut h = H::new(&[Clip::Text("one two one three ONE")]);
    let g = h.open();
    h.expand(g);
    h.vkey(g, Key::CtrlF);
    for c in "one".chars() {
        h.vkey(g, Key::Char(c));
    }
    let f = h.c.viewer_view().unwrap().find.unwrap();
    assert_eq!((f.count, f.current), (3, Some(0)));
    h.vkey(g, Key::Enter);
    h.vkey(g, Key::F3);
    assert_eq!(h.c.viewer_view().unwrap().find.unwrap().current, Some(2));
    h.vkey(g, Key::ShiftF3);
    assert_eq!(h.c.viewer_view().unwrap().find.unwrap().current, Some(1));
    assert!(h.p.pastes.is_empty(), "Enter steps through matches while finding");
}

#[test]
fn save_as_writes_the_clip_where_the_user_chose_or_to_the_desktop() {
    let mut h = H::new(&[Clip::Text("save me")]);
    let dir = h.dir.join("out");
    std::fs::create_dir_all(&dir).unwrap();
    h.p.save_choice = SaveChoice::Path(dir.join("chosen.txt"));
    let g = h.open();
    h.expand(g);
    h.vkey(g, Key::CtrlS);
    assert_eq!(std::fs::read_to_string(dir.join("chosen.txt")).unwrap(), "save me");
    h.p.save_choice = SaveChoice::Unavailable;
    h.p.desktop = Some(dir.clone());
    h.vkey(g, Key::CtrlS);
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(files.iter().any(|f| f.starts_with("Pasteroll text") && f.ends_with(".txt")), "{files:?}");
    h.p.save_choice = SaveChoice::Cancelled;
    h.vkey(g, Key::CtrlS);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    assert!(h.c.viewer_is_open());
}

#[test]
fn info_tells_what_the_clip_is() {
    let mut h = H::new(&[Clip::Text("two\nlines")]);
    let g = h.open();
    h.expand(g);
    h.vkey(g, Key::CtrlI);
    let info = h.c.viewer_view().unwrap().info.unwrap();
    let get = |k: &str| info.iter().find(|(l, _)| l == k).map(|(_, v)| v.clone());
    assert_eq!(get("Type").as_deref(), Some("Text"));
    assert!(get("Size").unwrap().starts_with("9 characters, 2 lines, 2 words"));
    assert!(get("Copied").is_some() && get("From").is_some());
}

#[test]
fn the_viewer_stays_open_while_reading_but_not_forever() {
    let mut h = H::new(&[Clip::Text("long read")]);
    let g = h.open();
    h.expand(g);
    let before = h.p.keepalives;
    for _ in 0..10 {
        h.p.now_ms += 30_000;
        h.send(Event::Tick);
    }
    assert!(h.c.viewer_is_open(), "five minutes of reading");
    assert!(h.p.keepalives > before, "the hooks are kept alive meanwhile");
    h.p.now_ms += 400_000;
    h.send(Event::Tick);
    assert!(!h.c.is_open() && !h.c.viewer_is_open() && h.p.viewer_rect.is_none());
    let _ = g;
}

#[test]
fn closing_the_viewer_gives_the_carousel_its_full_time_again() {
    let mut h = H::new(&[Clip::Text("a")]);
    let g = h.open();
    h.p.now_ms += 50_000;
    h.expand(g);
    h.p.now_ms += 120_000;
    h.send(Event::Tick);
    assert!(h.c.viewer_is_open());
    h.vkey(g, Key::Escape);
    h.settle();
    h.send(Event::Tick);
    assert!(h.c.is_open(), "not closed at once for having been open long");
    h.p.now_ms += 56_000;
    h.send(Event::Tick);
    assert!(!h.c.is_open());
}

#[test]
fn hovering_a_toolbar_button_explains_it() {
    let mut h = H::new(&[Clip::Text("x")]);
    let g = h.open();
    h.expand(g);
    let view = h.c.viewer_view().unwrap();
    let size = h.c.viewer_size_dip().unwrap();
    let gm = pasteroll_render::viewer_geometry(&view, size);
    let i = view.tools.iter().position(|t| t.tool == Tool::Pin).unwrap();
    let r = gm.tools[i];
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: r.x + 4.0, y: r.y + 4.0 }));
    assert_eq!(h.c.viewer_view().unwrap().hover, Some(VHit::Tool(Tool::Pin)));
    h.tool(Tool::Pin);
    let view = h.c.viewer_view().unwrap();
    assert!(view.tools.iter().any(|t| t.tool == Tool::Pin && t.on));
    assert!(view.subtitle.contains("Pinned"));
}
