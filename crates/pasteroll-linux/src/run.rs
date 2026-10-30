//! Start-up and the event loop of the X11 app.

use crate::clipboard::Capture;
use crate::platform::{Action, LinuxPlatform};
use crate::shell::{Shell, ShellAction};
use crate::sys::{self, LinuxOs};
use pasteroll_app::{
    Controller, Event, IngestPolicy, Logger, PasteKeys, PastePolicy, StoreFactory, Waker,
};
use pasteroll_core::keystore as ks;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;

const TICK: Duration = Duration::from_millis(25);
/// While something animates (the expanded viewer opening, closing, going fullscreen).
const FAST_TICK: Duration = Duration::from_millis(10);
/// If the loop does not turn for this long while input is grabbed, exit: a stuck grab freezes the
/// whole desktop, and the server releases every grab when the process goes away.
const WATCHDOG_MS: u64 = 4_000;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// Wakes the loop from another thread through a pipe.
struct PipeWaker(i32);

impl Waker for PipeWaker {
    fn wake(&self) {
        let b = [1u8];
        unsafe {
            libc::write(self.0, b.as_ptr().cast(), 1);
        }
    }
}

fn pipe() -> Result<(i32, i32), String> {
    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(format!("pipe: {}", std::io::Error::last_os_error()));
    }
    Ok((fds[0], fds[1]))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn run(db: PathBuf, capture: bool, max_seconds: Option<u64>) -> i32 {
    match run_inner(db, capture, max_seconds) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pasteroll: {e}");
            1
        }
    }
}

