//! The notification-area icon and its menu: pause or resume recording, the Complete / No Bloat
//! mode, settings, the data folder and quit. The icon is drawn in code, so there is no resource file to ship.

use crate::{about, app, settings};
use pasteroll_core::config::UiMode;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 240;
const ID_PAUSE: usize = 1;
const ID_SETTINGS: usize = 2;
const ID_FOLDER: usize = 3;
const ID_QUIT: usize = 4;
const ID_ABOUT: usize = 5;
const ID_COMPLETE: usize = 6;
const ID_NO_BLOAT: usize = 7;
const ICON_ID: u32 = 1;
const CLICK_TIMER: usize = 1;

#[derive(Default)]
struct ClickState {
    pending: Option<(i32, i32)>,
    ignore_up: bool,
}

impl ClickState {
    fn released(&mut self, at: (i32, i32)) -> bool {
        if self.ignore_up {
            self.ignore_up = false;
            return false;
        }
        self.pending = Some(at);
        true
    }

    fn double_clicked(&mut self) {
        self.pending = None;
        self.ignore_up = true;
    }

    fn cancel(&mut self) {
        self.pending = None;
        self.ignore_up = false;
    }
}

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    icon_paused: HICON,
    // Stable per-window storage; only the UI thread touches it through GWLP_USERDATA.
    _clicks: Box<UnsafeCell<ClickState>>,
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
    let state = if paused { "Recording paused" } else { "Recording" };
    let text = format!("Pasteroll {} - {state}", env!("CARGO_PKG_VERSION"));
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
            let clicks = Box::new(UnsafeCell::new(ClickState::default()));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, clicks.get() as isize);
            let nid = data(hwnd, if paused { icon_paused } else { icon }, paused);
            if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                let _ = DestroyWindow(hwnd);
                let _ = DestroyIcon(icon);
                let _ = DestroyIcon(icon_paused);
                return Err(windows::core::Error::new(E_FAIL, "could not add the notification icon"));
            }
            Ok(Tray { hwnd, icon, icon_paused, _clicks: clicks })
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
            let _ = KillTimer(Some(self.hwnd), CLICK_TIMER);
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

/// The right-click menu for the current recording state and mode. The caller destroys it.
unsafe fn build_menu(paused: bool, mode: UiMode) -> Option<HMENU> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let pause = wide(if paused { "Resume recording" } else { "Pause recording" });
        let complete = wide(UiMode::Complete.label());
        let no_bloat = wide(UiMode::NoBloat.label());
        let settings_t = wide("Settings...");
        let folder = wide("Open data folder");
        let about_t = wide("About Pasteroll...");
        let quit = wide("Quit Pasteroll");
        let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE, PCWSTR(pause.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_COMPLETE, PCWSTR(complete.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_NO_BLOAT, PCWSTR(no_bloat.as_ptr()));
        let current = if mode == UiMode::NoBloat { ID_NO_BLOAT } else { ID_COMPLETE };
        let _ = CheckMenuRadioItem(menu, ID_COMPLETE as u32, ID_NO_BLOAT as u32, current as u32, MF_BYCOMMAND.0);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, PCWSTR(settings_t.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_FOLDER, PCWSTR(folder.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_ABOUT, PCWSTR(about_t.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(quit.as_ptr()));
        Some(menu)
    }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let Some(menu) = build_menu(app::is_paused(), app::ui_mode()) else { return };
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // the menu only closes when clicking elsewhere if its owner is the foreground window
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, at.x, at.y, Some(0), hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 as usize {
            ID_PAUSE => app::toggle_pause(),
            ID_COMPLETE => app::set_ui_mode(UiMode::Complete),
            ID_NO_BLOAT => app::set_ui_mode(UiMode::NoBloat),
            ID_SETTINGS => settings::open(),
            ID_FOLDER => app::open_data_folder(),
            ID_ABOUT => about::show(Some(hwnd)),
            ID_QUIT => app::quit(),
            _ => {}
        }
    }
}

fn icon_anchor(hwnd: HWND) -> (i32, i32) {
    unsafe {
        let id = NOTIFYICONIDENTIFIER { cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: hwnd, uID: ICON_ID, ..Default::default() };
        if let Ok(rect) = Shell_NotifyIconGetRect(&id) {
            return ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
        }
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        (point.x, point.y)
    }
}

