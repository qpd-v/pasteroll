//! Overlay self-test: does the whole path work with *real* input and without stealing the foreground
//! from the app underneath? Real mouse and keyboard events go through the overlay window, the
//! controller and the portable renderer, exactly as in the app, but with no global hooks.
//!
//! Safety rules for running on a live desktop:
//! - the only window that receives synthetic input is one we created, and we check
//!   `WindowFromPoint` / `GetForegroundWindow` immediately before every injection;
//! - the first part installs no hooks; the typing part (`probe_typing`) installs the real keyboard
//!   hook for about three seconds, with our own window in the foreground, and the system clipboard
//!   is never read or written;
//! - runs disturbed by a person typing or moving the pointer are flagged and voided;
//! - the whole run is a couple of seconds and restores the cursor position.

use crate::{app, input};
use pasteroll_app::{Event, Key, Platform};
use pasteroll_core::image::{encode_png, Rgba};
use std::time::{Duration, Instant};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_GETSEL;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    unsafe {
        while Instant::now() < end {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            std::thread::sleep(Duration::from_millis(4));
        }
    }
}

fn root(h: HWND) -> HWND {
    unsafe { GetAncestor(h, GA_ROOT) }
}

fn edit_text(edit: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(edit) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(edit, &mut buf) as usize;
        String::from_utf16_lossy(&buf[..got])
    }
}

unsafe extern "system" fn target_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        // An Alt tap (used to unlock SetForegroundWindow) would otherwise open this window's system
        // menu in a modal loop inside DefWindowProc and block the whole probe.
        if m == WM_SYSCOMMAND && (w.0 & 0xFFF0) == SC_KEYMENU as usize {
            return LRESULT(0);
        }
        if m == WM_SYSKEYUP || m == WM_SYSKEYDOWN {
            return LRESULT(0);
        }
        DefWindowProcW(h, m, w, l)
    }
}

fn make_target() -> Result<(HWND, HWND)> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(target_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                (windows::Win32::Graphics::Gdi::COLOR_WINDOW.0 + 1) as usize as *mut _,
            ),
            lpszClassName: w!("PasterollProbeTarget"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let top = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("PasterollProbeTarget"),
            w!("Pasteroll probe: target"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            120,
            120,
            560,
            300,
            None,
            None,
            Some(hinst.into()),
            None,
        )?;
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
            10,
            10,
            520,
            230,
            Some(top),
            None,
            Some(hinst.into()),
            None,
        )?;
        Ok((top, edit))
    }
}

/// Brings `h` to the foreground. Windows blocks this from a background process, so tap Alt first
/// (the documented "we received input" unlock) and verify rather than trust.
fn make_foreground(h: HWND) -> bool {
    unsafe {
        for _ in 0..5 {
            if root(GetForegroundWindow()) == h {
                return true; // Windows already gave it to us; no need to tap Alt
            }
            let alt = |up: bool| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_MENU,
                        wScan: 0,
                        dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            SendInput(&[alt(false), alt(true)], size_of::<INPUT>() as i32);
            let _ = SetForegroundWindow(h);
            pump(120);
            if root(GetForegroundWindow()) == h {
                return true;
            }
        }
        false
    }
}

/// True when the pointer is exactly where we last put it. A mismatch means the user (or another
/// program) moved it, and a failed check afterwards is interference rather than a defect.
fn cursor_at(p: POINT) -> bool {
    let mut now = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut now);
    }
    let same = now.x == p.x && now.y == p.y;
    if !same {
        eprintln!("  .. pointer expected ({}, {}) but is at ({}, {})", p.x, p.y, now.x, now.y);
    }
    same
}

pub fn run() -> i32 {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    match probe() {
        Ok(checks) => {
            let mut failed = 0;
            for c in &checks {
                println!("{:<4} {:<30} {}", if c.ok { "PASS" } else { "FAIL" }, c.name, c.detail);
                if !c.ok {
                    failed += 1;
                }
            }
            println!("{} checks, {} failed", checks.len(), failed);
            i32::from(failed > 0)
        }
        Err(e) => {
            eprintln!("probe aborted: {e}");
            3
        }
    }
}

