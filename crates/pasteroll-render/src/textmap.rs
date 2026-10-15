//! Mapping between UTF-16 offsets (what the rest of the app uses for selection) and the
//! `(line, byte index)` cursors the text layout engine uses.
//!
//! The layout engine splits text into lines at `\r\n`, `\n` and `\r` and addresses positions by byte
//! index *within* a line, excluding the line ending. This reproduces exactly that split.

use cosmic_text::Cursor;

#[derive(Debug, Clone)]
struct Line {
    /// Byte offset of the line's first character in the whole string.
    byte_start: usize,
    /// Length of the line's content in bytes (ending excluded).
    byte_len: usize,
    /// UTF-16 offset of the line's first character in the whole string.
    u16_start: usize,
    /// Length of the line's content in UTF-16 units (ending excluded).
    u16_len: usize,
    /// Length of the line ending in UTF-16 units (0 for the last line).
    ending_u16: usize,
}

#[derive(Debug, Clone)]
pub struct TextMap {
    lines: Vec<Line>,
}

impl TextMap {
    pub fn new(s: &str) -> TextMap {
        let bytes = s.as_bytes();
        let mut lines = Vec::new();
        let (mut byte_start, mut u16_start) = (0usize, 0usize);
        let mut i = 0usize;
        let mut u16_pos = 0usize;
        while i <= bytes.len() {
            let at_end = i == bytes.len();
            let (is_break, ending_bytes) = if at_end {
                (true, 0)
            } else if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                (true, 2)
            } else if bytes[i] == b'\n' || bytes[i] == b'\r' {
                (true, 1)
            } else {
                (false, 0)
            };
            if is_break {
                let content = &s[byte_start..i];
                let u16_len = content.encode_utf16().count();
                lines.push(Line {
                    byte_start,
                    byte_len: i - byte_start,
                    u16_start,
                    u16_len,
                    ending_u16: ending_bytes, // CR / LF / CRLF are 1 UTF-16 unit per byte
                });
                if at_end {
                    break;
                }
                u16_pos += u16_len + ending_bytes;
                i += ending_bytes;
                byte_start = i;
                u16_start = u16_pos;
                // A trailing line ending produces one more (empty) line, like the layout engine.
                continue;
            }
            // advance one char
            let ch_len = utf8_len(bytes[i]);
            i += ch_len;
        }
        if lines.is_empty() {
            lines.push(Line { byte_start: 0, byte_len: 0, u16_start: 0, u16_len: 0, ending_u16: 0 });
        }
        TextMap { lines }
    }

    #[cfg(test)]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Total length in UTF-16 units.
    pub fn len_u16(&self) -> usize {
        let l = self.lines.last().unwrap();
        l.u16_start + l.u16_len + l.ending_u16
    }

    /// UTF-16 offset -> layout cursor. Offsets inside a line ending clamp to the end of that line's
    /// content; offsets past the end clamp to the end of the text.
    pub fn cursor(&self, s: &str, pos_u16: usize) -> Cursor {
        let pos = pos_u16.min(self.len_u16());
        let idx = self
            .lines
            .partition_point(|l| l.u16_start + l.u16_len + l.ending_u16 <= pos)
            .min(self.lines.len() - 1);
        let line = &self.lines[idx];
        let rel = pos.saturating_sub(line.u16_start).min(line.u16_len);
        let content = &s[line.byte_start..line.byte_start + line.byte_len];
        let mut units = 0usize;
        let mut byte = content.len();
        for (b, c) in content.char_indices() {
            if units + c.len_utf16() > rel {
                byte = b; // the position is at, or inside, this character: snap to its start
                break;
            }
            units += c.len_utf16();
        }
        Cursor::new(idx, byte)
    }

    /// Layout cursor -> UTF-16 offset.
    pub fn u16_of(&self, s: &str, c: Cursor) -> usize {
        let line = &self.lines[c.line.min(self.lines.len() - 1)];
        let content = &s[line.byte_start..line.byte_start + line.byte_len];
        let upto = c.index.min(content.len());
        // c.index should be a char boundary; be defensive.
        let mut b = upto;
        while b > 0 && !content.is_char_boundary(b) {
            b -= 1;
        }
        line.u16_start + content[..b].encode_utf16().count()
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(s: &str) -> usize {
        s.encode_utf16().count()
    }

    #[test]
    fn single_line() {
        let s = "hello";
        let m = TextMap::new(s);
        assert_eq!(m.line_count(), 1);
        assert_eq!(m.len_u16(), 5);
        let c = m.cursor(s, 3);
        assert_eq!((c.line, c.index), (0, 3));
        assert_eq!(m.u16_of(s, c), 3);
    }

    #[test]
    fn empty_text_has_one_empty_line() {
        let m = TextMap::new("");
        assert_eq!(m.line_count(), 1);
        let c = m.cursor("", 0);
        assert_eq!((c.line, c.index), (0, 0));
    }

    #[test]
    fn crlf_is_one_ending() {
        let s = "ab\r\ncd";
        let m = TextMap::new(s);
        assert_eq!(m.line_count(), 2);
        // offset 4 is 'c' (a=0 b=1 \r=2 \n=3 c=4)
        let c = m.cursor(s, 4);
        assert_eq!((c.line, c.index), (1, 0));
        assert_eq!(m.u16_of(s, c), 4);
        // offset 2 (the \r) clamps to the end of line 0
        let c = m.cursor(s, 2);
        assert_eq!((c.line, c.index), (0, 2));
    }

    #[test]
    fn lone_cr_and_lf_split_lines() {
        let s = "a\nb\rc";
        let m = TextMap::new(s);
        assert_eq!(m.line_count(), 3);
        for (pos, line) in [(0, 0), (2, 1), (4, 2)] {
            assert_eq!(m.cursor(s, pos).line, line, "pos {pos}");
        }
    }

    #[test]
    fn trailing_newline_makes_an_empty_last_line() {
        let s = "x\n";
        let m = TextMap::new(s);
        assert_eq!(m.line_count(), 2);
        let c = m.cursor(s, 2);
        assert_eq!((c.line, c.index), (1, 0));
    }

    #[test]
    fn astral_characters_use_two_units_and_four_bytes() {
        let s = "a😀b";
        let m = TextMap::new(s);
        assert_eq!(m.len_u16(), 4);
        let c = m.cursor(s, 3); // before 'b'
        assert_eq!((c.line, c.index), (0, 5));
        assert_eq!(m.u16_of(s, c), 3);
        // a position inside the surrogate pair snaps back to the start of the emoji
        let c = m.cursor(s, 2);
        assert_eq!(c.index, 1);
    }

    #[test]
    fn roundtrip_over_every_boundary() {
        let s = "héllo wörld\r\n😀 next line\nlast\tline\r\n";
        let m = TextMap::new(s);
        let mut pos = 0usize;
        for ch in s.chars() {
            let c = m.cursor(s, pos);
            let back = m.u16_of(s, c);
            // positions inside line endings clamp to the end of the line's content
            assert!(back <= pos, "pos {pos} -> {back}");
            if ch != '\r' && ch != '\n' {
                assert_eq!(back, pos, "pos {pos}");
            }
            pos += u16s(&ch.to_string());
        }
    }

    #[test]
    fn positions_past_the_end_clamp() {
        let s = "abc";
        let m = TextMap::new(s);
        let c = m.cursor(s, 99);
        assert_eq!((c.line, c.index), (0, 3));
    }
}
