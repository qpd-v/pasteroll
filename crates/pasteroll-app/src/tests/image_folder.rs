//! The folder pictures are saved in (Paste Path and the image editor), chosen in Settings.

use super::*;

fn path_text(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn without_a_setting_pictures_go_to_the_desktop() {
    let (mut h, desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    h.open_editor_in_window();
    assert_eq!(h.eview().folder.text, "");
    assert_eq!(h.eview().folder.placeholder, "Desktop");
    h.press_paste();
    assert!(h.p.created[0].starts_with(&desktop));
}

#[test]
fn the_folder_from_settings_is_where_paste_path_saves() {
    let (mut h, desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let pics = h.dir.join("Pictures");
    h.c.set_image_folder(&path_text(&pics));
    h.open_editor_in_window();
    assert_eq!(h.eview().folder.text, "", "nothing typed: the setting applies");
    assert_eq!(h.eview().folder.placeholder, path_text(&pics), "and the box says where");
    h.press_paste();
    assert!(pics.is_dir(), "a missing folder is made");
    assert_eq!(h.p.created.len(), 1);
    assert!(h.p.created[0].starts_with(&pics), "{:?}", h.p.created);
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 0, "nothing on the Desktop");
    // the path that was pasted is the real one
    assert_eq!(h.pasted_texts(), vec![format!("\"{}\"", path_text(&h.p.created[0]))]);
}

#[test]
fn a_folder_that_is_already_there_is_used_as_it_is() {
    let (mut h, _desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let pics = h.dir.join("Existing");
    std::fs::create_dir_all(&pics).unwrap();
    std::fs::write(pics.join("keep.txt"), b"mine").unwrap();
    h.c.set_image_folder(&format!("  {}  ", path_text(&pics)));
    h.open_editor_in_window();
    h.press_paste();
    assert!(h.p.created[0].starts_with(&pics));
    assert_eq!(std::fs::read(pics.join("keep.txt")).unwrap(), b"mine");
}

#[test]
fn what_is_typed_in_the_editor_beats_the_setting() {
    let (mut h, _desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let (pics, other) = (h.dir.join("Pictures"), h.dir.join("Other"));
    std::fs::create_dir_all(&other).unwrap();
    h.c.set_image_folder(&path_text(&pics));
    h.open_editor_in_window();
    h.type_folder(&path_text(&other));
    h.press_paste();
    assert!(h.p.created[0].starts_with(&other));
    assert!(!pics.exists(), "the setting's folder was not even made");
}

#[test]
fn a_folder_that_cannot_be_used_is_reported_and_nothing_is_saved_elsewhere() {
    // not a full path
    let (mut h, desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    h.c.set_image_folder("pictures\\pasteroll");
    h.open_editor_in_window();
    h.eclick_rect(|l| l.paste);
    assert!(h.c.editor_is_open(), "the editor stays so the person can choose another");
    assert!(h.eview().toast.contains("not a full path"), "{}", h.eview().toast);
    assert!(h.p.created.is_empty() && h.p.clipboard.is_empty());
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 0, "and the Desktop is not used instead");

    // a folder that cannot be made: its parent is a file
    let (mut h, desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let file = h.dir.join("a file");
    std::fs::write(&file, b"x").unwrap();
    h.c.set_image_folder(&path_text(&file.join("inside")));
    h.open_editor_in_window();
    h.eclick_rect(|l| l.paste);
    assert!(h.c.editor_is_open());
    assert!(h.eview().toast.contains("Could not use the image folder"), "{}", h.eview().toast);
    assert_eq!(std::fs::read_dir(&desktop).unwrap().count(), 0);

    // choosing a folder in the editor gets past it
    let other = h.dir.join("Other");
    std::fs::create_dir_all(&other).unwrap();
    h.type_folder(&path_text(&other));
    h.press_paste();
    assert!(h.p.created[0].starts_with(&other));
}

#[test]
fn the_settings_folder_is_not_remembered_as_a_choice() {
    let (mut h, _desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let pics = h.dir.join("Pictures");
    h.c.set_image_folder(&path_text(&pics));
    h.open_editor_in_window();
    h.press_paste();
    assert!(!h.p.presets.contains("@folder\t"), "{}", h.p.presets);
}

#[test]
fn a_folder_chosen_in_the_editor_is_remembered_while_the_setting_stays_the_same() {
    let (mut h, _desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let (pics, other) = (h.dir.join("Pictures"), h.dir.join("Other"));
    std::fs::create_dir_all(&other).unwrap();
    h.c.set_image_folder(&path_text(&pics));
    h.open_editor_in_window();
    h.type_folder(&path_text(&other));
    h.press_paste();
    assert!(h.p.presets.contains("@folder\t"), "{}", h.p.presets);
    assert!(h.p.presets.contains("@folder_base\t"), "{}", h.p.presets);
    // next time the editor opens on it
    h.open_editor_in_window();
    assert_eq!(h.eview().folder.text, path_text(&other));
}

#[test]
fn changing_the_setting_drops_the_folder_remembered_from_before() {
    let (mut h, _desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    let (first, other, second) = (h.dir.join("First"), h.dir.join("Other"), h.dir.join("Second"));
    std::fs::create_dir_all(&other).unwrap();
    h.c.set_image_folder(&path_text(&first));
    h.open_editor_in_window();
    h.type_folder(&path_text(&other));
    h.press_paste();
    // Settings now says somewhere else: that wins over the old choice
    h.c.set_image_folder(&path_text(&second));
    h.open_editor_in_window();
    assert_eq!(h.eview().folder.text, "", "the old choice is not offered");
    h.press_paste();
    assert!(h.p.created.last().unwrap().starts_with(&second), "{:?}", h.p.created);
}

#[test]
fn going_back_to_the_desktop_works_too() {
    let (mut h, desktop) = H::window_session(&[Clip::Gradient(30, 20)], "WindowsTerminal.exe");
    h.c.set_image_folder(&path_text(&h.dir.join("Pictures")));
    h.c.set_image_folder("");
    h.open_editor_in_window();
    assert_eq!(h.eview().folder.placeholder, "Desktop");
    h.press_paste();
    assert!(h.p.created[0].starts_with(&desktop));
}

#[test]
fn clicking_the_desktop_still_saves_to_the_desktop() {
    let (mut h, desktop) = H::desktop_session(&[Clip::Gradient(30, 20)]);
    h.c.set_image_folder(&path_text(&h.dir.join("Pictures")));
    let g = h.open();
    h.click_desktop(g, Modifiers::default());
    assert_eq!(h.p.created.len(), 1);
    assert!(h.p.created[0].starts_with(&desktop), "it is where the click was");
}
