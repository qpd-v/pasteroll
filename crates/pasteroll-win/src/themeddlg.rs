//! A small message window in the look of the settings and update windows: a heading, some text and a
//! few buttons, in Windows' light or dark colours. It replaces the stock dialogs of About and of the
//! one-time question about update checks. It runs on the calling thread and blocks until a button is
//! pressed or the window is closed, so call it from a thread of its own when the caller must not wait.

use crate::settingsui;
use std::cell::RefCell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;

const WIDTH: i32 = 460;
const MARGIN: i32 = 28;
const INNER: i32 = WIDTH - 2 * MARGIN;
const BUTTON_H: i32 = 32;
/// The main button is the accent-coloured one (`settingsui` knows this id); the others follow it.
const ID_FIRST: i32 = 1001;
const ID_HEADING: i32 = 1010;
const ID_BODY: i32 = 1011;

/// What to show. The first button is the main one (drawn in the accent colour, rightmost).
pub struct Spec<'a> {
    pub title: &'a str,
    pub heading: &'a str,
    pub body: &'a str,
    pub buttons: &'a [&'a str],
    /// The button that has the keyboard (the one Enter presses): a safe one, not necessarily the first.
    pub focus: usize,
}

struct Ctx {
    count: usize,
    chosen: Option<usize>,
    fonts: [HFONT; 2],
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn s(v: i32, dpi: u32) -> i32 {
    (v as i64 * dpi as i64 / 96) as i32
}

/// Roughly how many lines `text` takes at `per_line` characters a line.
fn lines(text: &str, per_line: usize) -> i32 {
    text.split('\n').map(|l| l.chars().count().max(1).div_ceil(per_line) as i32).sum::<i32>().max(1)
}

/// Where the buttons go: from the right edge, the first (main) one rightmost. DIP.
fn button_rects(labels: &[&str], y: i32) -> Vec<[i32; 4]> {
    let mut x = WIDTH - MARGIN;
    labels
        .iter()
        .map(|l| {
            let w = (l.chars().count() as i32 * 8 + 40).max(88);
            x -= w;
            let r = [x, y, w, BUTTON_H];
            x -= 8;
            r
        })
        .collect()
}

/// Shows the window and waits. `Some(i)` is button `i`; `None` is closed with Esc or the X.
pub fn show(spec: &Spec) -> Option<usize> {
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hbrBackground: HBRUSH::default(),
            hIcon: LoadIconW(Some(hinst.into()), PCWSTR(1usize as *const u16)).unwrap_or_default(),
            lpszClassName: w!("PasterollDialog"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let title = wide(spec.title);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("PasterollDialog"),
            PCWSTR(title.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            100,
            100,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        let dpi = GetDpiForWindow(hwnd).max(96);
        let make_font = |tenths: i32, weight: i32| {
            CreateFontW(
                -((tenths * dpi as i32) / 720),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        let (font, big) = (make_font(90, 400), make_font(140, 600));
        let theme = settingsui::Theme::current();
        settingsui::apply_title_bar(hwnd, theme.dark);
        settingsui::install(settingsui::Look::new(theme, font));

        let body_h = lines(spec.body, 62) * 17 + 4;
        let body_y = 64;
        let buttons_y = body_y + body_h + 22;
        let height = buttons_y + BUTTON_H + 24;
        let make_child = |class: PCWSTR, text: &str, style: u32, id: i32, r: [i32; 4], f: HFONT| {
            let t = wide(text);
            let h = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR(t.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
                s(r[0], dpi),
                s(r[1], dpi),
                s(r[2], dpi),
                s(r[3], dpi),
                Some(hwnd),
                Some(HMENU(id as usize as *mut _)),
                Some(hinst.into()),
                None,
            )
            .unwrap_or_default();
            SendMessageW(h, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1)));
            h
        };
        make_child(w!("STATIC"), spec.heading, 0x80 | 0x4000, ID_HEADING, [MARGIN, 22, INNER, 34], big);
        make_child(w!("STATIC"), spec.body, 0x80, ID_BODY, [MARGIN, body_y, INNER, body_h], font);
        let mut focus = None;
        for (i, (label, r)) in spec.buttons.iter().zip(button_rects(spec.buttons, buttons_y)).enumerate() {
            let h = make_child(w!("BUTTON"), label, BS_PUSHBUTTON as u32 | WS_TABSTOP.0, ID_FIRST + i as i32, r, font);
            if i == spec.focus {
                focus = Some(h);
            }
        }
        CTX.with(|c| *c.borrow_mut() = Some(Ctx { count: spec.buttons.len(), chosen: None, fonts: [font, big] }));

        let mut r = RECT { left: 0, top: 0, right: s(WIDTH, dpi), bottom: s(height, dpi) };
        let _ = AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let mut work = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let x = work.left + ((work.right - work.left) - ww) / 2;
        let y = work.top + ((work.bottom - work.top) - wh) / 3;
        let _ = SetWindowPos(hwnd, Some(HWND_TOP), x.max(work.left), y.max(work.top), ww, wh, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
        if let Some(f) = focus {
            let _ = SetFocus(Some(f));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let ctx = CTX.with(|c| c.borrow_mut().take());
        settingsui::remove();
        ctx.and_then(|c| {
            for f in c.fonts {
                let _ = DeleteObject(f.into());
            }
            c.chosen
        })
    }
}

fn paint(hwnd: HWND, hdc: HDC) {
    settingsui::with(|look| unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        FillRect(hdc, &rc, look.bg);
    });
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => {
                paint(hwnd, HDC(wparam.0 as *mut _));
                LRESULT(1)
            }
            WM_CTLCOLORSTATIC => {
                let id = GetDlgCtrlID(HWND(lparam.0 as *mut _));
                settingsui::control_colors(HDC(wparam.0 as *mut _), id, settingsui::ControlKind::Label)
                    .unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
            }
            WM_NOTIFY => settingsui::custom_draw(hwnd, lparam).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam)),
            WM_COMMAND => {
                // a press has notification code 0 (Esc arrives as IDCANCEL, code 0, and closes)
                if (wparam.0 >> 16) & 0xFFFF == 0 {
                    let id = (wparam.0 & 0xFFFF) as i32;
                    let count = CTX.with(|c| c.borrow().as_ref().map_or(0, |x| x.count)) as i32;
                    if (ID_FIRST..ID_FIRST + count).contains(&id) {
                        CTX.with(|c| {
                            if let Some(x) = c.borrow_mut().as_mut() {
                                x.chosen = Some((id - ID_FIRST) as usize);
                            }
                        });
                    }
                    if id == 2 || (ID_FIRST..ID_FIRST + count).contains(&id) {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                let name = if lparam.0 == 0 { String::new() } else { PCWSTR(lparam.0 as *const u16).to_string().unwrap_or_default() };
                if name == "ImmersiveColorSet" {
                    let theme = settingsui::Theme::current();
                    settingsui::with(|l| l.retheme(theme));
                    settingsui::apply_title_bar(hwnd, theme.dark);
                    let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Debug aid: opens the window on `what` (`about` or `question`), saves a picture to `out` and closes it.
pub fn snapshot(what: &str, out: &std::path::Path) -> Result<(), String> {
    let (title, heading, body, buttons, focus): (&str, String, String, &[&str], usize) = match what {
        "about" => ("About Pasteroll", crate::about::heading(), crate::about::body(), &["OK"], 0),
        "question" => (
            "Pasteroll updates",
            "Let Pasteroll check for updates?".into(),
            crate::update::QUESTION.to_owned(),
            &["Check for updates", "Never check"],
            1,
        ),
        other => return Err(format!("unknown dialog {other}")),
    };
    let out = out.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    let opened = std::thread::spawn(move || {
        let spec = Spec { title, heading: &heading, body: &body, buttons, focus };
        let _ = tx.send(());
        show(&spec)
    });
    let _ = rx.recv();
    std::thread::sleep(std::time::Duration::from_millis(900));
    // find the window this thread opened
    let h = unsafe { FindWindowW(w!("PasterollDialog"), PCWSTR::null()) }.map_err(|e| e.to_string())?;
    let picture = settingsui::capture_window(h);
    unsafe {
        let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    let _ = opened.join();
    std::fs::write(out, picture?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_sit_at_the_right_edge_without_touching() {
        let rects = button_rects(&["Check for updates", "Never check"], 100);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0][0] + rects[0][2], WIDTH - MARGIN, "the main button is rightmost");
        assert!(rects[1][0] + rects[1][2] < rects[0][0], "the second is to its left with a gap");
        assert!(rects.iter().all(|r| r[0] >= MARGIN && r[2] >= 88));
    }

    #[test]
    fn line_estimates_count_each_paragraph() {
        assert_eq!(lines("short", 62), 1);
        assert_eq!(lines("a\n\nb", 62), 3);
        assert_eq!(lines(&"x".repeat(125), 62), 3);
    }
}
