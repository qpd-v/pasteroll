//! The overlay windows: topmost, non-activating popups that show the frames the portable renderer
//! produced and forward mouse input to the controller. One is the carousel; the other is the
//! expanded viewer, which the controller moves and resizes every frame while it animates.
//!
//! It draws nothing itself. Because it is `WS_EX_NOACTIVATE` and answers `WM_MOUSEACTIVATE` with
//! `MA_NOACTIVATE`, clicking or dragging in it never moves the foreground or the keyboard focus
//! away from the application underneath.

use pasteroll_app::{Event, MouseEvent, MouseKind, OverlayInfo};
use pasteroll_core::config::UiMode;
use pasteroll_render::{window_dip, Frame};
use std::cell::RefCell;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Role {
    #[default]
    Carousel,
    Viewer,
}

#[derive(Default)]
struct State {
    frame: Option<Frame>,
    scale: f32,
    role: Role,
    /// A leave notification has been asked for.
    tracking: bool,
}

pub struct OverlayWindow {
    hwnd: HWND,
    state: Box<RefCell<State>>,
    /// The carousel size setting (percent of normal).
    size: std::cell::Cell<u32>,
    /// Complete or No Bloat: the window is a different size in each.
    mode: std::cell::Cell<UiMode>,
    /// The corner style last set (the viewer has square corners in fullscreen).
    rounded: std::cell::Cell<bool>,
}

fn set_rounded(hwnd: HWND, rounded: bool) {
    // Windows 11: DWMWCP_ROUND or DWMWCP_DONOTROUND. Older versions just fail; ignore.
    let pref = if rounded { 2u32 } else { 1u32 };
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &pref as *const _ as *const _, 4);
    }
}

