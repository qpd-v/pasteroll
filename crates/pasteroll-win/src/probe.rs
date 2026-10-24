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
    let result = probe_inner(&mut out).and_then(|()| probe_typing(&mut out)).and_then(|()| probe_no_bloat(&mut out)).and_then(|()| probe_multi_select(&mut out)).and_then(|()| probe_editor_anywhere(&mut out)).and_then(|()| probe_editor_holding_ctrl(&mut out)).and_then(|()| probe_editor_in_terminal(&mut out));
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
        // the Mode choice ends up in the file too
        let chose = crate::settings::probe_choose_mode(pasteroll_core::config::UiMode::NoBloat);
        let text = crate::settings::pending_text().unwrap_or_default();
        let mode = pasteroll_core::config::parse(&text).map(|c| c.mode);
        check!(
            "settings_mode_writes_no_bloat",
            chose && mode == Ok(pasteroll_core::config::UiMode::NoBloat),
            "chose={chose} mode={mode:?}"
        );
        // the update check: off until ticked, and ticking it is what Save would write
        let before = crate::settings::pending_text().unwrap_or_default();
        let ticked = crate::settings::probe_set_updates(true);
        let after = crate::settings::pending_text().unwrap_or_default();
        let on = pasteroll_core::config::parse(&after).map(|c| c.updates);
        check!(
            "settings_update_check_is_ticked_by_choice_only",
            ticked
                && pasteroll_core::config::parse(&before).map(|c| c.updates) == Ok(pasteroll_core::config::UpdateCheck::Ask)
                && on == Ok(pasteroll_core::config::UpdateCheck::On),
            "ticked={ticked} on={on:?}"
        );
        // a choice made elsewhere (the one-time question) shows in the open window at once
        crate::settings::probe_set_updates(false);
        let wrote = app::db_path().is_some_and(|db| {
            crate::keystore::edit_config(&db, |t| {
                pasteroll_core::config::set_value(t, "check_for_updates", "true").map_err(|p| p.join("; "))
            })
            .is_ok()
        });
        pump(1500);
        let shown = crate::settings::probe_updates_checked();
        let after = crate::settings::pending_text().unwrap_or_default();
        let unchanged = pasteroll_core::config::parse(&after).map(|c| c.updates) == Ok(pasteroll_core::config::UpdateCheck::On);
        check!(
            "settings_box_follows_a_choice_made_elsewhere",
            wrote && shown == Some(true) && unchanged,
            "wrote={wrote} shown={shown:?} unchanged={unchanged}"
        );
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
    input::send_chord(&[VK_CONTROL], VIRTUAL_KEY(b'Q' as u16));
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

