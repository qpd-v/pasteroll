//! XKB keysyms and modifier bits are shared by X11 and GNOME's Clutter events.
use pasteroll_core::bindings::{Combo, ComboKey, TiltAction};
use pasteroll_core::config::RollSettings;

pub fn roll_key(symbol: u32, mods: u32, settings: &RollSettings) -> Option<i8> {
    if mods & (64 | 128 | (1 << 26)) != 0 { return None; }
    let key = match symbol {
        0xff51 | 0xff96 => ComboKey::Left,
        0xff53 | 0xff98 => ComboKey::Right,
        0xff52 | 0xff97 => ComboKey::Up,
        0xff54 | 0xff99 => ComboKey::Down,
        0xff55 | 0xff9a => ComboKey::PageUp,
        0xff56 | 0xff9b => ComboKey::PageDown,
        0xff50 | 0xff95 => ComboKey::Home,
        0xff57 | 0xff9c => ComboKey::End,
        0xffbe..=0xffc9 => ComboKey::F((symbol - 0xffbe + 1) as u8),
        0x30..=0x39 | 0x41..=0x5a | 0x61..=0x7a => ComboKey::Char((symbol as u8 as char).to_ascii_lowercase()),
        _ => return None,
    };
    let combo = Some(Combo { ctrl: mods & 4 != 0, shift: mods & 1 != 0, alt: mods & 8 != 0, key });
    if settings.left_key == combo { Some(-1) }
    else if settings.right_key == combo { Some(1) }
    else { None }
}

pub fn roll_wheel(right: bool, settings: &RollSettings) -> Option<i8> {
    match if right { settings.tilt_right } else { settings.tilt_left } {
        TiltAction::RollLeft => Some(-1),
        TiltAction::RollRight => Some(1),
        TiltAction::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bindings_respect_modifiers_customization_and_disabled_wheel() {
        let mut s = RollSettings::default();
        assert_eq!(roll_key(0xff51, 4, &s), Some(-1));
        assert_eq!(roll_key(0xff53, 4 | 2 | 16, &s), Some(1)); // Caps/Num lock
        assert_eq!(roll_key(0xff53, 0, &s), None);
        assert_eq!(roll_key(0xff53, 4 | 64, &s), None);
        s.left_key = Some(Combo::parse("alt+f5").unwrap());
        assert_eq!(roll_key(0xffc2, 8, &s), Some(-1));
        assert_eq!(roll_key(0xff51, 4, &s), None);
        s.tilt_right = TiltAction::None;
        assert_eq!(roll_wheel(true, &s), None);
        assert_eq!(roll_wheel(false, &s), Some(-1));
    }
}
