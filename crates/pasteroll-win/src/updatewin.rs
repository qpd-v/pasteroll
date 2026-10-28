//! The update window: one place that shows checking for updates, what is new, the download with a
//! progress bar, installing, and anything that goes wrong. It follows Windows' light or dark mode and
//! is drawn like the settings window (`settingsui`). It replaces the confirmation dialog and the
//! stream of notifications the update used to make; the notification icon only announces a new
//! version once. The work itself (checking, downloading, verifying, installing) is unchanged and lives
//! in `update` and `install`; this only shows it.
//!
//! The window runs its own message loop on its own thread. Worker threads do the network work and
//! tell it what to show by setting [`PAGE`] and posting [`WM_UI`].

use crate::install::{self, Done, Mode, Trust};
use crate::{fetch, installui, settingsui, update};
use pasteroll_core::update::Version;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

/// A worker changed [`PAGE`]: show it.
const WM_UI: u32 = WM_APP + 243;
/// A worker says the window is no longer needed (a cancelled download).
const WM_UI_CLOSE: u32 = WM_APP + 244;
const TIMER_BAR: usize = 31;

const ID_PRIMARY: i32 = 1001;
/// IDCANCEL, so Esc reaches it: "Not now", "Close" or "Cancel".
const ID_CLOSE: i32 = 2;
const ID_NOTES: i32 = 1003;
const ID_SKIP: i32 = 1004;
const ID_HEADING: i32 = 1010;
const ID_SUB: i32 = 1011;
const ID_NOTE: i32 = 1012;
const ID_SIZE: i32 = 1013;
const ID_PERCENT: i32 = 1014;
/// On the card, not on the page: ids from 1100 on.
const ID_NEWS_HEAD: i32 = 1100;
const ID_NEWS: i32 = 1101;

/// The client area is this wide (DIP at 96 dpi); its height depends on the page.
const WIDTH: i32 = 580;
const MARGIN: i32 = 28;
const INNER: i32 = WIDTH - 2 * MARGIN;
const BUTTON_H: i32 = 32;
const BAR_H: i32 = 10;

/// What the window shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    /// Asking GitHub, or getting the details of a version.
    Checking { text: String },
    /// A newer version, with the plain lines its signed note carried.
    Available { found: Version, notes: Vec<String> },
    /// Downloading and installing. `got` and `total` are bytes of the download.
    Working { step: String, got: u64, total: Option<u64>, can_cancel: bool },
    UpToDate(Version),
    Failed { title: String, text: String, retry: Retry },
    /// The program file was replaced; the new one is about to start.
    Restarting,
}

/// What "Try again" does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    Check,
    Install,
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Bar {
    None,
    /// Something is happening but its length is not known.
    Moving,
    Done(f32),
}

type Rect4 = [i32; 4];

/// Everything one page needs: words, buttons and where things go (DIP).
#[derive(Clone, Debug, PartialEq)]
struct View {
    heading: String,
    sub: String,
    sub_rect: Rect4,
    note: String,
    note_rect: Option<Rect4>,
    news_head: bool,
    news: String,
    news_rect: Option<Rect4>,
    card: Option<Rect4>,
    size: String,
    percent: String,
    bar: Bar,
    bar_rect: Rect4,
    primary: Option<(String, Rect4)>,
    close: Option<(String, Rect4)>,
    notes_button: Option<Rect4>,
    skip_button: Option<Rect4>,
    height: i32,
}

/// Roughly how many lines `text` takes at `per_line` characters a line.
fn lines(text: &str, per_line: usize) -> i32 {
    text.split('\n').map(|l| (l.chars().count().max(1)).div_ceil(per_line) as i32).sum::<i32>().max(1)
}

/// An error message as a sentence for a person: it starts with a capital and ends with a full stop.
fn sentence(text: &str) -> String {
    let text = text.trim();
    let mut out = String::with_capacity(text.len() + 1);
    let mut chars = text.chars();
    if let Some(first) = chars.next() {
        out.extend(first.to_uppercase());
        out.push_str(chars.as_str());
    }
    if !out.is_empty() && !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_048_576.0)
}

/// The sentence under the news: what Pasteroll will do, for this kind of copy.
pub fn explain(mode: &Mode) -> String {
    let what = match mode {
        Mode::Setup { all_users: false } => String::new(),
        Mode::Setup { all_users: true } => " This copy is installed for all users, so Windows will ask for administrator permission.".to_owned(),
        Mode::Replace { exe } => format!(" Pasteroll will replace its own program file ({}).", exe.display()),
        Mode::NotifyOnly(why) => return format!("This copy cannot update itself: {why}. The release page has the new version."),
    };
    format!(
        "Pasteroll will download it from GitHub, check that it is signed with the Pasteroll release key, and install it. Your history and settings are kept. Pasteroll closes and starts again by itself.{what}"
    )
}

/// The "What's new" lines as one text.
fn news_text(notes: &[String]) -> String {
    notes.iter().map(|n| format!("\u{2022}  {n}")).collect::<Vec<_>>().join("\n")
}

fn button_row(y: i32, primary: &str, close: &str) -> (Option<(String, Rect4)>, Option<(String, Rect4)>) {
    let pw = 168;
    let p = [WIDTH - MARGIN - pw, y, pw, BUTTON_H];
    let cw = 88;
    let c = [p[0] - 8 - cw, y, cw, BUTTON_H];
    (Some((primary.to_owned(), p)), Some((close.to_owned(), c)))
}

