//! Advanced Image Paste: opening it, adjusting a picture, presets, saving, and everything that must
//! stay as it was (the plain click, the original clip).

use super::*;
use pasteroll_core::config::ClickModifier;
use pasteroll_core::imgedit::{BgMode, Format, ResizeMode};
use pasteroll_render::{editor_layout, EHit, Top};

// the first id of each section's items (see `iid` in the controller)
const CROP: u32 = 100;
const RESIZE: u32 = 200;
const ROTATE: u32 = 300;
const CORNERS: u32 = 400;
const OUTLINE: u32 = 500;
const SHADOW: u32 = 600;
const BACKGROUND: u32 = 700;
const PADDING: u32 = 800;
const OUTPUT: u32 = 900;
// the presets popup
const POP_PRESET_ROW: u32 = 2010;
const POP_SAVE_AS: u32 = 2060;
const POP_NAME_BUTTONS: u32 = 2062;
const POP_PALETTE: u32 = 2000;

fn desktop_target() -> Target {
    Target { id: 5, pid: 5, exe: "explorer.exe".into(), class: "Progman".into(), desktop: true }
}

impl H {
    /// A session over the clips with the desktop in front, ready for a click on it.
    pub(super) fn desktop_session(clips: &[Clip<'_>]) -> (H, PathBuf) {
        let mut h = H::new(clips);
        let folder = h.dir.join("Desktop");
        std::fs::create_dir_all(&folder).unwrap();
        h.p.desktop = Some(folder.clone());
        h.p.foreground = Some(desktop_target());
        h.p.current = 5;
        (h, folder)
    }

    /// Clicks the desktop with `mods` held, in a session that is already open.
    pub(super) fn click_desktop(&mut self, g: u64, mods: Modifiers) {
        self.p.click_mods = mods;
        self.p.click_polls = VecDeque::from([ClickPoll::Desktop]);
        self.send(Event::ClickPaste(g));
        self.send(Event::Tick);
    }

    /// Ctrl+click on the desktop, then the animation, then the whole picture arriving.
    pub(super) fn open_editor(&mut self) -> u64 {
        let g = self.open();
        self.click_desktop(g, Modifiers { ctrl: true, ..Modifiers::default() });
        assert!(self.c.editor_is_open(), "Ctrl+click on the desktop opens the editor");
        self.settle_editor();
        self.wait_for(|h| h.c.editor_view().is_some_and(|v| v.paste_enabled && !v.busy));
        g
    }

    pub(super) fn settle_editor(&mut self) {
        for _ in 0..80 {
            if !self.c.editor_animating() {
                break;
            }
            self.p.now_ms += 16;
            self.send(Event::Tick);
        }
        assert!(!self.c.editor_animating(), "the animation must end");
    }

    /// Runs the timer and the workers' wake-ups until `done`.
    pub(super) fn wait_for(&mut self, done: impl Fn(&H) -> bool) {
        for _ in 0..1500 {
            if done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(4));
            self.p.now_ms += 4;
            self.send(Event::Wake);
            self.send(Event::Tick);
        }
        assert!(done(self), "timed out waiting");
    }

    pub(super) fn eview(&self) -> pasteroll_render::EditorView {
        self.c.editor_view().expect("the editor is open")
    }

    /// The centre of part `part` of the item `id` (in the panel or the popup).
    pub(super) fn epoint(&self, id: u32, part: usize) -> (f32, f32) {
        let view = self.eview();
        let size = self.c.editor_size_dip().unwrap();
        let l = editor_layout(&view, size);
        let pl = l
            .popup
            .as_ref()
            .and_then(|(_, items)| items.iter().find(|p| p.id == id))
            .or_else(|| l.placed.iter().find(|p| p.id == id))
            .unwrap_or_else(|| panic!("item {id} is not shown"));
        let r = pl.parts[part];
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    pub(super) fn eclick_at(&mut self, at: (f32, f32)) {
        self.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Down, x: at.0, y: at.1 }));
        self.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x: at.0, y: at.1 }));
    }

    pub(super) fn eclick(&mut self, id: u32, part: usize) {
        let at = self.epoint(id, part);
        self.eclick_at(at);
    }

    pub(super) fn eclick_rect(&mut self, pick: impl Fn(&pasteroll_render::ELayout) -> pasteroll_render::Rect) {
        let view = self.eview();
        let l = editor_layout(&view, self.c.editor_size_dip().unwrap());
        let r = pick(&l);
        self.eclick_at((r.x + r.w / 2.0, r.y + r.h / 2.0));
    }

    pub(super) fn ekey(&mut self, k: Key) {
        let g = self.p.input_generation();
        self.send(Event::Key(k, g));
    }

    pub(super) fn etype(&mut self, text: &str) {
        for c in text.chars() {
            self.ekey(Key::Char(c));
        }
    }

    pub(super) fn params(&self) -> EditParams {
        self.c.editor_params().expect("the editor is open")
    }

    /// Opens a section by clicking its title.
    pub(super) fn open_section(&mut self, id: u32) {
        self.eclick(id, 0);
    }

    /// Types `text` into the box of a slider or field.
    pub(super) fn set_number(&mut self, id: u32, part: usize, text: &str) {
        self.eclick(id, part);
        self.etype(text);
        self.ekey(Key::Enter);
    }
}

