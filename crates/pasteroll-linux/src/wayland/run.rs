//! Startup and the native event/timer/waker loop for the GNOME Wayland backend.

use super::bridge::{Client, SettingsResult, UiState};
use super::platform::GnomePlatform;
use super::protocol;
use super::settings::{self, Proposal};
use crate::install;
use crate::sys::{self, LinuxOs};
use pasteroll_app::{
    Controller, Event, IngestPolicy, Logger, PasteKeys, PastePolicy, StoreFactory, Waker,
};
use pasteroll_core::config;
use pasteroll_core::keystore as ks;
use pasteroll_core::PrunePlan;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(25);
const FAST_TICK: Duration = Duration::from_millis(10);
const SETTINGS_POLL: Duration = Duration::from_millis(300);
const SETTINGS_WAIT: Duration = Duration::from_secs(15 * 60);

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

struct PipeWaker(i32);

impl Waker for PipeWaker {
    fn wake(&self) {
        let byte = [1u8];
        unsafe {
            libc::write(self.0, byte.as_ptr().cast(), 1);
        }
    }
}

fn pipe() -> Result<(i32, i32), String> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
        return Err("the event wake pipe could not be created".into());
    }
    Ok((fds[0], fds[1]))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn redact(message: &str, db: &Path) -> String {
    let mut clean = message.to_owned();
    let mut private = Vec::new();
    for name in ["HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"] {
        if let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) {
            private.push(PathBuf::from(value));
        }
    }
    if let Some(parent) = db.parent() {
        private.push(parent.to_path_buf());
    }
    private.sort_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));
    for path in private {
        let text = path.to_string_lossy();
        if !text.is_empty() {
            clean = clean.replace(text.as_ref(), "[private]");
        }
    }
    clean
}

fn safe_log(db: &Path) -> Logger {
    let db = db.to_path_buf();
    Arc::new(move |message| eprintln!("[pasteroll] {}", redact(message, &db)))
}

fn ui_apps(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let name = value.trim().to_ascii_lowercase();
        if name.is_empty()
            || name.len() > 128
            || name
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\' || c.is_whitespace())
        {
            continue;
        }
        if !out.contains(&name) {
            out.push(name);
        }
        if out.len() == 128 {
            break;
        }
    }
    out
}

fn state(cfg: &config::Config, paused: bool, first_run: bool) -> UiState {
    UiState {
        paused,
        size: cfg.size_percent,
        clips: cfg.limits.max_clips.min(u64::from(u32::MAX)) as u32,
        total_bytes: cfg.limits.max_bytes,
        age_days: cfg.limits.max_age_days.unwrap_or(0),
        clip_bytes: cfg.limits.max_clip_bytes,
        pins: cfg.limits.max_pins,
        exclude_apps: ui_apps(&cfg.exclude_apps),
        warn_multiline: cfg.warn_multiline_in_terminals,
        terminal_apps: ui_apps(&cfg.terminal_apps),
        autostart: install::autostart_enabled(),
        first_run,
    }
}

fn publish_state(
    client: &Client,
    cfg: &config::Config,
    paused: bool,
    first_run: bool,
) -> Result<(), String> {
    client
        .ui_state(&state(cfg, paused, first_run))
        .map_err(|_| "the GNOME panel state could not be updated".to_string())
}

struct PendingSettings {
    proposal: Proposal,
    text: String,
    plan: PrunePlan,
}

fn acknowledge_first_run(db: &Path, pending: &mut bool) -> bool {
    if !*pending {
        return false;
    }
    install::mark_first_run_done(db);
    *pending = false;
    true
}

