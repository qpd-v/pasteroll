//! What a platform backend must provide. Everything the controller needs from the operating system
//! goes through [`Platform`]; nothing in this crate calls an OS API.

use crate::pastekeys::PasteKey;
use pasteroll_render::Frame;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A window (or application) a paste can go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Opaque platform window identifier.
    pub id: u64,
    pub pid: u32,
    /// Lower-case executable name, for the per-app paste key table ("windowsterminal.exe").
    pub exe: String,
    /// The window's class name where the OS has one (Windows): consoles report their *client*
    /// process as the executable, so the class is what identifies a terminal window reliably.
    pub class: String,
    /// The desktop: browsing is allowed, Enter never pastes there, a click saves an image.
    pub desktop: bool,
}

/// What gets written to the system clipboard before the paste keystroke.
#[derive(Clone, PartialEq)]
pub enum Payload {
    Text(String),
    Files(Vec<String>),
    /// PNG bytes; the platform decodes and offers its native image formats.
    Image(Vec<u8>),
}

impl Payload {
    /// Overwrites the clip content in place.
    pub fn wipe(&mut self) {
        use zeroize::Zeroize;
        match self {
            Payload::Text(t) => t.zeroize(),
            Payload::Files(f) => f.iter_mut().for_each(|s| s.zeroize()),
            Payload::Image(b) => b.zeroize(),
        }
    }
}

impl std::fmt::Debug for Payload {
    // Never prints clip content.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Payload::Text(t) => write!(f, "Text({} chars)", t.chars().count()),
            Payload::Files(v) => write!(f, "Files({})", v.len()),
            Payload::Image(b) => write!(f, "Image({} bytes)", b.len()),
        }
    }
}

/// Outcome of checking a paste-at-click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClickPoll {
    /// Not settled yet (the click is still activating the window, or a key is held).
    Pending,
    /// Not a place to paste (title bar, scrollbar, button, taskbar...). The session is cancelled.
    Rejected(&'static str),
    /// The click landed on the desktop.
    Desktop,
    /// The click landed in an application; paste there.
    Target(Target),
}

/// Geometry the overlay was actually shown with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayInfo {
    /// Device pixels per DIP.
    pub scale: f32,
    /// Window size in device pixels (may be smaller than requested on a small display).
    pub size_px: (u32, u32),
}

/// Result of a clipboard capture. Not `Debug`-printable with content.
pub enum Captured {
    Text(String),
    Files(Vec<String>),
    /// PNG bytes (the platform converts from its native image format).
    Image(Vec<u8>),
}

impl std::fmt::Debug for Captured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Captured::Text(t) => write!(f, "Text({} chars)", t.chars().count()),
            Captured::Files(v) => write!(f, "Files({})", v.len()),
            Captured::Image(b) => write!(f, "Image({} bytes)", b.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Marked do-not-record by the writer.
    Excluded,
    /// Nothing we store.
    Unsupported,
    /// Clipboard busy, or it changed while being read.
    Busy,
    /// Over the size cap.
    TooLarge,
}

/// Reads the system clipboard. Lives on its own thread, hence `Send`.
pub trait ClipboardSource: Send {
    /// A number that changes whenever the clipboard content does.
    fn sequence(&mut self) -> u64;
    fn capture(&mut self) -> Result<Captured, Skip>;
    /// The program that most likely produced the current clipboard content (lower-case exe name),
    /// for the per-app capture exclusion list. `None` when it cannot be determined.
    fn source_app(&mut self) -> Option<String> {
        None
    }
}

/// Wakes the platform's event loop so it delivers [`crate::Event::Wake`].
pub trait Waker: Send + Sync {
    fn wake(&self);
}

pub trait Platform {
    // ---------------------------------------------------------------- targets
    /// The application that has the foreground now, or `None` when there is no suitable target
    /// (our own windows, the taskbar, no foreground at all).
    fn foreground(&mut self) -> Option<Target>;
    /// Is `target` still the foreground, in the same process?
    fn target_is_current(&mut self, target: &Target) -> bool;
    /// Does the target run at a higher privilege than we do, so our keystrokes would be blocked?
    fn target_privileged(&mut self, target: &Target) -> bool;

    // ---------------------------------------------------------------- overlay
    fn cursor(&self) -> (i32, i32);
    /// Shows the overlay near `at` without taking focus and reports the geometry it got.
    fn overlay_show(&mut self, at: (i32, i32)) -> OverlayInfo;
    fn overlay_present(&mut self, frame: &Frame);
    fn overlay_hide(&mut self);

    // ---------------------------------------------------------------- input
    /// Start/stop swallowing the wheel and the session keys. Starting a session bumps the
    /// generation returned by [`Platform::input_generation`].
    fn input_active(&mut self, active: bool);
    fn input_generation(&self) -> u64;
    /// Called every tick: proof the UI thread is alive (the input hooks stop swallowing otherwise).
    fn heartbeat(&mut self);
    /// Wait for Alt/Win to be released (a held one would turn the paste chord into something else).
    fn modifiers_released(&mut self, timeout: Duration) -> bool;
    /// Check a paste-at-click. `first` is true on the first poll after the click (reset state).
    fn poll_click(&mut self, first: bool) -> ClickPoll;

    // ---------------------------------------------------------------- paste
    fn write_clipboard(&mut self, payload: &Payload) -> Result<(), String>;
    /// Sends the paste keystroke. `true` only if the system accepted every event.
    fn send_paste(&mut self, key: PasteKey) -> bool;

    // ---------------------------------------------------------------- environment
    fn desktop_folder(&self) -> Option<PathBuf>;
    /// "YYYY-MM-DD HH-MM-SS" in local time, for file names.
    fn local_timestamp(&self) -> String;
    fn file_created(&mut self, path: &Path);
    fn now_secs(&self) -> i64;
    /// Monotonic milliseconds.
    fn now_ms(&self) -> u64;
    fn log(&self, msg: &str);

    // ---------------------------------------------------------------- settings
    /// Saves a new custom filter to the user's config and returns the full list of custom
    /// filters afterwards. The error text is shown to the user.
    fn add_custom_filter(
        &mut self,
        name: &str,
        extensions: &[String],
    ) -> Result<Vec<pasteroll_core::config::CustomFilter>, String> {
        let _ = (name, extensions);
        Err("Custom filters cannot be saved here.".to_owned())
    }
}
