//! The keys that roll between rolls, in Windows' terms: virtual-key codes, and the modifier bits the
//! settings window's hot-key control uses.

use pasteroll_core::bindings::{Combo, ComboKey};

/// `HOTKEYF_SHIFT`, `HOTKEYF_CONTROL`, `HOTKEYF_ALT`, `HOTKEYF_EXT` of the hot-key control.
pub const HK_SHIFT: u8 = 1;
pub const HK_CONTROL: u8 = 2;
pub const HK_ALT: u8 = 4;

/// The virtual-key code of a combination's key.
pub fn vk_of(key: ComboKey) -> u16 {
    match key {
        ComboKey::PageUp => 0x21,
        ComboKey::PageDown => 0x22,
        ComboKey::End => 0x23,
        ComboKey::Home => 0x24,
        ComboKey::Left => 0x25,
        ComboKey::Up => 0x26,
        ComboKey::Right => 0x27,
        ComboKey::Down => 0x28,
        ComboKey::F(n) => 0x6F + u16::from(n),
        ComboKey::Char(c) => c.to_ascii_uppercase() as u16,
    }
}

/// The key for a virtual-key code, if it is one a combination may use.
pub fn key_of(vk: u16) -> Option<ComboKey> {
    Some(match vk {
        0x21 => ComboKey::PageUp,
        0x22 => ComboKey::PageDown,
        0x23 => ComboKey::End,
        0x24 => ComboKey::Home,
        0x25 => ComboKey::Left,
        0x26 => ComboKey::Up,
        0x27 => ComboKey::Right,
        0x28 => ComboKey::Down,
        0x70..=0x7B => ComboKey::F((vk - 0x6F) as u8),
        0x41..=0x5A => ComboKey::Char((vk as u8 as char).to_ascii_lowercase()),
        0x30..=0x39 => ComboKey::Char(vk as u8 as char),
        _ => return None,
    })
}

/// What the hot-key control holds for `combo`: the key and the modifier bits.
pub fn hotkey_of(combo: &Combo) -> (u8, u8) {
    let mut mods = 0;
    if combo.ctrl {
        mods |= HK_CONTROL;
    }
    if combo.alt {
        mods |= HK_ALT;
    }
    if combo.shift {
        mods |= HK_SHIFT;
    }
    (vk_of(combo.key) as u8, mods)
}

/// The combination a hot-key control holds, `Ok(None)` when it is empty. `Err` says why a key
/// cannot be used (the carousel needs it, or it would take over typing).
pub fn combo_of(vk: u8, mods: u8) -> Result<Option<Combo>, String> {
    if vk == 0 {
        return Ok(None);
    }
    let key = key_of(u16::from(vk)).ok_or_else(|| "That key cannot be used for rolling.".to_owned())?;
    let combo = Combo { ctrl: mods & HK_CONTROL != 0, alt: mods & HK_ALT != 0, shift: mods & HK_SHIFT != 0, key };
    // the same rules as the settings file
    Combo::parse(&combo.text()).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_both_ways() {
        for key in [
            ComboKey::Left,
            ComboKey::Right,
            ComboKey::Up,
            ComboKey::Down,
            ComboKey::PageUp,
            ComboKey::PageDown,
            ComboKey::Home,
            ComboKey::End,
            ComboKey::F(1),
            ComboKey::F(12),
            ComboKey::Char('a'),
            ComboKey::Char('z'),
            ComboKey::Char('0'),
            ComboKey::Char('9'),
        ] {
            assert_eq!(key_of(vk_of(key)), Some(key), "{key:?}");
        }
        assert_eq!(vk_of(ComboKey::Left), 0x25);
        assert_eq!(vk_of(ComboKey::Char('a')), 0x41);
        assert_eq!(vk_of(ComboKey::F(5)), 0x74);
        assert_eq!(key_of(0x0D), None, "Enter is not a rolling key");
    }

    #[test]
    fn the_hot_key_control_round_trips() {
        let c = Combo::parse("ctrl+alt+left").unwrap();
        let (vk, mods) = hotkey_of(&c);
        assert_eq!((vk, mods), (0x25, HK_CONTROL | HK_ALT));
        assert_eq!(combo_of(vk, mods).unwrap(), Some(c));
        assert_eq!(combo_of(0, 0).unwrap(), None, "empty");
        assert!(combo_of(0x43, HK_CONTROL).is_err(), "Ctrl+C is the carousel's own");
        assert!(combo_of(0x25, 0).is_err(), "Left alone would take typing");
    }
}
