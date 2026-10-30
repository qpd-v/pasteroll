//! What rolls the carousel from one roll to the next: a key combination, and what each sideways click
//! of a tilt wheel (the thumb wheel or tilting scroll wheel of many mice) does.
//!
//! This is only the vocabulary: parsing, checking and writing the combinations as text. The platform
//! code turns a [`Combo`] into whatever its keyboard hook needs.

/// The key of a combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboKey {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    /// F1 to F12.
    F(u8),
    /// A letter (lower-case) or a digit.
    Char(char),
}

/// Ctrl, Alt, Shift and a key, e.g. `ctrl+alt+left`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: ComboKey,
}

/// Ctrl+letter combinations the carousel already uses itself.
const RESERVED_CTRL_LETTERS: [char; 10] = ['a', 'c', 'e', 'f', 'i', 'k', 'm', 's', 'y', 'z'];

impl Combo {
    /// Reads `ctrl+shift+left`, `Alt + F5`, `ctrl+,`... (case, spaces around `+` do not matter).
    /// Refuses a combination the carousel needs for itself or that would swallow ordinary typing.
    pub fn parse(text: &str) -> Result<Combo, String> {
        let mut c = Combo { ctrl: false, alt: false, shift: false, key: ComboKey::Left };
        let mut key = None;
        for part in text.split('+').map(|p| p.trim().to_ascii_lowercase()) {
            match part.as_str() {
                "" => return Err(format!("{text:?} is not a key combination")),
                "ctrl" | "control" => c.ctrl = true,
                "alt" => c.alt = true,
                "shift" => c.shift = true,
                name => {
                    if key.is_some() {
                        return Err(format!("{text:?} has more than one key"));
                    }
                    key = Some(parse_key(name).ok_or_else(|| format!("{name:?} is not a key that can be used"))?);
                }
            }
        }
        c.key = key.ok_or_else(|| format!("{text:?} needs a key after the modifiers"))?;
        c.check()?;
        Ok(c)
    }

    fn check(&self) -> Result<(), String> {
        let function = matches!(self.key, ComboKey::F(_));
        if !(self.ctrl || self.alt || function) {
            return Err("it needs Ctrl or Alt held (or be an F key), or it would take over ordinary typing".into());
        }
        if self.alt && !self.ctrl && self.key == ComboKey::F(4) {
            return Err("Alt+F4 closes windows".into());
        }
        let ctrl_only = self.ctrl && !self.alt;
        if ctrl_only {
            if let ComboKey::Char(ch) = self.key {
                if RESERVED_CTRL_LETTERS.contains(&ch) {
                    return Err(format!("Ctrl+{} is already used in the carousel", ch.to_ascii_uppercase()));
                }
            }
            if matches!(self.key, ComboKey::Up | ComboKey::Down) {
                return Err("Ctrl with Up or Down chooses clips in the carousel".into());
            }
        }
        if self.shift && !self.ctrl && !self.alt && matches!(self.key, ComboKey::Up | ComboKey::Down) {
            return Err("Shift with Up or Down chooses clips in the carousel".into());
        }
        Ok(())
    }

    /// The combination as written in the settings file: `ctrl+alt+left`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, "ctrl"), (self.alt, "alt"), (self.shift, "shift")] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&key_name(self.key));
        out
    }

    /// The combination as shown to a person: `Ctrl+Alt+Left`.
    pub fn label(&self) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift")] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        let k = key_name(self.key);
        let mut chars = k.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
        out
    }
}

fn parse_key(name: &str) -> Option<ComboKey> {
    Some(match name {
        "left" => ComboKey::Left,
        "right" => ComboKey::Right,
        "up" => ComboKey::Up,
        "down" => ComboKey::Down,
        "pageup" | "pgup" => ComboKey::PageUp,
        "pagedown" | "pgdn" | "pgdown" => ComboKey::PageDown,
        "home" => ComboKey::Home,
        "end" => ComboKey::End,
        f if f.starts_with('f') && f.len() >= 2 => {
            let n: u8 = f[1..].parse().ok()?;
            (1..=12).contains(&n).then_some(ComboKey::F(n))?
        }
        s if s.chars().count() == 1 => {
            let c = s.chars().next()?;
            (c.is_ascii_lowercase() || c.is_ascii_digit()).then_some(ComboKey::Char(c))?
        }
        _ => return None,
    })
}

fn key_name(k: ComboKey) -> String {
    match k {
        ComboKey::Left => "left".into(),
        ComboKey::Right => "right".into(),
        ComboKey::Up => "up".into(),
        ComboKey::Down => "down".into(),
        ComboKey::PageUp => "pageup".into(),
        ComboKey::PageDown => "pagedown".into(),
        ComboKey::Home => "home".into(),
        ComboKey::End => "end".into(),
        ComboKey::F(n) => format!("f{n}"),
        ComboKey::Char(c) => c.to_string(),
    }
}

/// `""` is no combination; anything else must parse.
pub fn parse_optional(text: &str) -> Result<Option<Combo>, String> {
    if text.trim().is_empty() {
        Ok(None)
    } else {
        Combo::parse(text).map(Some)
    }
}

/// What a sideways click of a tilt wheel does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TiltAction {
    /// Nothing: the click goes on to the program underneath as usual.
    None,
    #[default]
    RollLeft,
    RollRight,
}

