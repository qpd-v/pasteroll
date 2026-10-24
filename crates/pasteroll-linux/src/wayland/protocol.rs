//! Bounds and wire contracts shared by the real bridge and its native integration probe.
use pasteroll_app::{Captured, Event, Key, MouseEvent, MouseKind, Payload, Skip, Wheel};
use pasteroll_core::Step;
use zeroize::Zeroize;

pub const VERSION: u32 = 3;
pub const CLIPBOARD: u32 = 1;
pub const OVERLAY: u32 = 2;
pub const INPUT: u32 = 4;
pub const KEYBOARD_ACTIVATION: u32 = 8;
pub const MOUSE_ACTIVATION: u32 = 16;
pub const PASTE: u32 = 32;
pub const VIEWER: u32 = 64;
pub const SETTINGS: u32 = 128;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
// Complete viewer dimensions are bounded independently of each D-Bus message.
// At most one displayed and one pending tiled frame exist in the companion.
pub const MAX_VIEWER_BYTES: usize = 4096 * 4096 * 4;
pub const MAX_EVENTS: usize = 256;
pub const TEXT: &str = "text/plain;charset=utf-8";
pub const PNG: &str = "image/png";
pub const URIS: &str = "text/uri-list";
pub type WireEvent = (u32, u64, i32, i32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Missing,
    UntrustedPeer,
    Disconnected,
    Version,
    Bounds,
    Stale,
    Protocol,
    KeyboardOptInRequired,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "GNOME companion is not available",
            Self::UntrustedPeer => "GNOME companion identity could not be verified",
            Self::Disconnected => "GNOME companion disconnected or request timed out",
            Self::Version => "GNOME companion protocol is incompatible",
            Self::Bounds => "Wayland transfer exceeds the supported bounds",
            Self::Stale => "Wayland session or clipboard changed",
            Self::Protocol => "GNOME companion returned an invalid response",
            Self::KeyboardOptInRequired => "GNOME mouse activation needs the host helper; keyboard activation requires explicit opt-in",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy)]
pub struct Capabilities(pub u32);
impl Capabilities {
    pub fn negotiate(version: u32, bits: u32) -> Result<Self, Error> {
        if version != VERSION {
            return Err(Error::Version);
        }
        if bits & (CLIPBOARD | OVERLAY | INPUT) != CLIPBOARD | OVERLAY | INPUT {
            return Err(Error::Protocol);
        }
        Ok(Self(bits))
    }
    pub fn mouse_activation(self) -> bool {
        self.0 & MOUSE_ACTIVATION != 0
    }
    pub fn keyboard_activation(self) -> bool {
        self.0 & KEYBOARD_ACTIVATION != 0
    }
    pub fn paste(self) -> bool {
        self.0 & PASTE != 0
    }
    pub fn viewer(self) -> bool {
        self.0 & VIEWER != 0
    }
    pub fn settings(self) -> bool {
        self.0 & SETTINGS != 0
    }
    pub fn allow_interaction(self, keyboard_opt_in: bool) -> Result<(), Error> {
        if self.mouse_activation() || keyboard_opt_in && self.keyboard_activation() {
            Ok(())
        } else {
            Err(Error::KeyboardOptInRequired)
        }
    }
}

pub fn frame_len(width: u32, height: u32) -> Result<usize, Error> {
    let len = viewer_frame_len(width, height)?;
    if len > MAX_FRAME_BYTES {
        Err(Error::Bounds)
    } else {
        Ok(len)
    }
}

pub fn viewer_frame_len(width: u32, height: u32) -> Result<usize, Error> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(Error::Bounds);
    }
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(Error::Bounds)?;
    if len > MAX_VIEWER_BYTES {
        Err(Error::Bounds)
    } else {
        Ok(len)
    }
}