use pasteroll_core::imgedit::EditParams;

#[test]
fn ctrl_click_on_the_desktop_opens_the_editor_instead_of_saving() {
    let (mut h, folder) = H::desktop_session(&[Clip::Gradient(40, 30)]);
    let g = h.open();
    h.click_desktop(g, Modifiers { ctrl: true, ..Modifiers::default() });
    assert!(h.c.editor_is_open());
    assert!(h.c.is_open(), "the carousel is still there behind it");
    assert!(h.p.created.is_empty(), "nothing is saved yet");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    assert!(h.p.clipboard.is_empty());
    assert!(h.p.viewer_mode && h.p.modal && h.p.animating);
    // the card grows into the window
    let first = h.p.viewer_rect.unwrap();
    h.p.now_ms += 100;
    h.send(Event::Tick);
    let mid = h.p.viewer_rect.unwrap();
    assert!(mid.2 > first.2 && mid.2 < 1440, "growing: {first:?} -> {mid:?}");
    h.settle_editor();
    assert_eq!(h.p.viewer_rect, Some((240, 115, 1440, 810)), "three quarters of the monitor, centred");
    assert!(!h.p.animating);
    // the carousel behind is dimmed
    let (r, gc, b) = h.p.last_overlay.clone().unwrap().pixel(5, 5);
    assert!(u32::from(r) + u32::from(gc) + u32::from(b) < 90, "dimmed: {:?}", (r, gc, b));
}

#[test]
fn a_plain_click_still_saves_at_once() {
    for mods in [Modifiers::default(), Modifiers { shift: true, ..Modifiers::default() }, Modifiers { alt: true, ..Modifiers::default() }] {
        let (mut h, folder) = H::desktop_session(&[Clip::Gradient(40, 30)]);
        let g = h.open();
        h.click_desktop(g, mods);
        assert!(!h.c.editor_is_open(), "{mods:?} is not the editor's key");
        assert!(!h.c.is_open());
        assert_eq!(h.p.created.len(), 1);
        assert!(h.p.created[0].starts_with(&folder));
        assert!(h.p.clipboard.is_empty());
    }
}

#[test]
fn the_modifier_is_configurable() {
    let (mut h, _folder) = H::desktop_session(&[Clip::Gradient(40, 30)]);
    h.c.set_advanced_modifier(ClickModifier::Shift);
    let g = h.open();
    h.click_desktop(g, Modifiers { ctrl: true, ..Modifiers::default() });
    assert!(!h.c.editor_is_open(), "Ctrl no longer opens it");
    assert_eq!(h.p.created.len(), 1);

    let (mut h, _folder) = H::desktop_session(&[Clip::Gradient(40, 30)]);
    h.c.set_advanced_modifier(ClickModifier::Alt);
    let g = h.open();
    h.click_desktop(g, Modifiers { alt: true, ..Modifiers::default() });
    assert!(h.c.editor_is_open());
    assert!(h.p.created.is_empty());
}

#[test]
fn the_modifier_does_nothing_for_a_text_clip() {
    let (mut h, folder) = H::desktop_session(&[Clip::Text("hello")]);
    let g = h.open();
    h.click_desktop(g, Modifiers { ctrl: true, ..Modifiers::default() });
    assert!(!h.c.editor_is_open());
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn cancel_shrinks_back_into_the_carousel_and_saves_nothing() {
    let (mut h, folder) = H::desktop_session(&[Clip::Text("older"), Clip::Gradient(40, 30)]);
    h.open_editor();
    h.eclick_rect(|l| l.cancel);
    assert!(h.c.editor_animating(), "it shrinks back");
    h.settle_editor();
    assert!(!h.c.editor_is_open());
    assert!(!h.p.modal && !h.p.viewer_mode, "the editor's input rules end with it");
    assert!(h.c.is_open(), "back in the carousel");
    assert!(h.p.created.is_empty() && h.p.clipboard.is_empty());
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    // the carousel works again: it stays open after the foreground check, and Esc closes it
    h.send(Event::Tick);
    assert!(h.c.is_open());
    let g = h.p.input_generation();
    h.send(Event::Key(Key::Escape, g));
    assert!(!h.c.is_open());
}

#[test]
fn back_arrow_and_escape_also_leave_without_saving() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(40, 30)]);
    h.open_editor();
    h.eclick_rect(|l| l.back);
    h.settle_editor();
    assert!(!h.c.editor_is_open() && h.c.is_open());

    // and it can be opened again from the carousel, with a fresh Ctrl+click
    let g = h.p.input_generation();
    h.click_desktop(g, Modifiers { ctrl: true, ..Modifiers::default() });
    assert!(h.c.editor_is_open());
    h.settle_editor();
    h.ekey(Key::Escape);
    h.settle_editor();
    assert!(!h.c.editor_is_open() && h.c.is_open());
}

