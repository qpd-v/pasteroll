//! Button ownership is decided on press, never reconstructed from release modifiers.
pub const MIDDLE: u16 = 274;
pub const MODIFIERS: [u16; 8] = [29, 97, 42, 54, 56, 100, 125, 126];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(pub u8);
impl Modifiers {
    pub fn set(&mut self, code: u16, down: bool) {
        if let Some(bit) = MODIFIERS.iter().position(|k| *k == code) {
            if down {
                self.0 |= 1 << bit;
            } else {
                self.0 &= !(1 << bit);
            }
        }
    }
    pub fn chord(self) -> bool {
        self.0 & 3 != 0 && self.0 & 12 != 0 && self.0 & 240 == 0
    }
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Forward,
    Consume,
    Activate,
}

#[derive(Default)]
pub struct Buttons {
    // At most the Linux KEY_MAX button range, bounded independently of event count.
    down: std::collections::BTreeSet<u16>,
    claimed: bool,
}
impl Buttons {
    pub fn held(&self) -> bool {
        !self.down.is_empty()
    }
    pub fn process(&mut self, code: u16, value: i32, eligible: bool, mods: Modifiers) -> Decision {
        if !(256..=767).contains(&code) || !(0..=2).contains(&value) {
            return Decision::Forward;
        }
        if value == 0 {
            let was_down = self.down.remove(&code);
            if code == MIDDLE && self.claimed {
                self.claimed = false;
                return Decision::Consume;
            }
            return if was_down {
                Decision::Forward
            } else {
                Decision::Consume
            };
        }
        let first = self.down.insert(code);
        if code == MIDDLE && self.claimed {
            return Decision::Consume;
        }
        if !first || value == 2 {
            return Decision::Consume;
        }
        // Do not interrupt a drag or another mouse button sequence.
        if code == MIDDLE && self.down.len() == 1 && eligible && mods.chord() {
            self.claimed = true;
            Decision::Activate
        } else {
            Decision::Forward
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chord() -> Modifiers {
        let mut m = Modifiers::default();
        m.set(29, true);
        m.set(54, true);
        m
    }
    #[test]
    fn claimed_release_survives_modifiers_and_owner_loss() {
        let mut b = Buttons::default();
        assert_eq!(b.process(MIDDLE, 1, true, chord()), Decision::Activate);
        assert_eq!(b.process(MIDDLE, 1, true, chord()), Decision::Consume);
        assert_eq!(b.process(MIDDLE, 2, true, chord()), Decision::Consume);
        assert_eq!(b.process(MIDDLE, 0, false, Modifiers(0)), Decision::Consume);
        assert!(!b.held());
        assert_eq!(b.process(MIDDLE, 1, false, chord()), Decision::Forward);
        assert_eq!(b.process(MIDDLE, 0, false, Modifiers(0)), Decision::Forward);
    }
    #[test]
    fn ordinary_press_cannot_be_claimed_on_repeat_or_release() {
        let mut b = Buttons::default();
        assert_eq!(b.process(MIDDLE, 1, true, Modifiers(0)), Decision::Forward);
        assert_eq!(b.process(MIDDLE, 2, true, chord()), Decision::Consume);
        assert_eq!(b.process(MIDDLE, 0, true, chord()), Decision::Forward);
        assert_eq!(b.process(MIDDLE, 0, true, chord()), Decision::Consume);
    }
    #[test]
    fn exact_modifiers_across_devices_and_sides() {
        let mut a = Modifiers(0);
        a.set(97, true);
        let mut b = Modifiers(0);
        b.set(42, true);
        assert!(!a.chord());
        assert!(a.union(b).chord());
        for extra in [56, 100, 125, 126] {
            let mut m = a.union(b);
            m.set(extra, true);
            assert!(!m.chord());
        }
        a.set(97, false);
        assert!(!a.union(b).chord());
    }
    #[test]
    fn drag_and_other_buttons_are_preserved() {
        let mut b = Buttons::default();
        assert_eq!(b.process(272, 1, true, chord()), Decision::Forward);
        assert_eq!(b.process(MIDDLE, 1, true, chord()), Decision::Forward);
        assert_eq!(b.process(272, 0, true, chord()), Decision::Forward);
        assert_eq!(b.process(MIDDLE, 0, true, chord()), Decision::Forward);
    }
    #[test]
    fn two_mice_and_device_replacement_do_not_share_ownership() {
        let mut a = Buttons::default();
        let mut b = Buttons::default();
        assert_eq!(a.process(MIDDLE, 1, true, chord()), Decision::Activate);
        assert_eq!(b.process(MIDDLE, 1, false, chord()), Decision::Forward);
        assert_eq!(a.process(MIDDLE, 0, false, chord()), Decision::Consume);
        assert_eq!(b.process(MIDDLE, 0, false, chord()), Decision::Forward);
        let mut replacement = Buttons::default();
        assert_eq!(
            replacement.process(MIDDLE, 0, true, chord()),
            Decision::Consume
        );
    }
}
