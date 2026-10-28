//! Version and installation information, without opening clipboard history.

use crate::{setup, themeddlg};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::HWND;

pub fn text() -> String {
    let installation = if pasteroll_core::paths::is_portable() {
        "Portable"
    } else if setup::installed_by_setup() {
        "Installed (Windows Setup)"
    } else if setup::running_installed() {
        "Installed (per-user)"
    } else {
        "Standalone"
    };
    format!(
        "Pasteroll {}\n\nInstallation: {installation}\n\n\
         Open carousel: Ctrl+Shift+middle-click\n\
         Scroll with the wheel; type to search.\n\
         Tray: click for history; double-click for Settings.\n\n\
         Clipboard history stays on this computer.",
        env!("CARGO_PKG_VERSION")
    )
}

/// The window's heading.
pub fn heading() -> String {
    format!("Pasteroll {}", env!("CARGO_PKG_VERSION"))
}

/// What the window says under the heading.
pub fn body() -> String {
    text().splitn(2, "\n\n").nth(1).unwrap_or_default().to_owned()
}

/// Whether an About window is open already (a second one is not opened).
static OPEN: AtomicBool = AtomicBool::new(false);

fn show_window() {
    if OPEN.swap(true, Ordering::AcqRel) {
        return;
    }
    let (heading, body) = (heading(), body());
    themeddlg::show(&themeddlg::Spec { title: "About Pasteroll", heading: &heading, body: &body, buttons: &["OK"], focus: 0 });
    OPEN.store(false, Ordering::Release);
}

/// The About window. From the tray (`owner` given) it opens on a thread of its own and returns at
/// once; from the command line it waits until it is closed.
pub fn show(owner: Option<HWND>) {
    if owner.is_some() {
        let _ = std::thread::Builder::new().name("pasteroll-about".into()).spawn(show_window);
    } else {
        show_window();
    }
}
