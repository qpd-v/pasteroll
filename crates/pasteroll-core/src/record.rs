//! The small, searchable description kept (sealed) beside every clip, so the carousel can draw rows
//! and run searches without decrypting multi-megabyte bodies.
//!
//! Format: `version(1) | total_chars u32 | width u32 | height u32 | files u32 | utf-8 text`, all
//! integers little-endian. `text` is what search matches against: the first [`SEARCH_CHARS`]
//! characters of a text clip, the file names and paths of a files clip, or a short description of an
//! image.

pub const SEARCH_CHARS: usize = 2_000;
const VERSION: u8 = 1;
const HEADER: usize = 1 + 4 * 4;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    pub total_chars: u32,
    pub width: u32,
    pub height: u32,
    pub files: u32,
    pub text: String,
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

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(HEADER + self.text.len());
        v.push(VERSION);
        for n in [self.total_chars, self.width, self.height, self.files] {
            v.extend_from_slice(&n.to_le_bytes());
        }
        v.extend_from_slice(self.text.as_bytes());
        v
    }

    pub fn decode(b: &[u8]) -> Option<Record> {
        if b.len() < HEADER || b[0] != VERSION {
            return None;
        }
        let n = |i: usize| u32::from_le_bytes(b[1 + i * 4..5 + i * 4].try_into().unwrap());
        Some(Record {
            total_chars: n(0),
            width: n(1),
            height: n(2),
            files: n(3),
            text: String::from_utf8_lossy(&b[HEADER..]).into_owned(),
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
        let r = Record { total_chars: 12, width: 3, height: 4, files: 5, text: "héllo ✓".into() };
        assert_eq!(Record::decode(&r.encode()), Some(r));
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
        assert_eq!(Record::decode(&[2; 40]), None);
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