impl TiltAction {
    pub const ALL: [TiltAction; 3] = [TiltAction::RollLeft, TiltAction::RollRight, TiltAction::None];

    pub fn key(self) -> &'static str {
        match self {
            TiltAction::None => "none",
            TiltAction::RollLeft => "roll-left",
            TiltAction::RollRight => "roll-right",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TiltAction::None => "Nothing",
            TiltAction::RollLeft => "Roll left",
            TiltAction::RollRight => "Roll right",
        }
    }

    pub fn parse(text: &str) -> Option<TiltAction> {
        let flat: String = text.chars().filter(|c| !matches!(c, '-' | '_' | ' ')).map(|c| c.to_ascii_lowercase()).collect();
        match flat.as_str() {
            "none" | "nothing" | "off" => Some(TiltAction::None),
            "rollleft" | "left" => Some(TiltAction::RollLeft),
            "rollright" | "right" => Some(TiltAction::RollRight),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combinations_read_and_write_back() {
        let c = Combo::parse("Ctrl + Alt + Left").unwrap();
        assert_eq!(c, Combo { ctrl: true, alt: true, shift: false, key: ComboKey::Left });
        assert_eq!(c.text(), "ctrl+alt+left");
        assert_eq!(c.label(), "Ctrl+Alt+Left");
        assert_eq!(Combo::parse("ctrl+pgdn").unwrap().text(), "ctrl+pagedown");
        assert_eq!(Combo::parse("f6").unwrap().text(), "f6");
        assert_eq!(Combo::parse("alt+1").unwrap().label(), "Alt+1");
        assert_eq!(Combo::parse(&Combo::parse("ctrl+shift+right").unwrap().text()).unwrap().text(), "ctrl+shift+right");
    }

    #[test]
    fn what_would_take_over_typing_or_the_carousels_own_keys_is_refused() {
        for bad in ["left", "shift+left", "a", "ctrl", "ctrl+", "ctrl+left+right", "ctrl+f13", "ctrl+!", "alt+f4"] {
            assert!(Combo::parse(bad).is_err(), "{bad}");
        }
        for bad in ["ctrl+c", "ctrl+f", "ctrl+k", "ctrl+z", "ctrl+up", "ctrl+down", "shift+up"] {
            assert!(Combo::parse(bad).is_err(), "{bad} is used in the carousel");
        }
        for good in ["ctrl+left", "alt+right", "ctrl+shift+left", "f5", "ctrl+pageup", "alt+q", "ctrl+1"] {
            assert!(Combo::parse(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn nothing_is_a_valid_choice() {
        assert_eq!(parse_optional("  ").unwrap(), None);
        assert!(parse_optional("junk").is_err());
        assert_eq!(TiltAction::parse("Roll Left"), Some(TiltAction::RollLeft));
        assert_eq!(TiltAction::parse("none"), Some(TiltAction::None));
        assert_eq!(TiltAction::parse("sideways"), None);
        for a in TiltAction::ALL {
            assert_eq!(TiltAction::parse(a.key()), Some(a));
        }
    }
}

#[cfg(test)]
mod config_tests {
    use crate::bindings::{Combo, ComboKey, TiltAction};
    use crate::config;

    #[test]
    fn the_defaults_roll_with_ctrl_arrows_and_the_tilt_wheel() {
        let c = config::Config::default();
        assert_eq!(c.rolls.left_key.unwrap().text(), "ctrl+left");
        assert_eq!(c.rolls.right_key.unwrap().text(), "ctrl+right");
        assert_eq!((c.rolls.tilt_left, c.rolls.tilt_right), (TiltAction::RollLeft, TiltAction::RollRight));
        assert!(!c.rolls.most_used && !c.rolls.images);
        // the template says exactly the defaults
        assert_eq!(config::parse(config::TEMPLATE).unwrap(), c);
    }

    #[test]
    fn the_rolls_section_is_read_and_written_back() {
        let text = config::set_value("", "roll_left_key", "Alt+Left").unwrap();
        let text = config::set_value(&text, "tilt_right", "none").unwrap();
        let text = config::set_value(&text, "rolls.most_used", "yes").unwrap();
        let text = config::set_value(&text, "roll_right_key", "").unwrap();
        let c = config::parse(&text).unwrap();
        assert_eq!(c.rolls.left_key, Some(Combo { ctrl: false, alt: true, shift: false, key: ComboKey::Left }));
        assert_eq!(c.rolls.right_key, None);
        assert_eq!(c.rolls.tilt_right, TiltAction::None);
        assert!(c.rolls.most_used && !c.rolls.images);
    }

    #[test]
    fn bad_rolls_settings_are_refused_with_a_reason() {
        assert!(config::set_value("", "roll_left_key", "left").is_err(), "plain Left would take typing");
        assert!(config::set_value("", "roll_left_key", "ctrl+c").is_err(), "Ctrl+C is the carousel's own");
        assert!(config::set_value("", "tilt_left", "sideways").is_err());
        let same = "[rolls]\nroll_left_key = \"ctrl+left\"\nroll_right_key = \"ctrl+left\"\n";
        assert!(config::parse(same).unwrap_err().iter().any(|e| e.contains("same key")));
    }
}
