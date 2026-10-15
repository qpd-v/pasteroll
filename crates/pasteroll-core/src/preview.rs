//! Making a clip safe to *look at* without changing what gets pasted.
//!
//! A page can put invisible or misleading characters in a clip (a right-to-left override that
//! reorders text, zero-width characters, a hidden newline followed by a command). The overlay shows
//! those as visible `<U+XXXX>` tokens so the user sees what they are about to paste.
//!
//! What is pasted is still the real text: every displayed UTF-16 unit remembers which original
//! character it came from, so a selection made on the display maps back to the original characters
//! (a selected placeholder pastes the character it stands for).

/// Characters shown as placeholders: control codes except tab/newline/CR, zero-width and bidi
/// formatting characters, line/paragraph separators, BOM, soft hyphen.
fn is_hidden(c: char) -> bool {
    match c {
        '\t' | '\n' | '\r' => false,
        '\u{0000}'..='\u{001F}' | '\u{007F}'..='\u{009F}' => true,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' => true,
        '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}' => true,
        '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' => true,
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct Sanitized {
    pub display: String,
    /// Per displayed UTF-16 unit: `(start, end)` UTF-16 offsets of its source character in the
    /// original text.
    src: Vec<(u32, u32)>,
    original_units: u32,
}

pub fn sanitize(original: &str) -> Sanitized {
    let mut display = String::with_capacity(original.len());
    let mut src: Vec<(u32, u32)> = Vec::with_capacity(original.len());
    let mut at = 0u32;
    for c in original.chars() {
        let len = c.len_utf16() as u32;
        let span = (at, at + len);
        if is_hidden(c) {
            let token = format!("<U+{:04X}>", c as u32);
            for _ in token.encode_utf16() {
                src.push(span);
            }
            display.push_str(&token);
        } else {
            let mut buf = [0u16; 2];
            for _ in c.encode_utf16(&mut buf) {
                src.push(span);
            }
            display.push(c);
        }
        at += len;
    }
    Sanitized { display, src, original_units: at }
}

impl Sanitized {
    /// Appends text that is shown but is not part of the clip (it maps to nothing).
    pub fn with_footer(mut self, footer: &str) -> Sanitized {
        let end = (self.original_units, self.original_units);
        for _ in footer.encode_utf16() {
            self.src.push(end);
        }
        self.display.push_str(footer);
        self
    }

    /// Displayed UTF-16 range -> original UTF-16 range (both half-open).
    pub fn original_range(&self, start: usize, end: usize) -> (usize, usize) {
        if end <= start || start >= self.src.len() {
            return (self.original_units as usize, self.original_units as usize);
        }
        let end = end.min(self.src.len());
        (self.src[start].0 as usize, self.src[end - 1].1 as usize)
    }

    pub fn original_len(&self) -> usize {
        self.original_units as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn pasted(original: &str, s: &Sanitized, a: usize, b: usize) -> String {
        let (x, y) = s.original_range(a, b);
        String::from_utf16_lossy(&u(original)[x..y])
    }

    #[test]
    fn plain_text_is_unchanged() {
        let t = "Hello, wörld 😀\tTabbed\r\nNext line";
        let s = sanitize(t);
        assert_eq!(s.display, t);
        assert_eq!(pasted(t, &s, 0, u(t).len()), t);
    }

    #[test]
    fn hidden_characters_become_visible_tokens() {
        let t = "pay\u{202E}fdp.exe\u{200B}x\u{0007}";
        let s = sanitize(t);
        assert_eq!(s.display, "pay<U+202E>fdp.exe<U+200B>x<U+0007>");
    }

    #[test]
    fn a_selection_over_a_token_pastes_the_real_character() {
        let t = "a\u{200B}b";
        let s = sanitize(t);
        // display: "a<U+200B>b"; select the whole token and the b
        let a = u("a").len();
        let end = u(&s.display).len();
        assert_eq!(pasted(t, &s, a, end), "\u{200B}b");
    }

    #[test]
    fn a_partial_token_selection_still_maps_to_the_whole_character() {
        let t = "x\u{202E}y";
        let s = sanitize(t);
        // select just "U+202E" characters inside the token
        let r = pasted(t, &s, 3, 6);
        assert_eq!(r, "\u{202E}");
    }

    #[test]
    fn astral_characters_map_correctly() {
        let t = "😀\u{200B}😀";
        let s = sanitize(t);
        assert_eq!(pasted(t, &s, 0, 2), "😀");
        let all = u(&s.display).len();
        assert_eq!(pasted(t, &s, 0, all), t);
    }

    #[test]
    fn a_footer_is_shown_but_never_pasted() {
        let t = "abc";
        let s = sanitize(t).with_footer("\n... 7 more characters");
        assert!(s.display.ends_with("7 more characters"));
        let all = u(&s.display).len();
        assert_eq!(pasted(t, &s, 0, all), "abc");
        // selecting only the footer pastes nothing
        assert_eq!(pasted(t, &s, 4, all), "");
    }

    #[test]
    fn empty_and_out_of_range_selections_are_empty() {
        let s = sanitize("abc");
        assert_eq!(s.original_range(2, 2), (3, 3));
        assert_eq!(s.original_range(10, 20), (3, 3));
    }

    #[test]
    fn newlines_and_tabs_are_kept_as_they_are() {
        assert_eq!(sanitize("a\nb\tc").display, "a\nb\tc");
    }
}