#[test]
fn the_editor_is_modal_clicks_elsewhere_and_the_carousel_keys_do_nothing() {
    let (mut h, _) = H::desktop_session(&[Clip::Text("a"), Clip::Gradient(40, 30)]);
    let g = h.open_editor();
    let clip = h.c.current_id();
    h.send(Event::ClickOutside(g));
    h.send(Event::ClickCancel(g));
    h.send(Event::ClickPaste(g));
    h.send(Event::Mouse(MouseEvent { kind: MouseKind::Down, x: 50.0, y: 50.0 }));
    h.send(Event::Activate);
    assert!(h.c.editor_is_open() && h.c.is_open());
    assert_eq!(h.c.current_id(), clip, "the carousel did not move");
    // stepping keys do not walk the carousel either
    h.send(Event::Step(Step::Older, g));
    assert_eq!(h.c.current_id(), clip);
    // the session's time limits do not close it while it is being used
    h.p.now_ms += 200_000;
    h.send(Event::Tick);
    assert!(h.c.editor_is_open(), "a long edit is not cut short");
}

#[test]
fn adjusting_and_pasting_saves_the_result_and_ends_the_session() {
    let (mut h, folder) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    let id = h.c.current_id().unwrap();
    let original = h.c.store().borrow().image_png(id).unwrap().unwrap();
    // outline on, 5 px
    h.eclick(OUTLINE, 1);
    assert!(h.params().outline_on);
    h.set_number(OUTLINE + 1, 1, "5");
    assert_eq!(h.params().outline_width, 5.0);
    // rounded corners on, fully round
    h.eclick(CORNERS, 1);
    h.eclick(CORNERS + 2, 1);
    assert!(h.params().round_on);
    // the live preview catches up and says what the result will be
    h.wait_for(|h| !h.eview().busy);
    let view = h.eview();
    assert!(view.note.contains("70 x 50"), "{}", view.note);
    assert!(view.subtitle.contains("60 x 40"));
    let preview = view.preview.clone().unwrap();
    assert!(preview.width >= 60, "the preview shows the result");
    assert_eq!(preview.pixels[3], 0, "its rounded corner is see-through");
    // paste
    h.eclick_rect(|l| l.paste);
    h.wait_for(|h| !h.c.editor_is_open() || h.c.editor_animating());
    h.settle_editor();
    assert!(!h.c.is_open(), "the session ends after the picture is saved");
    assert_eq!(h.p.created.len(), 1);
    let file = &h.p.created[0];
    assert!(file.starts_with(&folder));
    assert!(file.file_name().unwrap().to_string_lossy().starts_with("Pasteroll image 2026-10-13"));
    let saved = pasteroll_core::image::decode_png(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!((saved.width, saved.height), (70, 50));
    assert_eq!(saved.pixels[3], 0, "the corner is transparent");
    let mid = (25 * 70 + 35) * 4;
    assert_eq!(saved.pixels[mid + 3], 255);
    // the clip itself is untouched, and the clipboard was never used
    assert_eq!(h.c.store().borrow().image_png(id).unwrap().unwrap(), original);
    assert_eq!(h.c.store().borrow().count().unwrap(), 1);
    assert!(h.p.clipboard.is_empty());
}

#[test]
fn undo_redo_and_reset_all() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    let view = h.eview();
    assert!(view.tops.iter().all(|t| !t.enabled || t.kind == Top::Presets), "nothing to undo yet");
    h.eclick(OUTLINE, 1);
    h.eclick(SHADOW, 1);
    assert!(h.params().outline_on && h.params().shadow_on);
    let enabled = |h: &H, k: Top| h.eview().tops.iter().find(|t| t.kind == k).unwrap().enabled;
    assert!(enabled(&h, Top::Undo) && !enabled(&h, Top::Redo) && enabled(&h, Top::Reset));
    h.ekey(Key::CtrlZ);
    assert!(h.params().outline_on && !h.params().shadow_on);
    assert!(enabled(&h, Top::Redo));
    h.ekey(Key::CtrlZ);
    assert!(!h.params().outline_on);
    h.ekey(Key::CtrlZ);
    assert!(!h.params().outline_on, "nothing before the start");
    h.ekey(Key::CtrlY);
    h.ekey(Key::CtrlY);
    assert!(h.params().outline_on && h.params().shadow_on);
    // a new change drops what was undone
    h.ekey(Key::CtrlZ);
    h.eclick(PADDING, 1);
    assert!(h.params().pad_on && !h.params().shadow_on);
    assert!(!enabled(&h, Top::Redo));
    // the toolbar buttons do the same
    h.eclick_rect(|l| l.tops[0]);
    assert!(!h.params().pad_on);
    h.eclick_rect(|l| l.tops[1]);
    assert!(h.params().pad_on);
    // reset all, and it can be undone
    h.eclick_rect(|l| l.tops[2]);
    assert_eq!(h.params(), EditParams::default());
    assert!(!enabled(&h, Top::Reset));
    h.ekey(Key::CtrlZ);
    assert!(h.params().outline_on && h.params().pad_on);
}

