//! The settings window: the Complete / No Bloat mode, how much history to keep, pausing, the
//! terminal warning, programs that are never recorded, and custom filters.
//!
//! Changes are written to `config.toml` (the same file `pasteroll config` edits). Lowering a limit
//! shows how many clips would be deleted and asks first. After saving, the app restarts itself so
//! every part (capture, limits, exclusions) starts from the same file.

use crate::{app, keystore};
use pasteroll_core::config::{self, ClickModifier, Config, UiMode, UpdateCheck};
use pasteroll_core::limits::MB;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::settingsui;
use pasteroll_core::settings::{self, fmt_age, fmt_mb, thousands, Choice};

// ------------------------------------------------------------------ window

const ID_TB: [i32; 4] = [101, 102, 103, 104];
const ID_VAL: [i32; 4] = [111, 112, 113, 114];
const ID_SIZE_TB: i32 = 105;
const ID_SIZE_VAL: i32 = 115;
const ID_PAUSE: i32 = 120;
const ID_WARN: i32 = 121;
const ID_AUTOSTART: i32 = 122;
const ID_MOD: [i32; 3] = [123, 124, 125];
const ID_MODE: [i32; 2] = [126, 127];
const ID_UPDATES: i32 = 128;
const ID_CHECK_NOW: i32 = 129;
const ID_LAST_CHECK: i32 = 134;
const ID_IMG_EDIT: i32 = 135;
const ID_IMG_BROWSE: i32 = 136;
const ID_IMG_RESET: i32 = 137;
const ID_EX_LIST: i32 = 130;
const ID_EX_EDIT: i32 = 131;
const ID_EX_ADD: i32 = 132;
const ID_EX_REMOVE: i32 = 133;
const ID_FL_LIST: i32 = 140;
const ID_FL_REMOVE: i32 = 141;
const ID_STATUS: i32 = 150;
const ID_SAVE: i32 = 1;
/// Looks every second for a choice made elsewhere (see `sync_update_choice`).
const TIMER_SYNC: usize = 77;
const ID_CANCEL: i32 = 2;

struct State {
    hwnd: HWND,
    font: HFONT,
    font_bold: HFONT,
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
    modifier: [HWND; 3],
    /// The box holding the folder pictures are saved in ("" = the Desktop).
    img_edit: HWND,
    mode: [HWND; 2],
    updates: HWND,
    check_now: HWND,
    last_check: HWND,
    ex_list: HWND,
    ex_edit: HWND,
    fl_list: HWND,
    status: HWND,
    exclude: Vec<String>,
    filters: Vec<String>,
}

/// A settings window is open (the one-time update question waits for it to close).
static OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_open() -> bool {
    OPEN.load(std::sync::atomic::Ordering::Relaxed)
}

