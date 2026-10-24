//! Advanced Image Paste works on any destination, not only the desktop: Ctrl+click a window with a
//! picture chosen, edit it, and "Paste Image" delivers the result there as the picture, its file's
//! path, or its file's name.

use super::*;
use pasteroll_core::config::UiMode;
use pasteroll_core::image::decode_png;
use pasteroll_render::EHit;

const ROTATE: u32 = 300;
// the Output section's "Paste as" chips
const PASTE_AS: u32 = 905;

fn ctrl() -> Modifiers {
    Modifiers { ctrl: true, ..Modifiers::default() }
}

impl H {
    /// A session over the clips with a window running `exe` in front (and a Desktop folder).
    pub(super) fn window_session(clips: &[Clip<'_>], exe: &str) -> (H, PathBuf) {
        let mut h = H::new(clips);
        let folder = h.dir.join("Desktop");
        std::fs::create_dir_all(&folder).unwrap();
        h.p.desktop = Some(folder.clone());
        h.p.foreground = Some(target(1, exe));
        h.p.current = 1;
        (h, folder)
    }

    /// Clicks the window in front with `mods` held, in a session that is already open.
    pub(super) fn click_window(&mut self, g: u64, mods: Modifiers) {
        self.p.click_mods = mods;
        let t = self.p.foreground.clone().expect("a window in front");
        self.p.click_polls = VecDeque::from([ClickPoll::Target(t)]);
        self.send(Event::ClickPaste(g));
        self.send(Event::Tick);
    }

    /// Ctrl+click on the window, then the animation, then the whole picture arriving.
    pub(super) fn open_editor_in_window(&mut self) -> u64 {
        let g = self.open();
        self.click_window(g, ctrl());
        assert!(self.c.editor_is_open(), "Ctrl+click in a window opens the editor");
        self.settle_editor();
        self.wait_for(|h| h.c.editor_view().is_some_and(|v| v.paste_enabled && !v.busy));
        g
    }

    /// Presses the editor's Paste button and lets the result be delivered.
    pub(super) fn press_paste(&mut self) {
        self.eclick_rect(|l| l.paste);
        self.wait_for(|h| !h.c.editor_is_open() || h.c.editor_animating());
        self.settle_editor();
    }

    fn pasted_image(&self) -> Option<pasteroll_core::image::Rgba> {
        self.p.clipboard.iter().find_map(|p| match p {
            Payload::Image(png) => decode_png(png),
            _ => None,
        })
    }
}

#[test]
fn ctrl_click_in_a_window_opens_the_editor_instead_of_pasting() {
    let (mut h, folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    let g = h.open();
    h.click_window(g, ctrl());
    assert!(h.c.editor_is_open());
    assert!(h.c.is_open(), "the carousel is still there behind it");
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty(), "nothing is pasted yet");
    assert!(h.p.created.is_empty() && std::fs::read_dir(&folder).unwrap().count() == 0, "nothing is saved yet");
    assert!(h.p.viewer_mode && h.p.modal, "modal, like on the desktop");
    h.settle_editor();
    // the choice of what to paste is the first thing in view
    let view = h.eview();
    assert_eq!(view.paste_label, "Paste Image");
    let _ = h.epoint(PASTE_AS, 0);
}

#[test]
fn the_edited_picture_is_pasted_and_nothing_is_saved() {
    let (mut h, folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    let id;
    let original;
    {
        let g = h.open();
        id = h.c.current_id().unwrap();
        original = h.c.store().borrow().image_png(id).unwrap().unwrap();
        h.click_window(g, ctrl());
        h.settle_editor();
        h.wait_for(|h| h.c.editor_view().is_some_and(|v| v.paste_enabled && !v.busy));
    }
    // rotate a quarter turn, so the result is 20 x 30
    h.open_section(ROTATE);
    h.eclick(ROTATE + 2, 1);
    h.wait_for(|h| !h.eview().busy);
    h.press_paste();
    assert!(!h.c.is_open(), "the session ends");
    let img = h.pasted_image().expect("the picture is on the clipboard");
    assert_eq!((img.width, img.height), (20, 30), "the edits are in it");
    assert_eq!(h.p.pastes.len(), 1, "and the paste key was sent");
    assert!(h.p.created.is_empty(), "nothing was saved");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    assert_eq!(h.c.store().borrow().image_png(id).unwrap().unwrap(), original, "the clip is unchanged");
    assert_eq!(h.c.store().borrow().count().unwrap(), 1, "and no new clip appeared");
}

#[test]
fn a_terminal_gets_the_files_path_by_default() {
    let (mut h, folder) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    h.open_editor_in_window();
    assert_eq!(h.eview().paste_label, "Paste Path");
    h.press_paste();
    assert_eq!(h.p.created.len(), 1, "the picture was saved");
    let file = h.p.created[0].clone();
    assert!(file.starts_with(&folder));
    // the name has spaces in it, so the path comes in quotes
    assert_eq!(h.pasted_texts(), vec![format!("\"{}\"", file.to_string_lossy())]);
    assert!(h.pasted_image().is_none());
    assert_eq!(h.p.pastes.len(), 1);
    assert!(decode_png(&std::fs::read(&file).unwrap()).is_some(), "a real picture was saved");
}

#[test]
fn the_choice_can_be_changed_in_the_editor() {
    // a window: Image first, then the file's name
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "chrome.exe");
    h.open_editor_in_window();
    assert_eq!(h.eview().paste_label, "Paste Image");
    h.eclick(PASTE_AS, 2);
    assert_eq!(h.eview().paste_label, "Paste Name");
    h.press_paste();
    let file = h.p.created[0].clone();
    assert_eq!(h.pasted_texts(), vec![file.file_name().unwrap().to_string_lossy().into_owned()], "just the name, as it is");
    assert!(h.pasted_image().is_none());

    // a terminal: Path first, then the picture
    let (mut h, folder) = H::window_session(&[Clip::Gradient(30, 20)], "cmd.exe");
    h.open_editor_in_window();
    h.eclick(PASTE_AS, 0);
    assert_eq!(h.eview().paste_label, "Paste Image");
    h.press_paste();
    assert!(h.pasted_image().is_some());
    assert!(h.pasted_texts().is_empty() && h.p.created.is_empty());
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn the_file_goes_where_the_folder_says() {
    let (mut h, desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    h.open_editor_in_window();
    let elsewhere = h.dir.join("Pictures");
    std::fs::create_dir_all(&elsewhere).unwrap();
    h.eclick_rect(|l| l.folder);
    h.etype(&elsewhere.to_string_lossy());
    h.ekey(Key::Enter);
    h.press_paste();
    assert!(h.p.created[0].starts_with(&elsewhere), "{:?}", h.p.created);
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 0);
}

#[test]
fn a_plain_click_in_a_window_still_pastes_the_picture_at_once() {
    for mods in [Modifiers::default(), Modifiers { shift: true, ..Modifiers::default() }] {
        let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
        let g = h.open();
        h.click_window(g, mods);
        assert!(!h.c.editor_is_open(), "{mods:?} is not the editor's key");
        assert!(h.pasted_image().is_some());
        assert_eq!(h.p.pastes.len(), 1);
        assert!(!h.c.is_open());
    }
}

#[test]
fn the_modifier_does_nothing_for_a_text_clip_in_a_window() {
    let (mut h, _folder) = H::window_session(&[Clip::Text("hello")], "notepad.exe");
    let g = h.open();
    h.click_window(g, ctrl());
    assert!(!h.c.editor_is_open());
    assert_eq!(h.pasted_texts(), vec!["hello"]);
}

#[test]
fn the_editor_key_can_be_changed_for_windows_too() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.c.set_advanced_modifier(pasteroll_core::config::ClickModifier::Alt);
    let g = h.open();
    h.click_window(g, ctrl());
    assert!(!h.c.editor_is_open(), "Ctrl no longer opens it");
    assert!(h.pasted_image().is_some());
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.c.set_advanced_modifier(pasteroll_core::config::ClickModifier::Alt);
    let g = h.open();
    h.click_window(g, Modifiers { alt: true, ..Modifiers::default() });
    assert!(h.c.editor_is_open());
}

#[test]
fn a_window_that_needs_more_rights_is_refused_up_front() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "taskmgr.exe");
    let g = h.open();
    h.p.privileged = true; // the click landed in a window that runs with more rights
    h.click_window(g, ctrl());
    assert!(!h.c.editor_is_open(), "no editor for a window we could not paste into");
    assert!(!h.c.is_open());
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty());
}