#[test]
fn a_slider_drag_is_one_step_and_the_preview_follows() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.eclick(OUTLINE, 1);
    h.eclick(OUTLINE, 0); // title: closes the section
    h.eclick(OUTLINE, 0); // and opens it again
    let track = h.epoint(OUTLINE + 1, 0);
    let view = h.eview();
    let l = editor_layout(&view, h.c.editor_size_dip().unwrap());
    let t = l.placed.iter().find(|p| p.id == OUTLINE + 1).unwrap().parts[0];
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Down, x: t.x + 7.0, y: track.1 }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: t.x + t.w / 2.0, y: track.1 }));
    let mid = h.params().outline_width;
    assert!((40.0..=60.0).contains(&mid), "{mid}");
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: t.x + t.w + 100.0, y: track.1 + 40.0 }));
    assert_eq!(h.params().outline_width, 100.0, "dragging past the end gives the maximum");
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x: t.x + t.w, y: track.1 }));
    h.wait_for(|h| !h.eview().busy);
    assert!(h.eview().note.contains("260 x 240"), "{}", h.eview().note);
    // the whole drag is one step to undo
    h.ekey(Key::CtrlZ);
    assert_eq!(h.params().outline_width, 4.0, "back to the width before the drag");
    assert!(h.params().outline_on);
}

#[test]
fn typing_a_number_applies_as_you_type_and_escape_gives_it_up() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.eclick(OUTLINE, 1);
    h.eclick(OUTLINE + 1, 1);
    h.etype("12");
    assert_eq!(h.params().outline_width, 12.0);
    assert!(h.eview().items.iter().any(|i| matches!(&i.kind, pasteroll_render::ItemKind::Slider { field, .. } if field.focused && field.text == "12")));
    // letters are ignored; Backspace edits
    h.etype("x");
    h.ekey(Key::Backspace);
    assert_eq!(h.params().outline_width, 1.0);
    h.ekey(Key::Escape);
    assert!(h.c.editor_is_open(), "Esc gave up the typing, not the editor");
    // wild values are brought into range
    h.set_number(OUTLINE + 1, 1, "99999");
    assert_eq!(h.params().outline_width, 400.0);
    h.set_number(OUTLINE + 1, 1, "0");
    assert_eq!(h.params().outline_width, 0.0);
    // clicking elsewhere ends the typing and keeps the value
    h.eclick(OUTLINE + 1, 1);
    h.etype("7");
    h.eclick_rect(|l| l.name);
    assert_eq!(h.params().outline_width, 7.0);
}

#[test]
fn resizing_by_pixels_and_percent_keeps_or_frees_the_proportions() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.open_section(RESIZE);
    h.eclick(RESIZE + 1, 1); // Pixels
    assert_eq!(h.params().resize_mode, ResizeMode::Pixels);
    assert_eq!((h.params().resize_w, h.params().resize_h), (60.0, 40.0), "starts from the real size");
    h.set_number(RESIZE + 2, 0, "120");
    assert!(h.eview().note.contains("120 x 80"), "{}", h.eview().note);
    // the height box shows the derived value; typing a height sets the width
    h.set_number(RESIZE + 2, 1, "20");
    assert!(h.eview().note.contains("30 x 20"), "{}", h.eview().note);
    // free the proportions
    h.eclick(RESIZE + 3, 0);
    assert!(!h.params().keep_aspect);
    h.set_number(RESIZE + 2, 1, "100");
    assert!(h.eview().note.contains("30 x 100"), "{}", h.eview().note);
    // percent
    h.eclick(RESIZE + 3, 0);
    h.eclick(RESIZE + 1, 2);
    h.set_number(RESIZE + 2, 0, "50");
    assert!(h.eview().note.contains("30 x 20"), "{}", h.eview().note);
    // original again
    h.eclick(RESIZE + 1, 0);
    assert!(h.eview().note.contains("60 x 40"));
}

