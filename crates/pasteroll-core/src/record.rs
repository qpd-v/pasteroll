//! The small, searchable description kept (sealed) beside every clip, so the carousel can draw rows
//! and run searches without decrypting multi-megabyte bodies.
//!
//! Format (version 2): `version(1) | total_chars u32 | width u32 | height u32 | files u32 |
//! copied_at i64 | source_len u16 | source utf-8 | utf-8 text`, all integers little-endian. Version 1
//! records (no `copied_at`, no `source`) are still read. `text` is what search matches against: the
//! first [`SEARCH_CHARS`] characters of a text clip, the file names and paths of a files clip, or a
//! short description of an image. `copied_at` (Unix seconds) and `source` (the program the copy
//! came from, when known) are shown in the viewer's Info panel; like the rest of the record they are
//! sealed, never stored in the clear.

pub const SEARCH_CHARS: usize = 2_000;
const VERSION: u8 = 2;
const HEADER_V1: usize = 1 + 4 * 4;
const HEADER: usize = HEADER_V1 + 8 + 2;
/// Longest program name kept, in bytes.
const SOURCE_MAX: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    pub total_chars: u32,
    pub width: u32,
    pub height: u32,
    pub files: u32,
    /// When the clip was (last) copied, Unix seconds; 0 when unknown (older records).
    pub copied_at: i64,
    /// The program the copy came from ("chrome.exe"); empty when unknown.
    pub source: String,
    pub text: String,
}

/// At most `max` bytes of `s`, cut at a character boundary.
fn cut_bytes(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

fn cut(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => s[..i].to_owned(),
        None => s.to_owned(),
    }
}

impl Record {
    pub fn for_text(text: &str) -> Record {
        Record {
            total_chars: text.chars().count().min(u32::MAX as usize) as u32,
            text: cut(text, SEARCH_CHARS),
            ..Record::default()
        }
    }

    pub fn for_files(paths: &[String]) -> Record {
        Record {
            files: paths.len().min(u32::MAX as usize) as u32,
            total_chars: paths.iter().map(|p| p.chars().count()).sum::<usize>().min(u32::MAX as usize) as u32,
            text: cut(&paths.join("\n"), SEARCH_CHARS),
            ..Record::default()
        }
    }

    pub fn for_image(width: u32, height: u32) -> Record {
        Record { width, height, text: format!("image {width}x{height}"), ..Record::default() }
    }

    /// The same record, saying when and where the copy was made.
    pub fn with_origin(mut self, copied_at: i64, source: Option<&str>) -> Record {
        self.copied_at = copied_at;
        self.source = cut_bytes(source.unwrap_or(""), SOURCE_MAX);
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let source = cut_bytes(&self.source, SOURCE_MAX);
        let mut v = Vec::with_capacity(HEADER + source.len() + self.text.len());
        v.push(VERSION);
        for n in [self.total_chars, self.width, self.height, self.files] {
            v.extend_from_slice(&n.to_le_bytes());
        }
        v.extend_from_slice(&self.copied_at.to_le_bytes());
        v.extend_from_slice(&(source.len() as u16).to_le_bytes());
        v.extend_from_slice(source.as_bytes());
        v.extend_from_slice(self.text.as_bytes());
        v
    }

    pub fn decode(b: &[u8]) -> Option<Record> {
        if b.len() < HEADER_V1 || !(1..=VERSION).contains(&b[0]) {
            return None;
        }
        let n = |i: usize| u32::from_le_bytes(b[1 + i * 4..5 + i * 4].try_into().unwrap());
        let (copied_at, source, text_at) = if b[0] == 1 {
            (0, String::new(), HEADER_V1)
        } else {
            if b.len() < HEADER {
                return None;
            }
            let at = i64::from_le_bytes(b[HEADER_V1..HEADER_V1 + 8].try_into().unwrap());
            let len = usize::from(u16::from_le_bytes(b[HEADER_V1 + 8..HEADER].try_into().unwrap()));
            if b.len() < HEADER + len {
                return None;
            }
            (at, String::from_utf8_lossy(&b[HEADER..HEADER + len]).into_owned(), HEADER + len)
        };
        Some(Record {
            total_chars: n(0),
            width: n(1),
            height: n(2),
            files: n(3),
            copied_at,
            source,
            text: String::from_utf8_lossy(&b[text_at..]).into_owned(),
        })
    }
}

/// Width and height from a PNG's IHDR chunk, without decoding the image.
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if png.len() < 24 || png[..8] != SIG || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let r = Record {
            total_chars: 12,
            width: 3,
            height: 4,
            files: 5,
            copied_at: 1_700_000_123,
            source: "chrome.exe".into(),
            text: "héllo ✓".into(),
        };
        assert_eq!(Record::decode(&r.encode()), Some(r));
    }

    #[test]
    fn version_1_records_are_still_read() {
        let mut v = vec![1u8];
        for n in [7u32, 0, 0, 0] {
            v.extend_from_slice(&n.to_le_bytes());
        }
        v.extend_from_slice("old clip".as_bytes());
        let r = Record::decode(&v).unwrap();
        assert_eq!((r.total_chars, r.copied_at, r.source.as_str(), r.text.as_str()), (7, 0, "", "old clip"));
    }

    #[test]
    fn the_origin_is_kept_and_a_long_name_is_cut() {
        let r = Record::for_text("x").with_origin(42, Some(&"é".repeat(300)));
        assert!(r.source.len() <= 255 && r.source.chars().all(|c| c == 'é'));
        let back = Record::decode(&r.encode()).unwrap();
        assert_eq!((back.copied_at, back.source.len()), (42, r.source.len()));
        // a source length pointing past the end is corrupt
        let mut bad = r.encode();
        bad.truncate(HEADER + 3);
        assert_eq!(Record::decode(&bad), None);
    }

    #[test]
    fn text_is_cut_at_the_search_limit_but_the_total_is_kept() {
        let long = "ab".repeat(5_000);
        let r = Record::for_text(&long);
        assert_eq!(r.total_chars, 10_000);
        assert_eq!(r.text.chars().count(), SEARCH_CHARS);
    }

    #[test]
    fn cutting_respects_character_boundaries() {
        let s = "😀".repeat(SEARCH_CHARS + 5);
        let r = Record::for_text(&s);
        assert_eq!(r.text.chars().count(), SEARCH_CHARS);
        assert!(r.text.chars().all(|c| c == '😀'));
    }

    #[test]
    fn files_keep_every_name_searchable() {
        let r = Record::for_files(&["C:\\docs\\invoice.pdf".into(), "C:\\docs\\notes.txt".into()]);
        assert_eq!(r.files, 2);
        assert!(r.text.contains("invoice.pdf") && r.text.contains("notes.txt"));
    }

    #[test]
    fn corrupt_records_are_rejected() {
        assert_eq!(Record::decode(&[]), None);
        assert_eq!(Record::decode(&[3; 40]), None);
        assert_eq!(Record::decode(&[1, 0, 0]), None);
    }

    #[test]
    fn png_dimensions_read_the_header() {
        let img = crate::image::Rgba { width: 37, height: 11, pixels: vec![0; 37 * 11 * 4] };
        let png = crate::image::encode_png(&img).unwrap();
        assert_eq!(png_dimensions(&png), Some((37, 11)));
        assert_eq!(png_dimensions(b"not a png at all, really not....."), None);
        assert_eq!(png_dimensions(&png[..10]), None);
    }
}