fn prepare_settings(
    controller: &Controller,
    db: &Path,
    result: &SettingsResult,
) -> Result<(Proposal, String, PrunePlan), String> {
    // Filters can be edited in the common carousel while the process is live.
    // Base proposals on the saved file, not the startup copy, so saving settings
    // cannot erase those edits or an unrelated editor option.
    let previous = ks::load_config(db)
        .map_err(|_| "the saved settings could not be read; nothing was changed".to_string())?;
    let proposal = settings::proposal(&previous, result)?;
    let text = settings::render_config(&proposal.config)?;
    let plan = controller
        .store()
        .borrow()
        .plan_prune(&proposal.config.limits, now_secs())
        .map_err(|_| "the pending deletions could not be calculated".to_string())?;
    Ok((proposal, text, plan))
}

fn apply_settings(
    db: &Path,
    controller: &mut Controller,
    platform: &mut GnomePlatform,
    current: &mut config::Config,
    proposal: Proposal,
    text: String,
) -> Result<(), String> {
    let old_autostart = install::autostart_enabled();
    if old_autostart != proposal.autostart {
        install::set_wayland_autostart_mode(proposal.autostart, platform.keyboard_opt_in())?;
    }
    if ks::write_config_text::<LinuxOs>(db, &text).is_err() {
        if old_autostart != proposal.autostart {
            let _ = install::set_wayland_autostart_mode(old_autostart, platform.keyboard_opt_in());
        }
        return Err("the settings could not be saved; nothing was changed".into());
    }
    let removed = {
        let store = controller.store();
        let mut store = store.borrow_mut();
        store
            .set_limits(proposal.config.limits)
            .map_err(|_| "the new limits could not be applied".to_string())?;
        let removed = store
            .maintain(now_secs())
            .map_err(|_| "the pending deletions could not be completed".to_string())?;
        if removed > 0 {
            let _ = store.request_rotation();
        }
        removed
    };
    controller.set_paused(proposal.config.paused);
    controller.set_paste_policy(PastePolicy::new(
        proposal.config.warn_multiline_in_terminals,
        &proposal.config.terminal_apps,
    ));
    controller.set_custom_filters(proposal.config.custom_filters.clone());
    controller.set_image_folder(&proposal.config.image_folder);
    platform.set_size_percent(proposal.config.size_percent);
    *current = proposal.config;
    eprintln!("[pasteroll] settings saved; {removed} clip(s) removed; restarting");
    Ok(())
}

fn restart() -> ! {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("pasteroll"));
    let err = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .exec();
    eprintln!("[pasteroll] could not restart: {err}. Start Pasteroll again to apply the settings.");
    std::process::exit(0);
}

pub fn run(db: PathBuf, capture: bool, max_seconds: Option<u64>, keyboard: bool) -> i32 {
    match run_inner(db, capture, max_seconds, keyboard) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pasteroll: {e}");
            1
        }
    }
}