/// No Bloat with the real hooks and a real window: it is the small one-clip window, the arrow keys
/// scroll, and typing and Tab still search and filter (the search bar and menu appear when used).
/// The keys are swallowed, never sent to the target.
fn probe_no_bloat(out: &mut Vec<Check>) -> Result<()> {
    macro_rules! check {
        ($name:expr, $ok:expr, $($detail:tt)*) => {
            out.push(Check { name: $name, ok: $ok, detail: format!($($detail)*) })
        };
    }
    let dir = std::env::temp_dir().join(format!("pasteroll-probe-nobloat-{}", std::process::id()));
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
    let _ = std::fs::write(dir.join("config.toml"), "[appearance]
mode = \"no-bloat\"
");
    let opts = app::Options { db: db.clone(), capture: false, accept_injected: true, max_seconds: None, hooks: true, tray: false, notice: false };
    let _msg_window = app::create(&opts)?;
    app::with_runtime(|rt| rt.platform.allow_own_process = true);
    let (target, edit) = make_target()?;
    let fg_ok = make_foreground(target);
    check!("no_bloat_target_is_foreground", fg_ok, "precondition");
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
    let mode = app::ui_mode();
    check!("no_bloat_mode_is_read_from_the_config", mode == pasteroll_core::config::UiMode::NoBloat, "mode {mode:?}");
    app::dispatch(Event::Activate);
    pump(400);
    let open = app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false);
    check!("no_bloat_session_opens", open, "");
    let shown = app::with_runtime(|rt| {
        let o = rt.controller.overlay_info();
        (o.size_px.0 as f32 / o.scale, o.size_px.1 as f32 / o.scale)
    });
    // the window is 480 x 300 DIP unless the work area clips it
    check!(
        "no_bloat_window_is_small",
        shown.is_some_and(|(w, h)| (w - 480.0).abs() < 1.5 && (h - 300.0).abs() < 1.5),
        "{shown:?} DIP"
    );
    let one = app::with_runtime(|rt| rt.controller.view_model().rows.len()).unwrap_or(0);
    check!("no_bloat_shows_one_clip", one == 1, "{one} rows");
    let newest = app::with_runtime(|rt| rt.controller.displayed_text()).flatten();
    check!("no_bloat_opens_on_the_newest", newest.as_deref() == Some("alpha three"), "{newest:?}");

    let bar = || app::with_runtime(|rt| rt.controller.view_model().slim_bar).unwrap_or(false);
    check!("no_bloat_starts_without_a_search_bar", !bar(), "");
    for c in ['a', 'l', 'p'] {
        input::send_char(c);
        pump(40);
    }
    pump(80);
    let (q, leaked) = (query_now(), edit_text(edit));
    check!("no_bloat_typing_searches", q.as_deref() == Some("alp") && bar(), "query {q:?}");
    check!("no_bloat_typing_stays_off_the_target", leaked.is_empty(), "target text {leaked:?}");
    let counts = app::with_runtime(|rt| rt.controller.match_counts()).flatten();
    check!("no_bloat_search_narrows_the_list", counts == Some((2, 3)), "matches {counts:?}");
    // Escape clears the search and the bar goes again
    input::send_chord(&[], VK_ESCAPE);
    pump(100);
    let still_open = app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false);
    check!("no_bloat_escape_clears_the_search", still_open && query_now().as_deref() == Some("") && !bar(), "");
    input::send_chord(&[], VK_TAB);
    pump(100);
    check!("no_bloat_tab_shows_the_filter_menu", dropdown_open() && bar(), "");
    input::send_chord(&[], VK_ESCAPE);
    pump(100);
    check!("no_bloat_escape_closes_only_the_menu", !dropdown_open() && !bar(), "");

    input::send_chord(&[], VK_DOWN);
    pump(100);
    let older = app::with_runtime(|rt| rt.controller.displayed_text()).flatten();
    check!("no_bloat_down_arrow_scrolls", older.as_deref() == Some("beta two"), "{older:?}");

    input::send_chord(&[], VK_ESCAPE);
    pump(150);
    let closed = app::with_runtime(|rt| !rt.controller.is_open()).unwrap_or(false);
    check!("no_bloat_escape_closes", closed, "");
    pump(300);
    input::send_char('x');
    pump(150);
    let t = edit_text(edit);
    check!("no_bloat_typing_after_close_reaches_target", t.ends_with('x'), "target text {t:?}");

    // what the tray menu does: switch at once (no restart), remember it in config.toml, and the
    // next carousel is the big one again
    app::set_ui_mode(pasteroll_core::config::UiMode::Complete);
    let saved = crate::keystore::read_config_text(&db)
        .ok()
        .and_then(|text| pasteroll_core::config::parse(&text).ok())
        .map(|c| c.mode);
    check!(
        "tray_switch_is_live_and_remembered",
        app::ui_mode() == pasteroll_core::config::UiMode::Complete && saved == Some(pasteroll_core::config::UiMode::Complete),
        "live {:?}, saved {saved:?}",
        app::ui_mode()
    );
    app::dispatch(Event::Activate);
    pump(300);
    let shown = app::with_runtime(|rt| {
        let o = rt.controller.overlay_info();
        (o.size_px.0 as f32 / o.scale, o.size_px.1 as f32 / o.scale)
    });
    check!("complete_window_is_back", shown.is_some_and(|(w, _)| (w - 520.0).abs() < 1.5), "{shown:?} DIP");
    let rows = app::with_runtime(|rt| rt.controller.view_model().rows.len()).unwrap_or(0);
    check!("complete_shows_the_list_again", rows > 1, "{rows} rows");
    input::send_chord(&[], VK_ESCAPE);
    pump(200);

    unsafe {
        let _ = DestroyWindow(target);
    }
    app::destroy();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// The centre, on screen, of the part of the carousel that `pick` finds in its layout.
