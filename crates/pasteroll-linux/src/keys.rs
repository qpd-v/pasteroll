//! Turns X11 key presses into what the controller understands, using the server's keyboard
//! mapping (no libxkbcommon: US-style and most Latin layouts, shift, caps lock, AltGr, keypad).
//! Dead keys and input methods are not supported, as on Windows.

use crate::x::Keymap;
use pasteroll_app::Key;

pub const SHIFT: u16 = 1;
pub const LOCK: u16 = 2;
pub const CONTROL: u16 = 4;
pub const ALT: u16 = 8;
pub const NUMLOCK: u16 = 16;
pub const SUPER: u16 = 64;
pub const ALTGR: u16 = 128;

fn keysym_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff | 0x20a0..=0x20bf => char::from_u32(sym),
        0x0100_0100..=0x0110_ffff => char::from_u32(sym - 0x0100_0000),
        // keypad
        0xffaa => Some('*'),
        0xffab => Some('+'),
        0xffac => Some(','),
        0xffad => Some('-'),
        0xffae => Some('.'),
        0xffaf => Some('/'),
        0xffb0..=0xffb9 => char::from_u32(u32::from(b'0') + (sym - 0xffb0)),
        _ => None,
    }
}

/// What a key press means while the overlay is open; `None` = swallow and ignore.
pub fn classify(km: &Keymap, code: u8, state: u16) -> Option<Key> {
    let ctrl = state & CONTROL != 0;
    let shift = state & SHIFT != 0;
    let altgr = state & ALTGR != 0;
    let base = km.sym(code, 0);
    // keypad keys give digits with NumLock on (shift inverts), navigation otherwise
    let keypad = (0xff80..=0xffbd).contains(&base);
    let keypad_text = keypad && base != 0xff8d && ((state & NUMLOCK != 0) != shift);
    let named = if keypad_text { None } else { match base {
        0xff0d | 0xff8d => Some(Key::Enter),
        0xff1b => Some(Key::Escape),
        0xff52 | 0xff97 if !ctrl && !shift => Some(Key::Up),
        0xff54 | 0xff99 if !ctrl && !shift => Some(Key::Down),
        0xff52 | 0xff97 if ctrl && !shift => Some(Key::CtrlUp),
        0xff54 | 0xff99 if ctrl && !shift => Some(Key::CtrlDown),
        0xff52 | 0xff97 if shift && !ctrl => Some(Key::ShiftUp),
        0xff54 | 0xff99 if shift && !ctrl => Some(Key::ShiftDown),
        0xff55 | 0xff9a if !ctrl => Some(Key::PageUp),
        0xff56 | 0xff9b if !ctrl => Some(Key::PageDown),
        0xff09 | 0xfe20 if !ctrl => Some(Key::Tab),
        0xff08 => Some(if ctrl { Key::CtrlBackspace } else { Key::Backspace }),
        0xffff | 0xff9f if shift && !ctrl => Some(Key::ShiftDelete),
        0xffff | 0xff9f => Some(if ctrl { Key::CtrlDelete } else { Key::Delete }),
        // the expanded viewer's keys (the keyboard is grabbed during a session, so they are
        // taken from the application underneath either way)
        0xff51 | 0xff96 if !ctrl => Some(Key::Left),
        0xff53 | 0xff98 if !ctrl => Some(Key::Right),
        0xff50 | 0xff95 if !ctrl => Some(Key::Home),
        0xff57 | 0xff9c if !ctrl => Some(Key::End),
        0xffc0 => Some(if shift { Key::ShiftF3 } else { Key::F3 }),
        0xffc8 => Some(Key::F11),
        _ => None,
    } };
    if named.is_some() {
        return named;
    }
    // text
    if (ctrl && !altgr) || state & (ALT | SUPER) != 0 && !altgr {
        // Ctrl+P pins, Ctrl+C/E/F/I/S/Z/A serve the viewer; everything else with Ctrl, Alt or
        // Super is not text
        // redo in the image editor: Ctrl+Shift+Z (and Ctrl+Y below)
        if ctrl && shift && state & (ALT | SUPER) == 0 && base | 0x20 == 0x7a {
            return Some(Key::CtrlY);
        }
        if ctrl && !shift && state & (ALT | SUPER) == 0 {
            return match base | 0x20 {
                0x79 => Some(Key::CtrlY),
                0x70 => Some(Key::CtrlP),
                0x6d => Some(Key::CtrlM),
                0x61 => Some(Key::CtrlA),
                0x63 => Some(Key::CtrlC),
                0x65 => Some(Key::CtrlE),
                0x66 => Some(Key::CtrlF),
                0x69 => Some(Key::CtrlI),
                0x73 => Some(Key::CtrlS),
                0x7a => Some(Key::CtrlZ),
                _ => None,
            };
        }
        return None;
    }
    let mut level = if keypad { usize::from(keypad_text) } else { usize::from(shift) };
    if altgr {
        level += 2;
    }
    let mut sym = km.sym(code, level);
    if sym == 0 {
        sym = km.sym(code, level & 1);
    }
    let mut c = keysym_char(sym)?;
    if state & LOCK != 0 && c.is_alphabetic() && !keypad {
        // Caps Lock flips letter case; with Shift also held it flips it back
        c = if shift {
            c.to_lowercase().next().unwrap_or(c)
        } else {
            c.to_uppercase().next().unwrap_or(c)
        };
    }
    (!c.is_control()).then_some(Key::Char(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keycodes 8..: a, s, p, Return, BackSpace, Tab, Delete, Up, KP_1, 1/!, and AltGr-e (euro).
    fn km() -> Keymap {
        let syms = vec![
            0x61, 0x41, 0, 0, // 8: a A
            0x73, 0x53, 0, 0, // 9: s S
            0x70, 0x50, 0, 0, // 10: p P
            0xff0d, 0, 0, 0, // 11: Return
            0xff08, 0, 0, 0, // 12: BackSpace
            0xff09, 0xfe20, 0, 0, // 13: Tab
            0xffff, 0, 0, 0, // 14: Delete
            0xff52, 0, 0, 0, // 15: Up
            0xff9c, 0xffb1, 0, 0, // 16: KP_End / KP_1 (digit with numlock)
            0x31, 0x21, 0, 0, // 17: 1 !
            0x65, 0x45, 0x20ac, 0, // 18: e E EuroSign
            0xe9, 0xc9, 0, 0, // 19: é É
        ];
        Keymap::from_parts(8, 4, syms)
    }

    #[test]
    fn letters_digits_and_shift() {
        let k = km();
        assert_eq!(classify(&k, 8, 0), Some(Key::Char('a')));
        assert_eq!(classify(&k, 8, SHIFT), Some(Key::Char('A')));
        assert_eq!(classify(&k, 17, 0), Some(Key::Char('1')));
        assert_eq!(classify(&k, 17, SHIFT), Some(Key::Char('!')));
        assert_eq!(classify(&k, 19, 0), Some(Key::Char('é')));
    }

    #[test]
    fn caps_lock_flips_letters_but_not_digits() {
        let k = km();
        assert_eq!(classify(&k, 8, LOCK), Some(Key::Char('A')));
        assert_eq!(classify(&k, 8, LOCK | SHIFT), Some(Key::Char('a')));
        assert_eq!(classify(&k, 17, LOCK), Some(Key::Char('1')));
    }

    #[test]
    fn altgr_reaches_the_third_level() {
        let k = km();
        assert_eq!(classify(&k, 18, ALTGR), Some(Key::Char('€')));
    }

    #[test]
    fn named_keys() {
        let k = km();
        assert_eq!(classify(&k, 11, 0), Some(Key::Enter));
        assert_eq!(classify(&k, 12, 0), Some(Key::Backspace));
        assert_eq!(classify(&k, 12, CONTROL), Some(Key::CtrlBackspace));
        assert_eq!(classify(&k, 13, 0), Some(Key::Tab));
        assert_eq!(classify(&k, 14, 0), Some(Key::Delete));
        assert_eq!(classify(&k, 14, CONTROL), Some(Key::CtrlDelete));
        assert_eq!(classify(&k, 15, 0), Some(Key::Up));
    }

    #[test]
    fn viewer_keys() {
        let k = Keymap::from_parts(
            8,
            2,
            vec![0xff51, 0, 0xff53, 0, 0xff50, 0, 0xff57, 0, 0xffc0, 0, 0xffc8, 0, 0xffff, 0, 0x63, 0x43, 0x7a, 0x5a],
        );
        assert_eq!(classify(&k, 8, 0), Some(Key::Left));
        assert_eq!(classify(&k, 9, 0), Some(Key::Right));
        assert_eq!(classify(&k, 10, 0), Some(Key::Home));
        assert_eq!(classify(&k, 11, 0), Some(Key::End));
        assert_eq!(classify(&k, 12, 0), Some(Key::F3));
        assert_eq!(classify(&k, 12, SHIFT), Some(Key::ShiftF3));
        assert_eq!(classify(&k, 13, 0), Some(Key::F11));
        assert_eq!(classify(&k, 14, SHIFT), Some(Key::ShiftDelete));
        assert_eq!(classify(&k, 15, CONTROL), Some(Key::CtrlC));
        assert_eq!(classify(&k, 16, CONTROL), Some(Key::CtrlZ));
        assert_eq!(classify(&k, 16, CONTROL | SHIFT), Some(Key::CtrlY), "Ctrl+Shift+Z redoes");
    }

    #[test]
    fn shift_and_ctrl_with_the_arrows_choose_clips_and_plain_arrows_move() {
        let k = km();
        assert_eq!(classify(&k, 15, 0), Some(Key::Up));
        assert_eq!(classify(&k, 15, SHIFT), Some(Key::ShiftUp));
        assert_eq!(classify(&k, 15, CONTROL), Some(Key::CtrlUp));
    }

    #[test]
    fn ctrl_p_pins_and_other_ctrl_keys_are_ignored() {
        let k = km();
        assert_eq!(classify(&k, 10, CONTROL), Some(Key::CtrlP));
        assert_eq!(classify(&k, 9, CONTROL), Some(Key::CtrlS), "Ctrl+S saves in the viewer");
        assert_eq!(classify(&k, 8, CONTROL), Some(Key::CtrlA));
        assert_eq!(classify(&k, 18, CONTROL), Some(Key::CtrlE));
        assert_eq!(classify(&k, 17, CONTROL), None);
        assert_eq!(classify(&k, 8, CONTROL | SHIFT), None, "a lingering activation chord types nothing");
        assert_eq!(classify(&k, 8, ALT), None);
        assert_eq!(classify(&k, 8, SUPER), None);
    }

    #[test]
    fn the_keypad_follows_numlock() {
        let k = km();
        assert_eq!(classify(&k, 16, NUMLOCK), Some(Key::Char('1')));
        assert_eq!(classify(&k, 16, 0), Some(Key::End), "End is not text: it is the viewer's End key");
    }

    #[test]
    fn unknown_codes_are_ignored() {
        assert_eq!(classify(&km(), 200, 0), None);
    }
}