fn run_inner(db: PathBuf, capture: bool, max_seconds: Option<u64>) -> Result<(), String> {
    sys::harden_process();
    let _single = sys::single_instance(&db)?;
    let mut stop = sys::StopListener::new(&db)?;
    let cfg = ks::load_config(&db).map_err(|p| {
        format!("{} is not valid:\n  - {}", ks::config_path(&db).display(), p.join("\n  - "))
    })?;
    match ks::rotate_if_due::<LinuxOs>(&db, now_secs()) {
        Ok(Some(r)) => eprintln!("[pasteroll] master key rotated; {} clip(s) re-sealed", r.rows),
        Ok(None) => {}
        Err(e) => eprintln!("[pasteroll] key rotation failed (will be retried at the next start): {e}"),
    }
    let _history_lock = ks::lock_history::<LinuxOs>(&db, false)?;
    let mut store = ks::open_store::<LinuxOs>(&db)?;
    store.set_limits(cfg.limits).map_err(|e| e.to_string())?;
    let (paste_keys, warnings) = PasteKeys::load(&db.with_file_name("paste_keys.txt"));
    for w in warnings {
        eprintln!("[pasteroll] paste_keys.txt: {w}");
    }

    let grabbed = Arc::new(AtomicBool::new(false));
    let mut platform = LinuxPlatform::new(db.clone(), grabbed.clone())
        .map_err(|e| format!("cannot use the X display: {e}"))?;
    platform.grab_activation()?;
    platform.set_size(cfg.size_percent);

    let mut shell = match Shell::new(&platform, 0, cfg.paused, cfg.mode) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("[pasteroll] no tray icon or settings window: {e}");
            None
        }
    };
    if let Some(sh) = shell.as_mut() {
        if std::env::var_os("PASTEROLL_NO_NOTICE").is_none() && crate::install::first_run_pending(&db) {
            sh.show_notice(&platform);
            crate::install::mark_first_run_done(&db);
        }
        // for the isolated screen tests, which have no tray icon to click
        if std::env::var_os("PASTEROLL_OPEN_SETTINGS").is_some() {
            sh.open_settings(&platform);
        }
    }
    let (wake_r, wake_w) = pipe()?;
    let waker: Arc<dyn Waker> = Arc::new(PipeWaker(wake_w));

    let source: Option<Box<dyn pasteroll_app::ClipboardSource>> = if capture {
        let c = Capture::new(
            platform.clip_counter.clone(),
            platform.own_window.clone(),
            cfg.limits.max_clip_bytes as usize,
        )
        .map_err(|e| format!("cannot open a second X connection: {e}"))?;
        Some(Box::new(c))
    } else {
        None
    };
    let limits = cfg.limits;
    let db2 = db.clone();
    let factory: StoreFactory = Arc::new(move || {
        let mut s = ks::open_store::<LinuxOs>(&db2)?;
        s.set_limits(limits).map_err(|e| e.to_string())?;
        Ok(s)
    });
    let log: Logger = Arc::new(|m| eprintln!("[pasteroll] {m}"));
    let mut controller = Controller::new(
        store,
        factory,
        paste_keys,
        waker,
        source,
        IngestPolicy {
            exclude_apps: cfg.exclude_apps.iter().cloned().collect(),
            paused: Arc::new(AtomicBool::new(cfg.paused)),
        },
        log,
    );
    controller.set_paste_policy(PastePolicy::new(cfg.warn_multiline_in_terminals, &cfg.terminal_apps));
    controller.set_advanced_modifier(cfg.advanced_image_modifier);
    controller.set_image_folder(&cfg.image_folder);
    controller.init_ui_mode(cfg.mode);
    pasteroll_app::Platform::ui_mode_changed(&mut platform, cfg.mode);
    controller.set_custom_filters(cfg.custom_filters.clone());
    controller.set_ocr(Arc::new(crate::dialogs::Tesseract));

    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
    }

    // watchdog: a hung loop with input grabbed would freeze the desktop
    let last_turn = Arc::new(AtomicU64::new(0));
    {
        let (last_turn, grabbed) = (last_turn.clone(), grabbed.clone());
        let start = Instant::now();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = start.elapsed().as_millis() as u64;
            if grabbed.load(Ordering::SeqCst) && now.saturating_sub(last_turn.load(Ordering::SeqCst)) > WATCHDOG_MS {
                eprintln!("[pasteroll] event loop stalled while input was grabbed; exiting to release it");
                unsafe { libc::_exit(3) };
            }
        });
    }
    let started = Instant::now();
    eprintln!(
        "[pasteroll] running; history={}; capture={}; clipboard=X11 (XFixes); activation=Ctrl+Shift+middle-click",
        db.display(),
        capture
    );
    let x_fd = platform.conn.stream().as_raw_fd();
    let quit_at = max_seconds.map(|s| started + Duration::from_secs(s));
    let mut next_tick = Instant::now() + TICK;

    while !STOP.load(Ordering::SeqCst) {
        if stop.requested() { break; }
        last_turn.store(started.elapsed().as_millis() as u64, Ordering::SeqCst);
        // everything the server has sent
        loop {
            match platform.conn.poll_for_event() {
                Ok(Some(ev)) => {
                    if let Some(sh) = shell.as_mut().filter(|sh| sh.owns(&ev)) {
                        for a in sh.handle(&platform, ev) {
                            shell_action(a, &mut controller, &mut platform, sh, &db);
                        }
                        continue;
                    }
                    for action in platform.translate(ev) {
                        match action {
                            Action::Event(e) => controller.handle(&mut platform, e),
                            Action::ClipboardChanged => controller.clipboard_changed(),
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => return Err(format!("lost the X connection: {e}")),
            }
        }
        if let Some(sh) = shell.as_mut() {
            for a in sh.poll_sni(&platform) {
                shell_action(a, &mut controller, &mut platform, sh, &db);
            }
            let _ = platform.conn.flush_if_needed();
        }
        let wait = next_tick.saturating_duration_since(Instant::now()).as_millis().min(i32::MAX as u128) as i32;
        let mut fds = [
            libc::pollfd { fd: x_fd, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake_r, events: libc::POLLIN, revents: 0 },
        ];
        unsafe {
            libc::poll(fds.as_mut_ptr(), 2, wait);
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            while unsafe { libc::read(wake_r, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
            controller.handle(&mut platform, Event::Wake);
        }
        if Instant::now() >= next_tick {
            next_tick = Instant::now() + if platform.fast_ticks { FAST_TICK } else { TICK };
            controller.handle(&mut platform, Event::Tick);
            let _ = platform.conn.flush_if_needed();
        }
        if quit_at.is_some_and(|q| Instant::now() >= q) {
            break;
        }
    }
    drop(controller);
    drop(platform);
    Ok(())
}

/// Carries out what the tray menu or the settings window asked for.
fn shell_action(a: ShellAction, controller: &mut Controller, p: &mut LinuxPlatform, shell: &mut Shell, db: &std::path::Path) {
    match a {
        ShellAction::Quit => STOP.store(true, Ordering::SeqCst),
        ShellAction::TogglePause => {
            let paused = !controller.is_paused();
            controller.set_paused(paused);
            shell.set_paused(p, paused);
            let saved = ks::read_config_text(db)
                .and_then(|t| {
                    pasteroll_core::config::set_value(&t, "paused", &paused.to_string()).map_err(|e| e.join("; "))
                })
                .and_then(|t| ks::write_config_text::<LinuxOs>(db, &t));
            if let Err(e) = saved {
                eprintln!("[pasteroll] could not remember the pause setting: {e}");
            }
        }
        ShellAction::SetMode(mode) => {
            if controller.ui_mode() == mode {
                return;
            }
            controller.set_ui_mode(p, mode);
            shell.set_mode(mode);
            let saved = ks::read_config_text(db)
                .and_then(|t| pasteroll_core::config::set_value(&t, "mode", mode.key()).map_err(|e| e.join("; ")))
                .and_then(|t| ks::write_config_text::<LinuxOs>(db, &t));
            if let Err(e) = saved {
                eprintln!("[pasteroll] could not remember the mode: {e}");
            }
        }
        ShellAction::OpenFolder => {
            if let Some(dir) = db.parent() {
                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
            }
        }
        ShellAction::Save { text, limits, confirmed } => {
            let now = now_secs();
            let plan = controller.store().borrow().plan_prune(&limits, now).ok();
            if let Some(plan) = plan.filter(|p| p.clips > 0 && !confirmed) {
                let msg = format!(
                    "Saving will delete {} clip(s), {:.1} MB ({} of them only because of age). Favorite clips are kept.",
                    plan.clips,
                    plan.bytes as f64 / (1024.0 * 1024.0),
                    plan.by_age
                );
                shell.ask_confirm(p, msg, text, limits);
                return;
            }
            if let Err(e) = ks::write_config_text::<LinuxOs>(db, &text) {
                shell.settings_failed(p, e);
                return;
            }
            let removed = {
                let store = controller.store();
                let mut st = store.borrow_mut();
                let _ = st.set_limits(limits);
                let n = st.maintain(now).unwrap_or(0);
                if n > 0 {
                    // deleted data stays decryptable under the old key until it is replaced
                    let _ = st.request_rotation();
                }
                n
            };
            eprintln!("[pasteroll] settings saved; {removed} clip(s) removed; restarting");
            shell.settings_saved(p);
            restart();
        }
    }
}

/// Starts this program again with the same arguments, in place: every part (capture, limits,
/// exclusions) then starts from the saved file. All descriptors are close-on-exec, so the
/// instance lock and the X connection are released by the exec itself.
fn restart() -> ! {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("pasteroll"));
    let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
    eprintln!("[pasteroll] could not restart: {err}. Start Pasteroll again to apply the settings.");
    std::process::exit(0);
}

trait FlushIfNeeded {
    fn flush_if_needed(&self) -> Result<(), ()>;
}

impl FlushIfNeeded for x11rb::rust_connection::RustConnection {
    fn flush_if_needed(&self) -> Result<(), ()> {
        use x11rb::connection::Connection;
        self.flush().map_err(|_| ())
    }
}