impl H {
    /// Runs the timer and the workers' wake-ups for a moment (the editor is expected to stay open).
    fn spin(&mut self) {
        for _ in 0..60 {
            self.p.now_ms += 4;
            self.send(Event::Wake);
            self.send(Event::Tick);
        }
    }

    /// Types `text` into the editor's Folder box.
    pub(super) fn type_folder(&mut self, text: &str) {
        self.eclick_rect(|l| l.folder);
        self.etype(text);
        self.ekey(Key::Enter);
    }
}

#[test]
fn the_editor_stays_open_if_the_window_lost_the_foreground_meanwhile() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.open_editor_in_window();
    h.p.current = 99; // the user switched to another window
    h.eclick_rect(|l| l.paste);
    h.spin();
    assert!(h.c.editor_is_open(), "nothing was lost: the editor is still there");
    assert!(h.eview().toast.contains("no longer in front"), "{}", h.eview().toast);
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty() && h.p.created.is_empty(), "never into a different window");
    // back in front: the same press works
    h.p.current = 1;
    h.press_paste();
    assert!(h.pasted_image().is_some() && h.p.pastes.len() == 1);
}

#[test]
fn a_window_that_changes_after_the_editor_closed_gets_nothing() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.open_editor_in_window();
    h.p.switch_after_write = Some(99); // the foreground moves while the clipboard is being written
    h.press_paste();
    assert!(!h.c.is_open());
    assert!(h.p.pastes.is_empty(), "the keystroke is not sent to the new foreground");
}