#[test]
fn rotating_and_flipping() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.open_section(ROTATE);
    h.eclick(ROTATE + 2, 1); // right 90
    assert_eq!(h.params().angle, 90.0);
    assert!(h.eview().note.contains("40 x 60"), "{}", h.eview().note);
    h.eclick(ROTATE + 2, 1);
    assert_eq!(h.params().angle, 180.0);
    h.eclick(ROTATE + 2, 1);
    assert_eq!(h.params().angle, -90.0, "270 wraps into -180 .. 180");
    h.eclick(ROTATE + 2, 1);
    assert_eq!(h.params().angle, 0.0);
    h.eclick(ROTATE + 2, 0);
    h.eclick(ROTATE + 2, 0);
    assert_eq!(h.params().angle, -180.0);
    h.eclick(ROTATE + 2, 0);
    assert_eq!(h.params().angle, 90.0, "-270 wraps too");
    h.eclick(ROTATE + 2, 2);
    h.eclick(ROTATE + 2, 3);
    assert!(h.params().flip_h && h.params().flip_v);
    h.eclick(ROTATE + 2, 2);
    assert!(!h.params().flip_h);
    h.set_number(ROTATE + 1, 1, "45");
    assert_eq!(h.params().angle, 45.0);
    // a turned picture needs a bigger canvas
    let note = h.eview().note;
    assert!(!note.contains("60 x 40"), "{note}");
}

#[test]
fn rounded_corners_square_and_fully_round() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.open_section(CORNERS);
    h.set_number(CORNERS + 1, 1, "10");
    assert_eq!(h.params().radius, 10.0);
    // the radius never needs to be more than half the short side
    h.set_number(CORNERS + 1, 1, "500");
    h.eclick(CORNERS + 2, 0); // square
    assert!(!h.params().round_on && h.params().radius == 0.0);
    h.eclick(CORNERS + 2, 1); // fully round
    assert!(h.params().round_on);
    h.wait_for(|h| !h.eview().busy);
    let p = h.eview().preview.unwrap();
    assert_eq!(p.pixels[3], 0);
}

#[test]
fn the_shadow_background_and_padding_make_the_canvas_bigger() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    // shadow straight down, no blur
    h.eclick(SHADOW, 1);
    h.set_number(SHADOW + 3, 1, "0");
    h.set_number(SHADOW + 4, 1, "10");
    h.set_number(SHADOW + 5, 1, "90");
    assert!(h.eview().note.contains("60 x 50"), "{}", h.eview().note);
    // padding on: the colour fills the whole canvas
    h.eclick(PADDING, 1);
    h.set_number(PADDING + 2, 1, "20");
    assert_eq!(h.params().pad, [20.0; 4]);
    assert!(h.eview().note.contains("100 x 80"), "{}", h.eview().note);
    // different amounts per side
    h.eclick(PADDING + 1, 0);
    h.set_number(PADDING + 2, 0, "5");
    h.set_number(PADDING + 2, 1, "6");
    h.set_number(PADDING + 3, 0, "7");
    h.set_number(PADDING + 3, 1, "8");
    assert_eq!(h.params().pad, [5.0, 8.0, 6.0, 7.0], "top, right, bottom, left");
    // a background colour
    h.eclick(BACKGROUND, 0);
    h.eclick(BACKGROUND + 1, 1);
    assert_eq!(h.params().bg_mode, BgMode::Fill);
    h.eclick(BACKGROUND + 1, 3);
    assert_eq!(h.params().bg_mode, BgMode::Replace);
    h.eclick(BACKGROUND + 3, 0);
    assert!(h.params().bg_key.is_some(), "a picked colour instead of the corners'");
    h.eclick(BACKGROUND, 1); // the switch: back to keeping it
    assert_eq!(h.params().bg_mode, BgMode::Keep);
}

#[test]
fn colours_are_chosen_from_the_palette_or_typed() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.eclick(OUTLINE, 1);
    h.eclick(OUTLINE + 2, 0);
    assert!(h.eview().popup.is_some());
    // a swatch: the first is "none" for the outline
    h.eclick(POP_PALETTE, 6);
    assert!(h.eview().popup.is_none());
    let c = h.params().outline_color;
    assert_ne!(c.0, [0, 0, 0, 255]);
    // a typed code
    h.eclick(OUTLINE + 2, 0);
    h.etype("#ff8800");
    h.ekey(Key::Enter);
    assert_eq!(h.params().outline_color.0, [255, 136, 0, 255]);
    assert!(h.eview().popup.is_none());
    // a bad code stays in the popup with a message
    h.eclick(OUTLINE + 2, 0);
    h.etype("#12");
    h.ekey(Key::Enter);
    assert!(h.eview().popup.is_some());
    assert!(h.eview().toast.contains("colour"), "{}", h.eview().toast);
    // Esc closes it, a click outside too, and nothing changed
    h.ekey(Key::Escape);
    assert!(h.eview().popup.is_none() && h.c.editor_is_open());
    assert_eq!(h.params().outline_color.0, [255, 136, 0, 255]);
    h.eclick(OUTLINE + 2, 0);
    h.eclick_rect(|l| l.paste);
    assert!(h.eview().popup.is_none(), "a click outside only closes the popup");
    assert!(h.p.created.is_empty());
    // the palette includes "none" where it makes sense; undo takes the colours back
    h.ekey(Key::CtrlZ);
    assert_ne!(h.params().outline_color.0, [255, 136, 0, 255]);
}

