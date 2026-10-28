//! What the tray window is told about an update: how this copy updates, and the few things that
//! need the program itself (starting the new version, saying it was updated, start-up findings).
//! What the person sees while updating is the update window (`updatewin`); the work is in `install`.

use crate::install::{self, Mode};
use pasteroll_core::update::Version;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Tells the tray window that something happened while installing (see [`take_events`]).
pub const WM_INSTALL_EVENT: u32 = WM_APP + 242;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A finding to tell the person (an update that did not finish, seen at start-up).
    Failed(String),
    /// An update finished and this is the new version.
    Info(String),
    /// Setup was started; it will close this program and start the new version.
    SetupStarted,
    /// The program file was replaced; restart into it.
    Replaced(PathBuf),
}

static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

pub fn take_events() -> Vec<Event> {
    EVENTS.lock().map(|mut e| std::mem::take(&mut *e)).unwrap_or_default()
}

fn post(window: isize, event: Event) {
    if let Ok(mut e) = EVENTS.lock() {
        e.push(event);
    }
    unsafe {
        let _ = PostMessageW(Some(HWND(window as *mut _)), WM_INSTALL_EVENT, WPARAM(0), LPARAM(0));
    }
}

/// A message for the person from outside an update they are running (start-up findings).
pub fn report(window: isize, text: String) {
    post(window, Event::Failed(text));
}

/// "Pasteroll updated to ...", at the first start after an update.
pub fn updated(window: isize, text: String) {
    post(window, Event::Info(text));
}

/// How this copy updates; it does not change while the program runs.
pub fn mode() -> &'static Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    MODE.get_or_init(install::mode)
}

/// Whether "Download and install" can be offered for this copy.
pub fn installable() -> bool {
    !matches!(mode(), Mode::NotifyOnly(_))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Tells the tray window that Setup was started: it closes this program and starts the new one.
pub fn setup_started(window: isize) {
    post(window, Event::SetupStarted);
}

/// Tells the tray window that the program file was replaced: start the new one.
pub fn replaced(window: isize, exe: PathBuf) {
    post(window, Event::Replaced(exe));
}

/// Opens the release page of `v` in the browser.
pub fn open_page(v: &Version) {
    use windows::core::w;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
    let url = wide(&v.release_page());
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), None, None, SW_SHOW);
    }
}