unsafe extern "system" fn tray_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let clicks = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ClickState;
        if msg == WM_TRAY {
            match (lparam.0 & 0xFFFF) as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    let _ = KillTimer(Some(hwnd), CLICK_TIMER);
                    if !clicks.is_null() { (*clicks).cancel(); }
                    show_menu(hwnd);
                }
                WM_LBUTTONUP => {
                    let at = icon_anchor(hwnd);
                    if !clicks.is_null() && (*clicks).released(at) {
                        // Wait for the shell's double-click notification before committing
                        // to a single click. The pointer may move while this timer runs.
                        SetTimer(Some(hwnd), CLICK_TIMER, GetDoubleClickTime().max(10), None);
                    }
                }
                WM_LBUTTONDBLCLK => {
                    let _ = KillTimer(Some(hwnd), CLICK_TIMER);
                    if !clicks.is_null() { (*clicks).double_clicked(); }
                    settings::open();
                }
                _ => {}
            }
            return LRESULT(0);
        }
        if msg == WM_TIMER && wparam.0 == CLICK_TIMER {
            let _ = KillTimer(Some(hwnd), CLICK_TIMER);
            let at = if clicks.is_null() { None } else { (*clicks).pending.take() };
            if let Some(at) = at { app::browse_from_tray(at); }
            return LRESULT(0);
        }
        if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
            let _ = KillTimer(Some(hwnd), CLICK_TIMER);
            if !clicks.is_null() { (*clicks).cancel(); }
            // Explorer restarted: its notification area is empty again
            app::readd_tray();
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0); }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_callback_waits_for_timer_and_double_click_cancels_queued_activation() {
        use std::time::{Duration, Instant};
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    let _ = KillTimer(Some(self.0), CLICK_TIMER);
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        unsafe {
            // Hidden fixture only: no notification-area icon, capture, hooks,
            // history, settings instance or live application Runtime is created.
            let clicks = Box::new(UnsafeCell::new(ClickState::default()));
            let instance = GetModuleHandleW(None).unwrap();
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(tray_proc), hInstance: instance.into(),
                lpszClassName: w!("PasterollTrayCallbackFixture"), ..Default::default() });
            let window = Window(CreateWindowExW(WS_EX_TOOLWINDOW, w!("PasterollTrayCallbackFixture"),
                w!("Tray callback fixture"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance.into()), None).unwrap());
            SetWindowLongPtrW(window.0, GWLP_USERDATA, clicks.get() as isize);
            SendMessageW(window.0, WM_TRAY, Some(WPARAM(0)), Some(LPARAM(WM_LBUTTONUP as isize)));
            assert!((*clicks.get()).pending.is_some(), "single click must wait, not open immediately");
            let deadline = Instant::now() + Duration::from_millis(GetDoubleClickTime() as u64 + 500);
            while (*clicks.get()).pending.is_some() && Instant::now() < deadline {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, Some(window.0), 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!((*clicks.get()).pending.is_none(), "native timer must consume the single click");
            SendMessageW(window.0, WM_TRAY, Some(WPARAM(0)), Some(LPARAM(WM_LBUTTONUP as isize)));
            SendMessageW(window.0, WM_TRAY, Some(WPARAM(0)), Some(LPARAM(WM_LBUTTONDBLCLK as isize)));
            SendMessageW(window.0, WM_TRAY, Some(WPARAM(0)), Some(LPARAM(WM_LBUTTONUP as isize)));
            SendMessageW(window.0, WM_TIMER, Some(WPARAM(CLICK_TIMER)), Some(LPARAM(0)));
            assert!((*clicks.get()).pending.is_none(), "double click and final release must not reopen history");
        }
    }

    #[test]
    fn single_click_keeps_the_icon_anchor_until_its_timer_fires() {
        let mut state = ClickState::default();
        assert!(state.released((1800, 1040)));
        assert_eq!(state.pending.take(), Some((1800, 1040)));
        assert_eq!(state.pending.take(), None);
    }

    #[test]
    fn double_click_cancels_single_and_swallows_its_final_release() {
        let mut state = ClickState::default();
        assert!(state.released((10, 20)));
        state.double_clicked();
        assert_eq!(state.pending.take(), None, "even a queued timer cannot open history");
        assert!(!state.released((10, 20)));
        assert_eq!(state.pending.take(), None);
        assert!(state.released((30, 40)), "a later single click works normally");
    }

    #[test]
    fn context_menu_or_explorer_reset_cancels_pending_single_click() {
        let mut state = ClickState::default();
        state.released((10, 20));
        state.cancel();
        assert_eq!(state.pending.take(), None);
        assert!(state.released((30, 40)));
    }

    #[test]
    fn the_menu_offers_both_modes_with_the_current_one_marked() {
        unsafe {
            for (mode, on, off) in [(UiMode::Complete, ID_COMPLETE, ID_NO_BLOAT), (UiMode::NoBloat, ID_NO_BLOAT, ID_COMPLETE)] {
                let menu = build_menu(false, mode).expect("menu");
                let checked = |id: usize| GetMenuState(menu, id as u32, MF_BYCOMMAND) & MF_CHECKED.0 != 0;
                assert!(checked(on), "{mode:?} is marked");
                assert!(!checked(off), "the other mode is not");
                // the order the person sees: recording, the two modes, the rest, quit last
                let ids: Vec<u32> = (0..GetMenuItemCount(Some(menu)) as u32).map(|i| GetMenuItemID(menu, i as i32)).collect();
                const SEPARATOR: u32 = 0; // what GetMenuItemID reports for a separator
                let item = |id: usize| id as u32;
                assert_eq!(
                    ids,
                    [item(ID_PAUSE), SEPARATOR, item(ID_COMPLETE), item(ID_NO_BLOAT), SEPARATOR, item(ID_SETTINGS), item(ID_FOLDER), item(ID_ABOUT), SEPARATOR, item(ID_QUIT)]
                );
                let _ = DestroyMenu(menu);
            }
        }
    }

    #[test]
    fn tooltip_identifies_version_and_recording_state_with_a_terminator() {
        for paused in [false, true] {
            let tip = tip(paused);
            let end = tip.iter().position(|c| *c == 0).unwrap();
            let text = String::from_utf16(&tip[..end]).unwrap();
            assert!(text.contains(env!("CARGO_PKG_VERSION")));
            assert!(text.ends_with(if paused { "Recording paused" } else { "Recording" }));
            assert!(tip[end..].iter().all(|c| *c == 0));
        }
    }
}