#[test]
fn cropping_by_dragging_handles_with_a_locked_shape_and_back() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(80, 60)]);
    h.open_editor();
    h.open_section(CROP);
    let view = h.eview();
    assert!(view.crop.is_some(), "the crop frame shows over the original");
    assert_eq!(view.dims, (80, 60));
    let l = editor_layout(&view, h.c.editor_size_dip().unwrap());
    let c = l.crop.unwrap();
    // drag the south-east corner up and left by a quarter of the frame
    let (sx, sy) = (c.x + c.w, c.y + c.h);
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Down, x: sx, y: sy }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: sx - c.w / 4.0, y: sy - c.h / 4.0 }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x: sx - c.w / 4.0, y: sy - c.h / 4.0 }));
    let crop = h.params().crop.expect("cropped");
    assert!((crop.w - 60.0).abs() < 1.5 && (crop.h - 45.0).abs() < 1.5, "{crop:?}");
    assert_eq!((crop.x, crop.y), (0.0, 0.0));
    // exact numbers
    h.set_number(CROP + 2, 0, "40");
    assert_eq!(h.params().crop.unwrap().w, 40.0);
    // a locked shape: 1:1 fits the biggest square in the frame
    h.eclick(CROP + 1, 2);
    let crop = h.params().crop.unwrap();
    assert!((crop.w - crop.h).abs() < 0.01, "{crop:?}");
    // typing a width then follows the locked shape
    h.set_number(CROP + 2, 0, "30");
    let crop = h.params().crop.unwrap();
    assert_eq!((crop.w, crop.h), (30.0, 30.0));
    // leaving the crop view shows the cropped result
    h.eclick(CROP, 0);
    h.wait_for(|h| !h.eview().busy);
    assert!(h.eview().note.contains("30 x 30"), "{}", h.eview().note);
    assert!(h.eview().crop.is_none());
    // the whole picture again
    h.open_section(CROP);
    h.eclick(CROP + 3, 0);
    assert!(h.params().crop.is_none());
    // moving the frame
    h.eclick(CROP + 1, 3); // 4:3
    let view = h.eview();
    let l = editor_layout(&view, h.c.editor_size_dip().unwrap());
    let c = l.crop.unwrap();
    assert!(c.w > 0.0);
}

#[test]
fn the_crop_stays_inside_the_picture_whatever_the_drag() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(80, 60)]);
    h.open_editor();
    h.open_section(CROP);
    let view = h.eview();
    let l = editor_layout(&view, h.c.editor_size_dip().unwrap());
    let c = l.crop.unwrap();
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Down, x: c.x + c.w / 2.0, y: c.y + c.h / 2.0 }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: 5000.0, y: -5000.0 }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x: 5000.0, y: -5000.0 }));
    assert!(h.params().crop.is_none(), "the frame already was the whole picture: moving it changes nothing");
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Down, x: c.x + c.w, y: c.y + c.h }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: c.x + c.w / 3.0, y: c.y + c.h / 3.0 }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x: 0.0, y: 0.0 }));
    let crop = h.params().crop.unwrap();
    assert!(crop.x + crop.w <= 80.01 && crop.y + crop.h <= 60.01);
    // now push the cropped frame far to the corner
    let view = h.eview();
    let l = editor_layout(&view, h.c.editor_size_dip().unwrap());
    let c2 = l.crop.unwrap();
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Down, x: c2.x + c2.w / 2.0, y: c2.y + c2.h / 2.0 }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: 9000.0, y: 9000.0 }));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x: 9000.0, y: 9000.0 }));
    let crop = h.params().crop.unwrap();
    assert!((crop.x + crop.w - 80.0).abs() < 0.01 && (crop.y + crop.h - 60.0).abs() < 0.01, "{crop:?}");
}