fn blit(hwnd: HWND, hdc: HDC, frame: &Frame) {
    unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (cw, ch) = (rc.right - rc.left, rc.bottom - rc.top);
        if cw <= 0 || ch <= 0 {
            return;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: frame.width as i32,
                biHeight: -(frame.height as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        StretchDIBits(
            hdc,
            0,
            0,
            cw,
            ch,
            0,
            0,
            frame.width as i32,
            frame.height as i32,
            Some(frame.bgra.as_ptr() as *const _),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

impl OverlayWindow {
    pub fn create() -> Result<OverlayWindow> {
        Self::create_as(Role::Carousel)
    }

    /// The expanded viewer's window.
    pub fn create_viewer() -> Result<OverlayWindow> {
        Self::create_as(Role::Viewer)
    }

    fn create_as(role: Role) -> Result<OverlayWindow> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS | CS_DROPSHADOW,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: w!("PasterollOverlay"),
                ..Default::default()
            };
            RegisterClassW(&class); // already registered is fine

            let state = Box::new(RefCell::new(State { frame: None, scale: 1.0, role, tracking: false }));
            let param = &*state as *const RefCell<State> as *const core::ffi::c_void;
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("PasterollOverlay"),
                w!("Pasteroll"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinst.into()),
                Some(param),
            )?;
            // Windows 11: rounded corners and a quiet border. Older versions just fail; ignore.
            set_rounded(hwnd, true);
            // the viewer draws its own accent edge, like the active card it grew from
            let border = if role == Role::Viewer {
                0xFFFF_FFFEu32 // DWMWA_COLOR_NONE
            } else {
                (58u32 << 16) | (50u32 << 8) | 50u32 // COLORREF 0x00BBGGRR
            };
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as *const _, 4);
            Ok(OverlayWindow {
                hwnd,
                state,
                size: std::cell::Cell::new(100),
                mode: std::cell::Cell::new(UiMode::Complete),
                rounded: std::cell::Cell::new(true),
            })
        }
    }

    pub fn set_size(&self, size: u32) {
        self.size.set(size);
    }

    pub fn set_mode(&self, mode: UiMode) {
        self.mode.set(mode);
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn screen_rect(&self) -> RECT {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut r);
        }
        r
    }

    /// Positions the overlay near `anchor` (screen px) on that monitor and shows it without
    /// activating it. Returns the scale and size it ended up with.
    pub fn show(&self, anchor: POINT) -> OverlayInfo {
        unsafe {
            let monitor = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let mut scale = 1.0f32;
            let (width, height) = window_dip(self.mode.get());
            let (mut x, mut y, mut w, mut h) = (anchor.x, anchor.y, width as i32, height as i32);
            let mut mon = (0, 0, 1920u32, 1080u32);
            let mut work = (0, 0, 1920u32, 1040u32);
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let (m, wk) = (info.rcMonitor, info.rcWork);
                mon = (m.left, m.top, (m.right - m.left).max(1) as u32, (m.bottom - m.top).max(1) as u32);
                work = (wk.left, wk.top, (wk.right - wk.left).max(1) as u32, (wk.bottom - wk.top).max(1) as u32);
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                scale = dx as f32 / 96.0;
                let work = info.rcWork;
                scale *= pasteroll_core::settings::size_factor(self.size.get());
                w = ((width * scale) as i32).min(work.right - work.left);
                h = ((height * scale) as i32).min(work.bottom - work.top);
                x = (anchor.x + (16.0 * scale) as i32).clamp(work.left, (work.right - w).max(work.left));
                y = (anchor.y + (20.0 * scale) as i32).clamp(work.top, (work.bottom - h).max(work.top));
            }
            if let Ok(mut s) = self.state.try_borrow_mut() {
                s.scale = scale;
            }
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            OverlayInfo { scale, size_px: (w as u32, h as u32), origin_px: (x, y), monitor_px: mon, work_px: work }
        }
    }

    /// Shows the window at `rect` (screen pixels), on top, without activating it.
    pub fn show_rect(&self, rect: (i32, i32, u32, u32), rounded: bool) {
        if self.rounded.get() != rounded {
            self.rounded.set(rounded);
            set_rounded(self.hwnd, rounded);
        }
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                rect.0,
                rect.1,
                rect.2.max(1) as i32,
                rect.3.max(1) as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOCOPYBITS,
            );
        }
    }

    pub fn present(&self, frame: &Frame) {
        if let Ok(mut s) = self.state.try_borrow_mut() {
            s.frame = Some(frame.clone());
        }
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            if !hdc.is_invalid() {
                blit(self.hwnd, hdc, frame);
                ReleaseDC(Some(self.hwnd), hdc);
            }
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        if let Ok(mut s) = self.state.try_borrow_mut() {
            s.frame = None;
        }
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Sent once the pointer leaves a window that asked for it with TrackMouseEvent.
const WM_MOUSELEAVE: u32 = 0x02A3;

fn xy(lparam: LPARAM) -> (f32, f32) {
    (
        (lparam.0 & 0xFFFF) as u16 as i16 as f32,
        ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32,
    )
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
        let role = if ptr.is_null() { Role::Carousel } else { (*ptr).try_borrow().map(|s| s.role).unwrap_or_default() };
        let send = |kind: MouseKind, x: f32, y: f32| {
            let m = MouseEvent { kind, x, y };
            crate::app::dispatch(match role {
                Role::Carousel => Event::Mouse(m),
                Role::Viewer => Event::ViewerMouse(m),
            });
        };
        match msg {
            // The point of the whole design: a click here never takes activation or focus.
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !ptr.is_null() {
                    if let Ok(s) = (*ptr).try_borrow() {
                        if let Some(f) = &s.frame {
                            blit(hwnd, hdc, f);
                        }
                    }
                }
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                SetCapture(hwnd);
                let (x, y) = xy(lparam);
                let kind = if msg == WM_LBUTTONDBLCLK { MouseKind::DoubleClick } else { MouseKind::Down };
                send(kind, x, y);
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                // ask once for a WM_MOUSELEAVE, so hover highlights go out when the pointer does
                if !ptr.is_null() {
                    if let Ok(mut s) = (*ptr).try_borrow_mut() {
                        if !s.tracking {
                            let mut tme = TRACKMOUSEEVENT {
                                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                                dwFlags: TME_LEAVE,
                                hwndTrack: hwnd,
                                dwHoverTime: 0,
                            };
                            s.tracking = TrackMouseEvent(&mut tme).is_ok();
                        }
                    }
                }
                let (x, y) = xy(lparam);
                send(MouseKind::Move, x, y);
                return LRESULT(0);
            }
            WM_MOUSELEAVE => {
                if !ptr.is_null() {
                    if let Ok(mut s) = (*ptr).try_borrow_mut() {
                        s.tracking = false;
                    }
                }
                send(MouseKind::Leave, 0.0, 0.0);
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lparam);
                let _ = ReleaseCapture();
                send(MouseKind::Up, x, y);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_setting_scales_the_overlay() {
        let o = OverlayWindow::create().expect("overlay window");
        let at = POINT { x: 100, y: 100 };
        o.set_size(100);
        let normal = o.show(at);
        o.set_size(150);
        let big = o.show(at);
        o.set_size(80);
        let half = {
            o.set_size(50);
            o.show(at)
        };
        assert!((half.scale / normal.scale - 0.5).abs() < 0.001);
        o.set_size(80);
        let small = o.show(at);
        o.hide();
        assert!((big.scale / normal.scale - 1.5).abs() < 0.001, "{} vs {}", big.scale, normal.scale);
        assert!((small.scale / normal.scale - 0.8).abs() < 0.001);
        assert!(big.size_px.0 > normal.size_px.0 && small.size_px.0 < normal.size_px.0);
    }

    #[test]
    fn no_bloat_is_a_smaller_window() {
        let o = OverlayWindow::create().expect("overlay window");
        let at = POINT { x: 100, y: 100 };
        o.set_size(100);
        let complete = o.show(at);
        o.set_mode(UiMode::NoBloat);
        let slim = o.show(at);
        o.hide();
        assert_eq!(slim.scale, complete.scale);
        let (cw, ch) = window_dip(UiMode::Complete);
        let (sw, sh) = window_dip(UiMode::NoBloat);
        assert!((slim.size_px.0 as f32 / complete.size_px.0 as f32 - sw / cw).abs() < 0.01, "{slim:?} vs {complete:?}");
        assert!((slim.size_px.1 as f32 / complete.size_px.1 as f32 - sh / ch).abs() < 0.01);
    }
}