/// What to show for `page`. Pure: the window code only places what this says.
fn view(page: &Page, mode: &Mode, current: &Version) -> View {
    let mut v = View {
        heading: String::new(),
        sub: String::new(),
        sub_rect: [MARGIN, 58, INNER, 22],
        note: String::new(),
        note_rect: None,
        news_head: false,
        news: String::new(),
        news_rect: None,
        card: None,
        size: String::new(),
        percent: String::new(),
        bar: Bar::None,
        bar_rect: [MARGIN, 102, INNER, BAR_H],
        primary: None,
        close: None,
        notes_button: None,
        skip_button: None,
        height: 0,
    };
    let close_only = |y: i32, text: &str| Some((text.to_owned(), [WIDTH - MARGIN - 96, y, 96, BUTTON_H]));
    match page {
        Page::Checking { text } => {
            v.heading = "Checking for updates".into();
            v.sub = text.clone();
            v.bar = Bar::Moving;
            v.bar_rect[1] = 98;
            v.close = close_only(134, "Close");
            v.height = 134 + BUTTON_H + 24;
        }
        Page::Available { found, notes } => {
            v.heading = format!("Pasteroll {found} is available");
            v.sub = format!("You have Pasteroll {current}.");
            let mut y = 94;
            if !notes.is_empty() {
                let text = news_text(notes);
                let news_h = lines(&text, 90) * 17 + 4;
                let card_h = 40 + news_h + 12;
                v.card = Some([MARGIN, y, INNER, card_h]);
                v.news_head = true;
                v.news = text;
                v.news_rect = Some([MARGIN + 18, y + 38, INNER - 36, news_h]);
                y += card_h + 14;
            }
            v.note = explain(mode);
            let note_h = lines(&v.note, 92) * 17 + 2;
            v.note_rect = Some([MARGIN, y, INNER, note_h]);
            y += note_h + 14;
            let installable = !matches!(mode, Mode::NotifyOnly(_));
            let (p, c) = button_row(y, if installable { "Download and install" } else { "Release notes" }, "Not now");
            v.primary = p;
            v.close = c;
            if installable {
                v.notes_button = Some([MARGIN, y, 112, BUTTON_H]);
                v.skip_button = Some([MARGIN + 112 + 8, y, 132, BUTTON_H]);
            }
            v.height = y + BUTTON_H + 24;
        }
        Page::Working { step, got, total, can_cancel } => {
            v.heading = "Updating Pasteroll".into();
            v.sub = step.clone();
            v.bar_rect[1] = 100;
            match total.filter(|t| *t > 0) {
                Some(t) if *got > 0 => {
                    v.bar = Bar::Done((*got as f32 / t as f32).clamp(0.0, 1.0));
                    v.size = format!("{} of {} MB", megabytes(*got), megabytes(t));
                    v.percent = format!("{}%", (*got * 100 / t).min(100));
                }
                _ => v.bar = Bar::Moving,
            }
            v.note = "Pasteroll closes and starts again by itself when the update is installed. Your history and settings are kept.".into();
            v.note_rect = Some([MARGIN, 160, INNER, 36]);
            if *can_cancel {
                v.close = close_only(210, "Cancel");
            }
            v.height = 210 + BUTTON_H + 24;
        }
        Page::UpToDate(version) => {
            v.heading = "Pasteroll is up to date".into();
            v.sub = format!("You have Pasteroll {version}, the newest version.");
            v.primary = close_only(98, "Close");
            v.height = 98 + BUTTON_H + 24;
        }
        Page::Failed { title, text, retry } => {
            v.heading = title.clone();
            v.sub = text.clone();
            let sub_h = lines(text, 88).min(6) * 17 + 2;
            v.sub_rect[3] = sub_h;
            let y = 58 + sub_h + 18;
            if *retry == Retry::Nothing {
                v.primary = close_only(y, "Close");
            } else {
                let (p, c) = button_row(y, "Try again", "Close");
                v.primary = p;
                v.close = c;
            }
            v.height = y + BUTTON_H + 24;
        }
        Page::Restarting => {
            v.heading = "Update installed".into();
            v.sub = "Pasteroll is restarting.".into();
            v.bar = Bar::Moving;
            v.bar_rect[1] = 98;
            v.height = 98 + BAR_H + 40;
        }
    }
    v
}

// ------------------------------------------------------------------ shared state

/// The window's handle while one is open (0 when none is).
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// What the window should show; set by whichever thread has news, then [`WM_UI`] is posted.
static PAGE: Mutex<Option<Page>> = Mutex::new(None);
/// Set when the window is closing, so a late worker does not post into a new window's place.
static CLOSING: AtomicBool = AtomicBool::new(false);

fn post(msg: u32) {
    let h = WINDOW.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), msg, WPARAM(0), LPARAM(0));
        }
    }
}

fn set_page(page: Page) {
    if let Ok(mut p) = PAGE.lock() {
        *p = Some(page);
    }
    post(WM_UI);
}

// ------------------------------------------------------------------ opening

/// What the window starts by doing.
#[derive(Clone, Debug)]
enum Start {
    /// Ask GitHub (the person chose "Check for updates now").
    Check,
    /// Show this newer version (a check found it earlier) and offer it.
    Offer(Version),
    /// Show a page as it is, doing nothing (for a picture of the window).
    Show(Page),
}

/// "Check for updates now": opens the window and checks.
pub fn open_check(tray: isize, db: Option<PathBuf>) {
    open(Start::Check, tray, db);
}

