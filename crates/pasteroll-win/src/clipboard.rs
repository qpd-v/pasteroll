//! System clipboard access: capture on change, and write for paste.
//!
//! Capture honours the opt-outs Windows documents for clipboard managers
//! (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory == 0`) plus the
//! long-standing "Clipboard Viewer Ignore" marker, so password managers and our own pastes are not
//! recorded. Writes carry the same markers so other clipboard managers on the machine skip them.
//!
//! The clipboard is a shared, contended resource: another process can hold it open, so every open
//! is retried briefly.

use std::thread::sleep;
use std::time::Duration;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use pasteroll_core::hdrop;
use pasteroll_core::image::{self, Rgba};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};

pub struct Formats {
    pub ignore: u32,
    pub exclude: u32,
    pub can_include: u32,
    pub can_upload: u32,
    pub png: u32,
}

impl Formats {
    pub fn register() -> Formats {
        unsafe {
            Formats {
                ignore: RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
                exclude: RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")),
                can_include: RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
                can_upload: RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")),
                png: RegisterClipboardFormatW(w!("PNG")),
            }
        }
    }
}

pub use pasteroll_app::{Captured, Skip};

/// Largest single clip we will store (same limit as the store applies to the stored body).
/// Default per-clip limit; the configured limit is passed to `capture`.
pub const MAX_CLIP_BYTES: usize = pasteroll_core::limits::DEFAULT_CLIP_BYTES as usize;

/// Raw-size caps per clipboard format, checked before anything is copied. Text arrives as UTF-16
/// (twice the UTF-8 size the store limits); a DIB is uncompressed pixels, so its cap follows the
/// largest image we will decode.
const DIB_CAP: usize = pasteroll_core::image::MAX_PIXELS as usize * 4 + 1024;

struct Open;

impl Open {
    fn acquire(owner: Option<HWND>) -> Option<Open> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Some(Open);
            }
            sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn available(fmt: u32) -> bool {
    unsafe { IsClipboardFormatAvailable(fmt).is_ok() }
}

/// Copies a global-memory clipboard item, refusing anything over `cap` bytes. `Err(())` = too
/// large; `Ok(None)` = could not be locked.
fn global_bytes(h: HANDLE, cap: usize) -> std::result::Result<Option<Vec<u8>>, ()> {
    unsafe {
        let g = HGLOBAL(h.0);
        let size = GlobalSize(g);
        if size > cap {
            return Err(());
        }
        let p = GlobalLock(g) as *const u8;
        if p.is_null() {
            return Ok(None);
        }
        let v = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(g);
        Ok(Some(v))
    }
}

