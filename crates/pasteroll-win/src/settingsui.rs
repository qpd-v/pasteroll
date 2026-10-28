//! The look of the settings window: Windows' light or dark colours, flat rounded cards, and the
//! drawing of the controls (check boxes, radio buttons, push buttons, sliders). The controls stay
//! ordinary Windows controls, so keyboard use, tab order and accessibility are unchanged; only their
//! painting is replaced (`NM_CUSTOMDRAW`). The shapes come from `pasteroll_render::widgets`.

use pasteroll_render::widgets::{self, Shape};
use std::cell::RefCell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
use windows::Win32::UI::WindowsAndMessaging::*;

pub type Rgb = (u8, u8, u8);

/// The colours of one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub bg: Rgb,
    pub card: Rgb,
    pub border: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub field: Rgb,
    pub field_border: Rgb,
    pub button: Rgb,
    pub button_hot: Rgb,
    pub button_border: Rgb,
    pub accent: Rgb,
    pub accent_hot: Rgb,
    pub on_accent: Rgb,
    pub track: Rgb,
}

impl Theme {
    pub fn light() -> Theme {
        Theme {
            dark: false,
            bg: (243, 243, 243),
            card: (255, 255, 255),
            border: (229, 229, 229),
            text: (27, 27, 27),
            muted: (96, 96, 96),
            field: (249, 249, 249),
            field_border: (196, 196, 196),
            button: (253, 253, 253),
            button_hot: (240, 240, 240),
            button_border: (208, 208, 208),
            accent: (20, 122, 104),
            accent_hot: (15, 104, 89),
            on_accent: (255, 255, 255),
            track: (206, 206, 206),
        }
    }

    pub fn dark() -> Theme {
        Theme {
            dark: true,
            bg: (32, 32, 32),
            card: (43, 43, 43),
            border: (62, 62, 62),
            text: (242, 242, 242),
            muted: (166, 166, 166),
            field: (30, 30, 30),
            field_border: (84, 84, 84),
            button: (58, 58, 58),
            button_hot: (70, 70, 70),
            button_border: (78, 78, 78),
            accent: (47, 174, 151),
            accent_hot: (74, 198, 175),
            on_accent: (12, 28, 24),
            track: (92, 92, 92),
        }
    }

    /// What Windows is set to (Settings > Personalization > Colors > app mode). The environment
    /// variable `PASTEROLL_SETTINGS_THEME` (`light` or `dark`) overrides it, for checking both looks.
    pub fn current() -> Theme {
        match std::env::var("PASTEROLL_SETTINGS_THEME").ok().as_deref() {
            Some("dark") => return Theme::dark(),
            Some("light") => return Theme::light(),
            _ => {}
        }
        if apps_use_light_theme().unwrap_or(true) { Theme::light() } else { Theme::dark() }
    }
}

/// Windows' "app mode": `Some(true)` light, `Some(false)` dark, `None` when it cannot be read.
fn apps_use_light_theme() -> Option<bool> {
    let mut data = 1u32;
    let mut size = size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    (r == ERROR_SUCCESS).then_some(data != 0)
}

pub fn colorref(c: Rgb) -> COLORREF {
    COLORREF(u32::from(c.0) | u32::from(c.1) << 8 | u32::from(c.2) << 16)
}

/// Everything the painting needs: the theme, its brushes, and where the cards and fields are.
pub struct Look {
    pub theme: Theme,
    pub(crate) bg: HBRUSH,
    pub(crate) card: HBRUSH,
    pub(crate) field: HBRUSH,
    /// Cards, in DIP: x, y, width, height.
    pub cards: Vec<[i32; 4]>,
    /// Frames of edit boxes and lists, in DIP.
    pub fields: Vec<[i32; 4]>,
    pub font: HFONT,
    /// The scale the window's controls were laid out for (changes when it moves to another monitor).
    pub dpi: u32,
}

impl Look {
    pub fn new(theme: Theme, font: HFONT) -> Look {
        let brush = |c: Rgb| unsafe { CreateSolidBrush(colorref(c)) };
        Look {
            theme,
            bg: brush(theme.bg),
            card: brush(theme.card),
            field: brush(theme.field),
            cards: Vec::new(),
            fields: Vec::new(),
            font,
            dpi: 96,
        }
    }