pub fn mime_for(types: &[String]) -> Result<&str, Skip> {
    if types.iter().any(|s| {
        matches!(
            s.as_str(),
            "x-kde-passwordManagerHint" | "application/x-keepassxc"
        )
    }) {
        return Err(Skip::Excluded);
    }
    [
        URIS,
        "x-special/gnome-copied-files",
        PNG,
        TEXT,
        "text/plain",
        "UTF8_STRING",
    ]
    .into_iter()
    .find(|mime| types.iter().any(|s| s == mime))
    .ok_or(Skip::Unsupported)
}

// Local file URIs only. Never turn an arbitrary URL into a stored file reference.
fn file_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest
        .strip_prefix("localhost/")
        .map(|s| format!("/{s}"))
        .unwrap_or_else(|| rest.to_owned());
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let digits = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(digits, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(decoded).ok()?;
    (!path.contains(['\0', '\r', '\n']) && path.starts_with('/')).then_some(path)
}

pub fn decode(mime: &str, mut bytes: Vec<u8>, limit: usize) -> Result<Captured, Skip> {
    if bytes.len() > limit.min(MAX_BYTES) {
        bytes.zeroize();
        return Err(Skip::TooLarge);
    }
    let result = match mime {
        TEXT | "text/plain" | "UTF8_STRING" => std::str::from_utf8(&bytes)
            .ok()
            .filter(|s| !s.is_empty() && !s.contains('\0'))
            .map(|s| Captured::Text(s.to_owned())),
        URIS | "x-special/gnome-copied-files" => {
            std::str::from_utf8(&bytes).ok().and_then(|text| {
                let paths: Vec<_> = text
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.starts_with('#'))
                    .filter_map(file_path)
                    .take(4097)
                    .collect();
                (!paths.is_empty() && paths.len() <= 4096).then_some(Captured::Files(paths))
            })
        }
        PNG => pasteroll_core::image::decode_png(&bytes)
            .map(|_| Captured::Image(std::mem::take(&mut bytes))),
        _ => None,
    };
    bytes.zeroize();
    result.ok_or(Skip::Unsupported)
}

pub fn encode(payload: &Payload) -> Result<(&'static str, Vec<u8>), Error> {
    let (mime, bytes) = match payload {
        Payload::Text(s) if !s.contains('\0') => (TEXT, s.as_bytes().to_vec()),
        Payload::Image(bytes) if pasteroll_core::image::decode_png(bytes).is_some() => {
            (PNG, bytes.clone())
        }
        Payload::Files(paths) if !paths.is_empty() && paths.len() <= 4096 => {
            let mut text = String::new();
            for path in paths {
                if !path.starts_with('/') || path.contains(['\0', '\r', '\n']) {
                    text.zeroize();
                    return Err(Error::Protocol);
                }
                text.push_str("file://");
                for b in path.bytes() {
                    if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
                        text.push(b as char);
                    } else {
                        use std::fmt::Write;
                        let _ = write!(text, "%{b:02X}");
                    }
                }
                text.push_str("\r\n");
            }
            (URIS, text.into_bytes())
        }
        _ => return Err(Error::Protocol),
    };
    if bytes.len() > MAX_BYTES {
        let mut bytes = bytes;
        bytes.zeroize();
        Err(Error::Bounds)
    } else {
        Ok((mime, bytes))
    }
}