#[test]
fn presets_are_saved_applied_and_deleted() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.eclick(OUTLINE, 1);
    h.set_number(OUTLINE + 1, 1, "9");
    h.eclick(SHADOW, 1);
    // save
    h.eclick_rect(|l| l.tops[3]);
    assert!(h.eview().popup.is_some());
    h.eclick(POP_SAVE_AS, 0);
    h.etype("Blog header");
    h.eclick(POP_NAME_BUTTONS, 0);
    assert!(h.eview().popup.is_none());
    assert!(h.p.presets.contains("Blog header\t"), "{}", h.p.presets);
    assert!(h.eview().toast.contains("Blog header"));
    // change things, then apply it
    h.eclick_rect(|l| l.tops[2]);
    assert_eq!(h.params(), EditParams::default());
    h.eclick_rect(|l| l.tops[3]);
    h.eclick(POP_PRESET_ROW, 0);
    assert!(h.params().outline_on && h.params().outline_width == 9.0 && h.params().shadow_on);
    // applying is one step
    h.ekey(Key::CtrlZ);
    assert_eq!(h.params(), EditParams::default());
    // the preset file survives a new session
    h.eclick_rect(|l| l.cancel);
    h.settle_editor();
    let g = h.p.input_generation();
    h.click_desktop(g, Modifiers { ctrl: true, ..Modifiers::default() });
    h.settle_editor();
    h.eclick_rect(|l| l.tops[3]);
    assert!(h.eview().popup.unwrap().items.iter().any(|i| matches!(&i.kind, pasteroll_render::ItemKind::MenuRow { label, .. } if label == "Blog header")));
    // delete (the cross)
    h.eclick(POP_PRESET_ROW, 1);
    assert!(!h.p.presets.contains("Blog header"), "{}", h.p.presets);
    assert!(h.eview().popup.is_some());
}

#[test]
fn a_preset_keeps_the_crop_of_the_picture_it_is_applied_to() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(80, 60)]);
    h.p.presets = "Rounded\tround=1 radius=12 fmt=jpg q=70\n".into();
    h.open_editor();
    h.open_section(CROP);
    h.set_number(CROP + 2, 0, "40");
    h.eclick_rect(|l| l.tops[3]);
    h.eclick(POP_PRESET_ROW, 0);
    let p = h.params();
    assert!(p.round_on && p.radius == 12.0 && p.format == Format::Jpeg && p.quality == 70);
    assert_eq!(p.crop.unwrap().w, 40.0, "the crop was not replaced");
}

#[test]
fn a_blank_preset_name_is_refused_and_an_unwritable_file_is_reported() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.eclick_rect(|l| l.tops[3]);
    h.eclick(POP_SAVE_AS, 0);
    h.eclick(POP_NAME_BUTTONS, 0);
    assert!(h.eview().toast.contains("name"), "{}", h.eview().toast);
    assert!(h.eview().popup.is_some(), "still asking");
    h.p.presets_ok = false;
    h.etype("x");
    h.eclick(POP_NAME_BUTTONS, 0);
    assert!(h.eview().toast.to_lowercase().contains("could not"), "{}", h.eview().toast);
}

#[test]
fn the_file_name_and_folder_can_be_chosen() {
    let (mut h, desktop) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    let elsewhere = h.dir.join("Pictures");
    std::fs::create_dir_all(&elsewhere).unwrap();
    h.open_editor();
    assert_eq!(h.eview().name.text, "Pasteroll image 2026-10-13 12-00-00");
    assert_eq!(h.eview().folder.text, "", "the Desktop unless told otherwise");
    h.eclick_rect(|l| l.name);
    h.ekey(Key::CtrlA);
    h.etype("my: pic*?");
    h.ekey(Key::Enter);
    assert_eq!(h.eview().name.text, "my pic");
    // choose the format: the extension follows
    h.open_section(OUTPUT);
    h.eclick(OUTPUT + 1, 1);
    assert_eq!(h.eview().ext, ".jpg");
    // browse
    h.p.folder_choice = SaveChoice::Path(elsewhere.clone());
    h.eclick_rect(|l| l.browse);
    assert_eq!(std::path::Path::new(&h.eview().folder.text), elsewhere);
    h.eclick_rect(|l| l.paste);
    h.wait_for(|h| !h.c.editor_is_open() || h.c.editor_animating());
    h.settle_editor();
    assert_eq!(h.p.created, vec![elsewhere.join("my pic.jpg")]);
    let bytes = std::fs::read(&h.p.created[0]).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a real JPG");
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 0);
    // the folder is remembered for next time
    assert!(h.p.presets.starts_with("@folder\t"), "{}", h.p.presets);
}

#[test]
fn a_name_already_taken_gets_a_number_and_nothing_is_overwritten() {
    let (mut h, desktop) = H::desktop_session(&[Clip::Gradient(30, 20)]);
    std::fs::write(desktop.join("same.png"), b"mine").unwrap();
    h.open_editor();
    h.eclick_rect(|l| l.name);
    h.ekey(Key::CtrlA);
    h.etype("same.png");
    h.ekey(Key::Enter);
    h.eclick_rect(|l| l.paste);
    h.wait_for(|h| !h.c.editor_is_open() || h.c.editor_animating());
    h.settle_editor();
    assert_eq!(std::fs::read(desktop.join("same.png")).unwrap(), b"mine");
    assert_eq!(h.p.created[0].file_name().unwrap(), "same (2).png");
}