fn run_inner(
    db: PathBuf,
    capture: bool,
    max_seconds: Option<u64>,
    keyboard: bool,
) -> Result<(), String> {
    STOP.store(false, Ordering::SeqCst);
    // The companion checks this executable through /proc while authenticating.
    // Never re-enable dumpability once history or key material has been opened.
    let authenticated = Client::connect();
    sys::harden_process();
    let mut client = authenticated.map_err(|e| e.to_string())?;
    if keyboard {
        client
            .arm_keyboard(true, true)
            .map_err(|e| format!("GNOME keyboard activation is unavailable: {e}"))?;
    } else {
        client.arm_mouse(true).map_err(|_|"GNOME mouse activation needs the enabled Pasteroll input service, an active local session and a supported mouse. Install the optional host helper; or explicitly choose --backend gnome --wayland-keyboard.".to_string())?;
    }
    let _single = sys::single_instance(&db).map_err(|_| {
        "Pasteroll is already running, or its data folder is not private".to_string()
    })?;
    let mut stop = sys::StopListener::new(&db)
        .map_err(|_| "the local stop control could not be created".to_string())?;
    let mut cfg = ks::load_config(&db).map_err(|problems| {
        format!(
            "settings are not valid ({} problem(s)); fix them before starting",
            problems.len()
        )
    })?;
    match ks::rotate_if_due::<LinuxOs>(&db, now_secs()) {
        Ok(Some(report)) => eprintln!(
            "[pasteroll] master key rotated; {} clip(s) re-sealed",
            report.rows
        ),
        Ok(None) => {}
        Err(_) => {
            eprintln!("[pasteroll] key rotation failed; it will be retried at the next start")
        }
    }
    let _history_lock = ks::lock_history::<LinuxOs>(&db, false)
        .map_err(|_| "the encrypted history could not be locked for this instance".to_string())?;
    let mut store = ks::open_store::<LinuxOs>(&db).map_err(|_| {
        "the encrypted history could not be opened; check the key and run `pasteroll status`"
            .to_string()
    })?;
    store
        .set_limits(cfg.limits)
        .map_err(|_| "the saved history limits could not be applied".to_string())?;
    let (paste_keys, warnings) = PasteKeys::load(&db.with_file_name("paste_keys.txt"));
    let log = safe_log(&db);
    for warning in warnings {
        log(&format!("paste_keys.txt: {warning}"));
    }

    if !client.capabilities.paste()
        || !client.capabilities.viewer()
        || !client.capabilities.settings()
    {
        return Err("the GNOME companion does not implement paste, viewer and settings".into());
    }
    let mut platform = GnomePlatform::new(client.clone(), db.clone(), cfg.size_percent, keyboard)
        .map_err(|e| e.to_string())?;
    let mut first_run = install::first_run_pending(&db);
    publish_state(&client, &cfg, cfg.paused, first_run)?;
    if first_run {
        let _ = client.open_settings();
    }

    let (wake_r, wake_w) = pipe()?;
    let waker: Arc<dyn Waker> = Arc::new(PipeWaker(wake_w));
    let source: Option<Box<dyn pasteroll_app::ClipboardSource>> = if capture {
        Some(Box::new(super::bridge::Capture::new(
            client.clone(),
            cfg.limits.max_clip_bytes as usize,
        )))
    } else {
        None
    };
    let limits = cfg.limits;
    let factory_db = db.clone();
    let factory: StoreFactory = Arc::new(move || {
        let mut store = ks::open_store::<LinuxOs>(&factory_db)?;
        store.set_limits(limits).map_err(|e| e.to_string())?;
        Ok(store)
    });
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
        log.clone(),
    );
    controller.set_paste_policy(PastePolicy::new(
        cfg.warn_multiline_in_terminals,
        &cfg.terminal_apps,
    ));
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

    let started = Instant::now();
    let quit_at = max_seconds.map(|s| started + Duration::from_secs(s));
    let mut next_tick = Instant::now() + TICK;
    let mut awaiting_settings = first_run;
    let mut settings_since = first_run.then(Instant::now);
    let mut next_settings_poll = Instant::now();
    let mut pending: Option<PendingSettings> = None;
    log(if keyboard {
        "running with the native GNOME Wayland backend; keyboard activation is opt-in"
    } else {
        "running with the native GNOME Wayland backend; Ctrl+Shift+middle-click activation"
    });

    let mut last_poll = Instant::now();

    while !STOP.load(Ordering::SeqCst) {
        if stop.requested() || quit_at.is_some_and(|q| Instant::now() >= q) {
            break;
        }
        let polled = match client.poll() {
            Err(protocol::Error::Disconnected) if last_poll.elapsed() >= Duration::from_secs(3) => {
                // A process suspended by the OS can resume after an in-flight
                // request expired, even though the pinned Shell is still alive.
                // Close local UI state first: a timed-out Poll may have drained
                // the watchdog cancellation event before its reply was lost.
                let generation = platform.active_generation();
                controller.handle(&mut platform, Event::ClickCancel(generation));
                client.poll()
            }
            result => result,
        };
        last_poll = Instant::now();
        let events = polled.map_err(|_| {
            "the GNOME companion disconnected or became unavailable; the session was closed"
                .to_string()
        })?;
        for event in events {
            match event.0 {
                20 => controller.handle(&mut platform, Event::Activate),
                21 => {
                    let paused = !controller.is_paused();
                    let saved = ks::read_config_text(&db)
                        .and_then(|text| {
                            config::set_value(&text, "paused", &paused.to_string())
                                .map_err(|problems| problems.join("; "))
                        })
                        .and_then(|text| ks::write_config_text::<LinuxOs>(&db, &text));
                    if saved.is_ok() {
                        cfg.paused = paused;
                        controller.set_paused(paused);
                    } else {
                        log("the pause setting could not be saved");
                    }
                    let _ = publish_state(&client, &cfg, controller.is_paused(), first_run);
                }
                22 => {
                    pending = None;
                    awaiting_settings = true;
                    settings_since = Some(Instant::now());
                    next_settings_poll = Instant::now();
                    if client.open_settings().is_err() {
                        log("the GNOME settings panel could not be opened");
                        awaiting_settings = false;
                    }
                }
                23 => {
                    if let Some(dir) = db.parent() {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(dir)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();
                    }
                }
                24 => {
                    STOP.store(true, Ordering::SeqCst);
                    break;
                }
                25 => {
                    if acknowledge_first_run(&db, &mut first_run) {
                        awaiting_settings = false;
                        settings_since = None;
                        let _ = publish_state(&client, &cfg, controller.is_paused(), false);
                        log("first-run notice acknowledged");
                    }
                }
                26 => controller.clipboard_changed(),
                27 if !keyboard => {
                    if platform.accept_mouse_ticket(event.1) {
                        controller.handle(&mut platform, Event::Activate);
                        platform.finish_mouse_activation();
                    }
                }
                28 if !keyboard => {
                    let generation = platform.active_generation();
                    controller.handle(&mut platform, Event::ClickCancel(generation));
                    awaiting_settings = false;
                    settings_since = None;
                    pending = None;
                    log("mouse activation unavailable; waiting for the host helper and a supported device");
                }
                _ => match protocol::event(event, platform.active_generation()) {
                    Ok(Some(mapped)) => {
                        if event.0 == 6 {
                            platform.note_click_modifiers(event.4);
                        }
                        controller.handle(&mut platform, mapped);
                    }
                    Ok(None) | Err(protocol::Error::Stale) => {}
                    Err(_) => {
                        return Err("the GNOME companion sent an invalid event".into());
                    }
                },
            }
        }

        if awaiting_settings && Instant::now() >= next_settings_poll {
            next_settings_poll = Instant::now() + SETTINGS_POLL;
            match client.settings_result() {
                Ok(result) if result.changed => {
                    if result.confirmed && pending.is_none() {
                        log("an unrequested settings confirmation was ignored");
                    } else if let Some(current) = pending.as_ref() {
                        if result.confirmed {
                            let (proposal, text, plan) = match prepare_settings(
                                &controller,
                                &db,
                                &result,
                            ) {
                                Ok(v) => v,
                                Err(_) => {
                                    log("the confirmed settings proposal was invalid; nothing was changed");
                                    pending = None;
                                    awaiting_settings = false;
                                    continue;
                                }
                            };
                            if proposal != current.proposal {
                                log(
                                    "the confirmed proposal changed; the settings were not applied",
                                );
                                pending = None;
                                awaiting_settings = false;
                                continue;
                            }
                            if plan.clips > current.plan.clips || plan.bytes > current.plan.bytes {
                                log("the prune changed after confirmation; the settings were not applied");
                                pending = None;
                                awaiting_settings = false;
                                continue;
                            }
                            if apply_settings(
                                &db,
                                &mut controller,
                                &mut platform,
                                &mut cfg,
                                proposal,
                                text,
                            )
                            .is_err()
                            {
                                log("the settings could not be applied; nothing further was changed");
                                pending = None;
                                awaiting_settings = false;
                                let _ =
                                    publish_state(&client, &cfg, controller.is_paused(), first_run);
                                continue;
                            }
                            restart();
                        }
                    } else {
                        let (proposal, text, plan) =
                            match prepare_settings(&controller, &db, &result) {
                                Ok(v) => v,
                                Err(_) => {
                                    log("the settings proposal was invalid; nothing was changed");
                                    awaiting_settings = false;
                                    let _ = publish_state(
                                        &client,
                                        &cfg,
                                        controller.is_paused(),
                                        first_run,
                                    );
                                    continue;
                                }
                            };
                        if plan.clips > 0 {
                            pending = Some(PendingSettings {
                                proposal,
                                text,
                                plan,
                            });
                            if client
                                .confirm_settings(plan.clips as u32, plan.bytes, plan.by_age as u32)
                                .is_err()
                            {
                                log("the prune confirmation could not be shown");
                                pending = None;
                                awaiting_settings = false;
                            }
                        } else if apply_settings(
                            &db,
                            &mut controller,
                            &mut platform,
                            &mut cfg,
                            proposal,
                            text,
                        )
                        .is_err()
                        {
                            log("the settings could not be applied; nothing further was changed");
                            awaiting_settings = false;
                            let _ = publish_state(&client, &cfg, controller.is_paused(), first_run);
                        } else {
                            restart();
                        }
                    }
                }
                Ok(_) => {
                    if settings_since.is_some_and(|since| since.elapsed() > SETTINGS_WAIT) {
                        awaiting_settings = false;
                        pending = None;
                        settings_since = None;
                    }
                }
                Err(_) => {
                    return Err("the GNOME settings panel became unavailable".into());
                }
            }
        }

        if Instant::now() >= next_tick {
            next_tick = Instant::now()
                + if platform.fast_ticks() {
                    FAST_TICK
                } else {
                    TICK
                };
            controller.handle(&mut platform, Event::Tick);
        }

        let wait = next_tick
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as i32;
        let mut fd = libc::pollfd {
            fd: wake_r,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe {
            libc::poll(&mut fd, 1, wait);
        }
        if fd.revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            while unsafe { libc::read(wake_r, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
            controller.handle(&mut platform, Event::Wake);
        }
    }

    drop(controller);
    drop(platform);
    if keyboard {
        let _ = client.arm_keyboard(false, true);
    } else {
        let _ = client.arm_mouse(false);
    }
    unsafe {
        libc::close(wake_r);
        libc::close(wake_w);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_limits_are_never_sent_as_unlimited() {
        let cfg = config::Config::default();
        let state = state(&cfg, false, true);
        assert_eq!(state.clips, cfg.limits.max_clips as u32);
        assert_eq!(state.total_bytes, cfg.limits.max_bytes);
        assert_eq!(state.age_days, cfg.limits.max_age_days.unwrap());
        assert!(state.first_run);
    }

    #[test]
    fn logs_replace_the_data_folder() {
        let db = Path::new("/tmp/example/history.db");
        assert_eq!(
            redact("cannot open /tmp/example/key", db),
            "cannot open [private]/key"
        );
    }

    #[test]
    fn the_first_run_marker_is_written_only_for_an_acknowledgement() {
        let dir =
            std::env::temp_dir().join(format!("pasteroll-wayland-ack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = dir.join("history.db");
        let mut pending = true;
        assert!(install::first_run_pending(&db));
        assert!(acknowledge_first_run(&db, &mut pending));
        assert!(!pending);
        assert!(!install::first_run_pending(&db));
        assert!(!acknowledge_first_run(&db, &mut pending));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn panel_app_lists_drop_invalid_entries_without_changing_config() {
        let apps = ui_apps(&[
            " KeepassXC ".into(),
            "keepassxc".into(),
            "../bad".into(),
            "two words".into(),
        ]);
        assert_eq!(apps, vec!["keepassxc"]);
    }
}