fn probe() -> Result<Vec<Check>> {
    let mut out: Vec<Check> = Vec::new();
    let mut cursor = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cursor);
    }
    let result = probe_inner(&mut out).and_then(|()| probe_typing(&mut out));
    // restore the user's pointer whatever happened
    input::move_to(cursor);
    result?;
    Ok(out)
}

const TEXT: &str = "The quick brown fox jumps over the lazy dog.\r\nSecond line: carousel test.";

fn find(needle: &str) -> usize {
    let hay: Vec<u16> = TEXT.encode_utf16().collect();
    let n: Vec<u16> = needle.encode_utf16().collect();
    hay.windows(n.len()).position(|w| w == n.as_slice()).expect("needle in text")
}

fn screen_point(px: (f32, f32)) -> Option<POINT> {
    let r = app::with_runtime(|rt| rt.platform.overlay.screen_rect())?;
    Some(POINT { x: r.left + px.0 as i32, y: r.top + px.1 as i32 })
}

fn probe_inner(out: &mut Vec<Check>) -> Result<()> {
    macro_rules! check {
        ($name:expr, $ok:expr, $($detail:tt)*) => {
            out.push(Check { name: $name, ok: $ok, detail: format!($($detail)*) })
        };
    }

    // A private history: one image (older) and one text clip (newest).
    let dir = std::env::temp_dir().join(format!("pasteroll-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = dir.join("history.db");
    {
        let mut store = crate::keystore::open_store(&db).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
        let red = Rgba { width: 100, height: 60, pixels: [255u8, 0, 0, 255].repeat(100 * 60) };
        // real timestamps: start-up maintenance expires anything older than the age limit
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        store.add_image_png(&encode_png(&red).unwrap(), now).ok();
        store.add_text(TEXT, now).ok();
    }
    // PASTEROLL_PROBE_SIZE=150 runs the whole probe at that carousel size
    let probe_size = std::env::var("PASTEROLL_PROBE_SIZE").ok();
    if let Some(sz) = &probe_size {
        let _ = std::fs::write(dir.join("config.toml"), format!("[appearance]
size = {sz}
"));
    }
    let opts = app::Options { db: db.clone(), capture: false, accept_injected: false, max_seconds: None, hooks: false, tray: false, notice: false };
    let _msg_window = app::create(&opts)?;

    // the probe's target window is ours, so the platform must be told that is allowed
    app::with_runtime(|rt| rt.platform.allow_own_process = true);
    let (target, edit) = make_target()?;
    let fg_ok = make_foreground(target);
    check!("target_is_foreground", fg_ok, "precondition");
    if !fg_ok {
        // Without it, any input we inject could reach the user's real foreground window. Stop.
        unsafe {
            let _ = DestroyWindow(target);
        }
        app::destroy();
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }
    unsafe {
        let _ = SetFocus(Some(edit));
    }

    // --- open the carousel exactly as the activation gesture would
    app::dispatch(Event::Activate);
    pump(250);
    if probe_size.is_some() {
        if let Some(r) = app::with_runtime(|rt| rt.platform.overlay.screen_rect()) {
            check!("overlay_size_reported", true, "{} x {}", r.right - r.left, r.bottom - r.top);
        }
    }
    // --- the settings window's size slider ends up in the file Save would write
    if std::env::var("PASTEROLL_PROBE_SETTINGS").is_ok() {
        crate::settings::open();
        pump(300);
        let moved = crate::settings::probe_set_size_slider(8); // 50 60 70 80 90 100 110 125 150 ...
        let text = crate::settings::pending_text().unwrap_or_default();
        let ok = moved && pasteroll_core::config::parse(&text).map(|c| c.size_percent == 150).unwrap_or(false);
        check!("settings_slider_writes_size", ok, "moved={moved} text={:?}", text.lines().filter(|l| l.contains("size")).collect::<Vec<_>>());
    }
    let (open, visible, text_ok) = app::with_runtime(|rt| {
        (
            rt.controller.is_open(),
            rt.platform.overlay.is_visible(),
            rt.controller.displayed_text().as_deref() == Some(TEXT),
        )
    })
    .unwrap_or((false, false, false));
    check!("session_opens", open, "controller open");
    check!("overlay_visible", visible, "");
    check!("newest_clip_is_shown", text_ok, "");
    let fg = unsafe { GetForegroundWindow() };
    check!("show_keeps_foreground", root(fg) == target, "foreground after show: {:?} (target {:?})", fg, target);

    let overlay_hwnd = app::with_runtime(|rt| rt.platform.overlay.hwnd()).unwrap_or_default();

    // --- drag selection with real mouse input
    let q = find("quick brown");
    let pa = app::with_runtime(|rt| rt.controller.char_centre_px(q)).flatten();
    let pb = app::with_runtime(|rt| rt.controller.char_centre_px(q + 11)).flatten();
    if let (Some(pa), Some(pb)) = (pa, pb) {
        // left edge of the character: two pixels left of its centre-line start
        let a = screen_point((pa.0 - 2.0, pa.1));
        let b = screen_point((pb.0 - 2.0, pb.1));
        if let (Some(a), Some(b)) = (a, b) {
            let hit_a = unsafe { WindowFromPoint(a) };
            let hit_b = unsafe { WindowFromPoint(b) };
            let aimed = hit_a == overlay_hwnd && hit_b == overlay_hwnd;
            check!("drag_aim_is_overlay", aimed, "start {:?}, end {:?}", hit_a, hit_b);
            if aimed {
                input::move_to(a);
                pump(30);
                let at_start = cursor_at(a);
                input::left_down();
                pump(30);
                let mid = POINT { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
                input::move_to(mid);
                pump(30);
                let at_mid = cursor_at(mid);
                input::move_to(b);
                pump(30);
                let steady = at_start && at_mid && cursor_at(b);
                input::left_up();
                pump(120);
                let sel = app::with_runtime(|rt| rt.controller.selected_text()).flatten();
                check!(
                    "drag_selects_range",
                    sel.as_deref() == Some("quick brown") || !steady,
                    "selected {:?}{}",
                    sel,
                    if steady { "" } else { "  [INTERFERENCE: pointer moved during drag, result void]" }
                );
                let fg = unsafe { GetForegroundWindow() };
                check!("drag_keeps_foreground", root(fg) == target, "foreground {:?}", fg);
            }
        }
    } else {
        check!("drag_aim_is_overlay", false, "no layout geometry");
    }

    // --- double click selects a word
    let l = find("lazy");
    let pl = app::with_runtime(|rt| rt.controller.char_centre_px(l + 2)).flatten();
    if let Some(p) = pl.and_then(screen_point) {
        if unsafe { WindowFromPoint(p) } == overlay_hwnd {
            input::move_to(p);
            pump(30);
            let steady = cursor_at(p);
            input::left_down();
            input::left_up();
            input::left_down();
            input::left_up();
            pump(150);
            let sel = app::with_runtime(|rt| rt.controller.selected_text()).flatten();
            check!(
                "double_click_selects_word",
                sel.as_deref() == Some("lazy") || !steady,
                "selected {:?}{}",
                sel,
                if steady { "" } else { "  [INTERFERENCE: pointer moved before click, result void]" }
            );
        } else {
            check!("double_click_selects_word", false, "aim missed overlay");
        }
    }

    // --- the target still gets keystrokes while the overlay is up
    let fg = unsafe { GetForegroundWindow() };
    if root(fg) == target {
        // The probe window is foreground for a couple of seconds, so a person typing at the same
        // time types into it. Anything in the box before our key is not ours: void, don't fail.
        let before = edit_text(edit);
        let clean = before.is_empty();
        input::send_char('x');
        pump(120);
        let t = edit_text(edit);
        check!(
            "keys_reach_target",
            t.ends_with('x') && (clean || !t.is_empty()),
            "target text {:?}{}",
            t,
            if clean { "" } else { "  [INTERFERENCE: other keystrokes reached the target first, result void]" }
        );
        // chord injection: Shift+Left extends the selection over the 'x' (arrows need the
        // extended-key flag, or NumLock turns Shift+Left into Shift+Numpad4)
        input::send_chord(&[VK_SHIFT], VK_LEFT);
        pump(120);
        let (mut s, mut e) = (0u32, 0u32);
        unsafe {
            SendMessageW(
                edit,
                EM_GETSEL,
                Some(WPARAM(&mut s as *mut u32 as usize)),
                Some(LPARAM(&mut e as *mut u32 as isize)),
            );
        }
        check!(
            "chord_reaches_target",
            (s == 0 && e == 1) || !clean,
            "Shift+Left -> EM_GETSEL = ({s}, {e}){}",
            if clean { "" } else { "  [INTERFERENCE: result void]" }
        );
    } else {
        check!("keys_reach_target", false, "foreground lost: {:?}", fg);
    }

    // --- step to the image clip: a thumbnail is actually drawn (read back from the screen)
    {
        let gen = app::with_runtime(|rt| rt.platform.input_generation()).unwrap_or(0);
        app::dispatch(Event::Key(Key::Down, gen)); // wheel down = older clip = the image
        let mut drawn = (0u32, 0u32, 0u32);
        for _ in 0..100 {
            pump(30);
            let centre = app::with_runtime(|rt| {
                let r = rt.platform.overlay.screen_rect();
                // centre of the body area: horizontally centred, in the lower part of the overlay
                (r.left + (r.right - r.left) / 2, r.top + (r.bottom - r.top) * 55 / 100)
            });
            if let Some((cx, cy)) = centre {
                drawn = unsafe {
                    let dc = GetDC(None);
                    let c = GetPixel(dc, cx, cy).0;
                    ReleaseDC(None, dc);
                    (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF)
                };
                if drawn.0 > 200 && drawn.1 < 60 && drawn.2 < 60 {
                    break;
                }
            }
        }
        check!(
            "image_is_drawn",
            drawn.0 > 200 && drawn.1 < 60 && drawn.2 < 60,
            "body pixel {:?} (expected red)",
            drawn
        );
        let fg = unsafe { GetForegroundWindow() };
        check!("image_keeps_foreground", root(fg) == target, "foreground {:?}", fg);
    }

    // --- close
    let gen = app::with_runtime(|rt| rt.platform.input_generation()).unwrap_or(0);
    app::dispatch(Event::Key(Key::Escape, gen));
    pump(80);
    let closed = app::with_runtime(|rt| !rt.controller.is_open() && !rt.platform.overlay.is_visible())
        .unwrap_or(false);
    check!("escape_closes", closed, "");
    let fg = unsafe { GetForegroundWindow() };
    check!("close_keeps_foreground", root(fg) == target, "foreground {:?}", fg);

    unsafe {
        let _ = DestroyWindow(target);
    }
    app::destroy();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn query_now() -> Option<String> {
    app::with_runtime(|rt| rt.controller.query().map(str::to_owned)).flatten()
}

fn dropdown_open() -> bool {
    app::with_runtime(|rt| rt.controller.view_model().dropdown.is_some()).unwrap_or(false)
}

/// Typing into the search box with the real keyboard hook: keys must be translated and swallowed
/// while a session is live (the target must not see them), and reach the target again afterwards.
fn probe_typing(out: &mut Vec<Check>) -> Result<()> {
    macro_rules! check {
        ($name:expr, $ok:expr, $($detail:tt)*) => {
            out.push(Check { name: $name, ok: $ok, detail: format!($($detail)*) })
        };
    }
    let dir = std::env::temp_dir().join(format!("pasteroll-probe-typing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = dir.join("history.db");
    {
        let mut store = crate::keystore::open_store(&db).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        store.add_text("alpha one", now).ok();
        store.add_text("beta two", now + 1).ok();
        store.add_text("alpha three", now + 2).ok();
    }
    let opts = app::Options { db: db.clone(), capture: false, accept_injected: true, max_seconds: None, hooks: true, tray: false, notice: false };
    let _msg_window = app::create(&opts)?;
    app::with_runtime(|rt| rt.platform.allow_own_process = true);
    let (target, edit) = make_target()?;
    let fg_ok = make_foreground(target);
    check!("typing_target_is_foreground", fg_ok, "precondition");
    if !fg_ok {
        unsafe {
            let _ = DestroyWindow(target);
        }
        app::destroy();
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }
    unsafe {
        let _ = SetFocus(Some(edit));
    }
    app::dispatch(Event::Activate);
    pump(400); // the keyboard hook is installed on its own thread after the session opens
    let open = app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false);
    check!("typing_session_opens", open, "");
    let clean = edit_text(edit).is_empty();
    let note = if clean { "" } else { "  [INTERFERENCE: the target already had text]" };

    // letters are translated and swallowed
    for c in ['a', 'l', 'p'] {
        input::send_char(c);
        pump(40);
    }
    pump(80);
    let q = query_now();
    let leaked = edit_text(edit);
    check!("typed_letters_reach_the_search_box", q.as_deref() == Some("alp"), "query {:?}{}", q, note);
    check!("typed_letters_do_not_reach_the_target", leaked.is_empty(), "target text {:?}", leaked);
    let counts = app::with_runtime(|rt| rt.controller.match_counts()).flatten();
    check!("search_narrows_the_list", counts == Some((2, 3)), "matches {:?}", counts);

    // shift gives upper case, digits and space come through
    input::send_chord(&[VK_SHIFT], VIRTUAL_KEY(b'B' as u16));
    pump(60);
    input::send_chord(&[], VIRTUAL_KEY(b'1' as u16));
    pump(60);
    input::send_chord(&[], VK_SPACE);
    pump(80);
    let q = query_now();
    check!("shift_digit_and_space", q.as_deref() == Some("alpB1 "), "query {:?}", q);

    // Backspace and Ctrl+Backspace
    input::send_chord(&[], VK_BACK);
    pump(60);
    input::send_chord(&[], VK_BACK);
    pump(60);
    let q = query_now();
    check!("backspace_erases", q.as_deref() == Some("alpB"), "query {:?}", q);
    input::send_chord(&[VK_CONTROL], VK_BACK);
    pump(80);
    let q = query_now();
    check!("ctrl_backspace_erases_the_word", q.as_deref() == Some(""), "query {:?}", q);

    // Tab opens the dropdown, Escape closes just it
    input::send_chord(&[], VK_TAB);
    pump(80);
    check!("tab_opens_the_dropdown", dropdown_open(), "");
    input::send_chord(&[], VK_ESCAPE);
    pump(80);
    let still_open = app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false);
    check!("escape_closes_only_the_dropdown", !dropdown_open() && still_open, "");

    // Ctrl+letter must be swallowed and ignored, not turned into text or sent to the target
    input::send_chord(&[VK_CONTROL], VIRTUAL_KEY(b'C' as u16));
    pump(80);
    let q = query_now();
    let leaked = edit_text(edit);
    check!("ctrl_letter_is_ignored", q.as_deref() == Some("") && leaked.is_empty(), "query {:?}, target {:?}", q, leaked);

    // Escape closes; afterwards typing goes to the target again
    input::send_chord(&[], VK_ESCAPE);
    pump(150);
    let closed = app::with_runtime(|rt| !rt.controller.is_open()).unwrap_or(false);
    check!("typing_escape_closes", closed, "");
    pump(300); // the hook is removed once its keys are released
    input::send_char('x');
    pump(150);
    let t = edit_text(edit);
    check!("typing_after_close_reaches_target", t.ends_with('x'), "target text {:?}", t);

    unsafe {
        let _ = DestroyWindow(target);
    }
    app::destroy();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