    /// Switches to `theme` (Windows' colours changed while the window is open).
    pub fn retheme(&mut self, theme: Theme) {
        unsafe {
            let _ = DeleteObject(self.bg.into());
            let _ = DeleteObject(self.card.into());
            let _ = DeleteObject(self.field.into());
        }
        let brush = |c: Rgb| unsafe { CreateSolidBrush(colorref(c)) };
        self.theme = theme;
        self.bg = brush(theme.bg);
        self.card = brush(theme.card);
        self.field = brush(theme.field);
    }

}

impl Drop for Look {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.bg.into());
            let _ = DeleteObject(self.card.into());
            let _ = DeleteObject(self.field.into());
        }
    }
}

thread_local! {
    static LOOK: RefCell<Option<Look>> = const { RefCell::new(None) };
}

pub fn install(look: Look) {
    LOOK.with(|l| *l.borrow_mut() = Some(look));
}

pub fn remove() {
    LOOK.with(|l| *l.borrow_mut() = None);
}

/// Runs `f` on the installed look, if there is one.
pub fn with<R>(f: impl FnOnce(&mut Look) -> R) -> Option<R> {
    LOOK.with(|l| l.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// A dark or light title bar to match (Windows 10 1903 and later; older versions ignore it).
pub fn apply_title_bar(hwnd: HWND, dark: bool) {
    let on: i32 = i32::from(dark);
    unsafe {
        // DWMWA_USE_IMMERSIVE_DARK_MODE
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const i32 as *const _, 4);
    }
}

/// Dark scroll bars and borders on a list or edit box in the dark theme.
pub fn theme_control(h: HWND, dark: bool) {
    unsafe {
        let _ = SetWindowTheme(h, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, PCWSTR::null());
    }
}

fn scaled(v: i32, dpi: u32) -> i32 {
    (v as i64 * dpi as i64 / 96) as i32
}

/// Draws `shape` over what is already in `hdc` (an alpha blend).
pub fn blit(hdc: HDC, x: i32, y: i32, w: i32, h: i32, shape: Shape, scale: f32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let bytes = widgets::render(shape, w as u32, h as u32, scale);
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(hdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
        if bits.is_null() {
            let _ = DeleteObject(bmp.into());
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), bits as *mut u8, bytes.len());
        let mem = CreateCompatibleDC(Some(hdc));
        let old = SelectObject(mem, bmp.into());
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        let _ = AlphaBlend(hdc, x, y, w, h, mem, 0, 0, w, h, blend);
        SelectObject(mem, old);
        let _ = DeleteDC(mem);
        let _ = DeleteObject(bmp.into());
    }
}

/// The window's background: the page colour, the cards and the frames of edit boxes and lists.
pub fn paint_background(hwnd: HWND, hdc: HDC, look: &Look) {
    unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        let s = dpi as f32 / 96.0;
        // drawn off screen first, so nothing flickers
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old = SelectObject(mem, bmp.into());
        FillRect(mem, &rc, look.bg);
        let t = look.theme;
        for c in &look.cards {
            let shape = Shape::Box { radius: 8.0, fill: Some(t.card), border: Some(t.border) };
            blit(mem, scaled(c[0], dpi), scaled(c[1], dpi), scaled(c[2], dpi), scaled(c[3], dpi), shape, s);
        }
        for f in &look.fields {
            let shape = Shape::Box { radius: 5.0, fill: Some(t.field), border: Some(t.field_border) };
            blit(mem, scaled(f[0], dpi), scaled(f[1], dpi), scaled(f[2], dpi), scaled(f[3], dpi), shape, s);
        }
        let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
    }
}

fn text_of(h: HWND) -> Vec<u16> {
    unsafe {
        let n = GetWindowTextLengthW(h).max(0) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(h, &mut buf).max(0) as usize;
        buf.truncate(got);
        buf
    }
}

/// Draws `text` in `rc`, vertically centred, wrapped when `wrap` is set.
fn label(hdc: HDC, font: HFONT, text: &[u16], rc: RECT, color: Rgb, centre: bool, wrap: bool) {
    unsafe {
        let old = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, colorref(color));
        let mut t = text.to_vec();
        let mut flags = DT_NOPREFIX | DT_END_ELLIPSIS;
        flags |= if centre { DT_CENTER } else { DT_LEFT };
        let mut r = rc;
        if wrap {
            // centre a wrapped label by measuring it first
            let mut measure = rc;
            DrawTextW(hdc, &mut t, &mut measure, flags | DT_WORDBREAK | DT_CALCRECT);
            let used = measure.bottom - measure.top;
            let free = (rc.bottom - rc.top - used).max(0);
            r.top = rc.top + free / 2;
            r.bottom = r.top + used.max(1);
            DrawTextW(hdc, &mut t, &mut r, flags | DT_WORDBREAK);
        } else {
            DrawTextW(hdc, &mut t, &mut r, flags | DT_SINGLELINE | DT_VCENTER);
        }
        SelectObject(hdc, old);
    }
}

