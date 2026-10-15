//! Parsing a `CF_HDROP` file list that another process put on the clipboard.
//!
//! The data is attacker-influenced, so it is parsed here with explicit bounds rather than handed to
//! the shell's `DragQueryFile`. Layout: a 20-byte `DROPFILES` header
//! (`pFiles`, `pt.x`, `pt.y`, `fNC`, `fWide`), then NUL-separated paths ending with an empty one.

/// Most files we keep from one clip, and the longest path (in characters).
pub const MAX_FILES: usize = 4096;
pub const MAX_PATH_CHARS: usize = 32_768;
/// Raw bytes of a file list we are willing to look at.
pub const MAX_RAW_BYTES: usize = 4 * 1024 * 1024;

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// `None` if the header is malformed. Entries past the limits are dropped, not an error.
pub fn parse(raw: &[u8]) -> Option<Vec<String>> {
    if raw.len() < 20 || raw.len() > MAX_RAW_BYTES {
        return None;
    }
    let offset = u32le(raw, 0)? as usize;
    let wide = u32le(raw, 16)? != 0;
    if offset < 20 || offset > raw.len() {
        return None;
    }
    let list = &raw[offset..];
    let mut files = Vec::new();
    if wide {
        let units: Vec<u16> = list.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        for s in units.split(|&u| u == 0) {
            if s.is_empty() {
                break; // the terminating empty string
            }
            if files.len() >= MAX_FILES {
                break;
            }
            let s = &s[..s.len().min(MAX_PATH_CHARS)];
            files.push(String::from_utf16_lossy(s));
        }
    } else {
        for s in list.split(|&b| b == 0) {
            if s.is_empty() {
                break;
            }
            if files.len() >= MAX_FILES {
                break;
            }
            let s = &s[..s.len().min(MAX_PATH_CHARS)];
            files.push(String::from_utf8_lossy(s).into_owned());
        }
    }
    (!files.is_empty()).then_some(files)
}

/// The inverse: what we put on the clipboard to paste files.
pub fn build(paths: &[String]) -> Vec<u8> {
    let mut data: Vec<u8> = Vec::new();
    data.extend_from_slice(&20u32.to_le_bytes()); // pFiles
    data.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    data.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    data.extend_from_slice(&0u32.to_le_bytes()); // fNC
    data.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for p in paths {
        for u in p.encode_utf16().chain(std::iter::once(0)) {
            data.extend_from_slice(&u.to_le_bytes());
        }
    }
    data.extend_from_slice(&0u16.to_le_bytes());
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_what_we_build() {
        let paths = vec!["C:\\a.txt".to_string(), "D:\\my docs\\ü ñ 😀.txt".to_string()];
        assert_eq!(parse(&build(&paths)).unwrap(), paths);
    }

    #[test]
    fn parses_ansi_lists() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&20u32.to_le_bytes());
        raw.extend_from_slice(&[0u8; 12]);
        raw.extend_from_slice(&0u32.to_le_bytes()); // fWide = 0
        raw.extend_from_slice(b"C:\\one.txt\0C:\\two.txt\0\0");
        assert_eq!(parse(&raw).unwrap(), vec!["C:\\one.txt", "C:\\two.txt"]);
    }

    #[test]
    fn rejects_malformed_headers() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0; 19]).is_none());
        let mut raw = build(&["x".to_string()]);
        raw[0..4].copy_from_slice(&10u32.to_le_bytes()); // offset inside the header
        assert!(parse(&raw).is_none());
        let mut raw = build(&["x".to_string()]);
        raw[0..4].copy_from_slice(&u32::MAX.to_le_bytes()); // offset beyond the data
        assert!(parse(&raw).is_none());
    }

    #[test]
    fn a_list_with_no_terminator_still_parses_what_is_there() {
        let mut raw = build(&["C:\\a".to_string()]);
        raw.truncate(raw.len() - 2); // drop the final empty string
        assert_eq!(parse(&raw).unwrap(), vec!["C:\\a"]);
    }

    #[test]
    fn counts_and_lengths_are_bounded() {
        let many: Vec<String> = (0..MAX_FILES + 50).map(|i| format!("C:\\f{i}")).collect();
        assert_eq!(parse(&build(&many)).unwrap().len(), MAX_FILES);
        let long = "x".repeat(MAX_PATH_CHARS + 500);
        assert_eq!(parse(&build(&[long])).unwrap()[0].len(), MAX_PATH_CHARS);
    }

    #[test]
    fn oversized_input_is_refused() {
        let raw = vec![0u8; MAX_RAW_BYTES + 1];
        assert!(parse(&raw).is_none());
    }

    #[test]
    fn lone_surrogates_do_not_panic() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&20u32.to_le_bytes());
        raw.extend_from_slice(&[0u8; 12]);
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&0xD800u16.to_le_bytes());
        raw.extend_from_slice(&[0, 0, 0, 0]);
        assert!(parse(&raw).is_some());
    }
}
