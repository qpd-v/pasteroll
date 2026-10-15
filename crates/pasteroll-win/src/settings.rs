//! The settings window: how much history to keep, pausing, the terminal warning, programs that are
//! never recorded, and custom filters.
//!
//! Changes are written to `config.toml` (the same file `pasteroll config` edits). Lowering a limit
//! shows how many clips would be deleted and asks first. After saving, the app restarts itself so
//! every part (capture, limits, exclusions) starts from the same file.

use crate::{app, keystore};
use pasteroll_core::config::{self, Config};
use pasteroll_core::limits::MB;
use std::cell::RefCell;
use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use pasteroll_core::settings::{self, fmt_age, fmt_mb, thousands, Choice};

// ------------------------------------------------------------------ window

const ID_TB: [i32; 4] = [101, 102, 103, 104];
const ID_VAL: [i32; 4] = [111, 112, 113, 114];
const ID_SIZE_TB: i32 = 105;
const ID_SIZE_VAL: i32 = 115;
const ID_PAUSE: i32 = 120;
const ID_WARN: i32 = 121;
const ID_AUTOSTART: i32 = 122;
const ID_EX_LIST: i32 = 130;
const ID_EX_EDIT: i32 = 131;
const ID_EX_ADD: i32 = 132;
const ID_EX_REMOVE: i32 = 133;
const ID_FL_LIST: i32 = 140;
const ID_FL_REMOVE: i32 = 141;
const ID_STATUS: i32 = 150;
const ID_SAVE: i32 = 1;
const ID_CANCEL: i32 = 2;

struct State {
    hwnd: HWND,
    font: HFONT,
    db: PathBuf,
    orig: Config,
    clips: Vec<u64>,
    sizes_mb: Vec<u64>,
    ages: Vec<Option<u32>>,
    clip_mb: Vec<u64>,
    size_choices: Vec<u32>,
    size_tb: HWND,
    size_val: HWND,
    tb: [HWND; 4],
    val: [HWND; 4],
    pause: HWND,
    warn: HWND,
    autostart: HWND,
    autostart_was: bool,
    ex_list: HWND,
    ex_edit: HWND,
    fl_list: HWND,
    status: HWND,
    exclude: Vec<String>,
    filters: Vec<String>,
}

thread_local! {
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_text(h: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        let _ = SetWindowTextW(h, PCWSTR(w.as_ptr()));
    }
}

fn get_text(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h).max(0) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(h, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..got.min(n)])
    }
}

fn list_fill(list: HWND, items: &[String]) {
    unsafe {
        SendMessageW(list, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
        for it in items {
            let w = wide(it);
            SendMessageW(list, LB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(w.as_ptr() as isize)));
        }
    }
}

fn list_selected(list: HWND) -> Option<usize> {
    let r = unsafe { SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0;
    (r >= 0).then_some(r as usize)
}

fn child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: u32,
    ex: u32,
    rect: (i32, i32, i32, i32),
    id: i32,
    dpi: u32,
    font: HFONT,
) -> HWND {
    let s = |v: i32| (v as i64 * dpi as i64 / 96) as i32;
    let t = wide(text);
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(ex),
            class,
            PCWSTR(t.as_ptr()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
            s(rect.0),
            s(rect.1),
            s(rect.2),
            s(rect.3),
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            Some(hinst.into()),
            None,
        )
        .unwrap_or_default();
        SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        h
    }
}

fn status(msg: &str) {
    ST.with(|c| {
        if let Ok(g) = c.try_borrow() {
            if let Some(s) = g.as_ref() {
                set_text(s.status, msg);
            }
        }
    });
}