const NM_CUSTOMDRAW_CODE: u32 = (-12i32) as u32;
const CDDS_PREPAINT_STAGE: u32 = 1;
const CDRF_SKIPDEFAULT_RESULT: isize = 4;
const CDIS_SELECTED_BIT: u32 = 0x1;
const CDIS_DISABLED_BIT: u32 = 0x4;
const CDIS_FOCUS_BIT: u32 = 0x10;
const CDIS_HOT_BIT: u32 = 0x40;

/// Ids of the controls that sit on the page, not on a card: Save and Cancel, the status line of the
/// settings window, and the update window's own (1000 to 1099).
fn on_page(id: i32) -> bool {
    matches!(id, 1 | 2 | 150) || (1000..1100).contains(&id)
}

/// The update window's main button (Download and install, Try again): the accent colour.
const ID_PRIMARY: i32 = 1001;

/// Handles `WM_NOTIFY`: paints check boxes, radio buttons, push buttons and sliders. `None` leaves
/// the message to Windows.
pub fn custom_draw(hwnd: HWND, lparam: LPARAM) -> Option<LRESULT> {
    let nm = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };
    if nm.hdr.code != NM_CUSTOMDRAW_CODE || nm.dwDrawStage != NMCUSTOMDRAW_DRAW_STAGE(CDDS_PREPAINT_STAGE) {
        return None;
    }
    let mut class = [0u16; 32];
    let n = unsafe { GetClassNameW(nm.hdr.hwndFrom, &mut class) }.max(0) as usize;
    let class = String::from_utf16_lossy(&class[..n]);
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let drawn = with(|look| match class.as_str() {
        "Button" => draw_button(nm, look, dpi),
        "msctls_trackbar32" => draw_slider(nm, look, dpi),
        _ => false,
    })
    .unwrap_or(false);
    drawn.then_some(LRESULT(CDRF_SKIPDEFAULT_RESULT))
}

fn draw_button(nm: &NMCUSTOMDRAW, look: &mut Look, dpi: u32) -> bool {
    let hwnd = nm.hdr.hwndFrom;
    let hdc = nm.hdc;
    let t = look.theme;
    let s = dpi as f32 / 96.0;
    let rc = nm.rc;
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let id = nm.hdr.idFrom as i32;
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    let kind = style & 0xF;
    let state = nm.uItemState.0;
    let (hot, pressed, disabled, focus) =
        (state & CDIS_HOT_BIT != 0, state & CDIS_SELECTED_BIT != 0, state & CDIS_DISABLED_BIT != 0, state & CDIS_FOCUS_BIT != 0);
    let checked = unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None) }.0 == 1;
    let text = text_of(hwnd);
    let page = on_page(id);
    unsafe {
        FillRect(hdc, &rc, if page { look.bg } else { look.card });
    }
    let font = look.font;
    match kind {
        // push buttons, and the default one (Save), which is filled with the accent colour
        0 | 1 => {
            let default = kind == 1 || id == ID_PRIMARY;
            let (fill, border, color) = if default {
                (if hot || pressed { t.accent_hot } else { t.accent }, None, t.on_accent)
            } else {
                (if pressed || hot { t.button_hot } else { t.button }, Some(t.button_border), t.text)
            };
            let (fill, color) = if disabled { (if default { t.track } else { t.button }, t.muted) } else { (fill, color) };
            blit(hdc, rc.left, rc.top, w, h, Shape::Box { radius: 6.0, fill: Some(fill), border }, s);
            if focus && !disabled {
                blit(hdc, rc.left, rc.top, w, h, Shape::Box { radius: 6.0, fill: None, border: Some(t.accent) }, s);
            }
            label(hdc, font, &text, rc, color, true, false);
            true
        }
        // check boxes and radio buttons: the mark, then the label
        2 | 3 | 4 | 9 => {
            let radio = kind == 4 || kind == 9;
            let size = (16.0 * s).round() as i32;
            let top = rc.top + (h - size) / 2;
            let shape = if radio {
                Shape::Radio { checked, accent: if hot { t.accent_hot } else { t.accent }, border: if hot { t.text } else { t.muted }, dot: t.on_accent }
            } else {
                Shape::Checkbox { checked, accent: if hot { t.accent_hot } else { t.accent }, border: if hot { t.text } else { t.muted }, tick: t.on_accent }
            };
            if disabled {
                blit(hdc, rc.left, top, size, size, Shape::Box { radius: 4.0, fill: None, border: Some(t.track) }, s);
            } else {
                blit(hdc, rc.left, top, size, size, shape, s);
            }
            if focus && !disabled {
                let pad = (2.0 * s) as i32;
                blit(hdc, rc.left - pad, top - pad, size + 2 * pad, size + 2 * pad, Shape::Box { radius: 6.0, fill: None, border: Some(t.accent) }, s);
            }
            let gap = (8.0 * s).round() as i32;
            let text_rc = RECT { left: rc.left + size + gap, top: rc.top, right: rc.right, bottom: rc.bottom };
            label(hdc, font, &text, text_rc, if disabled { t.muted } else { t.text }, false, true);
            true
        }
        _ => false,
    }
}

