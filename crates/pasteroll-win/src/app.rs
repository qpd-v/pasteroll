//! The Windows runtime: owns the message window, the hooks, the overlay window and the controller,
//! and turns window messages into controller events.
//!
//! Everything runs on one thread. Window messages can arrive while we are inside a handler (for
//! example `SetWindowPos` sends `WM_SIZE`), so the runtime lives in a `RefCell` and a message that
//! arrives re-entrantly is dropped instead of aliased.

use crate::clipboard::{self, WinClipboard};
use crate::hooks::{Command, Hooks, WM_PASTEROLL, WM_PASTEROLL_CHAR, WM_PASTEROLL_WHEEL};
use crate::keystore;
use crate::overlay::OverlayWindow;
use crate::platform::WinPlatform;
use crate::tray::Tray;
use pasteroll_app::{
    Controller, Event, IngestPolicy, Key, Logger, PasteKeys, PastePolicy, StoreFactory, Waker, Wheel,
};
use pasteroll_core::config::UiMode;
use pasteroll_core::Step;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// A worker finished: deliver `Event::Wake`.
const WM_WAKE: u32 = WM_APP + 221;
pub const TIMER_TICK: usize = 1;

pub struct Options {
    pub db: PathBuf,
    pub capture: bool,
    pub accept_injected: bool,
    pub max_seconds: Option<u64>,
    /// Install the global input hooks (off for the overlay self-test).
    pub hooks: bool,
    /// Show the notification-area icon (off for the self-tests).
    pub tray: bool,
    /// Show the first-run privacy notice when it has not been shown yet.
    pub notice: bool,
}

pub struct Runtime {
    pub controller: Controller,
    pub platform: WinPlatform,
    quit_at: Option<Instant>,
    db: PathBuf,
    tray: Option<Tray>,
    /// Shared lock on the history for as long as the app runs: a key replacement cannot happen
    /// under it, and the app cannot start while one is in progress.
    _history_lock: crate::winsys::FileLock,
}

thread_local! {
    static RT: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Delivers one event to the controller. Dropped if we are already inside a handler.
pub fn dispatch(ev: Event) {
    RT.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(rt) = guard.as_mut() {
                rt.controller.handle(&mut rt.platform, ev);
            }
        }
    });
}