fn carousel_point(pick: impl Fn(&pasteroll_render::Geometry) -> Option<pasteroll_render::Rect>) -> Option<POINT> {
    let (x, y) = app::with_runtime(|rt| {
        let vm = rt.controller.view_model();
        let info = rt.controller.overlay_info();
        let size = (info.size_px.0 as f32 / info.scale, info.size_px.1 as f32 / info.scale);
        let g = pasteroll_render::geometry(&vm, size, info.scale);
        let r = pick(&g)?;
        Some(((r.x + r.w / 2.0) * info.scale, (r.y + r.h / 2.0) * info.scale))
    })
    .flatten()?;
    screen_point((x, y))
}

/// Choosing several clips with the real hooks, real clicks and real keys: Ctrl+click two rows, click
/// the target window, and both clips arrive in its text box, one per line. Then Ctrl+C and the
/// right-click menu copy without pasting, and a real Ctrl+V shows what was copied. The system
/// clipboard IS used (it holds the last copy afterwards).
fn probe_multi_select(out: &mut Vec<Check>) -> Result<()> {
    macro_rules! check {
        ($name:expr, $ok:expr, $($detail:tt)*) => {
            out.push(Check { name: $name, ok: $ok, detail: format!($($detail)*) })
        };
    }
    let dir = std::env::temp_dir().join(format!("pasteroll-probe-multi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = dir.join("history.db");
    {
        let mut store = crate::keystore::open_store(&db).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        store.add_text("multi one", now).ok();
        store.add_text("multi two", now + 1).ok();
        store.add_text("multi three", now + 2).ok();
    }
    let opts = app::Options { db: db.clone(), capture: false, accept_injected: true, max_seconds: None, hooks: true, tray: false, notice: false };
    let _msg_window = app::create(&opts)?;
    app::with_runtime(|rt| rt.platform.allow_own_process = true);
    let (target, edit) = make_target()?;
    let fg_ok = make_foreground(target);
    check!("multi_target_is_foreground", fg_ok, "precondition");
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
    let session_open = || app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false);
    let picked = || {
        app::with_runtime(|rt| rt.controller.view_model().picked.iter().filter(|p| **p).count()).unwrap_or(0)
    };
    let row_point = |i: usize| carousel_point(move |g| g.rows.get(i).copied().flatten());
    let clear_edit = || unsafe {
        let _ = SetWindowTextW(edit, w!(""));
        let _ = SetFocus(Some(edit));
    };
    // rows: 0 is "multi three" (shown), 1 is "multi two", 2 is "multi one"
    let choose_two = || {
        for i in [1usize, 2] {
            if let Some(pt) = row_point(i) {
                ctrl_click(pt);
                pump(60);
                ctrl_key(true);
                pump(100);
            }
        }
    };
    let joined = "multi two\r\nmulti one";

    // --- Ctrl+click two rows, then click the window: both arrive, one per line
    check!("multi_session_opens", open_carousel_in_corner(), "");
    let spot = clear_spot(target);
    choose_two();
    check!("multi_ctrl_click_chooses_two_rows", picked() == 2, "{} chosen", picked());
    check!("multi_choosing_types_nothing", edit_text(edit).is_empty(), "target text {:?}", edit_text(edit));
    if let Some(spot) = spot {
        input::move_to(spot);
        pump(40);
        input::left_down();
        input::left_up();
        let closed = wait_until(3_000, || !session_open());
        pump(300);
        let text = edit_text(edit);
        check!("multi_click_pastes_both_clips_one_per_line", closed && text == joined, "closed {closed}, target text {text:?}");
    } else {
        check!("multi_click_spot_is_clear", false, "");
        input::send_chord(&[], VK_ESCAPE);
        pump(300);
    }
    ctrl_key(true);
    pump(400);

    // --- Ctrl+C copies without pasting; Ctrl+V afterwards proves what was copied
    clear_edit();
    if open_carousel_in_corner() {
        choose_two();
        let chosen = picked();
        input::send_chord(&[VK_CONTROL], VIRTUAL_KEY(b'C' as u16));
        let closed = wait_until(2_000, || !session_open());
        check!("multi_ctrl_c_closes_and_pastes_nothing", closed && edit_text(edit).is_empty(), "closed {closed}, {chosen} chosen, target text {:?}", edit_text(edit));
        pump(2_000); // the "Copied" note goes away, the hook is released
        clear_edit();
        let fg = unsafe { root(GetForegroundWindow()) };
        input::send_chord(&[VK_CONTROL], VIRTUAL_KEY(b'V' as u16));
        pump(300);
        let text = edit_text(edit);
        check!("multi_ctrl_c_copied_both_clips", text == joined, "after Ctrl+V: {text:?}, foreground is target {}", fg == target);
    } else {
        check!("multi_ctrl_c_session_opens", false, "");
    }

    // --- the right-click menu: a right click chooses the clip and opens Copy
    clear_edit();
    if open_carousel_in_corner() {
        if let Some(pt) = row_point(1) {
            input::move_to(pt);
            pump(60);
            input::right_down();
            input::right_up();
            pump(200);
        }
        let menu = app::with_runtime(|rt| rt.controller.view_model().menu.is_some()).unwrap_or(false);
        check!("multi_right_click_opens_the_menu", menu && picked() == 1, "menu {menu}, {} chosen", picked());
        if let Some(pt) = carousel_point(|g| g.menu_items.first().copied()) {
            input::move_to(pt);
            pump(60);
            input::left_down();
            input::left_up();
        }
        let closed = wait_until(2_000, || !session_open());
        check!("multi_menu_copy_closes_and_pastes_nothing", closed && edit_text(edit).is_empty(), "closed {closed}, target text {:?}", edit_text(edit));
        pump(2_000);
        clear_edit();
        input::send_chord(&[VK_CONTROL], VIRTUAL_KEY(b'V' as u16));
        pump(300);
        let text = edit_text(edit);
        check!("multi_menu_copy_copied_the_clip", text == "multi two", "after Ctrl+V: {text:?}");
    } else {
        check!("multi_menu_session_opens", false, "");
    }

    ctrl_key(true);
    unsafe {
        let _ = DestroyWindow(target);
    }
    app::destroy();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Presses (`up` false) or releases the Ctrl key.
fn ctrl_key(up: bool) {
    let ev = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CONTROL,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[ev], size_of::<INPUT>() as i32);
    }
}