thread_local! {
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
    /// Added to the y of every control `child` makes, so the sections keep their own layout.
    static TOP: Cell<i32> = const { Cell::new(0) };
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
            s(rect.1 + TOP.get()),
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

/// Size of the window's client area, and the layout of its two columns of cards, in DIP at 96 dpi.
const WIN_W: i32 = 720;
const WIN_H: i32 = 568;
const MARGIN: i32 = 16;
const COL_W: i32 = 338;
const COL2_X: i32 = MARGIN + COL_W + 12;
/// Space inside a card, left and right.
const PAD: i32 = 14;
/// A label in the quiet colour (see `settingsui::is_muted`).
const ID_NOTE: i32 = 160;
const SS_NOPREFIX_: u32 = 0x80;
const SS_CENTERIMAGE_: u32 = 0x200;
const SS_RIGHT_: u32 = 0x2;
const SS_ENDELLIPSIS_: u32 = 0x4000;

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
            // the background is painted by `settingsui` (theme colours and cards)
            hbrBackground: HBRUSH::default(),
            hIcon: LoadIconW(Some(hinst.into()), PCWSTR(1usize as *const u16)).unwrap_or_default(),
            lpszClassName: w!("PasterollSettings"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("PasterollSettings"),
            w!("Pasteroll settings"),
            style,
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
        let make_font = |points_x10: i32, weight: i32| {
            CreateFontW(
                -((points_x10 * dpi as i32) / 720),
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
        let font = make_font(90, 400);
        let font_bold = make_font(95, 600);

        // size the window so the client area is exactly the layout, centred in the work area
        let mut r = RECT { left: 0, top: 0, right: s(WIN_W), bottom: s(WIN_H) };
        let _ = AdjustWindowRectEx(&mut r, style, false, WINDOW_EX_STYLE(0));
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let mut work = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let x = work.left + ((work.right - work.left) - ww) / 2;
        let y = (work.top + ((work.bottom - work.top) - wh) / 2).max(work.top);
        let _ = SetWindowPos(hwnd, None, x.max(work.left), y, ww, wh, SWP_NOZORDER);

        let theme = settingsui::Theme::current();
        let mut look = settingsui::Look::new(theme, font);
        settingsui::apply_title_bar(hwnd, theme.dark);

        let lim = cfg.limits;
        let steps = settings::Steps::for_limits(&lim);
        let (clips, sizes_mb, ages, clip_mb) =
            (steps.clips.clone(), steps.sizes_mb.clone(), steps.ages.clone(), steps.clip_mb.clone());

        TOP.set(0);
        let label = |text: &str, x: i32, y: i32, w: i32, h: i32, id: i32, style: u32| {
            child(hwnd, w!("STATIC"), text, style | SS_NOPREFIX_, 0, (x, y, w, h), id, dpi, font)
        };
        let heading = |text: &str, x: i32, y: i32| {
            child(hwnd, w!("STATIC"), text, SS_NOPREFIX_ | SS_CENTERIMAGE_, 0, (x + PAD, y + 10, COL_W - 2 * PAD, 22), 0, dpi, font_bold)
        };
        let button = |text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32| {
            child(hwnd, w!("BUTTON"), text, style | WS_TABSTOP.0, 0, (x, y, w, h), id, dpi, font)
        };
        let slider = |x: i32, y: i32, w: i32, id: i32| {
            child(hwnd, w!("msctls_trackbar32"), "", TBS_NOTICKS | TBS_HORZ | WS_TABSTOP.0, 0, (x, y, w, 28), id, dpi, font)
        };

        // ---------------------------------------------------------------- left column
        // Interface
        let (cx, cy) = (MARGIN, MARGIN);
        look.cards.push([cx, cy, COL_W, 128]);
        heading("Interface", cx, cy);
        let mode_labels = ["Complete: everything on show", "No Bloat: just the clip (tools appear when used)"];
        let mut mode = [HWND::default(); 2];
        for (i, m) in UiMode::ALL.iter().enumerate() {
            let group = if i == 0 { WS_GROUP.0 } else { 0 };
            mode[i] = button(mode_labels[i], BS_AUTORADIOBUTTON as u32 | group, cx + PAD, cy + 36 + 24 * i as i32, COL_W - 2 * PAD, 22, ID_MODE[i]);
            SendMessageW(mode[i], BM_SETCHECK, Some(WPARAM(usize::from(*m == cfg.mode))), Some(LPARAM(0)));
        }
        label("Carousel size", cx + PAD, cy + 94, 100, 22, 0, SS_CENTERIMAGE_);
        let size_choices = settings::size_choices(cfg.size_percent);
        let size_tb = slider(cx + PAD + 104, cy + 91, 148, ID_SIZE_TB);
        let size_val = label("", cx + COL_W - PAD - 56, cy + 94, 56, 22, ID_SIZE_VAL, SS_CENTERIMAGE_ | SS_RIGHT_);
        SendMessageW(size_tb, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(((size_choices.len() as isize - 1) << 16) | 0)));
        SendMessageW(size_tb, TBM_SETPAGESIZE, Some(WPARAM(0)), Some(LPARAM(1)));
        let size_pos = size_choices.iter().position(|v| *v == cfg.size_percent).unwrap_or(size_choices.len() / 2);
        SendMessageW(size_tb, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(size_pos as isize)));

        // Updates
        let cy = 154;
        look.cards.push([cx, cy, COL_W, 118]);
        heading("Updates", cx, cy);
        let updates = button(
            "Check for updates once a day (asks GitHub; you choose whether to install)",
            BS_AUTOCHECKBOX as u32,
            cx + PAD,
            cy + 36,
            COL_W - 2 * PAD,
            36,
            ID_UPDATES,
        );
        SendMessageW(updates, BM_SETCHECK, Some(WPARAM(usize::from(cfg.updates == UpdateCheck::On))), Some(LPARAM(0)));
        let check_now = button("Check for updates now", BS_PUSHBUTTON as u32, cx + PAD, cy + 80, 170, 28, ID_CHECK_NOW);
        // when it last checked: on the heading line, at the right
        let last_check = label("", cx + PAD + 80, cy + 10, COL_W - 2 * PAD - 80, 22, ID_LAST_CHECK, SS_CENTERIMAGE_ | SS_RIGHT_ | SS_ENDELLIPSIS_);

        // Behaviour
        let cy = 282;
        look.cards.push([cx, cy, COL_W, 120]);
        heading("Behavior", cx, cy);
        let pause = button("Pause recording", BS_AUTOCHECKBOX as u32, cx + PAD, cy + 36, COL_W - 2 * PAD, 22, ID_PAUSE);
        let warn = button(
            "Ask before pasting several lines into a terminal",
            BS_AUTOCHECKBOX as u32,
            cx + PAD,
            cy + 62,
            COL_W - 2 * PAD,
            22,
            ID_WARN,
        );
        let autostart = button(
            if pasteroll_core::paths::is_portable() { pasteroll_core::paths::PORTABLE_AUTOSTART } else { "Start Pasteroll when I sign in to Windows" },
            BS_AUTOCHECKBOX as u32,
            cx + PAD,
            cy + 88,
            COL_W - 2 * PAD,
            22,
            ID_AUTOSTART,
        );
        SendMessageW(autostart, BM_SETCHECK, Some(WPARAM(usize::from(crate::setup::autostart_enabled()))), Some(LPARAM(0)));
        if pasteroll_core::paths::is_portable() {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(autostart, false);
        }
        SendMessageW(pause, BM_SETCHECK, Some(WPARAM(usize::from(cfg.paused))), Some(LPARAM(0)));
        SendMessageW(warn, BM_SETCHECK, Some(WPARAM(usize::from(cfg.warn_multiline_in_terminals))), Some(LPARAM(0)));

        // Custom filters
        let cy = 412;
        look.cards.push([cx, cy, COL_W, 98]);
        heading("Custom filters", cx, cy);
        let lb = (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL.0 | WS_TABSTOP.0;
        look.fields.push([cx + PAD, cy + 38, 196, 52]);
        let fl_list = child(hwnd, w!("LISTBOX"), "", lb, 0, (cx + PAD + 4, cy + 42, 188, 44), ID_FL_LIST, dpi, font);
        button("Remove", BS_PUSHBUTTON as u32, cx + PAD + 206, cy + 38, 90, 28, ID_FL_REMOVE);
        label("Add them from the filter menu", cx + PAD + 110, cy + 10, COL_W - 2 * PAD - 110, 22, ID_NOTE + 1, SS_CENTERIMAGE_ | SS_RIGHT_);

        // ---------------------------------------------------------------- right column
        // History limits
        let (cx, cy) = (COL2_X, MARGIN);
        look.cards.push([cx, cy, COL_W, 190]);
        heading("How much history to keep", cx, cy);
        let rows: [(&str, i32); 4] = [("Clips kept", 0), ("Total size", 1), ("Keep for", 2), ("Largest single clip", 3)];
        let mut tb = [HWND::default(); 4];
        let mut val = [HWND::default(); 4];
        for (i, (text, n)) in rows.iter().enumerate() {
            let y = cy + 36 + *n * 28;
            label(text, cx + PAD, y + 3, 110, 22, 0, SS_CENTERIMAGE_);
            tb[i] = slider(cx + PAD + 112, y, 128, ID_TB[i]);
            val[i] = label("", cx + COL_W - PAD - 64, y + 3, 64, 22, ID_VAL[i], SS_CENTERIMAGE_ | SS_RIGHT_);
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
        label(
            "When a limit is reached the oldest clips are removed first. Pinned clips are never removed.",
            cx + PAD,
            cy + 36 + 4 * 28 + 4,
            COL_W - 2 * PAD,
            32,
            ID_NOTE,
            0,
        );

        // Pictures
        let cy = 216;
        look.cards.push([cx, cy, COL_W, 162]);
        heading("Pictures", cx, cy);
        label(
            "To edit a picture before it is pasted or saved, hold this key while you click where it should go:",
            cx + PAD,
            cy + 36,
            COL_W - 2 * PAD,
            32,
            0,
            0,
        );
        let mut modifier = [HWND::default(); 3];
        for (i, m) in ClickModifier::ALL.iter().enumerate() {
            let group = if i == 0 { WS_GROUP.0 } else { 0 };
            modifier[i] = button(m.label(), BS_AUTORADIOBUTTON as u32 | group, cx + PAD + 88 * i as i32, cy + 72, 80, 22, ID_MOD[i]);
            let on = *m == cfg.advanced_image_modifier;
            SendMessageW(modifier[i], BM_SETCHECK, Some(WPARAM(usize::from(on))), Some(LPARAM(0)));
        }
        label("Save pictures in this folder (Paste Path, image editor):", cx + PAD, cy + 102, COL_W - 2 * PAD, 18, 0, SS_CENTERIMAGE_);
        look.fields.push([cx + PAD, cy + 124, 172, 28]);
        let img_edit = child(
            hwnd,
            w!("EDIT"),
            &cfg.image_folder,
            ES_AUTOHSCROLL as u32 | WS_TABSTOP.0,
            0,
            (cx + PAD + 8, cy + 129, 156, 18),
            ID_IMG_EDIT,
            dpi,
            font,
        );
        SendMessageW(img_edit, EM_SETCUEBANNER, Some(WPARAM(0)), Some(LPARAM(w!("The Desktop").as_ptr() as isize)));
        button("Browse...", BS_PUSHBUTTON as u32, cx + PAD + 178, cy + 124, 66, 28, ID_IMG_BROWSE);
        button("Desktop", BS_PUSHBUTTON as u32, cx + PAD + 250, cy + 124, 60, 28, ID_IMG_RESET);

        // Programs that are never recorded
        let cy = 388;
        look.cards.push([cx, cy, COL_W, 110]);
        heading("Never record copies made in these programs", cx, cy);
        look.fields.push([cx + PAD, cy + 38, 176, 64]);
        let ex_list = child(hwnd, w!("LISTBOX"), "", lb, 0, (cx + PAD + 4, cy + 42, 168, 56), ID_EX_LIST, dpi, font);
        look.fields.push([cx + PAD + 184, cy + 38, 126, 28]);
        let ex_edit = child(
            hwnd,
            w!("EDIT"),
            "",
            ES_AUTOHSCROLL as u32 | WS_TABSTOP.0,
            0,
            (cx + PAD + 192, cy + 43, 110, 18),
            ID_EX_EDIT,
            dpi,
            font,
        );
        SendMessageW(ex_edit, EM_SETCUEBANNER, Some(WPARAM(0)), Some(LPARAM(w!("keepass.exe").as_ptr() as isize)));
        button("Add", BS_PUSHBUTTON as u32, cx + PAD + 184, cy + 72, 60, 28, ID_EX_ADD);
        button("Remove", BS_PUSHBUTTON as u32, cx + PAD + 250, cy + 72, 60, 28, ID_EX_REMOVE);

        // ---------------------------------------------------------------- bottom bar
        let status_h = label("", MARGIN, 520, 490, 36, ID_STATUS, SS_CENTERIMAGE_);
        button("Cancel", BS_PUSHBUTTON as u32, WIN_W - MARGIN - 8 - 176, 522, 84, 32, ID_CANCEL);
        button("Save", BS_DEFPUSHBUTTON as u32, WIN_W - MARGIN - 84, 522, 84, 32, ID_SAVE);

        // a little room inside the edit boxes
        for e in [img_edit, ex_edit] {
            SendMessageW(e, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(0)));
        }
        for l in [ex_list, fl_list] {
            settingsui::theme_control(l, theme.dark);
        }
        settingsui::install(look);

        let exclude = cfg.exclude_apps.clone();
        let filters: Vec<String> = cfg.custom_filters.iter().map(|f| f.name.clone()).collect();
        list_fill(ex_list, &exclude);
        list_fill(fl_list, &filters);

        let state = State {
            hwnd,
            font,
            font_bold,
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
            modifier,
            img_edit,
            mode,
            updates,
            check_now,
            last_check,
            ex_list,
            ex_edit,
            fl_list,
            status: status_h,
            exclude,
            filters,
        };
        ST.with(|c| *c.borrow_mut() = Some(state));
        OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
        SetTimer(Some(hwnd), TIMER_SYNC, 1000, None);
        update_values();
        refresh_last_check();
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

/// The update-check choice is also made outside this window (the one-time question can appear while
/// it is open, and `config.toml` can be edited). When the file now says something this window does
/// not know about, the box follows it, and what Save compares against follows too, so Save neither
/// shows a stale box nor undoes the choice.
fn sync_update_choice() {
    ST.with(|c| {
        let Ok(mut g) = c.try_borrow_mut() else { return };
        let Some(s) = g.as_mut() else { return };
        let Some(now) = keystore::read_config_text(&s.db).ok().and_then(|t| config::parse(&t).ok()) else { return };
        if now.updates != s.orig.updates && now.updates != UpdateCheck::Ask {
            s.orig.updates = now.updates;
            unsafe {
                SendMessageW(s.updates, BM_SETCHECK, Some(WPARAM(usize::from(now.updates == UpdateCheck::On))), Some(LPARAM(0)));
            }
        }
    });
}

/// Shows when the update check last got an answer, and keeps the button off while one is running.
fn refresh_last_check() {
    ST.with(|c| {
        let Ok(g) = c.try_borrow() else { return };
        let Some(s) = g.as_ref() else { return };
        let checking = crate::update::checking();
        let text = if checking {
            "Checking...".to_owned()
        } else {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            crate::update::last_checked_text(crate::update::last_checked(&s.db), now)
        };
        if get_text(s.last_check) != text {
            set_text(s.last_check, &text);
        }
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(s.check_now, !checking);
        }
    });
}

/// "Check for updates now": the result is shown by the notification icon, like the menu entry's.
fn check_now() {
    let db = ST.with(|c| c.try_borrow().ok().and_then(|g| g.as_ref().map(|s| s.db.clone())));
    if let (Some(db), Some(window)) = (db, app::tray_window()) {
        crate::updatewin::open_check(window, Some(db));
    }
    refresh_last_check();
}

/// "Browse..." beside the picture folder: a folder chooser, starting at the folder in the box (or
/// the Desktop). Cancelling changes nothing.
fn browse_image_folder(hwnd: HWND) {
    let current = ST.with(|c| c.try_borrow().ok().and_then(|g| g.as_ref().map(|s| (s.img_edit, get_text(s.img_edit)))));
    let Some((edit, typed)) = current else { return };
    let typed = typed.trim().to_owned();
    let start = Some(PathBuf::from(&typed)).filter(|p| !typed.is_empty() && p.is_dir());
    if let pasteroll_app::SaveChoice::Path(dir) = crate::dialogs::pick_folder(hwnd, start.as_deref(), hwnd) {
        set_text(edit, &dir.to_string_lossy());
    }
}

/// Whether the update box is ticked right now (self-test).
pub fn probe_updates_checked() -> Option<bool> {
    sync_update_choice();
    ST.with(|c| {
        let g = c.try_borrow().ok()?;
        let s = g.as_ref()?;
        Some(unsafe { SendMessageW(s.updates, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 == 1 })
    })
}

fn chosen_mode(s: &State) -> UiMode {
    let check = |h: HWND| unsafe { SendMessageW(h, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 == 1 };
    UiMode::ALL
        .iter()
        .zip(s.mode.iter())
        .find(|(_, h)| check(**h))
        .map_or(s.orig.mode, |(m, _)| *m)
}

fn close() {
    OPEN.store(false, std::sync::atomic::Ordering::Relaxed);
    let taken = ST.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.take()));
    if let Some(s) = taken {
        unsafe {
            let _ = DestroyWindow(s.hwnd);
            let _ = DeleteObject(s.font.into());
            let _ = DeleteObject(s.font_bold.into());
        }
    }
    settingsui::remove();
}

struct Snapshot {
    hwnd: HWND,
    db: PathBuf,
    orig: Config,
    limits: (u64, u64, Option<u32>, u64),
    size: u32,
    paused: bool,
    warn: bool,
    modifier: ClickModifier,
    image_folder: String,
    mode: UiMode,
    updates: bool,
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
            modifier: ClickModifier::ALL
                .iter()
                .zip(s.modifier.iter())
                .find(|(_, h)| check(**h))
                .map_or(s.orig.advanced_image_modifier, |(m, _)| *m),
            image_folder: get_text(s.img_edit),
            mode: chosen_mode(s),
            updates: check(s.updates),
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
        advanced_modifier: snap.modifier,
        image_folder: snap.image_folder.clone(),
        mode: snap.mode,
        exclude_apps: snap.exclude.clone(),
        filter_names: snap.filters.clone(),
    };
    let text = settings::build_text(&snap.orig, &choice, text)?;
    // the update check is a Windows-only setting: written only when the box was changed (an
    // undecided setting with the box left empty stays undecided, so the one-time question is kept)
    if snap.updates != (snap.orig.updates == UpdateCheck::On) {
        return config::set_value(&text, "check_for_updates", &snap.updates.to_string());
    }
    Ok(text)
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

/// Picks a mode in the window as a click on its radio button would (self-test).
pub fn probe_choose_mode(mode: UiMode) -> bool {
    ST.with(|c| {
        let Ok(g) = c.try_borrow() else { return false };
        let Some(s) = g.as_ref() else { return false };
        for (m, h) in UiMode::ALL.iter().zip(s.mode.iter()) {
            unsafe {
                SendMessageW(*h, BM_SETCHECK, Some(WPARAM(usize::from(*m == mode))), Some(LPARAM(0)));
            }
        }
        true
    })
}

/// Ticks or clears the update-check box as a click would (self-test).
pub fn probe_set_updates(on: bool) -> bool {
    ST.with(|c| {
        let Ok(g) = c.try_borrow() else { return false };
        let Some(s) = g.as_ref() else { return false };
        unsafe {
            SendMessageW(s.updates, BM_SETCHECK, Some(WPARAM(usize::from(on))), Some(LPARAM(0)));
        }
        true
    })
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
            WM_TIMER if wparam.0 == TIMER_SYNC => {
                sync_update_choice();
                refresh_last_check();
                LRESULT(0)
            }
            WM_HSCROLL => {
                update_values();
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                let painted = settingsui::with(|l| settingsui::paint_background(hwnd, HDC(wparam.0 as *mut _), l));
                if painted.is_some() { LRESULT(1) } else { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                let id = GetDlgCtrlID(HWND(lparam.0 as *mut _));
                let kind = if msg == WM_CTLCOLORSTATIC { settingsui::ControlKind::Label } else { settingsui::ControlKind::Field };
                settingsui::control_colors(HDC(wparam.0 as *mut _), id, kind).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
            }
            WM_NOTIFY => settingsui::custom_draw(hwnd, lparam).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam)),
            WM_SETTINGCHANGE => {
                // Windows' light/dark choice changed while the window is open
                let name = if lparam.0 == 0 { String::new() } else { PCWSTR(lparam.0 as *const u16).to_string().unwrap_or_default() };
                if name == "ImmersiveColorSet" {
                    let theme = settingsui::Theme::current();
                    settingsui::with(|l| l.retheme(theme));
                    settingsui::apply_title_bar(hwnd, theme.dark);
                    let lists = ST.with(|c| c.try_borrow().ok().and_then(|g| g.as_ref().map(|s| [s.ex_list, s.fl_list])));
                    for l in lists.into_iter().flatten() {
                        settingsui::theme_control(l, theme.dark);
                    }
                    let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                match id {
                    ID_SAVE => save(),
                    ID_CANCEL => close(),
                    ID_CHECK_NOW => check_now(),
                    ID_IMG_BROWSE => browse_image_folder(hwnd),
                    ID_IMG_RESET => ST.with(|c| {
                        if let Ok(g) = c.try_borrow() {
                            if let Some(s) = g.as_ref() {
                                set_text(s.img_edit, "");
                            }
                        }
                    }),
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
    let result = settingsui::capture_window(hwnd);
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
            modifier: ClickModifier::Ctrl,
            image_folder: String::new(),
            mode: UiMode::Complete,
            updates: false,
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
    fn the_mode_is_written_only_when_it_is_changed() {
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
            modifier: ClickModifier::Ctrl,
            image_folder: String::new(),
            mode: UiMode::Complete,
            updates: false,
            exclude: vec![],
            filters: vec![],
        };
        assert_eq!(build_text(&snap, "").unwrap(), "", "untouched, nothing written");
        let slim = Snapshot { mode: UiMode::NoBloat, ..snap };
        let out = build_text(&slim, "").unwrap();
        assert_eq!(config::parse(&out).unwrap().mode, UiMode::NoBloat);
    }

    #[test]
    fn the_update_check_is_written_only_when_the_box_changes() {
        let snap = |orig: &str, updates: bool| {
            let orig = config::parse(orig).unwrap();
            let lim = orig.limits;
            Snapshot {
                hwnd: HWND::default(),
                db: PathBuf::new(),
                orig: orig.clone(),
                limits: (lim.max_clips, lim.max_bytes / MB, lim.max_age_days, lim.max_clip_bytes / MB),
                size: 100,
                paused: false,
                warn: true,
                modifier: ClickModifier::Ctrl,
                image_folder: String::new(),
                mode: UiMode::Complete,
                updates,
                exclude: vec![],
                filters: vec![],
            }
        };
        // undecided and left unticked: nothing is written, so the one-time question still comes
        assert_eq!(build_text(&snap("", false), "").unwrap(), "");
        // ticking it turns the check on; unticking a decided "on" turns it off
        let on = build_text(&snap("", true), "").unwrap();
        assert_eq!(config::parse(&on).unwrap().updates, UpdateCheck::On);
        let off = build_text(&snap(&on, false), &on).unwrap();
        assert_eq!(config::parse(&off).unwrap().updates, UpdateCheck::Off);
        // an explicit "off" with the box still empty is left alone
        assert_eq!(build_text(&snap(&off, false), &off).unwrap(), off);
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
            modifier: ClickModifier::Ctrl,
            image_folder: String::new(),
            mode: UiMode::Complete,
            updates: false,
            exclude: vec!["bitwarden.exe".into()],
            filters: vec![],
        };
        let out = build_text(&snap, &base).unwrap();
        let c = config::parse(&out).unwrap();
        assert_eq!(c.exclude_apps, vec!["bitwarden.exe".to_string()]);
        assert!(c.custom_filters.is_empty());
    }
}
