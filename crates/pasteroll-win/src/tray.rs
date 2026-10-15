//! The notification-area icon and its menu: pause or resume recording, settings, the data folder
//! and quit. The icon is drawn in code, so there is no resource file to ship.

use crate::{app, settings};
use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 240;
const ID_PAUSE: usize = 1;
const ID_SETTINGS: usize = 2;
const ID_FOLDER: usize = 3;
const ID_QUIT: usize = 4;
const ICON_ID: u32 = 1;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    icon_paused: HICON,
}

/// The logo as a 32x32 icon (premultiplied BGRA, the form Windows wants), normal or paused.
fn make_icon(paused: bool) -> Result<HICON> {
    let n = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    let rgba = pasteroll_core::logo::rgba(n as u32, paused);
    let mut px = vec![0u8; rgba.len()];
    for (o, i) in px.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
        let a = u32::from(i[3]);
        o[0] = (u32::from(i[2]) * a / 255) as u8;
        o[1] = (u32::from(i[1]) * a / 255) as u8;
        o[2] = (u32::from(i[0]) * a / 255) as u8;
        o[3] = i[3];
    }
    unsafe {
        let color = CreateBitmap(n, n, 1, 32, Some(px.as_ptr() as *const _));
        let mask = CreateBitmap(n, n, 1, 1, None);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn tip(paused: bool) -> [u16; 128] {
    let text = if paused { "Pasteroll (recording paused)" } else { "Pasteroll" };
    let mut out = [0u16; 128];
    for (i, u) in text.encode_utf16().take(127).enumerate() {
        out[i] = u;
    }
    out
}

fn data(hwnd: HWND, icon: HICON, paused: bool) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        szTip: tip(paused),
        ..Default::default()
    }
}

impl Tray {
    pub fn create(paused: bool) -> Result<Tray> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(tray_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("PasterollTray"),
                ..Default::default()
            };
            RegisterClassW(&class);
            TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
            // a real (never shown) top-level window: the shell sends the icon's messages to it
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("PasterollTray"),
                w!("Pasteroll"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinst.into()),
                None,
            )?;
            let icon = make_icon(false)?;
            let icon_paused = make_icon(true)?;
            let nid = data(hwnd, if paused { icon_paused } else { icon }, paused);
            if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                let _ = DestroyWindow(hwnd);
                let _ = DestroyIcon(icon);
                let _ = DestroyIcon(icon_paused);
                return Err(windows::core::Error::new(E_FAIL, "could not add the notification icon"));
            }
            Ok(Tray { hwnd, icon, icon_paused })
        }
    }

    /// Refreshes the tooltip after recording was paused or resumed.
    pub fn update(&self, paused: bool) {
        unsafe {
            let nid = data(self.hwnd, if paused { self.icon_paused } else { self.icon }, paused);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let nid = data(self.hwnd, self.icon, false);
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyIcon(self.icon);
            let _ = DestroyIcon(self.icon_paused);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let paused = app::is_paused();
        let pause = wide(if paused { "Resume recording" } else { "Pause recording" });
        let settings_t = wide("Settings...");
        let folder = wide("Open data folder");
        let quit = wide("Quit Pasteroll");
        let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE, PCWSTR(pause.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, PCWSTR(settings_t.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_FOLDER, PCWSTR(folder.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(quit.as_ptr()));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // the menu only closes when clicking elsewhere if its owner is the foreground window
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, at.x, at.y, Some(0), hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 as usize {
            ID_PAUSE => app::toggle_pause(),
            ID_SETTINGS => settings::open(),
            ID_FOLDER => app::open_data_folder(),
            ID_QUIT => app::quit(),
            _ => {}
        }
    }
}

unsafe extern "system" fn tray_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_TRAY {
            match (lparam.0 & 0xFFFF) as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => settings::open(),
                _ => {}
            }
            return LRESULT(0);
        }
        if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
            // Explorer restarted: its notification area is empty again
            app::readd_tray();
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

impl Tray {
    /// Puts the icon back (Explorer was restarted).
    pub fn readd(&self, paused: bool) {
        unsafe {
            let nid = data(self.hwnd, if paused { self.icon_paused } else { self.icon }, paused);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }
}