/// Opens the window, or brings the open one forward.
pub fn open() {
    let existing = ST.with(|c| c.try_borrow().ok().and_then(|g| g.as_ref().map(|s| s.hwnd)));
    if let Some(h) = existing {
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    let Some(db) = app::db_path() else { return };
    let cfg = match keystore::load_config(&db) {
        Ok(c) => c,
        Err(problems) => {
            message(None, "Pasteroll", &format!("config.toml is not valid:\n{}", problems.join("\n")));
            return;
        }
    };
    if let Err(e) = create(db, cfg) {
        eprintln!("[pasteroll] could not open settings: {e}");
    }
}

fn message(parent: Option<HWND>, title: &str, text: &str) {
    let (t, x) = (wide(title), wide(text));
    unsafe {
        MessageBoxW(parent, PCWSTR(x.as_ptr()), PCWSTR(t.as_ptr()), MB_OK | MB_ICONINFORMATION);
    }
}

fn ask(parent: HWND, title: &str, text: &str) -> bool {
    let (t, x) = (wide(title), wide(text));
    unsafe { MessageBoxW(Some(parent), PCWSTR(x.as_ptr()), PCWSTR(t.as_ptr()), MB_YESNO | MB_ICONWARNING) == IDYES }
}

fn create(db: PathBuf, cfg: Config) -> windows::core::Result<()> {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES | ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
        let hinst = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as usize as *mut _),
            hIcon: LoadIconW(Some(hinst.into()), PCWSTR(1usize as *const u16)).unwrap_or_default(),
            lpszClassName: w!("PasterollSettings"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("PasterollSettings"),
            w!("Pasteroll settings"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            100,
            100,
            None,
            None,
            Some(hinst.into()),
            None,
        )?;
        let dpi = GetDpiForWindow(hwnd).max(96);
        let s = |v: i32| (v as i64 * dpi as i64 / 96) as i32;
        let font = CreateFontW(
            -((9 * dpi as i32) / 72),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        // client area 520 x 600 at 96 dpi
        let (cw, ch) = (s(520), s(672));
        let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
        let _ = AdjustWindowRectEx(&mut r, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, false, WINDOW_EX_STYLE(0));
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let sx = GetSystemMetrics(SM_CXSCREEN);
        let sy = GetSystemMetrics(SM_CYSCREEN);
        let _ = SetWindowPos(hwnd, None, (sx - ww) / 2, (sy - wh) / 2 - s(20), ww, wh, SWP_NOZORDER);

        let lim = cfg.limits;
        let steps = settings::Steps::for_limits(&lim);
        let (clips, sizes_mb, ages, clip_mb) =
            (steps.clips.clone(), steps.sizes_mb.clone(), steps.ages.clone(), steps.clip_mb.clone());

        child(hwnd, w!("STATIC"), "How much history to keep", 0, 0, (16, 12, 480, 20), 0, dpi, font);
        let rows: [(&str, i32); 4] = [("Clips kept", 40), ("Total size", 76), ("Keep for", 112), ("Largest single clip", 148)];
        let mut tb = [HWND::default(); 4];
        let mut val = [HWND::default(); 4];
        for (i, (label, y)) in rows.iter().enumerate() {
            child(hwnd, w!("STATIC"), label, 0, 0, (16, *y + 4, 150, 20), 0, dpi, font);
            tb[i] = child(
                hwnd,
                w!("msctls_trackbar32"),
                "",
                TBS_AUTOTICKS | TBS_HORZ | WS_TABSTOP.0,
                0,
                (170, *y, 250, 30),
                ID_TB[i],
                dpi,
                font,
            );
            val[i] = child(hwnd, w!("STATIC"), "", 0, 0, (430, *y + 4, 84, 20), ID_VAL[i], dpi, font);
        }
        let lens = [clips.len(), sizes_mb.len(), ages.len(), clip_mb.len()];
        let cur = [
            clips.iter().position(|v| *v == lim.max_clips).unwrap_or(0),
            sizes_mb.iter().position(|v| *v == lim.max_bytes / MB).unwrap_or(0),
            ages.iter().position(|v| *v == lim.max_age_days).unwrap_or(0),
            clip_mb.iter().position(|v| *v == lim.max_clip_bytes / MB).unwrap_or(0),
        ];
        for i in 0..4 {
            SendMessageW(tb[i], TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(((lens[i] as isize - 1) << 16) | 0)));
            SendMessageW(tb[i], TBM_SETPAGESIZE, Some(WPARAM(0)), Some(LPARAM(1)));
            SendMessageW(tb[i], TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(cur[i] as isize)));
        }
        child(
            hwnd,
            w!("STATIC"),
            "When a limit is reached the oldest clips are removed first. Pinned clips are never removed.",
            0,
            0,
            (16, 188, 490, 36),
            0,
            dpi,
            font,
        );

        child(hwnd, w!("STATIC"), "Carousel size", 0, 0, (16, 232, 150, 20), 0, dpi, font);
        let size_choices = settings::size_choices(cfg.size_percent);
        let size_tb = child(
            hwnd,
            w!("msctls_trackbar32"),
            "",
            TBS_AUTOTICKS | TBS_HORZ | WS_TABSTOP.0,
            0,
            (170, 228, 250, 30),
            ID_SIZE_TB,
            dpi,
            font,
        );
        let size_val = child(hwnd, w!("STATIC"), "", 0, 0, (430, 232, 84, 20), ID_SIZE_VAL, dpi, font);
        SendMessageW(size_tb, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(((size_choices.len() as isize - 1) << 16) | 0)));
        SendMessageW(size_tb, TBM_SETPAGESIZE, Some(WPARAM(0)), Some(LPARAM(1)));
        let size_pos = size_choices.iter().position(|v| *v == cfg.size_percent).unwrap_or(size_choices.len() / 2);
        SendMessageW(size_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(size_pos as isize)));

        let pause = child(hwnd, w!("BUTTON"), "Pause recording", BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0, 0, (16, 272, 400, 22), ID_PAUSE, dpi, font);
        let warn = child(
            hwnd,
            w!("BUTTON"),
            "Ask before pasting several lines into a terminal",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
            0,
            (16, 298, 400, 22),
            ID_WARN,
            dpi,
            font,
        );
        let autostart = child(
            hwnd,
            w!("BUTTON"),
            "Start Pasteroll when I sign in to Windows",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
            0,
            (16, 326, 488, 22),
            ID_AUTOSTART,
            dpi,
            font,
        );
        SendMessageW(autostart, BM_SETCHECK, Some(WPARAM(usize::from(crate::setup::autostart_enabled()))), Some(LPARAM(0)));
        SendMessageW(pause, BM_SETCHECK, Some(WPARAM(usize::from(cfg.paused))), Some(LPARAM(0)));
        SendMessageW(warn, BM_SETCHECK, Some(WPARAM(usize::from(cfg.warn_multiline_in_terminals))), Some(LPARAM(0)));

        child(hwnd, w!("STATIC"), "Never record copies made in these programs", 0, 0, (16, 364, 480, 20), 0, dpi, font);
        let lb = (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL.0 | WS_TABSTOP.0;
        let ex_list = child(hwnd, w!("LISTBOX"), "", lb, WS_EX_CLIENTEDGE.0, (16, 388, 300, 84), ID_EX_LIST, dpi, font);
        let ex_edit = child(
            hwnd,
            w!("EDIT"),
            "",
            ES_AUTOHSCROLL as u32 | WS_TABSTOP.0,
            WS_EX_CLIENTEDGE.0,
            (330, 388, 174, 24),
            ID_EX_EDIT,
            dpi,
            font,
        );
        child(hwnd, w!("BUTTON"), "Add", BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 0, (330, 420, 84, 26), ID_EX_ADD, dpi, font);
        child(hwnd, w!("BUTTON"), "Remove", BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 0, (420, 420, 84, 26), ID_EX_REMOVE, dpi, font);
        child(hwnd, w!("STATIC"), "Program file name, for example keepass.exe", 0, 0, (330, 452, 180, 36), 0, dpi, font);

        child(
            hwnd,
            w!("STATIC"),
            "Custom filters (add them from the filter menu in the overlay)",
            0,
            0,
            (16, 496, 490, 20),
            0,
            dpi,
            font,
        );
        let fl_list = child(hwnd, w!("LISTBOX"), "", lb, WS_EX_CLIENTEDGE.0, (16, 520, 300, 84), ID_FL_LIST, dpi, font);
        child(hwnd, w!("BUTTON"), "Remove", BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 0, (330, 520, 84, 26), ID_FL_REMOVE, dpi, font);

        let status_h = child(hwnd, w!("STATIC"), "", 0, 0, (16, 620, 300, 40), ID_STATUS, dpi, font);
        child(hwnd, w!("BUTTON"), "Save", BS_DEFPUSHBUTTON as u32 | WS_TABSTOP.0, 0, (330, 628, 84, 30), ID_SAVE, dpi, font);
        child(hwnd, w!("BUTTON"), "Cancel", BS_PUSHBUTTON as u32 | WS_TABSTOP.0, 0, (420, 628, 84, 30), ID_CANCEL, dpi, font);

        let exclude = cfg.exclude_apps.clone();
        let filters: Vec<String> = cfg.custom_filters.iter().map(|f| f.name.clone()).collect();
        list_fill(ex_list, &exclude);
        list_fill(fl_list, &filters);

        let state = State {
            hwnd,
            font,
            db,
            orig: cfg,
            clips,
            sizes_mb,
            ages,
            clip_mb,
            size_choices,
            size_tb,
            size_val,
            tb,
            val,
            pause,
            warn,
            autostart,
            autostart_was: crate::setup::autostart_enabled(),
            ex_list,
            ex_edit,
            fl_list,
            status: status_h,
            exclude,
            filters,
        };
        ST.with(|c| *c.borrow_mut() = Some(state));
        update_values();
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

fn trackbar_pos(tb: HWND) -> usize {
    unsafe { SendMessageW(tb, WM_USER, Some(WPARAM(0)), Some(LPARAM(0))) }.0.max(0) as usize
}

/// The four limits as the sliders currently show them.
fn chosen(s: &State) -> (u64, u64, Option<u32>, u64) {
    let pick = |v: &Vec<u64>, i: usize| v[trackbar_pos(s.tb[i]).min(v.len() - 1)];
    let age = s.ages[trackbar_pos(s.tb[2]).min(s.ages.len() - 1)];
    (pick(&s.clips, 0), pick(&s.sizes_mb, 1), age, pick(&s.clip_mb, 3))
}

/// The carousel size as the slider shows it.
fn chosen_size(s: &State) -> u32 {
    s.size_choices[trackbar_pos(s.size_tb).min(s.size_choices.len() - 1)]
}

fn update_values() {
    ST.with(|c| {
        let Ok(g) = c.try_borrow() else { return };
        let Some(s) = g.as_ref() else { return };
        let (clips, size, age, clip) = chosen(s);
        set_text(s.val[0], &format!("{} clips", thousands(clips)));
        set_text(s.val[1], &fmt_mb(size));
        set_text(s.val[2], &fmt_age(age));
        set_text(s.val[3], &fmt_mb(clip));
        set_text(s.size_val, &settings::fmt_size(chosen_size(s)));
    });
}

fn close() {
    let taken = ST.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.take()));
    if let Some(s) = taken {
        unsafe {
            let _ = DestroyWindow(s.hwnd);
            let _ = DeleteObject(s.font.into());
        }
    }
}

