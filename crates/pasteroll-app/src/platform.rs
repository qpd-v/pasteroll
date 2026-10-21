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
    /// Screen position of the window's top-left corner.
    pub origin_px: (i32, i32),
    /// The monitor it is on, `(x, y, w, h)` in screen pixels, and the part of it not taken by
    /// task bars and panels. The expanded viewer opens on this monitor.
    pub monitor_px: (i32, i32, u32, u32),
    pub work_px: (i32, i32, u32, u32),
}

impl OverlayInfo {
    /// A plain 1920 x 1080 monitor (until the overlay has been shown, and in tests).
    pub fn new(scale: f32, size_px: (u32, u32)) -> OverlayInfo {
        OverlayInfo {
            scale,
            size_px,
            origin_px: (0, 0),
            monitor_px: (0, 0, 1920, 1080),
            work_px: (0, 0, 1920, 1040),
        }
    }
}

/// Which of Ctrl, Shift and Alt were held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Modifiers {
    pub fn has(&self, m: pasteroll_core::config::ClickModifier) -> bool {
        use pasteroll_core::config::ClickModifier;
        match m {
            ClickModifier::Ctrl => self.ctrl,
            ClickModifier::Shift => self.shift,
            ClickModifier::Alt => self.alt,
        }
    }
}

/// What the user did in a save dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveChoice {
    Path(PathBuf),
    Cancelled,
    /// This system has no save dialog to show: the controller saves to the Desktop instead.
    Unavailable,
}

/// Recognises text in a picture (Windows' built-in OCR, Tesseract on Linux). Runs on a worker
/// thread; nothing leaves the computer.
pub trait TextRecognizer: Send + Sync {
    fn recognize(&self, img: &pasteroll_core::image::Rgba) -> Result<String, String>;
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
    /// The carousel switched between Complete and No Bloat (its window has a different size in
    /// each). Called with no carousel open.
    fn ui_mode_changed(&mut self, _mode: pasteroll_core::config::UiMode) {}
    /// Shell/tray browsing has no paste destination, but must release keyboard
    /// capture if focus moves to another application (or the desktop locks).
    fn begin_browse(&mut self) {}
    fn browse_focus_current(&self) -> bool { true }

    // ---------------------------------------------------------------- overlay
    fn cursor(&self) -> (i32, i32);
    /// Shows the overlay near `at` without taking focus and reports the geometry it got.
    fn overlay_show(&mut self, at: (i32, i32)) -> OverlayInfo;
    fn overlay_present(&mut self, frame: &Frame);
    fn overlay_hide(&mut self);

    // ---------------------------------------------------------------- expanded viewer
    /// Shows the viewer window (without taking focus, above the overlay) at `rect` = `(x, y, w, h)`
    /// screen pixels, or moves and resizes it there. `rounded` is false in fullscreen.
    fn viewer_show(&mut self, rect: (i32, i32, u32, u32), rounded: bool) {
        let _ = (rect, rounded);
    }
    /// Draws a frame in the viewer's current rectangle. Animation frames may be smaller than the
    /// window (same shape): they are stretched to fill it.
    fn viewer_present(&mut self, frame: &Frame) {
        let _ = frame;
    }
    fn viewer_hide(&mut self) {}
    /// The viewer is open: input goes to it (the wheel turns raw, the arrow keys, Home, End, F3 and
    /// F11 are taken, a click elsewhere closes it instead of pasting there).
    fn viewer_mode(&mut self, on: bool) {
        let _ = on;
    }
    /// Something is animating: deliver ticks more often (about every 10 ms) until it stops.
    fn animating(&mut self, on: bool) {
        let _ = on;
    }
    /// Is Shift held right now (Shift+click on the delete button deletes for good)?
    fn shift_held(&self) -> bool {
        false
    }
    /// Asks where to save a clip. `name` is the suggested file name, `ext` its extension. The
    /// platform takes care of input and focus around the dialog and gives the keyboard back to
    /// `target` afterwards.
    fn save_dialog(&mut self, name: &str, ext: &str, target: &Target) -> SaveChoice {
        let _ = (name, ext, target);
        SaveChoice::Unavailable
    }
    /// Asks which folder to save into (a folder chooser). `start` is where it opens; `target` is
    /// given the keyboard back afterwards, as for [`Platform::save_dialog`].
    fn folder_dialog(&mut self, start: Option<&Path>, target: &Target) -> SaveChoice {
        let _ = (start, target);
        SaveChoice::Unavailable
    }
    /// While on, a click outside the viewer's window is swallowed: it does not reach the program
    /// underneath (the image editor is modal, and a stray click must not lose the work in it).
    fn modal_clicks(&mut self, on: bool) {
        let _ = on;
    }
    /// Which of Ctrl, Shift and Alt were held when the last paste-at-click was made.
    fn click_modifiers(&self) -> Modifiers {
        Modifiers::default()
    }
    /// The text of the image editor's presets file ("" when there is none).
    fn load_presets(&self) -> String {
        String::new()
    }
    /// Stores the image editor's presets file.
    fn save_presets(&mut self, text: &str) -> Result<(), String> {
        let _ = text;
        Err("Presets cannot be saved here.".to_owned())
    }
    /// Local date and time of a Unix time, "YYYY-MM-DD HH:MM".
    fn format_time(&self, secs: i64) -> String {
        crate::controller::utc_date_time(secs)
    }

    // ---------------------------------------------------------------- input
    /// Start/stop swallowing the wheel and the session keys. Starting a session bumps the
    /// generation returned by [`Platform::input_generation`].
    fn input_active(&mut self, active: bool);
    fn input_generation(&self) -> u64;
    /// Called every tick: proof the UI thread is alive (the input hooks stop swallowing otherwise).
    fn heartbeat(&mut self);
    /// The user is still using the session (or reading in the viewer): restart the hooks' time limit.
    fn input_keepalive(&mut self) {}
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