/// Bytes taken from the clipboard while it was open. Nothing is parsed or converted yet.
enum Raw {
    Text(Vec<u8>),
    Files(Vec<u8>),
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

/// Phase 1, with the clipboard open: check the do-not-record markers, copy the bytes of the one
/// format we will keep, and confirm nothing changed meanwhile. The clipboard is closed on return, so
/// slow work (PNG validation, DIB conversion) never blocks other programs' copies or our own paste.
///
/// The clipboard is opened *first* and held while the markers are checked and the data is read:
/// checking markers before opening would leave a window in which a password manager could replace
/// the content between the check and the read. A marker that is present but unreadable is treated
/// as "exclude".
fn grab(f: &Formats, max_clip: usize) -> std::result::Result<Raw, Skip> {
    // Text arrives as UTF-16 (twice the UTF-8 size the store limits).
    let text_cap = 2 * max_clip + 2;
    let _open = Open::acquire(None).ok_or(Skip::Busy)?;
    let seq = sequence();
    if available(f.ignore) || available(f.exclude) {
        return Err(Skip::Excluded);
    }
    unsafe {
        if available(f.can_include) {
            let h = GetClipboardData(f.can_include).map_err(|_| Skip::Excluded)?;
            match global_bytes(h, 64) {
                Ok(Some(b)) if b.len() >= 4 => {
                    if u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0 {
                        return Err(Skip::Excluded);
                    }
                }
                _ => return Err(Skip::Excluded),
            }
        }
        let pick = |fmt: u32, cap: usize| -> std::result::Result<Option<Vec<u8>>, Skip> {
            match GetClipboardData(fmt) {
                Ok(h) => global_bytes(h, cap).map_err(|_| Skip::TooLarge),
                Err(_) => Ok(None),
            }
        };
        let raw = if available(CF_UNICODETEXT.0 as u32) {
            pick(CF_UNICODETEXT.0 as u32, text_cap)?.map(Raw::Text)
        } else if available(CF_HDROP.0 as u32) {
            pick(CF_HDROP.0 as u32, hdrop::MAX_RAW_BYTES)?.map(Raw::Files)
        } else if available(f.png) {
            pick(f.png, max_clip)?.map(Raw::Png)
        } else if available(CF_DIBV5.0 as u32) {
            pick(CF_DIBV5.0 as u32, DIB_CAP)?.map(Raw::Dib)
        } else if available(CF_DIB.0 as u32) {
            pick(CF_DIB.0 as u32, DIB_CAP)?.map(Raw::Dib)
        } else {
            None
        };
        if sequence() != seq {
            return Err(Skip::Busy); // changed under us: discard rather than store a torn read
        }
        raw.ok_or(Skip::Unsupported)
    }
}

/// Reads the current clipboard into what the store keeps.
pub fn capture(f: &Formats, max_clip: usize) -> std::result::Result<Captured, Skip> {
    // Phase 2 runs with the clipboard closed.
    match grab(f, max_clip)? {
        Raw::Text(mut b) => {
            let mut units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            let text = String::from_utf16_lossy(&units[..end]);
            // the raw copies of the clipboard text are not needed any more
            zeroize::Zeroize::zeroize(&mut b);
            zeroize::Zeroize::zeroize(&mut units);
            if text.is_empty() {
                Err(Skip::Unsupported)
            } else {
                Ok(Captured::Text(text))
            }
        }
        Raw::Files(b) => hdrop::parse(&b).map(Captured::Files).ok_or(Skip::Unsupported),
        Raw::Png(b) => {
            if image::decode_png(&b).is_some() {
                Ok(Captured::Image(b))
            } else {
                Err(Skip::Unsupported)
            }
        }
        Raw::Dib(b) => {
            let png = image::dib_to_rgba(&b)
                .and_then(|i| image::encode_png(&i))
                .ok_or(Skip::Unsupported)?;
            if png.len() > max_clip {
                Err(Skip::TooLarge)
            } else {
                Ok(Captured::Image(png))
            }
        }
    }
}

fn put(format: u32, bytes: &[u8]) -> Result<()> {
    unsafe {
        let g = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let p = GlobalLock(g) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(g));
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(g);
        // On success the clipboard owns the memory; on failure we must free it.
        if let Err(e) = SetClipboardData(format, Some(HANDLE(g.0))) {
            let _ = GlobalFree(Some(g));
            return Err(e);
        }
        Ok(())
    }
}

/// Opens the clipboard, empties it, writes the "do not record" markers FIRST (so a partial failure
/// can never leave unmarked data for another clipboard manager, Windows history or cloud sync to
/// pick up), then runs `payload`. Any failure empties the clipboard again.
fn write_marked(owner: HWND, f: &Formats, payload: impl FnOnce() -> Result<()>) -> Result<()> {
    let _open = Open::acquire(Some(owner))
        .ok_or_else(|| windows::core::Error::new(E_FAIL, "clipboard is busy"))?;
    unsafe {
        EmptyClipboard()?;
    }
    let r = (|| {
        put(f.ignore, b"Ignore\0")?;
        put(f.exclude, &1u32.to_le_bytes())?;
        put(f.can_include, &0u32.to_le_bytes())?;
        put(f.can_upload, &0u32.to_le_bytes())?;
        payload()
    })();
    if r.is_err() {
        unsafe {
            let _ = EmptyClipboard();
        }
    }
    r
}