#[test]
fn a_desktop_click_saves_a_file_even_if_another_program_is_named_foreground() {
    // some platforms report the previously active program when the desktop is clicked
    let (mut h, folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    let g = h.open();
    h.click_desktop(g, ctrl());
    assert!(h.c.editor_is_open());
    h.settle_editor();
    h.wait_for(|h| h.c.editor_view().is_some_and(|v| v.paste_enabled && !v.busy));
    assert_eq!(h.eview().paste_label, "Paste Image");
    assert!(!h.eview().items.iter().any(|i| i.id == PASTE_AS), "no Paste as: it is a file");
    h.press_paste();
    assert_eq!(h.p.created.len(), 1);
    assert!(h.p.created[0].starts_with(&folder));
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty(), "nothing is pasted into a program nobody clicked");
}

#[test]
fn a_folder_has_to_be_typed_in_full() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    h.open_editor_in_window();
    h.type_folder("Pictures");
    h.eclick_rect(|l| l.paste);
    h.spin();
    assert!(h.c.editor_is_open());
    assert!(h.eview().toast.contains("full path"), "{}", h.eview().toast);
    assert!(h.p.created.is_empty() && h.p.pastes.is_empty());
}

#[test]
fn a_terminal_is_not_given_a_path_it_would_act_on() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    h.open_editor_in_window();
    let odd = h.dir.join("a$b");
    std::fs::create_dir_all(&odd).unwrap();
    h.type_folder(&odd.to_string_lossy());
    h.eclick_rect(|l| l.paste);
    h.spin();
    assert!(h.c.editor_is_open(), "still open to choose something else");
    assert!(h.eview().toast.contains("not safe to paste into a terminal"), "{}", h.eview().toast);
    assert!(h.p.created.is_empty() && h.p.clipboard.is_empty() && h.p.pastes.is_empty());
    assert_eq!(std::fs::read_dir(&odd).unwrap().count(), 0, "and nothing was saved there");
    // the picture itself has no path in it, so that is still fine
    h.eclick(PASTE_AS, 0);
    h.press_paste();
    assert!(h.pasted_image().is_some() && h.p.pastes.len() == 1);
}

#[test]
fn another_window_may_be_given_such_a_path_in_quotes() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "chrome.exe");
    h.open_editor_in_window();
    let odd = h.dir.join("a$b");
    std::fs::create_dir_all(&odd).unwrap();
    h.type_folder(&odd.to_string_lossy());
    h.eclick(PASTE_AS, 1);
    h.press_paste();
    let pasted = h.pasted_texts();
    assert_eq!(pasted.len(), 1);
    assert!(pasted[0].starts_with('"') && pasted[0].ends_with(".png\"") && pasted[0].contains("a$b"), "{pasted:?}");
}

#[test]
fn a_bare_file_name_is_quoted_only_for_a_terminal() {
    for (exe, quoted) in [("chrome.exe", false), ("WindowsTerminal.exe", true)] {
        let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], exe);
        h.open_editor_in_window();
        h.eclick_rect(|l| l.name);
        h.ekey(Key::CtrlA);
        h.etype("a&b");
        h.ekey(Key::Enter);
        h.eclick(PASTE_AS, 2);
        h.press_paste();
        let want = if quoted { "\"a&b.png\"" } else { "a&b.png" };
        assert_eq!(h.pasted_texts(), vec![want.to_owned()], "{exe}");
    }
}

#[test]
fn a_blocked_keystroke_leaves_nothing_but_the_clipboard() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.open_editor_in_window();
    h.p.paste_ok = false;
    h.press_paste();
    assert!(!h.c.is_open());
    assert!(h.p.pastes.is_empty());
}

