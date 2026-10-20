//! Version and installation information, without opening clipboard history.

use crate::setup;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

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

pub fn show(owner: Option<HWND>) {
    let content: Vec<u16> = text().encode_utf16().chain(Some(0)).collect();
    unsafe {
        let params = MSGBOXPARAMSW {
            cbSize: size_of::<MSGBOXPARAMSW>() as u32,
            hwndOwner: owner.unwrap_or_default(),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            lpszText: PCWSTR(content.as_ptr()),
            lpszCaption: w!("About Pasteroll"),
            dwStyle: MB_OK | MB_USERICON,
            lpszIcon: PCWSTR(1usize as *const u16),
            ..Default::default()
        };
        MessageBoxIndirectW(&params);
    }
}
