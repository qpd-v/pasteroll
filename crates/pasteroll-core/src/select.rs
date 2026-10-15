//! Text selection over UTF-16 code units.
//!
//! DirectWrite hit-testing returns UTF-16 positions, so the overlay's text is kept as `&[u16]`
//! and selection is expressed the same way. Positions are always moved to code-point boundaries
//! so a selection never splits a surrogate pair.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// Where the drag started (fixed end).
    pub anchor: usize,
    /// Where the drag currently is (moving end).
    pub caret: usize,
}

fn is_high(u: u16) -> bool {
    (0xD800..0xDC00).contains(&u)
}
fn is_low(u: u16) -> bool {
    (0xDC00..0xE000).contains(&u)
}

/// Moves `pos` onto a code-point boundary (backwards if it falls inside a surrogate pair).
pub fn snap(text: &[u16], pos: usize) -> usize {
    let pos = pos.min(text.len());
    if pos > 0 && pos < text.len() && is_low(text[pos]) && is_high(text[pos - 1]) {
        pos - 1
    } else {
        pos
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Word,
    Space,
    Other,
}

fn class(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

impl Selection {
    pub fn empty_at(pos: usize) -> Self {
        Self { anchor: pos, caret: pos }
    }

    pub fn all(text: &[u16]) -> Self {
        Self { anchor: 0, caret: text.len() }
    }

    /// `(start, end)` with `start <= end`.
    pub fn range(&self) -> (usize, usize) {
        (self.anchor.min(self.caret), self.anchor.max(self.caret))
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.caret
    }

    /// Extends the moving end (a drag).
    pub fn drag_to(&mut self, text: &[u16], pos: usize) {
        self.caret = snap(text, pos);
    }

    /// The selected text, or `None` when nothing is selected.
    pub fn text(&self, text: &[u16]) -> Option<String> {
        let (s, e) = self.range();
        let (s, e) = (snap(text, s), snap(text, e));
        (e > s).then(|| String::from_utf16_lossy(&text[s..e]))
    }

    /// Double-click: the run of same-class characters (word, spaces, or punctuation) at `pos`.
    pub fn word_at(text: &[u16], pos: usize) -> Self {
        if text.is_empty() {
            return Self::default();
        }
        // Decode to (utf16_start, char) so classes are computed on whole code points.
        let mut chars: Vec<(usize, char)> = Vec::with_capacity(text.len());
        let mut at = 0;
        for r in char::decode_utf16(text.iter().copied()) {
            let c = r.unwrap_or('\u{FFFD}');
            chars.push((at, c));
            at += c.len_utf16();
        }
        let pos = pos.min(text.len().saturating_sub(1));
        let idx = chars.partition_point(|&(s, _)| s <= pos).saturating_sub(1);
        let want = class(chars[idx].1);
        let mut lo = idx;
        while lo > 0 && class(chars[lo - 1].1) == want {
            lo -= 1;
        }
        let mut hi = idx;
        while hi + 1 < chars.len() && class(chars[hi + 1].1) == want {
            hi += 1;
        }
        let start = chars[lo].0;
        let end = chars[hi].0 + chars[hi].1.len_utf16();
        Self { anchor: start, caret: end }
    }

    /// Triple-click: the whole line (between newlines) containing `pos`.
    pub fn line_at(text: &[u16], pos: usize) -> Self {
        let pos = pos.min(text.len());
        let nl = |u: &u16| *u == 0x0A || *u == 0x0D;
        let start = text[..pos].iter().rposition(nl).map_or(0, |i| i + 1);
        let end = text[pos..].iter().position(nl).map_or(text.len(), |i| pos + i);
        Self { anchor: start, caret: end }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn drag_forward_and_backward_give_the_same_range() {
        let t = u("hello world");
        let mut a = Selection::empty_at(2);
        a.drag_to(&t, 7);
        let mut b = Selection::empty_at(7);
        b.drag_to(&t, 2);
        assert_eq!(a.range(), (2, 7));
        assert_eq!(b.range(), (2, 7));
        assert_eq!(a.text(&t).as_deref(), Some("llo w"));
    }

    #[test]
    fn empty_selection_has_no_text() {
        let t = u("abc");
        assert_eq!(Selection::empty_at(1).text(&t), None);
        assert!(Selection::empty_at(1).is_empty());
    }

    #[test]
    fn select_all_covers_everything() {
        let t = u("one two");
        assert_eq!(Selection::all(&t).text(&t).as_deref(), Some("one two"));
    }

    #[test]
    fn double_click_selects_a_word() {
        let t = u("the quick brown fox");
        let s = Selection::word_at(&t, 6); // inside "quick"
        assert_eq!(s.text(&t).as_deref(), Some("quick"));
    }

    #[test]
    fn double_click_on_punctuation_selects_the_punctuation_run() {
        let t = u("a ,. b");
        assert_eq!(Selection::word_at(&t, 3).text(&t).as_deref(), Some(",."));
    }

    #[test]
    fn double_click_keeps_underscored_identifiers_whole() {
        let t = u("let foo_bar = 1;");
        assert_eq!(Selection::word_at(&t, 6).text(&t).as_deref(), Some("foo_bar"));
    }

    #[test]
    fn selection_never_splits_a_surrogate_pair() {
        let t = u("a😀b"); // 😀 is two UTF-16 units at 1..3
        let mut s = Selection::empty_at(0);
        s.drag_to(&t, 2); // lands inside the pair
        assert_eq!(s.range().1, 1);
        assert_eq!(snap(&t, 2), 1);
        let all = Selection { anchor: 0, caret: 3 };
        assert_eq!(all.text(&t).as_deref(), Some("a😀"));
    }

    #[test]
    fn word_at_handles_astral_characters() {
        let t = u("go 𝒳𝒴 now");
        let s = Selection::word_at(&t, 4);
        assert_eq!(s.text(&t).as_deref(), Some("𝒳𝒴"));
    }

    #[test]
    fn triple_click_selects_the_line() {
        let t = u("first\r\nsecond line\r\nthird");
        let s = Selection::line_at(&t, 9);
        assert_eq!(s.text(&t).as_deref(), Some("second line"));
    }

    #[test]
    fn word_at_on_empty_text_is_empty() {
        assert!(Selection::word_at(&[], 0).is_empty());
    }
}