#[test]
fn cancelling_goes_back_to_the_carousel_and_pastes_nothing() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    let g = h.open_editor_in_window();
    h.ekey(Key::Escape);
    h.settle_editor();
    assert!(!h.c.editor_is_open());
    assert!(h.c.is_open(), "the carousel carries on");
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty() && h.p.created.is_empty());
    // and the next Ctrl+click on the window works again
    h.click_window(g, ctrl());
    assert!(h.c.editor_is_open());
}

#[test]
fn enter_pastes_from_the_editor_too() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.open_editor_in_window();
    h.ekey(Key::Enter);
    h.wait_for(|h| !h.c.editor_is_open() || h.c.editor_animating());
    h.settle_editor();
    assert!(h.pasted_image().is_some() && h.p.pastes.len() == 1);
}

#[test]
fn a_browse_from_the_tray_can_edit_for_the_window_that_is_clicked() {
    let mut h = H::new(&[Clip::Gradient(30, 20)]);
    h.p.foreground = None;
    h.send(Event::BrowseAt(1800, 1040));
    let g = h.p.input_generation();
    let dest = target(2, "notepad.exe");
    h.p.current = dest.id;
    h.p.click_mods = ctrl();
    h.p.click_polls = VecDeque::from([ClickPoll::Target(dest)]);
    h.send(Event::ClickPaste(g));
    h.send(Event::Tick);
    assert!(h.c.editor_is_open());
    h.settle_editor();
    h.wait_for(|h| h.c.editor_view().is_some_and(|v| v.paste_enabled && !v.busy));
    h.press_paste();
    assert!(h.pasted_image().is_some() && h.p.pastes.len() == 1);
}

#[test]
fn it_works_in_no_bloat_too() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.c.init_ui_mode(UiMode::NoBloat);
    h.p.mode = UiMode::NoBloat;
    h.open_editor_in_window();
    h.press_paste();
    assert!(h.pasted_image().is_some() && h.p.pastes.len() == 1);
    assert!(!h.c.is_open());
}

#[test]
fn the_desktop_still_saves_a_file_and_never_pastes() {
    let (mut h, folder) = H::desktop_session(&[Clip::Gradient(30, 20)]);
    let g = h.open();
    h.click_desktop(g, ctrl());
    assert!(h.c.editor_is_open());
    h.settle_editor();
    h.wait_for(|h| h.c.editor_view().is_some_and(|v| v.paste_enabled && !v.busy));
    assert_eq!(h.eview().paste_label, "Paste Image");
    // no "Paste as" choice on the desktop: it is a file
    assert!(!h.eview().items.iter().any(|i| i.id == PASTE_AS));
    h.press_paste();
    assert_eq!(h.p.created.len(), 1);
    assert!(h.p.created[0].starts_with(&folder));
    assert!(h.p.clipboard.is_empty() && h.p.pastes.is_empty());
}

#[test]
fn the_paste_button_is_a_button_of_the_editor() {
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "notepad.exe");
    h.open_editor_in_window();
    let view = h.eview();
    let l = pasteroll_render::editor_layout(&view, h.c.editor_size_dip().unwrap());
    let at = (l.paste.x + l.paste.w / 2.0, l.paste.y + l.paste.h / 2.0);
    assert_eq!(h.c.editor_hit(at.0, at.1), Some(EHit::Paste));
}

#[test]
fn a_click_that_only_opens_the_editor_does_not_wait_for_held_keys() {
    // Ctrl+click on a picture: nothing is typed yet, so the Ctrl still being down is no reason to wait
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let g = h.open();
    h.click_window(g, ctrl());
    assert_eq!(h.p.editor_polls, 1, "polled as an editor click");
    assert!(h.c.editor_is_open());

    // a plain click is the paste keystroke's business: it waits, as it always did
    let (mut h, _folder) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let g = h.open();
    h.click_window(g, Modifiers::default());
    assert_eq!(h.p.editor_polls, 0);
    // and so does Ctrl+click on text, which is an ordinary paste
    let (mut h, _folder) = H::window_session(&[Clip::Text("hello")], "WindowsTerminal.exe");
    let g = h.open();
    h.click_window(g, ctrl());
    assert_eq!(h.p.editor_polls, 0);
    assert_eq!(h.pasted_texts(), vec!["hello"]);
}

#[test]
fn the_desktop_click_for_the_editor_is_polled_the_same_way() {
    let (mut h, _folder) = H::desktop_session(&[Clip::Gradient(30, 20)]);
    let g = h.open();
    h.click_desktop(g, ctrl());
    assert_eq!(h.p.editor_polls, 1);
    assert!(h.c.editor_is_open());
}