pub fn event(
    (kind, generation, a, b, mods): WireEvent,
    active: u64,
) -> Result<Option<Event>, Error> {
    if kind == 0 {
        return Ok(Some(Event::Activate));
    }
    if generation != active || active == 0 {
        return Err(Error::Stale);
    }
    Ok(match kind {
        1 => Some(Event::Step(
            if a > 0 { Step::Older } else { Step::Newer },
            generation,
        )),
        2 => {
            let ctrl = mods & 4 != 0;
            let shift = mods & 1 != 0;
            let key = match a as u32 {
                0xff0d => Some(Key::Enter),
                0xff1b => Some(Key::Escape),
                0xff52 => Some(Key::Up),
                0xff54 => Some(Key::Down),
                0xff55 => Some(Key::PageUp),
                0xff56 => Some(Key::PageDown),
                0xff51 => Some(Key::Left),
                0xff53 => Some(Key::Right),
                0xff50 => Some(Key::Home),
                0xff57 => Some(Key::End),
                0xff08 => Some(if ctrl {
                    Key::CtrlBackspace
                } else {
                    Key::Backspace
                }),
                0xffff => Some(if ctrl {
                    Key::CtrlDelete
                } else if shift {
                    Key::ShiftDelete
                } else {
                    Key::Delete
                }),
                0xff09 => Some(Key::Tab),
                0xffc8 => Some(Key::F11),
                0xffc0 => Some(if shift { Key::ShiftF3 } else { Key::F3 }),
                value if ctrl => match value {
                    0x61 | 0x41 => Some(Key::CtrlA),
                    0x63 | 0x43 => Some(Key::CtrlC),
                    0x65 | 0x45 => Some(Key::CtrlE),
                    0x66 | 0x46 => Some(Key::CtrlF),
                    0x69 | 0x49 => Some(Key::CtrlI),
                    0x70 | 0x50 => Some(Key::CtrlP),
                    0x73 | 0x53 => Some(Key::CtrlS),
                    0x79 | 0x59 => Some(Key::CtrlY),
                    0x7a | 0x5a => Some(if shift { Key::CtrlY } else { Key::CtrlZ }),
                    _ => None,
                },
                _ if b > 0 => char::from_u32(b as u32)
                    .filter(|c| !c.is_control())
                    .map(Key::Char),
                _ => None,
            };
            key.map(|k| Event::Key(k, generation))
        }
        3 | 4 | 5 => Some(Event::Mouse(MouseEvent {
            kind: match kind {
                3 => MouseKind::Down,
                4 => MouseKind::Up,
                _ => MouseKind::Move,
            },
            x: a as f32,
            y: b as f32,
        })),
        6 => Some(Event::ClickPaste(generation)),
        7 => Some(Event::ClickCancel(generation)),
        8 => Some(Event::Wheel(
            Wheel {
                delta: a,
                horizontal: b != 0,
                ctrl: mods & 4 != 0,
                shift: mods & 1 != 0,
            },
            generation,
        )),
        9..=12 => Some(Event::ViewerMouse(MouseEvent {
            kind: match kind {
                9 => MouseKind::Down,
                10 => MouseKind::Up,
                11 => MouseKind::Move,
                _ => MouseKind::DoubleClick,
            },
            x: a as f32,
            y: b as f32,
        })),
        13 => Some(Event::ViewerMouse(MouseEvent {
            kind: MouseKind::Leave,
            x: a as f32,
            y: b as f32,
        })),
        14 => Some(Event::ClickOutside(generation)),
        // a right click on the list (the Copy menu)
        15 => Some(Event::Mouse(MouseEvent { kind: MouseKind::RightDown, x: a as f32, y: b as f32 })),
        _ => return Err(Error::Protocol),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_events_and_incompatible_peers_are_rejected() {
        assert!(matches!(
            Capabilities::negotiate(1, 15),
            Err(Error::Version)
        ));
        assert_eq!(
            Capabilities(15).allow_interaction(false),
            Err(Error::KeyboardOptInRequired)
        );
        assert!(Capabilities(15).allow_interaction(true).is_ok());
        assert_eq!(event((2, 4, 0xff0d, 0, 0), 5), Err(Error::Stale));
        assert_eq!(event((123, 5, 0, 0, 0), 5), Err(Error::Protocol));
        assert_eq!(event((6, 4, 0, 0, 0), 5), Err(Error::Stale));
        assert_eq!(event((14, 4, 0, 0, 0), 5), Err(Error::Stale));
    }
    #[test]
    fn malformed_and_overlarge_clipboards_are_not_ingested() {
        assert_eq!(
            mime_for(&[TEXT.into(), "x-kde-passwordManagerHint".into()]),
            Err(Skip::Excluded)
        );
        assert!(matches!(decode(TEXT, vec![255], 9), Err(Skip::Unsupported)));
        assert!(matches!(
            decode(TEXT, b"hello".to_vec(), 4),
            Err(Skip::TooLarge)
        ));
        assert!(matches!(
            decode(PNG, b"not a PNG".to_vec(), 99),
            Err(Skip::Unsupported)
        ));
    }
    #[test]
    fn file_lists_round_trip_and_reject_remote_or_injected_paths() {
        let payload = Payload::Files(vec![
            "/tmp/synthetic clip.png".into(),
            "/tmp/idea.txt".into(),
        ]);
        let (mime, bytes) = encode(&payload).unwrap();
        let Captured::Files(paths) = decode(mime, bytes, 4096).unwrap() else {
            panic!("not files")
        };
        assert_eq!(paths, vec!["/tmp/synthetic clip.png", "/tmp/idea.txt"]);
        assert!(matches!(
            decode(
                URIS,
                b"file://remote/private\r\nhttps://example.com/x".to_vec(),
                4096
            ),
            Err(Skip::Unsupported)
        ));
        assert!(encode(&Payload::Files(vec!["/tmp/a\r\nb".into()])).is_err());
        assert!(matches!(
            decode(URIS, b"file:///tmp/a%00b".to_vec(), 4096),
            Err(Skip::Unsupported)
        ));
    }
    #[test]
    fn image_and_frame_bounds_are_checked_before_transfer() {
        assert_eq!(frame_len(u32::MAX, 99), Err(Error::Bounds));
        assert_eq!(frame_len(0, 20), Err(Error::Bounds));
        assert_eq!(frame_len(64, 48), Ok(12288));
        let img = pasteroll_core::image::Rgba {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 255],
        };
        let bytes = pasteroll_core::image::encode_png(&img).unwrap();
        assert!(matches!(decode(PNG, bytes, 4096), Ok(Captured::Image(_))));
    }
    #[test]
    fn input_keeps_session_generation_and_non_latin_characters() {
        assert_eq!(
            event((2, 3, 0, 0x4e2d, 0), 3),
            Ok(Some(Event::Key(Key::Char('中'), 3)))
        );
        assert_eq!(
            event((2, 3, 0x7a, 0, 5), 3),
            Ok(Some(Event::Key(Key::CtrlY, 3)))
        );
    }

    #[test]
    fn phase_three_capabilities_and_viewer_events_are_opt_in_and_generation_bound() {
        let bits = CLIPBOARD | OVERLAY | INPUT | KEYBOARD_ACTIVATION | PASTE | VIEWER | SETTINGS;
        let caps = Capabilities::negotiate(VERSION, bits).unwrap();
        assert!(caps.paste() && caps.viewer() && caps.settings());
        assert!(!caps.mouse_activation());
        assert_eq!(
            event((9, 7, 12, 34, 0), 7),
            Ok(Some(Event::ViewerMouse(MouseEvent {
                kind: MouseKind::Down,
                x: 12.0,
                y: 34.0,
            })))
        );
        assert_eq!(
            event((12, 7, 1, 2, 0), 7),
            Ok(Some(Event::ViewerMouse(MouseEvent {
                kind: MouseKind::DoubleClick,
                x: 1.0,
                y: 2.0,
            })))
        );
        assert_eq!(
            event((13, 7, 0, 0, 0), 7),
            Ok(Some(Event::ViewerMouse(MouseEvent {
                kind: MouseKind::Leave,
                x: 0.0,
                y: 0.0,
            })))
        );
        assert_eq!(event((14, 7, 0, 0, 0), 7), Ok(Some(Event::ClickOutside(7))));
        assert_eq!(event((20, 7, 0, 0, 0), 7), Err(Error::Protocol));
        assert_eq!(event((25, 7, 0, 0, 0), 7), Err(Error::Protocol));
    }
}
