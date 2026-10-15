//! The overlay window: a topmost, non-activating popup that shows the frame the portable renderer
//! produced and forwards mouse input to the controller.
//!
//! It draws nothing itself. Because it is `WS_EX_NOACTIVATE` and answers `WM_MOUSEACTIVATE` with
//! `MA_NOACTIVATE`, clicking or dragging in it never moves the foreground or the keyboard focus
//! away from the application underneath.

use pasteroll_app::{Event, MouseEvent, MouseKind, OverlayInfo};
use pasteroll_render::{Frame, HEIGHT, WIDTH};
use std::cell::RefCell;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Default)]
struct State {
    frame: Option<Frame>,
    scale: f32,
}

pub struct OverlayWindow {
    hwnd: HWND,
    state: Box<RefCell<State>>,
    /// The carousel size setting (percent of normal).
    size: std::cell::Cell<u32>,
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

            let state = Box::new(RefCell::new(State { frame: None, scale: 1.0 }));
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
            let round = 2u32; // DWMWCP_ROUND
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &round as *const _ as *const _, 4);
            let border = (58u32 << 16) | (50u32 << 8) | 50u32; // COLORREF 0x00BBGGRR
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as *const _, 4);
            Ok(OverlayWindow { hwnd, state, size: std::cell::Cell::new(100) })
        }
    }

    pub fn set_size(&self, size: u32) {
        self.size.set(size);
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
            let (mut x, mut y, mut w, mut h) = (anchor.x, anchor.y, WIDTH as i32, HEIGHT as i32);
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                scale = dx as f32 / 96.0;
                let work = info.rcWork;
                scale *= pasteroll_core::settings::size_factor(self.size.get());
                w = ((WIDTH * scale) as i32).min(work.right - work.left);
                h = ((HEIGHT * scale) as i32).min(work.bottom - work.top);
                x = (anchor.x + (16.0 * scale) as i32).clamp(work.left, (work.right - w).max(work.left));
                y = (anchor.y + (20.0 * scale) as i32).clamp(work.top, (work.bottom - h).max(work.top));
            }
            if let Ok(mut s) = self.state.try_borrow_mut() {
                s.scale = scale;
            }
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            OverlayInfo { scale, size_px: (w as u32, h as u32) }
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
                crate::app::dispatch(Event::Mouse(MouseEvent { kind, x, y }));
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lparam);
                crate::app::dispatch(Event::Mouse(MouseEvent { kind: MouseKind::Move, x, y }));
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lparam);
                let _ = ReleaseCapture();
                crate::app::dispatch(Event::Mouse(MouseEvent { kind: MouseKind::Up, x, y }));
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
}