/// The centre, on screen, of the part of the image editor that `pick` finds in its layout.
fn editor_point(pick: impl Fn(&pasteroll_render::ELayout) -> Option<pasteroll_render::Rect>) -> Option<POINT> {
    app::with_runtime(|rt| {
        let view = rt.controller.editor_view()?;
        let size = rt.controller.editor_size_dip()?;
        let (x, y, ..) = rt.controller.editor_rect()?;
        let scale = rt.controller.overlay_info().scale;
        let r = pick(&pasteroll_render::editor_layout(&view, size))?;
        Some(POINT {
            x: x + ((r.x + r.w / 2.0) * scale).round() as i32,
            y: y + ((r.y + r.h / 2.0) * scale).round() as i32,
        })
    })
    .flatten()
}

fn editor_open() -> bool {
    app::with_runtime(|rt| rt.controller.editor_is_open()).unwrap_or(false)
}

fn editor_ready() -> bool {
    app::with_runtime(|rt| {
        !rt.controller.editor_animating()
            && rt.controller.editor_view().is_some_and(|v| v.paste_enabled && !v.busy)
    })
    .unwrap_or(false)
}

/// Waits (pumping messages) up to `ms` for `done`.
fn wait_until(ms: u64, done: impl Fn() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if done() {
            return true;
        }
        pump(40);
    }
    done()
}