struct Snapshot {
    hwnd: HWND,
    db: PathBuf,
    orig: Config,
    limits: (u64, u64, Option<u32>, u64),
    size: u32,
    paused: bool,
    warn: bool,
    exclude: Vec<String>,
    filters: Vec<String>,
}

fn current_choices() -> Option<Snapshot> {
    ST.with(|c| {
        let g = c.try_borrow().ok()?;
        let s = g.as_ref()?;
        let check = |h: HWND| unsafe { SendMessageW(h, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 == 1 };
        Some(Snapshot {
            hwnd: s.hwnd,
            db: s.db.clone(),
            orig: s.orig.clone(),
            limits: chosen(s),
            size: chosen_size(s),
            paused: check(s.pause),
            warn: check(s.warn),
            exclude: s.exclude.clone(),
            filters: s.filters.clone(),
        })
    })
}

/// Builds the new config text from the window's choices (the rules live in the core).
fn build_text(snap: &Snapshot, text: &str) -> Result<String, Vec<String>> {
    let (clips, size_mb, age_days, clip_mb) = snap.limits;
    let choice = Choice {
        clips,
        size_mb,
        age_days,
        clip_mb,
        paused: snap.paused,
        warn_multiline: snap.warn,
        size_percent: snap.size,
        exclude_apps: snap.exclude.clone(),
        filter_names: snap.filters.clone(),
    };
    settings::build_text(&snap.orig, &choice, text)
}

/// Applies the "start with Windows" box if it changed (a registry entry, not part of config.toml).
fn apply_autostart() -> Result<(), String> {
    let want = ST.with(|c| {
        let g = c.try_borrow().ok()?;
        let s = g.as_ref()?;
        let on = unsafe { SendMessageW(s.autostart, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 == 1 };
        Some((on, s.autostart_was))
    });
    match want {
        Some((on, was)) if on != was => crate::setup::set_autostart(on),
        _ => Ok(()),
    }
}

/// What Save would write right now, for the self-test (nothing is written or restarted).
pub fn pending_text() -> Option<String> {
    let snap = current_choices()?;
    let text = keystore::read_config_text(&snap.db).ok()?;
    build_text(&snap, &text).ok()
}

/// Moves the carousel-size slider to a position (self-test).
pub fn probe_set_size_slider(index: usize) -> bool {
    ST.with(|c| {
        let Ok(g) = c.try_borrow() else { return false };
        let Some(s) = g.as_ref() else { return false };
        unsafe {
            SendMessageW(s.size_tb, WM_USER + 5, Some(WPARAM(1)), Some(LPARAM(index as isize)));
            SendMessageW(s.hwnd, WM_HSCROLL, Some(WPARAM(0)), Some(LPARAM(s.size_tb.0 as isize)));
        }
        true
    })
}

fn save() {
    if let Err(e) = apply_autostart() {
        return status(&e);
    }
    let Some(snap) = current_choices() else { return };
    let text = match keystore::read_config_text(&snap.db) {
        Ok(t) => t,
        Err(e) => return status(&e),
    };
    let new_text = match build_text(&snap, &text) {
        Ok(t) => t,
        Err(problems) => return status(&problems.join(" ")),
    };
    if new_text == text {
        close();
        return;
    }
    let new_cfg = match config::parse(&new_text) {
        Ok(c) => c,
        Err(problems) => return status(&problems.join(" ")),
    };
    // what lowering a limit would delete, shown before anything is written
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let plan = app::with_runtime(|rt| rt.controller.store().borrow().plan_prune(&new_cfg.limits, now).ok())
        .flatten();
    if let Some(p) = plan {
        if p.clips > 0 {
            let msg = format!(
                "Saving will delete {} clip(s), {:.1} MB ({} of them only because of age). Pinned clips are kept.\n\nDelete them?",
                p.clips,
                p.bytes as f64 / MB as f64,
                p.by_age
            );
            if !ask(snap.hwnd, "Pasteroll", &msg) {
                return;
            }
        }
    }
    if let Err(e) = keystore::write_config_text(&snap.db, &new_text) {
        return status(&e);
    }
    let removed = app::with_runtime(|rt| {
        let store = rt.controller.store();
        let mut st = store.borrow_mut();
        let _ = st.set_limits(new_cfg.limits);
        let n = st.maintain(now).unwrap_or(0);
        if n > 0 {
            // deleted data stays decryptable under the old key until it is replaced
            let _ = st.request_rotation();
        }
        n
    })
    .unwrap_or(0);
    eprintln!("[pasteroll] settings saved; {removed} clip(s) removed; restarting");
    close();
    app::restart();
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_HSCROLL => {
                update_values();
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                match id {
                    ID_SAVE => save(),
                    ID_CANCEL => close(),
                    ID_EX_ADD => {
                        let added = ST.with(|c| {
                            let Ok(mut g) = c.try_borrow_mut() else { return None };
                            let s = g.as_mut()?;
                            let name = get_text(s.ex_edit).trim().to_ascii_lowercase();
                            if name.is_empty() || name.contains(['\\', '/']) || name.chars().any(char::is_control) {
                                return Some(Err("Type a program's file name, such as keepass.exe".to_owned()));
                            }
                            if !s.exclude.contains(&name) {
                                s.exclude.push(name);
                                list_fill(s.ex_list, &s.exclude);
                            }
                            set_text(s.ex_edit, "");
                            Some(Ok(()))
                        });
                        if let Some(Err(e)) = added {
                            status(&e);
                        }
                    }
                    ID_EX_REMOVE => ST.with(|c| {
                        if let Ok(mut g) = c.try_borrow_mut() {
                            if let Some(s) = g.as_mut() {
                                if let Some(i) = list_selected(s.ex_list) {
                                    if i < s.exclude.len() {
                                        s.exclude.remove(i);
                                        list_fill(s.ex_list, &s.exclude);
                                    }
                                }
                            }
                        }
                    }),
                    ID_FL_REMOVE => ST.with(|c| {
                        if let Ok(mut g) = c.try_borrow_mut() {
                            if let Some(s) = g.as_mut() {
                                if let Some(i) = list_selected(s.fl_list) {
                                    if i < s.filters.len() {
                                        s.filters.remove(i);
                                        list_fill(s.fl_list, &s.filters);
                                    }
                                }
                            }
                        }
                    }),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                close();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Debug aid: opens the window against `db`'s config, saves a picture of it to `out` (PNG) and
/// closes it. Nothing is written to the config.
pub fn snapshot(db: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    let cfg = keystore::load_config(db).map_err(|p| p.join("; "))?;
    create(db.to_path_buf(), cfg).map_err(|e| e.to_string())?;
    // let it paint
    let end = std::time::Instant::now() + std::time::Duration::from_millis(600);
    unsafe {
        while std::time::Instant::now() < end {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let hwnd = ST.with(|c| c.borrow().as_ref().map(|s| s.hwnd)).ok_or("window did not open")?;
    let result = unsafe {
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
            Err("PrintWindow failed".to_owned())
        } else {
            let mut rgba = Vec::with_capacity(buf.len());
            for p in buf.chunks_exact(4) {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
            let img = pasteroll_core::image::Rgba { width: w as u32, height: h as u32, pixels: rgba };
            pasteroll_core::image::encode_png(&img).ok_or_else(|| "could not encode the picture".to_owned())
        }
    };
    close();
    std::fs::write(out, result?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changed_values_are_written() {
        let orig = config::parse("").unwrap();
        let lim = orig.limits;
        let snap = Snapshot {
            hwnd: HWND::default(),
            db: PathBuf::new(),
            orig: orig.clone(),
            limits: (lim.max_clips, lim.max_bytes / MB, lim.max_age_days, lim.max_clip_bytes / MB),
            size: 100,
            paused: false,
            warn: true,
            exclude: vec![],
            filters: vec![],
        };
        let text = "# my notes\n[limits]\nmax_clips = 1000\n";
        assert_eq!(build_text(&snap, text).unwrap(), text, "nothing changed, nothing rewritten");
        let snap2 = Snapshot { limits: (250, 100, None, 4), paused: true, warn: false, ..snap };
        let out = build_text(&snap2, text).unwrap();
        let c = config::parse(&out).unwrap();
        assert_eq!(c.limits.max_clips, 250);
        assert_eq!(c.limits.max_bytes, 100 * MB);
        assert_eq!(c.limits.max_age_days, None);
        assert_eq!(c.limits.max_clip_bytes, 4 * MB);
        assert!(c.paused && !c.warn_multiline_in_terminals);
        assert!(out.contains("# my notes"), "comments survive");
    }

    #[test]
    fn list_edits_become_adds_and_removes() {
        let base = config::add_excluded_app("", "keepass.exe").unwrap();
        let base = config::add_custom_filter(&base, "Design", &["psd".into()]).unwrap();
        let orig = config::parse(&base).unwrap();
        let lim = orig.limits;
        let snap = Snapshot {
            hwnd: HWND::default(),
            db: PathBuf::new(),
            orig: orig.clone(),
            limits: (lim.max_clips, lim.max_bytes / MB, lim.max_age_days, lim.max_clip_bytes / MB),
            size: 100,
            paused: false,
            warn: true,
            exclude: vec!["bitwarden.exe".into()],
            filters: vec![],
        };
        let out = build_text(&snap, &base).unwrap();
        let c = config::parse(&out).unwrap();
        assert_eq!(c.exclude_apps, vec!["bitwarden.exe".to_string()]);
        assert!(c.custom_filters.is_empty());
    }
}