/// A newer version is known: opens the window offering it.
pub fn open_offer(tray: isize, db: Option<PathBuf>, found: Version) {
    open(Start::Offer(found), tray, db);
}

fn open(start: Start, tray: isize, db: Option<PathBuf>) {
    let existing = WINDOW.load(Ordering::Acquire);
    if existing != 0 {
        // one window: bring it forward
        unsafe {
            let h = HWND(existing as *mut _);
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("pasteroll-update-window".into())
        .spawn(move || {
            if let Err(e) = run(start, tray, db) {
                eprintln!("[pasteroll] could not open the update window: {e}");
            }
        });
    if let Err(e) = spawned {
        eprintln!("[pasteroll] could not open the update window: {e}");
    }
}

// ------------------------------------------------------------------ the window

struct Controls {
    heading: HWND,
    sub: HWND,
    note: HWND,
    size: HWND,
    percent: HWND,
    news_head: HWND,
    news: HWND,
    primary: HWND,
    close: HWND,
    notes: HWND,
    skip: HWND,
}

struct Ctx {
    tray: isize,
    db: Option<PathBuf>,
    dpi: u32,
    controls: Controls,
    fonts: [HFONT; 3],
    mode: Mode,
    current: Version,
    page: Page,
    view: View,
    /// The version on offer, for the buttons.
    found: Option<Version>,
    /// Steps of the moving bar.
    phase: u32,
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

#[allow(clippy::too_many_arguments)]
fn make_child(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = wide(text);
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(t.as_ptr()),
            WS_CHILD | WINDOW_STYLE(style),
            0,
            0,
            10,
            10,
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

/// The three fonts of the window at `dpi`: the text, the small heading and the big heading.
fn make_fonts(dpi: u32) -> [HFONT; 3] {
    let make = |tenths: i32, weight: i32| unsafe {
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
    [make(90, 400), make(95, 600), make(140, 600)]
}

fn run(start: Start, tray: isize, db: Option<PathBuf>) -> windows::core::Result<()> {
    CLOSING.store(false, Ordering::Release);
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH::default(),
            hIcon: LoadIconW(Some(hinst.into()), PCWSTR(1usize as *const u16)).unwrap_or_default(),
            lpszClassName: w!("PasterollUpdate"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("PasterollUpdate"),
            w!("Pasteroll update"),
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
        let [font, bold, big] = make_fonts(dpi);
        let theme = settingsui::Theme::current();
        settingsui::apply_title_bar(hwnd, theme.dark);
        settingsui::install(settingsui::Look::new(theme, font));

        let (single, wrap) = (0x80u32 | 0x4000, 0x80u32); // SS_NOPREFIX (| SS_ENDELLIPSIS)
        let tab = WS_TABSTOP.0;
        let controls = Controls {
            heading: make_child(hwnd, w!("STATIC"), "", single, ID_HEADING, big),
            sub: make_child(hwnd, w!("STATIC"), "", wrap, ID_SUB, font),
            note: make_child(hwnd, w!("STATIC"), "", wrap, ID_NOTE, font),
            size: make_child(hwnd, w!("STATIC"), "", single, ID_SIZE, font),
            percent: make_child(hwnd, w!("STATIC"), "", single | 0x2, ID_PERCENT, font),
            news_head: make_child(hwnd, w!("STATIC"), "What's new", single, ID_NEWS_HEAD, bold),
            news: make_child(hwnd, w!("STATIC"), "", wrap, ID_NEWS, font),
            primary: make_child(hwnd, w!("BUTTON"), "", BS_PUSHBUTTON as u32 | tab, ID_PRIMARY, font),
            close: make_child(hwnd, w!("BUTTON"), "", BS_PUSHBUTTON as u32 | tab, ID_CLOSE, font),
            notes: make_child(hwnd, w!("BUTTON"), "Release notes", BS_PUSHBUTTON as u32 | tab, ID_NOTES, font),
            skip: make_child(hwnd, w!("BUTTON"), "Skip this version", BS_PUSHBUTTON as u32 | tab, ID_SKIP, font),
        };
        // a picture of the window shows the usual kind of copy, whatever this one is
        let mode = if matches!(start, Start::Show(_)) { Mode::Setup { all_users: false } } else { installui::mode().clone() };
        let current = Version::current();
        let page = match &start {
            Start::Show(p) => p.clone(),
            Start::Check => Page::Checking { text: "Asking GitHub for the newest version...".into() },
            Start::Offer(_) => Page::Checking { text: "Getting the details...".into() },
        };
        let found = match &page {
            Page::Available { found, .. } => Some(found.clone()),
            _ => match &start {
                Start::Offer(v) => Some(v.clone()),
                _ => None,
            },
        };
        let first = view(&page, &mode, &current);
        CTX.with(|c| {
            *c.borrow_mut() = Some(Ctx {
                tray,
                db: db.clone(),
                dpi,
                controls,
                fonts: [font, bold, big],
                mode,
                current,
                page: page.clone(),
                view: first.clone(),
                found,
                phase: 0,
            });
        });
        WINDOW.store(hwnd.0 as isize, Ordering::Release);
        if let Ok(mut p) = PAGE.lock() {
            *p = Some(page);
        }
        apply_view(hwnd, true);
        place_window(hwnd, dpi, first.height);

        match start {
            Start::Check => spawn_check(db.clone()),
            Start::Offer(v) => spawn_details(v),
            Start::Show(_) => {}
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    Ok(())
}

/// Sizes the window so its client area is `height` DIP tall, keeping it centred where it first was.
fn place_window(hwnd: HWND, dpi: u32, height: i32) {
    unsafe {
        let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
        let mut r = RECT { left: 0, top: 0, right: s(WIDTH, dpi), bottom: s(height, dpi) };
        let _ = AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let mut now = RECT::default();
        let first = GetWindowRect(hwnd, &mut now).is_err() || (now.right - now.left) <= 120;
        let mut work = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let (x, y) = if first {
            (work.left + ((work.right - work.left) - ww) / 2, work.top + ((work.bottom - work.top) - wh) / 3)
        } else {
            (now.left, now.top)
        };
        let _ = SetWindowPos(hwnd, None, x.max(work.left), y.max(work.top), ww, wh, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn set_text(h: HWND, text: &str) {
    let t = wide(text);
    unsafe {
        let _ = SetWindowTextW(h, PCWSTR(t.as_ptr()));
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

fn place(h: HWND, r: Option<Rect4>, dpi: u32) {
    unsafe {
        match r {
            Some(r) => {
                let _ = MoveWindow(h, s(r[0], dpi), s(r[1], dpi), s(r[2], dpi), s(r[3], dpi), true);
                let _ = ShowWindow(h, SW_SHOWNA);
            }
            None => {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
    }
}

/// Puts the page in `CTX.page` on the window: words, buttons, cards and the bar. `force` redoes
/// everything even if nothing changed.
fn apply_view(hwnd: HWND, force: bool) {
    let applied = CTX.with(|c| {
        let Ok(mut g) = c.try_borrow_mut() else { return None };
        let ctx = g.as_mut()?;
        let new = view(&ctx.page, &ctx.mode, &ctx.current);
        if !force && new == ctx.view {
            return Some((false, ctx.dpi, new.height, false));
        }
        let old = std::mem::replace(&mut ctx.view, new.clone());
        let d = ctx.dpi;
        let ct = &ctx.controls;
        let text_if = |h: HWND, t: &str| {
            if get_text(h) != t {
                set_text(h, t);
            }
        };
        text_if(ct.heading, &new.heading);
        text_if(ct.sub, &new.sub);
        text_if(ct.note, &new.note);
        text_if(ct.size, &new.size);
        text_if(ct.percent, &new.percent);
        text_if(ct.news, &new.news);
        place(ct.heading, Some([MARGIN, 22, INNER, 34]), d);
        place(ct.sub, if new.sub.is_empty() { None } else { Some(new.sub_rect) }, d);
        place(ct.note, new.note_rect, d);
        place(ct.news_head, new.card.map(|c| [c[0] + 18, c[1] + 10, 300, 22]), d);
        place(ct.news, new.news_rect, d);
        let bar_texts = matches!(new.bar, Bar::Done(_));
        place(ct.size, bar_texts.then_some([MARGIN, new.bar_rect[1] + BAR_H + 8, 340, 20]), d);
        place(ct.percent, bar_texts.then_some([WIDTH - MARGIN - 120, new.bar_rect[1] + BAR_H + 8, 120, 20]), d);
        let button = |h: HWND, b: &Option<(String, Rect4)>| {
            if let Some((t, _)) = b {
                text_if(h, t);
            }
            place(h, b.as_ref().map(|(_, r)| *r), d);
        };
        button(ct.primary, &new.primary);
        button(ct.close, &new.close);
        place(ct.notes, new.notes_button, d);
        place(ct.skip, new.skip_button, d);
        // the card behind the news
        settingsui::with(|look| look.cards = new.card.into_iter().collect());
        Some((true, d, new.height, old.height != new.height))
    });
    let Some((changed, dpi, height, resized)) = applied else { return };
    if !changed {
        return;
    }
    if resized {
        place_window(hwnd, dpi, height);
    }
    unsafe {
        // the moving bar needs its timer; a still page does not
        let moving = CTX.with(|c| c.try_borrow().ok().and_then(|g| g.as_ref().map(|x| x.view.bar == Bar::Moving))).unwrap_or(false);
        if moving {
            SetTimer(Some(hwnd), TIMER_BAR, 30, None);
        } else {
            let _ = KillTimer(Some(hwnd), TIMER_BAR);
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
        // the safe button is the one Enter presses
        let focus = CTX.with(|c| {
            c.try_borrow().ok().and_then(|g| {
                g.as_ref().map(|x| match x.page {
                    Page::Available { .. } => x.controls.close,
                    _ if x.view.primary.is_some() => x.controls.primary,
                    _ => x.controls.close,
                })
            })
        });
        if let Some(f) = focus {
            if IsWindowVisible(f).as_bool() && GetFocus() != f {
                let _ = SetFocus(Some(f));
            }
        }
    }
}

// ------------------------------------------------------------------ workers

fn spawn_check(db: Option<PathBuf>) {
    let _ = std::thread::Builder::new().name("pasteroll-update-check-now".into()).spawn(move || {
        set_page(Page::Checking { text: "Asking GitHub for the newest version...".into() });
        let current = Version::current();
        match update::check_now_blocking(db.as_deref()) {
            None => set_page(Page::Failed {
                title: "Could not check for updates".into(),
                text: "A check is already running. Try again in a moment.".into(),
                retry: Retry::Check,
            }),
            Some(update::Outcome::Newer(found)) => set_page(Page::Available { found: found.version, notes: found.notes }),
            Some(update::Outcome::UpToDate(_)) => set_page(Page::UpToDate(current)),
            Some(update::Outcome::Failed(e)) => {
                set_page(Page::Failed { title: "Could not check for updates".into(), text: sentence(&e), retry: Retry::Check });
            }
        }
    });
}

/// A newer version is known: get what its signed note says, then show it.
fn spawn_details(found: Version) {
    let _ = std::thread::Builder::new().name("pasteroll-update-details".into()).spawn(move || {
        let known = update::newer_found().filter(|f| f.version == found).map(|f| f.notes).filter(|n| !n.is_empty());
        let notes = known.unwrap_or_else(|| update::verified_notes(&found));
        set_page(Page::Available { found, notes });
    });
}

/// Downloads, verifies and installs `found`, showing each step. The work is `install::run_update`.
fn spawn_install(found: Version, tray: isize, db: Option<PathBuf>, mode: Mode) {
    let spawned = std::thread::Builder::new().name("pasteroll-update-install".into()).spawn(move || {
        fetch::reset_cancel();
        let current = Version::current();
        let Some(data_dir) = db.as_ref().and_then(|d| d.parent()).map(PathBuf::from) else {
            set_page(Page::Failed { title: "The update was not installed".into(), text: "Pasteroll's data folder is unknown.".into(), retry: Retry::Nothing });
            return;
        };
        let step = std::sync::Arc::new(Mutex::new((String::from("Starting..."), true)));
        set_page(Page::Working { step: "Starting...".into(), got: 0, total: None, can_cancel: true });
        let step_say = step.clone();
        let step_progress = step.clone();
        let result = install::run_update(
            &Trust::github(),
            &mode,
            &found,
            &current,
            &data_dir,
            &mut |text| {
                // only the checking and the download can still be cancelled
                let can_cancel = text.starts_with("Checking") || text.starts_with("Downloading");
                if let Ok(mut s) = step_say.lock() {
                    *s = (text.to_owned(), can_cancel);
                }
                set_page(Page::Working { step: text.to_owned(), got: 0, total: None, can_cancel });
            },
            &mut |got, total| {
                let (text, can_cancel) = step_progress.lock().map(|s| s.clone()).unwrap_or_default();
                set_page(Page::Working { step: text, got, total, can_cancel });
            },
        );
        match result {
            Ok(Done::SetupStarted) => {
                set_page(Page::Working {
                    step: "Installing. Pasteroll will close and start again by itself.".into(),
                    got: 0,
                    total: None,
                    can_cancel: false,
                });
                installui::setup_started(tray);
            }
            Ok(Done::Replaced(exe)) => {
                set_page(Page::Restarting);
                installui::replaced(tray, exe);
            }
            Err(e) => {
                if fetch::cancel_requested() {
                    post(WM_UI_CLOSE);
                } else {
                    set_page(Page::Failed { title: "The update was not installed".into(), text: sentence(&e), retry: Retry::Install });
                }
            }
        }
    });
    if spawned.is_err() {
        set_page(Page::Failed { title: "The update was not installed".into(), text: "Could not start the update.".into(), retry: Retry::Nothing });
    }
}

// ------------------------------------------------------------------ messages

fn with_ctx<R>(f: impl FnOnce(&mut Ctx) -> R) -> Option<R> {
    CTX.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// The person pressed the button `id`.
fn command(hwnd: HWND, id: i32) {
    let Some((page, found, tray, db, mode)) = with_ctx(|c| (c.page.clone(), c.found.clone(), c.tray, c.db.clone(), c.mode.clone())) else { return };
    match (id, &page) {
        (ID_PRIMARY, Page::Available { found, .. }) => {
            if matches!(mode, Mode::NotifyOnly(_)) {
                installui::open_page(found);
            } else {
                with_ctx(|c| c.found = Some(found.clone()));
                spawn_install(found.clone(), tray, db, mode);
            }
        }
        (ID_PRIMARY, Page::Failed { retry: Retry::Check, .. }) => {
            set_page(Page::Checking { text: "Asking GitHub for the newest version...".into() });
            spawn_check(db);
        }
        (ID_PRIMARY, Page::Failed { retry: Retry::Install, .. }) => {
            if let Some(v) = found {
                spawn_install(v, tray, db, mode);
            }
        }
        (ID_PRIMARY, Page::UpToDate(_) | Page::Failed { .. }) => close_request(hwnd),
        (ID_NOTES, Page::Available { found, .. }) => installui::open_page(found),
        (ID_SKIP, Page::Available { found, .. }) => {
            if let Some(db) = &db {
                update::skip(db, found);
            }
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        (ID_CLOSE, _) => close_request(hwnd),
        _ => {}
    }
}

/// Close, Not now, Esc or the X: closes, except while an update is being installed (it cannot be
/// stopped then) and that a download in progress is cancelled instead.
fn close_request(hwnd: HWND) {
    let page = with_ctx(|c| c.page.clone());
    match page {
        Some(Page::Working { can_cancel: true, .. }) => {
            fetch::request_cancel();
            set_page(Page::Working { step: "Cancelling...".into(), got: 0, total: None, can_cancel: false });
        }
        Some(Page::Working { .. } | Page::Restarting) => {}
        _ => unsafe {
            let _ = DestroyWindow(hwnd);
        },
    }
}

fn paint(hwnd: HWND, hdc: HDC) {
    let Some((view, dpi, phase)) = CTX.with(|c| c.try_borrow().ok().and_then(|g| g.as_ref().map(|x| (x.view.clone(), x.dpi, x.phase)))) else { return };
    settingsui::with(|look| unsafe {
        use pasteroll_render::widgets::Shape;
        let t = look.theme;
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return;
        }
        let sc = dpi as f32 / 96.0;
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old = SelectObject(mem, bmp.into());
        FillRect(mem, &rc, look.bg);
        if let Some(c) = view.card {
            let shape = Shape::Box { radius: 8.0, fill: Some(t.card), border: Some(t.border) };
            settingsui::blit(mem, s(c[0], dpi), s(c[1], dpi), s(c[2], dpi), s(c[3], dpi), shape, sc);
        }
        let b = view.bar_rect;
        let (bx, by, bw, bh) = (s(b[0], dpi), s(b[1], dpi), s(b[2], dpi), s(b[3], dpi));
        match view.bar {
            Bar::None => {}
            Bar::Done(f) => {
                settingsui::blit(mem, bx, by, bw, bh, Shape::Track { filled: f, accent: t.accent, rest: t.track }, sc);
            }
            Bar::Moving => {
                settingsui::blit(mem, bx, by, bw, bh, Shape::Track { filled: 0.0, accent: t.accent, rest: t.track }, sc);
                // a block that slides across and starts again
                let seg = bw * 3 / 10;
                let travel = bw + seg;
                let at = ((phase as i32 * s(6, dpi)) % travel) - seg;
                let (x0, x1) = (at.max(0), (at + seg).min(bw));
                if x1 > x0 {
                    settingsui::blit(mem, bx + x0, by, x1 - x0, bh, Shape::Box { radius: b[3] as f32 / 2.0, fill: Some(t.accent), border: None }, sc);
                }
            }
        }
        let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
    });
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_UI => {
                let page = PAGE.lock().ok().and_then(|mut p| p.take());
                if let Some(page) = page {
                    #[cfg(feature = "e2e-test-server")]
                    e2e_note(&page);
                    with_ctx(|c| {
                        if let Page::Available { found, .. } = &page {
                            c.found = Some(found.clone());
                        }
                        c.page = page;
                    });
                    apply_view(hwnd, false);
                }
                LRESULT(0)
            }
            WM_UI_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_BAR => {
                let rect = with_ctx(|c| {
                    c.phase = c.phase.wrapping_add(1);
                    let b = c.view.bar_rect;
                    RECT { left: s(b[0], c.dpi), top: s(b[1], c.dpi), right: s(b[0] + b[2], c.dpi), bottom: s(b[1] + b[3], c.dpi) }
                });
                if let Some(r) = rect {
                    let _ = InvalidateRect(Some(hwnd), Some(&r), true);
                }
                LRESULT(0)
            }
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
                // a click (or Esc) has notification code 0; anything else is not a press
                if (wparam.0 >> 16) & 0xFFFF == 0 {
                    command(hwnd, (wparam.0 & 0xFFFF) as i32);
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
            WM_DPICHANGED => {
                // moved to a monitor with another scale: new fonts, everything laid out again
                let new = (wparam.0 & 0xFFFF) as u32;
                let suggested = *(lparam.0 as *const RECT);
                let changed = with_ctx(|c| {
                    if new == 0 || new == c.dpi {
                        return None;
                    }
                    c.dpi = new;
                    let fresh = make_fonts(new);
                    let ct = &c.controls;
                    let set = |h: HWND, f: HFONT| {
                        SendMessageW(h, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1)));
                    };
                    for h in [ct.sub, ct.note, ct.size, ct.percent, ct.news, ct.primary, ct.close, ct.notes, ct.skip] {
                        set(h, fresh[0]);
                    }
                    set(ct.news_head, fresh[1]);
                    set(ct.heading, fresh[2]);
                    let old = std::mem::replace(&mut c.fonts, fresh);
                    for f in old {
                        let _ = DeleteObject(f.into());
                    }
                    Some((fresh[0], c.view.height))
                })
                .flatten();
                if let Some((font, height)) = changed {
                    settingsui::with(|l| l.font = font);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    apply_view(hwnd, true);
                    place_window(hwnd, new, height);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                close_request(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                CLOSING.store(true, Ordering::Release);
                WINDOW.store(0, Ordering::Release);
                let _ = KillTimer(Some(hwnd), TIMER_BAR);
                if let Some(ctx) = CTX.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.take())) {
                    for f in ctx.fonts {
                        let _ = DeleteObject(f.into());
                    }
                }
                settingsui::remove();
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// ------------------------------------------------------------------ end-to-end test build only

/// What the window showed, in order (the end-to-end test reads it).
#[cfg(feature = "e2e-test-server")]
static E2E_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(feature = "e2e-test-server")]
fn e2e_note(page: &Page) {
    let line = match page {
        Page::Checking { text } => format!("checking: {text}"),
        Page::Available { notes, .. } => format!("available notes={}", notes.len()),
        Page::Working { step, got, total, can_cancel } => format!("working: {step} ({got}/{total:?}, cancel {can_cancel})"),
        Page::UpToDate(_) => "uptodate".to_owned(),
        Page::Failed { text, .. } => format!("failed: {text}"),
        Page::Restarting => "restarting".to_owned(),
    };
    if let Ok(mut log) = E2E_LOG.lock() {
        log.push(line);
    }
}

/// Opens the real window offering `PASTEROLL_UPDATE_ANNOUNCE`, presses "Download and install" once it
/// is offered, prints every page it goes through, and returns 0 when it ends on "restarting" (the
/// program file was replaced) or 1 when it ends on a failure. For the end-to-end test only.
#[cfg(feature = "e2e-test-server")]
pub fn e2e(db: &std::path::Path) -> i32 {
    let Some(found) = std::env::var("PASTEROLL_UPDATE_ANNOUNCE").ok().and_then(|v| Version::parse(&v)) else {
        eprintln!("PASTEROLL_UPDATE_ANNOUNCE names the version to offer");
        return 2;
    };
    open(Start::Offer(found), 0, Some(db.to_path_buf()));
    let started = std::time::Instant::now();
    let (mut clicked, mut shown) = (false, 0usize);
    let code = loop {
        let log = E2E_LOG.lock().map(|l| l.clone()).unwrap_or_default();
        for line in &log[shown..] {
            println!("page: {line}");
        }
        shown = log.len();
        if !clicked && log.iter().any(|l| l.starts_with("available")) {
            let h = WINDOW.load(Ordering::Acquire);
            if h != 0 {
                unsafe {
                    let _ = PostMessageW(Some(HWND(h as *mut _)), WM_COMMAND, WPARAM(ID_PRIMARY as usize), LPARAM(0));
                }
                clicked = true;
            }
        }
        match log.last().map(String::as_str) {
            Some("restarting") => break 0,
            Some(l) if l.starts_with("failed") => break 1,
            _ => {}
        }
        if started.elapsed() > std::time::Duration::from_secs(100) {
            eprintln!("the window did not finish in time");
            break 3;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    println!("done: {code}");
    code
}

// ------------------------------------------------------------------ a picture of the window

/// Debug aid: opens the window on `page` (no network, nothing happens), saves a picture of it to
/// `out` and closes it. `page` is one of `checking`, `available`, `available-long`, `downloading`,
/// `installing`, `uptodate`, `failed`, `restarting`.
pub fn snapshot(page: &str, out: &std::path::Path) -> Result<(), String> {
    let v = Version::parse("0.1.0-beta.13").ok_or("version")?;
    let notes = vec![
        "A short line about what changed in this version.".to_owned(),
        "Another short line, a little longer so that it shows how a long line wraps in the list.".to_owned(),
        "A third line.".to_owned(),
    ];
    let page = match page {
        "checking" => Page::Checking { text: "Asking GitHub for the newest version...".into() },
        "available" => Page::Available { found: v, notes },
        "available-bare" => Page::Available { found: v, notes: Vec::new() },
        "downloading" => Page::Working { step: "Downloading the update...".into(), got: 3_300_000, total: Some(4_950_000), can_cancel: true },
        "installing" => Page::Working { step: "Installing. Pasteroll will close and start again by itself.".into(), got: 0, total: None, can_cancel: false },
        "uptodate" => Page::UpToDate(Version::current()),
        "failed" => Page::Failed {
            title: "The update was not installed".into(),
            text: "The downloaded file did not match its checksum, so it was deleted and nothing was changed.".into(),
            retry: Retry::Install,
        },
        "restarting" => Page::Restarting,
        other => return Err(format!("unknown page {other}")),
    };
    // the window runs its own message loop, so it is opened on a thread and looked at from here
    let opened = std::thread::spawn(move || {
        let _ = run(Start::Show(page), 0, None);
    });
    let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while WINDOW.load(Ordering::Acquire) == 0 && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(700));
    let h = WINDOW.load(Ordering::Acquire);
    if h == 0 {
        return Err("the window did not open".into());
    }
    let hwnd = HWND(h as *mut _);
    let picture = settingsui::capture_window(hwnd);
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_UI_CLOSE, WPARAM(0), LPARAM(0));
    }
    let _ = opened.join();
    std::fs::write(out, picture?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cur() -> Version {
        Version::parse("0.1.0-beta.12").unwrap()
    }
    fn newer() -> Version {
        Version::parse("0.1.0-beta.13").unwrap()
    }
    fn setup() -> Mode {
        Mode::Setup { all_users: false }
    }
    fn inside(r: Rect4, height: i32) -> bool {
        r[0] >= 0 && r[1] >= 0 && r[0] + r[2] <= WIDTH && r[1] + r[3] <= height
    }
    fn overlap(a: Rect4, b: Rect4) -> bool {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    }

    fn every_page() -> Vec<Page> {
        let notes = vec!["One.".to_owned(), "Two, a bit longer than the first one so it takes more room.".to_owned()];
        vec![
            Page::Checking { text: "Asking GitHub...".into() },
            Page::Available { found: newer(), notes: notes.clone() },
            Page::Available { found: newer(), notes: Vec::new() },
            Page::Working { step: "Downloading the update...".into(), got: 1_000_000, total: Some(4_000_000), can_cancel: true },
            Page::Working { step: "Installing.".into(), got: 0, total: None, can_cancel: false },
            Page::UpToDate(cur()),
            Page::Failed { title: "The update was not installed".into(), text: "x ".repeat(200), retry: Retry::Install },
            Page::Failed { title: "No".into(), text: "short".into(), retry: Retry::Nothing },
            Page::Restarting,
        ]
    }

    #[test]
    fn everything_on_every_page_fits_in_the_window_and_buttons_do_not_overlap() {
        for mode in [setup(), Mode::Setup { all_users: true }, Mode::Replace { exe: PathBuf::from(r"D:\tools\pasteroll.exe") }, Mode::NotifyOnly("x".into())] {
            for page in every_page() {
                let v = view(&page, &mode, &cur());
                let mut rects: Vec<(&str, Rect4)> = vec![("sub", v.sub_rect)];
                if let Some((_, r)) = &v.primary {
                    rects.push(("primary", *r));
                }
                if let Some((_, r)) = &v.close {
                    rects.push(("close", *r));
                }
                for (name, r) in [("notes", v.notes_button), ("skip", v.skip_button), ("note", v.note_rect), ("news", v.news_rect), ("card", v.card)] {
                    if let Some(r) = r {
                        rects.push((name, r));
                    }
                }
                for (name, r) in &rects {
                    assert!(inside(*r, v.height), "{name} of {page:?} is outside the window: {r:?} in {}", v.height);
                }
                let buttons: Vec<_> = rects.iter().filter(|(n, _)| ["primary", "close", "notes", "skip"].contains(n)).collect();
                for (i, (na, a)) in buttons.iter().enumerate() {
                    for (nb, b) in &buttons[i + 1..] {
                        assert!(!overlap(*a, *b), "{na} and {nb} overlap on {page:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_download_shows_size_and_percent_and_an_unknown_length_just_moves() {
        let page = Page::Working { step: "Downloading".into(), got: 1_572_864, total: Some(4_718_592), can_cancel: true };
        let v = view(&page, &setup(), &cur());
        assert_eq!((v.size.as_str(), v.percent.as_str()), ("1.5 of 4.5 MB", "33%"));
        assert!(matches!(v.bar, Bar::Done(f) if (f - 1.0 / 3.0).abs() < 0.001));
        let moving = view(&Page::Working { step: "Verifying".into(), got: 0, total: None, can_cancel: false }, &setup(), &cur());
        assert_eq!((moving.bar, moving.size.as_str()), (Bar::Moving, ""));
        let start = view(&Page::Working { step: "Downloading".into(), got: 0, total: Some(10), can_cancel: true }, &setup(), &cur());
        assert_eq!(start.bar, Bar::Moving, "nothing has arrived yet");
    }

    #[test]
    fn only_a_download_can_be_cancelled() {
        let cancel = view(&Page::Working { step: "Downloading".into(), got: 5, total: Some(10), can_cancel: true }, &setup(), &cur());
        assert_eq!(cancel.close.as_ref().map(|c| c.0.as_str()), Some("Cancel"));
        let installing = view(&Page::Working { step: "Installing".into(), got: 0, total: None, can_cancel: false }, &setup(), &cur());
        assert!(installing.close.is_none() && installing.primary.is_none(), "nothing to press while installing");
    }

    #[test]
    fn the_offer_names_the_versions_lists_the_news_and_says_what_will_happen() {
        let notes = vec!["Check now in Settings.".to_owned(), "Skip a version.".to_owned()];
        let v = view(&Page::Available { found: newer(), notes }, &setup(), &cur());
        assert!(v.heading.contains("0.1.0-beta.13") && v.sub.contains("0.1.0-beta.12"));
        assert!(v.news.contains("\u{2022}  Check now in Settings.\n\u{2022}  Skip a version."), "{}", v.news);
        assert!(v.note.contains("signed") && v.note.contains("history and settings are kept"));
        assert!(!v.note.contains("administrator"));
        assert_eq!(v.primary.as_ref().map(|p| p.0.as_str()), Some("Download and install"));
        assert_eq!(v.close.as_ref().map(|p| p.0.as_str()), Some("Not now"));
        assert!(v.notes_button.is_some() && v.skip_button.is_some() && v.card.is_some());
        let bare = view(&Page::Available { found: newer(), notes: Vec::new() }, &setup(), &cur());
        assert!(bare.card.is_none() && !bare.news_head, "no notes, no card");
    }

    #[test]
    fn each_kind_of_copy_hears_what_happens_to_it() {
        assert!(explain(&Mode::Setup { all_users: true }).contains("administrator permission"));
        let portable = explain(&Mode::Replace { exe: PathBuf::from(r"D:\tools\pasteroll.exe") });
        assert!(portable.contains(r"D:\tools\pasteroll.exe") && portable.contains("replace its own program file"));
        let none = view(&Page::Available { found: newer(), notes: Vec::new() }, &Mode::NotifyOnly("it is read-only".into()), &cur());
        assert!(none.note.contains("cannot update itself") && none.note.contains("read-only"));
        assert_eq!(none.primary.as_ref().map(|p| p.0.as_str()), Some("Release notes"));
        assert!(none.skip_button.is_none(), "nothing to install, nothing to skip");
    }

    #[test]
    fn a_failure_offers_try_again_only_when_it_can() {
        let again = view(&Page::Failed { title: "t".into(), text: "x".into(), retry: Retry::Check }, &setup(), &cur());
        assert_eq!(again.primary.as_ref().map(|p| p.0.as_str()), Some("Try again"));
        let none = view(&Page::Failed { title: "t".into(), text: "x".into(), retry: Retry::Nothing }, &setup(), &cur());
        assert_eq!(none.primary.as_ref().map(|p| p.0.as_str()), Some("Close"));
        assert!(none.close.is_none());
    }

    #[test]
    fn an_error_becomes_a_sentence() {
        assert_eq!(sentence("the download was cut short"), "The download was cut short.");
        assert_eq!(sentence("  Already a sentence.  "), "Already a sentence.");
        assert_eq!(sentence("really?"), "Really?");
        assert_eq!(sentence(""), "");
        assert_eq!(sentence("\u{e9}cole"), "\u{c9}cole.");
    }

    #[test]
    fn line_estimates_grow_with_the_text() {
        assert_eq!(lines("short", 80), 1);
        assert_eq!(lines(&"x".repeat(161), 80), 3);
        assert_eq!(lines("a\nb\nc", 80), 3);
    }
}