/// Runs `f` against the runtime (self-tests inspect controller state and the overlay).
pub fn with_runtime<R>(f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    RT.with(|cell| cell.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

struct WinWaker(isize);

impl Waker for WinWaker {
    fn wake(&self) {
        unsafe {
            // A doorbell only: the results are in a channel, never in the message.
            let _ = PostMessageW(Some(HWND(self.0 as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

fn to_event(cmd: Command, gen: u64) -> Event {
    match cmd {
        Command::Activate => Event::Activate,
        Command::Older => Event::Step(Step::Older, gen),
        Command::Newer => Event::Step(Step::Newer, gen),
        Command::Confirm => Event::Key(Key::Enter, gen),
        Command::Cancel => Event::Key(Key::Escape, gen),
        Command::PageUp => Event::Key(Key::PageUp, gen),
        Command::PageDown => Event::Key(Key::PageDown, gen),
        Command::Delete => Event::Key(Key::Delete, gen),
        Command::CtrlDelete => Event::Key(Key::CtrlDelete, gen),
        Command::TogglePin => Event::Key(Key::CtrlP, gen),
        Command::Backspace => Event::Key(Key::Backspace, gen),
        Command::CtrlBackspace => Event::Key(Key::CtrlBackspace, gen),
        Command::Tab => Event::Key(Key::Tab, gen),
        Command::ClickPaste => Event::ClickPaste(gen),
        Command::ClickCancel => Event::ClickCancel(gen),
        Command::ClickOutside => Event::ClickOutside(gen),
        Command::Left => Event::Key(Key::Left, gen),
        Command::Right => Event::Key(Key::Right, gen),
        Command::Home => Event::Key(Key::Home, gen),
        Command::End => Event::Key(Key::End, gen),
        Command::F3 => Event::Key(Key::F3, gen),
        Command::ShiftF3 => Event::Key(Key::ShiftF3, gen),
        Command::F11 => Event::Key(Key::F11, gen),
        Command::ShiftDelete => Event::Key(Key::ShiftDelete, gen),
        Command::CtrlA => Event::Key(Key::CtrlA, gen),
        Command::CtrlC => Event::Key(Key::CtrlC, gen),
        Command::CtrlE => Event::Key(Key::CtrlE, gen),
        Command::CtrlF => Event::Key(Key::CtrlF, gen),
        Command::CtrlI => Event::Key(Key::CtrlI, gen),
        Command::CtrlS => Event::Key(Key::CtrlS, gen),
        Command::CtrlZ => Event::Key(Key::CtrlZ, gen),
        Command::CtrlY => Event::Key(Key::CtrlY, gen),
        Command::CtrlM => Event::Key(Key::CtrlM, gen),
        Command::CtrlUp => Event::Key(Key::CtrlUp, gen),
        Command::CtrlDown => Event::Key(Key::CtrlDown, gen),
        Command::ShiftUp => Event::Key(Key::ShiftUp, gen),
        Command::ShiftDown => Event::Key(Key::ShiftDown, gen),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PASTEROLL => {
                if let Some(cmd) = Command::from_usize(wparam.0) {
                    dispatch(to_event(cmd, lparam.0 as u64));
                }
                LRESULT(0)
            }
            WM_PASTEROLL_WHEEL => {
                let flags = wparam.0 >> 16;
                let w = Wheel {
                    delta: i32::from((wparam.0 & 0xFFFF) as u16 as i16),
                    horizontal: flags & 1 != 0,
                    ctrl: flags & 2 != 0,
                    shift: flags & 4 != 0,
                };
                dispatch(Event::Wheel(w, lparam.0 as u64));
                LRESULT(0)
            }
            WM_PASTEROLL_CHAR => {
                if let Some(c) = char::from_u32(wparam.0 as u32).filter(|c| !c.is_control()) {
                    dispatch(Event::Key(Key::Char(c), lparam.0 as u64));
                }
                LRESULT(0)
            }
            WM_WAKE => {
                dispatch(Event::Wake);
                LRESULT(0)
            }
            WM_CLIPBOARDUPDATE => {
                RT.with(|cell| {
                    if let Ok(guard) = cell.try_borrow() {
                        if let Some(rt) = guard.as_ref() {
                            rt.controller.clipboard_changed();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_TIMER => {
                let quit = RT.with(|cell| {
                    cell.try_borrow().ok().and_then(|g| g.as_ref().and_then(|rt| rt.quit_at))
                });
                if quit.is_some_and(|q| Instant::now() >= q) {
                    PostQuitMessage(0);
                }
                dispatch(Event::Tick);
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

/// Builds the runtime on this thread. `hwnd` is the hidden message window.
pub fn create(opts: &Options) -> Result<HWND> {
    // The config is validated before anything else: a typo must not quietly change your limits.
    let cfg = keystore::load_config(&opts.db).map_err(|problems| {
        windows::core::Error::new(
            E_FAIL,
            format!("{} is not valid:\n  - {}", keystore::config_path(&opts.db).display(), problems.join("\n  - ")),
        )
    })?;
    let limits = cfg.limits;
    // Key rotation happens here, before the workers (and their connections) exist.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    match keystore::rotate_if_due(&opts.db, now) {
        Ok(Some(r)) => eprintln!("[pasteroll] master key rotated; {} clip(s) re-sealed", r.rows),
        Ok(None) => {}
        Err(e) => eprintln!("[pasteroll] key rotation failed (will be retried at the next start): {e}"),
    }
    if let Some(dir) = opts.db.parent() {
        crate::winsys::set_crash_log(dir.join("pasteroll.log"));
    }
    let history_lock = keystore::lock_history(&opts.db, false).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
    let mut store = keystore::open_store(&opts.db).map_err(|e| windows::core::Error::new(E_FAIL, e))?;
    store.set_limits(limits).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
    let (paste_keys, warnings) = PasteKeys::load(&opts.db.with_file_name("paste_keys.txt"));
    for w in warnings {
        eprintln!("[pasteroll] paste_keys.txt: {w}");
    }
    unsafe {
        let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("PasterollApp"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("PasterollApp"),
            w!("Pasteroll"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )?;
        let overlay = OverlayWindow::create()?;
        overlay.set_size(cfg.size_percent);
        overlay.set_mode(cfg.mode);
        let viewer = OverlayWindow::create_viewer()?;
        crate::hooks::set_viewer_window(viewer.hwnd());
        let hooks = if opts.hooks { Some(Hooks::start(hwnd, overlay.hwnd(), opts.accept_injected)?) } else { None };
        if opts.capture {
            clipboard::listen(hwnd)?;
        }
        let db = opts.db.clone();
        let factory: StoreFactory = Arc::new(move || {
            let mut s = keystore::open_store(&db)?;
            s.set_limits(limits).map_err(|e| e.to_string())?;
            Ok(s)
        });
        let log: Logger = Arc::new(|m| eprintln!("[pasteroll] {m}"));
        let source: Option<Box<dyn pasteroll_app::ClipboardSource>> =
            opts.capture.then(|| {
            Box::new(WinClipboard::new(limits.max_clip_bytes as usize)) as Box<dyn pasteroll_app::ClipboardSource>
        });
        let mut controller = Controller::new(
            store,
            factory,
            paste_keys,
            Arc::new(WinWaker(hwnd.0 as isize)),
            source,
            IngestPolicy {
                exclude_apps: cfg.exclude_apps.iter().cloned().collect(),
                paused: Arc::new(std::sync::atomic::AtomicBool::new(cfg.paused)),
            },
            log,
        );
        controller.set_paste_policy(PastePolicy::new(cfg.warn_multiline_in_terminals, &cfg.terminal_apps));
        controller.set_custom_filters(cfg.custom_filters.clone());
        controller.set_advanced_modifier(cfg.advanced_image_modifier);
        controller.set_image_folder(&cfg.image_folder);
        controller.init_ui_mode(cfg.mode);
        controller.set_ocr(Arc::new(crate::ocr::WinOcr));
        let platform = WinPlatform::new(hwnd, overlay, viewer, hooks).with_db(&opts.db);
        SetTimer(Some(hwnd), TIMER_TICK, 25, None);
        let rt = Runtime {
            controller,
            platform,
            quit_at: opts.max_seconds.map(|s| Instant::now() + Duration::from_secs(s)),
            db: opts.db.clone(),
            tray: None,
            _history_lock: history_lock,
        };
        RT.with(|c| *c.borrow_mut() = Some(rt));
        if opts.tray {
            match Tray::create(cfg.paused) {
                Ok(t) => {
                    let tray_window = t.window().0 as isize;
                    with_runtime(|rt| rt.tray = Some(t));
                    // the update check runs only when the setting is on; "not decided" asks once
                    crate::update::begin(tray_window, &opts.db, cfg.updates, opts.notice && opts.hooks);
                    // an update that was started earlier and did not finish is worth saying
                    match opts.db.parent().and_then(crate::install::startup_report) {
                        Some(crate::install::Startup::DidNotFinish(text)) => crate::installui::report(tray_window, text),
                        Some(crate::install::Startup::Updated(v)) => {
                            crate::installui::updated(tray_window, format!("Pasteroll is now {v}. Your history and settings were kept."));
                        }
                        None => {}
                    }
                }
                Err(e) => eprintln!("[pasteroll] no notification icon: {e}"),
            }
        }
        // Libraries loaded during start-up may have replaced the crash filter: put ours back.
        crate::winsys::harden_process();
        Ok(hwnd)
    }
}

/// Drops the runtime (hooks, workers, overlay) on this thread.
pub fn destroy() {
    RT.with(|c| {
        if let Some(rt) = c.borrow_mut().take() {
            clipboard::unlisten(rt.platform.hwnd);
            drop(rt);
        }
    });
}

// ------------------------------------------------------------------ tray and settings actions

pub fn browse_from_tray(at: (i32, i32)) {
    dispatch(Event::BrowseAt(at.0, at.1));
}

pub fn is_paused() -> bool {
    with_runtime(|rt| rt.controller.is_paused()).unwrap_or(false)
}

pub fn db_path() -> Option<PathBuf> {
    with_runtime(|rt| rt.db.clone())
}

/// The notification icon's window (results of update checks are delivered to it).
pub fn tray_window() -> Option<isize> {
    with_runtime(|rt| rt.tray.as_ref().map(|t| t.window().0 as isize)).flatten()
}

/// Pauses or resumes recording and remembers the choice in config.toml.
pub fn toggle_pause() {
    let state = with_runtime(|rt| {
        let paused = !rt.controller.is_paused();
        rt.controller.set_paused(paused);
        if let Some(t) = &rt.tray {
            t.update(paused);
        }
        (paused, rt.db.clone())
    });
    let Some((paused, db)) = state else { return };
    let saved = keystore::edit_config(&db, |text| {
        pasteroll_core::config::set_value(text, "paused", &paused.to_string()).map_err(|p| p.join("; "))
    });
    if let Err(e) = saved {
        eprintln!("[pasteroll] could not remember the pause setting: {e}");
    }
}

pub fn ui_mode() -> UiMode {
    with_runtime(|rt| rt.controller.ui_mode()).unwrap_or_default()
}

/// Switches between Complete and No Bloat at once (no restart) and remembers the choice in
/// config.toml. A carousel that is open is closed; the next one opens in the new mode.
pub fn set_ui_mode(mode: UiMode) {
    let db = with_runtime(|rt| {
        if rt.controller.ui_mode() == mode {
            return None;
        }
        rt.controller.set_ui_mode(&mut rt.platform, mode);
        Some(rt.db.clone())
    })
    .flatten();
    let Some(db) = db else { return };
    let saved = keystore::edit_config(&db, |text| {
        pasteroll_core::config::set_value(text, "mode", mode.key()).map_err(|p| p.join("; "))
    });
    if let Err(e) = saved {
        eprintln!("[pasteroll] could not remember the mode: {e}");
    }
}

pub fn quit() {
    unsafe {
        PostQuitMessage(0);
    }
}

/// Explorer restarted: the notification area is empty again.
pub fn readd_tray() {
    let paused = is_paused();
    with_runtime(|rt| {
        if let Some(t) = &rt.tray {
            t.readd(paused);
        }
    });
}

pub fn open_data_folder() {
    let Some(db) = db_path() else { return };
    let Some(dir) = db.parent() else { return };
    let wide: Vec<u16> = dir.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            w!("open"),
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOW,
        );
    }
}

/// Appends a line to `pasteroll.log` in the data folder (the program has no console to print to).
/// Only events and errors are logged, never clip content.
pub fn log_line(dir: Option<&std::path::Path>, msg: &str) {
    use std::io::Write;
    eprintln!("[pasteroll] {msg}");
    let Some(dir) = dir else { return };
    let path = dir.join("pasteroll.log");
    // keep it small: start over once it is large
    if std::fs::metadata(&path).map(|m| m.len() > 64 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "{secs} pid {}: {msg}", std::process::id());
    }
}

/// Starts a fresh copy of this program with the same arguments and quits this one. The new copy
/// waits for this one's single-instance mutex, so the two never run side by side.
pub fn restart() {
    restart_to(None);
}

/// Like [`restart`], starting the program file `program` (an updated copy that replaced this one).
pub fn restart_to(program: Option<&std::path::Path>) {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let dir = db_path().and_then(|d| d.parent().map(|p| p.to_path_buf()));
    let exe = match program {
        Some(p) => p.to_path_buf(),
        None => match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                log_line(dir.as_deref(), &format!("restart: cannot find the program: {e}"));
                restart_failed(&e.to_string());
                return;
            }
        },
    };
    let mut args: Vec<std::ffi::OsString> =
        std::env::args_os().skip(1).filter(|a| a != "--restart").collect();
    args.push("--restart".into());
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: the new copy must outlive a closing console.
    // Standard handles are set explicitly: a program started from Explorer has none to inherit.
    let spawned = std::process::Command::new(&exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0000_0008 | 0x0000_0200)
        .spawn();
    match spawned {
        Ok(child) => {
            log_line(dir.as_deref(), &format!("restart: started pid {}", child.id()));
            quit()
        }
        Err(e) => {
            log_line(dir.as_deref(), &format!("restart: could not start {}: {e}", exe.display()));
            restart_failed(&e.to_string());
        }
    }
}

/// The settings are saved but the program could not start itself again: say so.
fn restart_failed(why: &str) {
    let text: Vec<u16> = format!(
        "Your settings were saved, but Pasteroll could not restart itself ({why}).

Quit Pasteroll from its tray icon and start it again to use them."
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect();
    unsafe {
        MessageBoxW(None, windows::core::PCWSTR(text.as_ptr()), w!("Pasteroll"), MB_OK | MB_ICONWARNING);
    }
}

pub fn run(opts: Options) -> i32 {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // One instance only: two would install two hooks and paste twice. A restarted copy waits for
    // the old one to finish exiting.
    let restarting = std::env::args().any(|a| a == "--restart");
    // debug builds can run beside the real program (self-tests of the settings and restart)
    let mutex_name: Vec<u16> = {
        let mut n = String::from(r"Local\PasterollApp");
        if cfg!(debug_assertions) {
            if let Ok(id) = std::env::var("PASTEROLL_TEST_INSTANCE") {
                n.push('-');
                n.push_str(&id);
            }
        }
        n.encode_utf16().chain(std::iter::once(0)).collect()
    };
    let _single = unsafe {
        let mut waited = 0u32;
        loop {
            match CreateMutexW(None, true, windows::core::PCWSTR(mutex_name.as_ptr())) {
                Ok(m) => {
                    if GetLastError() == ERROR_ALREADY_EXISTS {
                        if restarting && waited < 100 {
                            let _ = CloseHandle(m);
                            waited += 1;
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        eprintln!("pasteroll: Pasteroll is already running");
                        return 1;
                    }
                    break m;
                }
                Err(e) => {
                    eprintln!("pasteroll: {e}");
                    return 1;
                }
            }
        }
    };
    // the old program file left by an update that replaced it
    crate::install::remove_leftover();
    if opts.notice && opts.hooks {
        crate::setup::first_run_notice(&opts.db);
    }
    if let Err(e) = create(&opts) {
        eprintln!("pasteroll: {e}");
        return 1;
    }
    eprintln!(
        "[pasteroll] running; history={}; capture={}; activation=Ctrl+Shift+middle-click",
        opts.db.display(),
        opts.capture
    );
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    destroy();
    0
}