fn text_payload(text: &str) -> Result<()> {
    let mut wide: Vec<u8> = Vec::with_capacity((text.len() + 1) * 2);
    for u in text.encode_utf16().chain(std::iter::once(0)) {
        wide.extend_from_slice(&u.to_le_bytes());
    }
    put(CF_UNICODETEXT.0 as u32, &wide)
}

/// An image as both registered "PNG" (lossless, alpha) and `CF_DIB` (what older programs read).
fn image_payload(f: &Formats, png: &[u8], rgba: &Rgba) -> Result<()> {
    put(f.png, png)?;
    put(CF_DIB.0 as u32, &image::rgba_to_dib(rgba))
}

/// A file list (`CF_HDROP`): a `DROPFILES` header, then NUL-separated wide paths and a final NUL.
fn files_payload(paths: &[String]) -> Result<()> {
    put(CF_HDROP.0 as u32, &hdrop::build(paths))
}

pub fn set_text(owner: HWND, f: &Formats, text: &str) -> Result<()> {
    write_marked(owner, f, || text_payload(text))
}

pub fn set_image(owner: HWND, f: &Formats, png: &[u8], rgba: &Rgba) -> Result<()> {
    write_marked(owner, f, || image_payload(f, png, rgba))
}

pub fn set_files(owner: HWND, f: &Formats, paths: &[String]) -> Result<()> {
    write_marked(owner, f, || files_payload(paths))
}

// ---- self-test support: simulate "another app copied something" (no do-not-record markers) ----

pub fn write_unmarked(owner: HWND, payload: impl FnOnce() -> Result<()>) -> Result<()> {
    let _open = Open::acquire(Some(owner))
        .ok_or_else(|| windows::core::Error::new(E_FAIL, "clipboard is busy"))?;
    unsafe {
        EmptyClipboard()?;
    }
    payload()
}

pub fn put_raw(format: u32, bytes: &[u8]) -> Result<()> {
    put(format, bytes)
}

pub fn put_text_raw(text: &str) -> Result<()> {
    text_payload(text)
}

pub fn put_files_raw(paths: &[String]) -> Result<()> {
    files_payload(paths)
}

/// Which of our payload formats are on the clipboard right now (for the self-test).
pub fn has_format(format: u32) -> bool {
    available(format)
}

pub fn dib_format() -> u32 {
    CF_DIB.0 as u32
}

pub fn clear(owner: HWND) {
    if let Some(_o) = Open::acquire(Some(owner)) {
        unsafe {
            let _ = EmptyClipboard();
        }
    }
}

/// `AddClipboardFormatListener` wrapper.
pub fn listen(hwnd: HWND) -> Result<()> {
    unsafe { AddClipboardFormatListener(hwnd) }
}

pub fn unlisten(hwnd: HWND) {
    unsafe {
        let _ = RemoveClipboardFormatListener(hwnd);
    }
}

pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

#[allow(dead_code)]
fn _keep(_: PCWSTR) {}

/// The system clipboard as a capture source (used on the ingest worker thread).
pub struct WinClipboard {
    formats: Formats,
    max_clip_bytes: usize,
}

impl WinClipboard {
    pub fn new(max_clip_bytes: usize) -> WinClipboard {
        WinClipboard { formats: Formats::register(), max_clip_bytes }
    }
}

impl pasteroll_app::ClipboardSource for WinClipboard {
    fn sequence(&mut self) -> u64 {
        u64::from(sequence())
    }

    fn capture(&mut self) -> std::result::Result<Captured, Skip> {
        capture(&self.formats, self.max_clip_bytes)
    }

    /// The program that owns the clipboard right now, by executable name.
    fn source_app(&mut self) -> Option<String> {
        unsafe {
            let owner = GetClipboardOwner().ok()?;
            let mut pid = 0u32;
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(owner, Some(&mut pid));
            if pid == 0 {
                return None;
            }
            crate::winsys::exe_name(pid)
        }
    }
}