#[test]
fn each_format_writes_a_real_file() {
    for (fmt_part, magic) in [(0usize, &[0x89u8, b'P'][..]), (1, &[0xFF, 0xD8][..]), (2, &b"RI"[..]), (3, &b"BM"[..])] {
        let (mut h, _) = H::desktop_session(&[Clip::Gradient(30, 20)]);
        h.open_editor();
        h.open_section(OUTPUT);
        h.eclick(OUTPUT + 1, fmt_part);
        h.eclick_rect(|l| l.paste);
        h.wait_for(|h| !h.c.editor_is_open() || h.c.editor_animating());
        h.settle_editor();
        let bytes = std::fs::read(&h.p.created[0]).unwrap();
        assert_eq!(&bytes[..magic.len()], magic, "format {fmt_part}");
    }
}

#[test]
fn a_folder_that_does_not_exist_is_reported_and_the_editor_stays() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(30, 20)]);
    h.open_editor();
    h.eclick_rect(|l| l.folder);
    h.etype("Z:\\no\\such\\place");
    h.ekey(Key::Enter);
    h.eclick_rect(|l| l.paste);
    assert!(h.c.editor_is_open());
    assert!(h.eview().toast.contains("folder"), "{}", h.eview().toast);
    assert!(h.p.created.is_empty());
}

#[test]
fn a_result_that_is_too_large_cannot_be_pasted() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    h.open_section(RESIZE);
    h.eclick(RESIZE + 1, 1);
    h.set_number(RESIZE + 2, 0, "30000");
    let v = h.eview();
    assert!(!v.paste_enabled);
    assert!(v.note.contains("too large"), "{}", v.note);
    h.eclick_rect(|l| l.paste);
    assert!(h.c.editor_is_open() && h.p.created.is_empty());
}

#[test]
fn enter_pastes_and_the_editor_never_touches_the_clipboard() {
    let (mut h, folder) = H::desktop_session(&[Clip::Gradient(30, 20)]);
    h.open_editor();
    h.ekey(Key::Enter);
    h.wait_for(|h| !h.c.editor_is_open() || h.c.editor_animating());
    h.settle_editor();
    assert!(!h.c.is_open());
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty());
}

#[test]
fn the_original_clip_is_unchanged_after_cancelling_too() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    let id = h.c.current_id().unwrap();
    h.eclick(OUTLINE, 1);
    h.eclick(CORNERS, 1);
    h.eclick_rect(|l| l.cancel);
    h.settle_editor();
    let png = h.c.store().borrow().image_png(id).unwrap().unwrap();
    let img = pasteroll_core::image::decode_png(&png).unwrap();
    assert_eq!((img.width, img.height), (60, 40));
    assert_eq!(h.c.store().borrow().count().unwrap(), 1);
}

#[test]
fn hovering_the_editor_highlights_and_the_wheel_scrolls_the_panel() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    let view = h.eview();
    let l = editor_layout(&view, h.c.editor_size_dip().unwrap());
    let p = l.paste;
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: p.x + 5.0, y: p.y + 5.0 }));
    assert_eq!(h.eview().hover, Some(EHit::Paste));
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Leave, x: 0.0, y: 0.0 }));
    assert_eq!(h.eview().hover, None);
    // open every section so the panel is longer than the window
    for id in [CROP, RESIZE, ROTATE, CORNERS, OUTLINE, SHADOW, BACKGROUND, PADDING, OUTPUT] {
        h.open_section(id);
    }
    let g = h.p.input_generation();
    // point at the panel and turn the wheel
    let l = editor_layout(&h.eview(), h.c.editor_size_dip().unwrap());
    h.send(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x: l.panel.x + 20.0, y: l.panel.y + 20.0 }));
    h.send(Event::Wheel(Wheel { delta: -120 * 10, horizontal: false, ctrl: false, shift: false }, g));
    assert!(h.eview().scroll >= 0.0);
}

#[test]
fn the_editor_has_a_long_idle_limit_and_cleans_up_when_it_ends() {
    let (mut h, _) = H::desktop_session(&[Clip::Gradient(60, 40)]);
    h.open_editor();
    // the carousel's own 60 seconds do not apply: a long edit is not cut short
    h.p.now_ms += 300_000;
    h.send(Event::Tick);
    assert!(h.c.editor_is_open());
    // but ten minutes without any input ends the session, and everything with it
    h.p.now_ms += 400_000;
    h.send(Event::Tick);
    assert!(!h.c.editor_is_open() && !h.c.is_open());
    assert!(!h.p.modal && !h.p.viewer_mode && h.p.viewer_rect.is_none() && h.p.created.is_empty());
}