fn draw_slider(nm: &NMCUSTOMDRAW, look: &mut Look, dpi: u32) -> bool {
    let hwnd = nm.hdr.hwndFrom;
    let hdc = nm.hdc;
    let t = look.theme;
    let s = dpi as f32 / 96.0;
    unsafe {
        // the whole control, not only the part Windows asks about
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        FillRect(hdc, &rc, look.card);
        let mut channel = RECT::default();
        let mut thumb = RECT::default();
        SendMessageW(hwnd, TBM_GETCHANNELRECT, None, Some(LPARAM(&mut channel as *mut RECT as isize)));
        SendMessageW(hwnd, TBM_GETTHUMBRECT, None, Some(LPARAM(&mut thumb as *mut RECT as isize)));
        let enabled = IsWindowEnabled(hwnd).as_bool();
        let cy = (thumb.top + thumb.bottom) / 2;
        let cx = (thumb.left + thumb.right) / 2;
        let reach = (6.0 * s) as i32;
        let (left, right) = (channel.left - reach, channel.right + reach);
        let thick = (4.0 * s).round().max(2.0) as i32;
        let filled = if right > left { ((cx - left) as f32 / (right - left) as f32).clamp(0.0, 1.0) } else { 0.0 };
        blit(hdc, left, cy - thick / 2, right - left, thick, Shape::Track { filled, accent: if enabled { t.accent } else { t.track }, rest: t.track }, s);
        let d = (16.0 * s).round() as i32;
        let focused = GetFocus() == hwnd;
        let ring = if focused { t.accent_hot } else { t.card };
        blit(hdc, cx - d / 2, cy - d / 2, d, d, Shape::Thumb { accent: if enabled { t.accent } else { t.track }, ring }, s);
    }
    true
}

/// `WM_CTLCOLOR*`: the colours of labels, edit boxes and lists. `id` is the control's id.
pub fn control_colors(hdc: HDC, id: i32, kind: ControlKind) -> Option<LRESULT> {
    with(|look| {
        let t = look.theme;
        unsafe {
            SetBkMode(hdc, OPAQUE);
            match kind {
                ControlKind::Label => {
                    let (bg, brush) = if on_page(id) { (t.bg, look.bg) } else { (t.card, look.card) };
                    SetBkColor(hdc, colorref(bg));
                    SetTextColor(hdc, colorref(if is_muted(id) { t.muted } else { t.text }));
                    LRESULT(brush.0 as isize)
                }
                ControlKind::Field => {
                    SetBkColor(hdc, colorref(t.field));
                    SetTextColor(hdc, colorref(t.text));
                    LRESULT(look.field.0 as isize)
                }
            }
        }
    })
}

#[derive(Clone, Copy)]
pub enum ControlKind {
    Label,
    Field,
}

/// Labels drawn in the quieter colour: notes and values.
fn is_muted(id: i32) -> bool {
    matches!(id, 111..=115 | 134 | 150 | 160..=169 | 1011..=1019)
}

/// `v` pixels at `old` dpi as pixels at `new` dpi.
pub fn rescale(v: i32, old: u32, new: u32) -> i32 {
    (v as i64 * new as i64 / old.max(1) as i64) as i32
}