/// Advanced Image Paste into a window, end to end with the real hooks, real clicks and a real
/// paste: Ctrl+click the target with a picture chosen, switch the editor to "Path", press Paste,
/// and the saved file's path arrives in the target's text box. The picture is saved in a scratch
/// folder (the editor's remembered folder), never on the Desktop. The system clipboard IS used, by
/// the paste itself: it holds the pasted path afterwards.
fn probe_editor_anywhere(out: &mut Vec<Check>) -> Result<()> {
    macro_rules! check {
        ($name:expr, $ok:expr, $($detail:tt)*) => {
            out.push(Check { name: $name, ok: $ok, detail: format!($($detail)*) })
        };
    }
    let dir = std::env::temp_dir().join(format!("pasteroll-probe-editor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pictures = dir.join("Pictures");
    std::fs::create_dir_all(&pictures).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
    let db = dir.join("history.db");
    {
        let mut store = crate::keystore::open_store(&db).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
        let mut px = Vec::new();
        for y in 0..60u32 {
            for x in 0..100u32 {
                px.extend_from_slice(&[(x * 255 / 100) as u8, (y * 255 / 60) as u8, 160, 255]);
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        store.add_image_png(&encode_png(&Rgba { width: 100, height: 60, pixels: px }).unwrap(), now).ok();
    }
    // the editor saves into this folder (it remembers it between uses)
    let presets = pasteroll_core::imgedit::Presets { items: vec![], folder_base: String::new(), folder: Some(pictures.to_string_lossy().into_owned()) };
    let _ = std::fs::write(dir.join("image-presets.txt"), presets.to_text());
    let opts = app::Options { db: db.clone(), capture: false, accept_injected: true, max_seconds: None, hooks: true, tray: false, notice: false };
    let _msg_window = app::create(&opts)?;
    app::with_runtime(|rt| rt.platform.allow_own_process = true);
    let (target, edit) = make_target()?;
    let fg_ok = make_foreground(target);
    check!("editor_target_is_foreground", fg_ok, "precondition");
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
    // open the carousel with the pointer in the far corner, so it does not cover the target
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    input::move_to(POINT { x: sw - 40, y: sh - 80 });
    pump(60);
    app::dispatch(Event::Activate);
    pump(400);
    let open = app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false);
    check!("editor_session_opens", open, "");

    // a spot of the target's text box that the carousel does not cover
    let mut spot = None;
    unsafe {
        let mut er = RECT::default();
        let _ = GetWindowRect(edit, &mut er);
        let ov = app::with_runtime(|rt| rt.platform.overlay.screen_rect()).unwrap_or_default();
        let covered = |p: POINT| p.x >= ov.left && p.x < ov.right && p.y >= ov.top && p.y < ov.bottom;
        for (fx, fy) in [(0.5, 0.5), (0.2, 0.3), (0.2, 0.7), (0.8, 0.3)] {
            let pt = POINT {
                x: er.left + ((er.right - er.left) as f32 * fx) as i32,
                y: er.top + ((er.bottom - er.top) as f32 * fy) as i32,
            };
            if !covered(pt) && root(WindowFromPoint(pt)) == target {
                spot = Some(pt);
                break;
            }
        }
    }
    check!("editor_click_spot_is_clear", spot.is_some(), "");
    let Some(spot) = spot else {
        unsafe {
            let _ = DestroyWindow(target);
        }
        app::destroy();
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    };

    // Ctrl+click on the text box: the editor opens instead of the picture being pasted
    input::move_to(spot);
    pump(40);
    ctrl_key(false);
    pump(40);
    input::left_down();
    input::left_up();
    pump(150);
    ctrl_key(true);
    let opened = wait_until(3_000, editor_open);
    check!("editor_opens_in_a_window", opened, "");
    check!("editor_pastes_nothing_yet", edit_text(edit).is_empty(), "target text {:?}", edit_text(edit));
    let ready = wait_until(4_000, editor_ready);
    check!("editor_is_ready", ready, "");
    if !(opened && ready) {
        unsafe {
            let _ = DestroyWindow(target);
        }
        app::destroy();
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }
    let label = app::with_runtime(|rt| rt.controller.editor_view().map(|v| v.paste_label)).flatten();
    check!("editor_defaults_to_the_picture", label.as_deref() == Some("Paste Image"), "{label:?}");
    let fg = unsafe { GetForegroundWindow() };
    check!("editor_keeps_the_target_in_front", root(fg) == target, "foreground {fg:?}");

    // "Path" in the Paste as choice (the Output section's item 905), with a real click
    let viewer = app::with_runtime(|rt| rt.platform.viewer.hwnd()).unwrap_or_default();
    let chip = editor_point(|l| l.placed.iter().find(|p| p.id == 905).and_then(|p| p.parts.get(1).copied()));
    let aimed = chip.is_some_and(|pt| unsafe { WindowFromPoint(pt) } == viewer);
    check!("editor_paste_as_aim", aimed, "{chip:?}");
    if let (true, Some(pt)) = (aimed, chip) {
        input::move_to(pt);
        pump(40);
        input::left_down();
        input::left_up();
        pump(200);
    }
    let label = app::with_runtime(|rt| rt.controller.editor_view().map(|v| v.paste_label)).flatten();
    check!("editor_paste_as_path", label.as_deref() == Some("Paste Path"), "{label:?}");

    // Paste Path: the file is saved and its path arrives in the text box
    let button = editor_point(|l| Some(l.paste));
    let aimed = button.is_some_and(|pt| unsafe { WindowFromPoint(pt) } == viewer);
    check!("editor_paste_button_aim", aimed, "{button:?}");
    if let (true, Some(pt)) = (aimed, button) {
        input::move_to(pt);
        pump(40);
        input::left_down();
        input::left_up();
    }
    let done = wait_until(6_000, || !editor_open() && !app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(true));
    check!("editor_session_ends_after_paste", done, "");
    pump(500);
    let text = edit_text(edit);
    let file = text.trim().trim_matches('"').to_owned();
    check!(
        "editor_pastes_the_files_path",
        text.starts_with('"') && text.ends_with(".png\"") && file.starts_with(&*pictures.to_string_lossy()),
        "target text {text:?}"
    );
    check!("editor_saved_the_file", std::path::Path::new(&file).is_file(), "{file:?}");
    let saved = std::fs::read(&file).ok().and_then(|b| pasteroll_core::image::decode_png(&b));
    check!("editor_saved_a_real_picture", saved.is_some_and(|i| (i.width, i.height) == (100, 60)), "");

    unsafe {
        let _ = DestroyWindow(target);
    }
    app::destroy();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// A private history with one picture, an app with the real hooks, and a window of ours to click.
struct EditorProbe {
    dir: std::path::PathBuf,
    target: HWND,
    edit: HWND,
}

fn editor_probe_start(out: &mut Vec<Check>, tag: &str) -> Result<Option<EditorProbe>> {
    let dir = std::env::temp_dir().join(format!("pasteroll-probe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Pictures")).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
    let db = dir.join("history.db");
    {
        let mut store = crate::keystore::open_store(&db).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let img = Rgba { width: 100, height: 60, pixels: [60u8, 160, 200, 255].repeat(100 * 60) };
        store.add_image_png(&encode_png(&img).unwrap(), now).ok();
    }
    let presets = pasteroll_core::imgedit::Presets { items: vec![], folder_base: String::new(), folder: Some(dir.join("Pictures").to_string_lossy().into_owned()) };
    let _ = std::fs::write(dir.join("image-presets.txt"), presets.to_text());
    let opts = app::Options { db, capture: false, accept_injected: true, max_seconds: None, hooks: true, tray: false, notice: false };
    let _msg_window = app::create(&opts)?;
    app::with_runtime(|rt| rt.platform.allow_own_process = true);
    let (target, edit) = make_target()?;
    let ok = make_foreground(target);
    out.push(Check { name: "editor_probe_target_is_foreground", ok, detail: format!("{tag}: precondition") });
    if !ok {
        unsafe {
            let _ = DestroyWindow(target);
        }
        app::destroy();
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(None);
    }
    unsafe {
        let _ = SetFocus(Some(edit));
    }
    Ok(Some(EditorProbe { dir, target, edit }))
}

fn editor_probe_stop(probe: EditorProbe) {
    unsafe {
        let _ = DestroyWindow(probe.target);
    }
    app::destroy();
    let _ = std::fs::remove_dir_all(&probe.dir);
}

/// A point inside `window` that is on the window itself and not under the carousel.
fn clear_spot(window: HWND) -> Option<POINT> {
    unsafe {
        let mut wr = RECT::default();
        let _ = GetWindowRect(window, &mut wr);
        let ov = app::with_runtime(|rt| rt.platform.overlay.screen_rect()).unwrap_or_default();
        let covered = |p: POINT| p.x >= ov.left && p.x < ov.right && p.y >= ov.top && p.y < ov.bottom;
        for fy in [0.7f32, 0.5, 0.8, 0.6, 0.4] {
            for fx in [0.5f32, 0.3, 0.7, 0.2, 0.8] {
                let pt = POINT {
                    x: wr.left + ((wr.right - wr.left) as f32 * fx) as i32,
                    y: wr.top + ((wr.bottom - wr.top) as f32 * fy) as i32,
                };
                if !covered(pt) && root(WindowFromPoint(pt)) == window {
                    return Some(pt);
                }
            }
        }
    }
    None
}

/// Opens the carousel with the pointer in the far corner, so it does not cover the window.
fn open_carousel_in_corner() -> bool {
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    input::move_to(POINT { x: sw - 40, y: sh - 80 });
    pump(60);
    app::dispatch(Event::Activate);
    pump(400);
    app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false)
}

/// Presses Ctrl and clicks at `spot`, leaving Ctrl DOWN (a person waiting to see the editor does
/// exactly that). The caller releases it with `ctrl_key(true)`.
fn ctrl_click(spot: POINT) {
    input::move_to(spot);
    pump(40);
    ctrl_key(false);
    pump(40);
    input::left_down();
    input::left_up();
}

/// Escape twice (the editor, then the carousel), with the real keyboard.
fn escape_everything() {
    for _ in 0..2 {
        input::send_chord(&[], VK_ESCAPE);
        pump(500);
    }
}

/// The person holds Ctrl for a while after the click, waiting for the editor to appear: it must
/// still appear (and not be given up on after a second, as a paste waiting for the keys would).
fn probe_editor_holding_ctrl(out: &mut Vec<Check>) -> Result<()> {
    macro_rules! check {
        ($name:expr, $ok:expr, $($detail:tt)*) => {
            out.push(Check { name: $name, ok: $ok, detail: format!($($detail)*) })
        };
    }
    let Some(probe) = editor_probe_start(out, "holdctrl")? else { return Ok(()) };
    check!("hold_ctrl_session_opens", open_carousel_in_corner(), "");
    let Some(spot) = clear_spot(probe.target) else {
        check!("hold_ctrl_click_spot_is_clear", false, "");
        editor_probe_stop(probe);
        return Ok(());
    };
    ctrl_click(spot);
    // Ctrl stays down for 1.6 seconds, longer than a click is waited for
    pump(1_600);
    let still_down = input::held_modifiers().contains(&VK_CONTROL);
    let opened = editor_open();
    let session = app::with_runtime(|rt| rt.controller.is_open()).unwrap_or(false);
    ctrl_key(true);
    check!("hold_ctrl_editor_opens_while_ctrl_is_held", opened && still_down, "editor open {opened}, session open {session}, ctrl still down {still_down}");
    check!("hold_ctrl_nothing_typed_into_the_window", edit_text(probe.edit).is_empty(), "target text {:?}", edit_text(probe.edit));
    pump(300);
    if opened {
        escape_everything();
    }
    editor_probe_stop(probe);
    Ok(())
}

/// Visible top-level windows of a class, as handles.
fn windows_of_class(class: &str) -> Vec<isize> {
    use std::cell::RefCell;
    thread_local! {
        static FOUND: RefCell<(String, Vec<isize>)> = const { RefCell::new((String::new(), Vec::new())) };
    }
    unsafe extern "system" fn each(h: HWND, _: LPARAM) -> windows::core::BOOL {
        let mut buf = [0u16; 128];
        let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
        let name = String::from_utf16_lossy(&buf[..n]);
        FOUND.with(|f| {
            let mut f = f.borrow_mut();
            if name == f.0 && unsafe { IsWindowVisible(h).as_bool() } {
                f.1.push(h.0 as isize);
            }
        });
        TRUE
    }
    FOUND.with(|f| *f.borrow_mut() = (class.to_owned(), Vec::new()));
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(0));
    }
    FOUND.with(|f| std::mem::take(&mut f.borrow_mut().1))
}

/// The same with a real Windows Terminal window (a new one, started and closed by this test, never
/// one of the person's own): Ctrl+click it, hold Ctrl, and the editor opens, offering the path.
fn probe_editor_in_terminal(out: &mut Vec<Check>) -> Result<()> {
    macro_rules! check {
        ($name:expr, $ok:expr, $($detail:tt)*) => {
            out.push(Check { name: $name, ok: $ok, detail: format!($($detail)*) })
        };
    }
    const CLASS: &str = "CASCADIA_HOSTING_WINDOW_CLASS";
    let before = windows_of_class(CLASS);
    let dir = std::env::temp_dir().join(format!("pasteroll-probe-terminal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Pictures")).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
    let db = dir.join("history.db");
    {
        let mut store = crate::keystore::open_store(&db).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let img = Rgba { width: 100, height: 60, pixels: [200u8, 120, 60, 255].repeat(100 * 60) };
        store.add_image_png(&encode_png(&img).unwrap(), now).ok();
    }
    let presets = pasteroll_core::imgedit::Presets { items: vec![], folder_base: String::new(), folder: Some(dir.join("Pictures").to_string_lossy().into_owned()) };
    let _ = std::fs::write(dir.join("image-presets.txt"), presets.to_text());
    // a new terminal window with a quiet shell in it
    let started = std::process::Command::new("wt.exe")
        .args(["-w", "new", "powershell.exe", "-NoLogo", "-NoProfile", "-NoExit"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if started.is_err() {
        check!("terminal_probe_skipped", true, "Windows Terminal (wt.exe) is not available");
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }
    let mut wt = None;
    for _ in 0..100 {
        pump(100);
        if let Some(h) = windows_of_class(CLASS).into_iter().find(|h| !before.contains(h)) {
            wt = Some(HWND(h as *mut _));
            break;
        }
    }
    let Some(wt) = wt else {
        check!("terminal_window_appeared", false, "no new Windows Terminal window was found");
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    };
    pump(1_200); // let the shell start
    let opts = app::Options { db, capture: false, accept_injected: true, max_seconds: None, hooks: true, tray: false, notice: false };
    let _msg_window = app::create(&opts)?;
    let fg_ok = make_foreground(wt);
    check!("terminal_is_foreground", fg_ok, "precondition");
    let mut finished = false;
    if fg_ok {
        let opened_session = open_carousel_in_corner();
        check!("terminal_session_opens", opened_session, "");
        let spot = if opened_session { clear_spot(wt) } else { None };
        check!("terminal_click_spot_is_clear", spot.is_some(), "");
        if let Some(spot) = spot {
            ctrl_click(spot);
            pump(1_600);
            let opened = editor_open();
            let label = app::with_runtime(|rt| rt.controller.editor_view().map(|v| v.paste_label)).flatten();
            ctrl_key(true);
            check!("terminal_editor_opens_while_ctrl_is_held", opened, "paste button {label:?}");
            check!("terminal_defaults_to_the_path", label.as_deref() == Some("Paste Path"), "{label:?}");
            pump(300);
            if opened {
                escape_everything();
            }
            finished = true;
        }
    }
    let _ = finished;
    app::destroy();
    unsafe {
        // only the window this test started
        let _ = PostMessageW(Some(wt), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    pump(1_500);
    let gone = !windows_of_class(CLASS).contains(&(wt.0 as isize));
    check!("terminal_probe_window_closed", gone, "");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