/// The monitor's scale changed from `old_dpi` to `new_dpi`: moves and resizes every control of `hwnd`
/// to match, gives each its font again at the new size, and returns the fonts replaced as (old, new)
/// so the caller can update what it holds. The old fonts are deleted.
pub fn rescale_children(hwnd: HWND, old_dpi: u32, new_dpi: u32) -> Vec<(HFONT, HFONT)> {
    unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> windows::core::BOOL {
        unsafe { (*(l.0 as *mut Vec<HWND>)).push(h) };
        windows::core::BOOL(1)
    }
    let mut children: Vec<HWND> = Vec::new();
    let font_of = |c: HWND| HFONT(unsafe { SendMessageW(c, WM_GETFONT, None, None) }.0 as *mut _);
    let mut map: Vec<(HFONT, HFONT)> = Vec::new();
    unsafe {
        let _ = EnumChildWindows(Some(hwnd), Some(collect), LPARAM(&mut children as *mut Vec<HWND> as isize));
        for &c in &children {
            let old = font_of(c);
            if old.is_invalid() || map.iter().any(|(o, _)| *o == old) {
                continue;
            }
            let mut lf = LOGFONTW::default();
            if GetObjectW(old.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut LOGFONTW as *mut _)) > 0 {
                lf.lfHeight = rescale(lf.lfHeight, old_dpi, new_dpi);
                map.push((old, CreateFontIndirectW(&lf)));
            }
        }
        for &c in &children {
            let old = font_of(c);
            if let Some((_, new)) = map.iter().find(|(o, _)| *o == old) {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(new.0 as usize)), Some(LPARAM(0)));
            }
            let mut r = RECT::default();
            let _ = GetWindowRect(c, &mut r);
            let mut top_left = POINT { x: r.left, y: r.top };
            let _ = ScreenToClient(hwnd, &mut top_left);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let _ = SetWindowPos(
                c,
                None,
                rescale(top_left.x, old_dpi, new_dpi),
                rescale(top_left.y, old_dpi, new_dpi),
                rescale(w, old_dpi, new_dpi),
                rescale(h, old_dpi, new_dpi),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        for (old, _) in &map {
            let _ = DeleteObject((*old).into());
        }
    }
    map
}

/// A picture (PNG bytes) of the window as it is drawn now: a debugging aid for looking at the windows.
pub fn capture_window(hwnd: HWND) -> Result<Vec<u8>, String> {
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let dc = GetDC(None);
        let mem = CreateCompatibleDC(Some(dc));
        let bmp = CreateCompatibleBitmap(dc, w, h);
        let old = SelectObject(mem, bmp.into());
        let ok = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(2)).as_bool();
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, dc);
        if !ok {
            return Err("PrintWindow failed".to_owned());
        }
        let mut rgba = Vec::with_capacity(buf.len());
        for p in buf.chunks_exact(4) {
            rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        let img = pasteroll_core::image::Rgba { width: w as u32, height: h as u32, pixels: rgba };
        pasteroll_core::image::encode_png(&img).ok_or_else(|| "could not encode the picture".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_themes_have_readable_text() {
        fn lum(c: Rgb) -> f32 {
            let f = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
        }
        let ratio = |a: Rgb, b: Rgb| {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        for t in [Theme::light(), Theme::dark()] {
            assert!(ratio(t.text, t.card) >= 7.0, "text on cards");
            assert!(ratio(t.muted, t.card) >= 4.5, "quiet text on cards");
            assert!(ratio(t.text, t.bg) >= 7.0, "text on the page");
            assert!(ratio(t.on_accent, t.accent) >= 4.5, "text on the accent colour");
            assert!(ratio(t.text, t.field) >= 7.0, "text in edit boxes");
        }
    }

    #[test]
    fn pixels_rescale_between_dpis() {
        assert_eq!(rescale(100, 96, 144), 150);
        assert_eq!(rescale(150, 144, 96), 100);
        assert_eq!(rescale(-12, 96, 192), -24, "a font height keeps its sign");
        assert_eq!(rescale(33, 96, 96), 33);
    }

    #[test]
    fn the_theme_can_be_forced_for_checking() {
        assert!(!Theme::light().dark && Theme::dark().dark);
        assert_ne!(Theme::light().card, Theme::dark().card);
    }
}
