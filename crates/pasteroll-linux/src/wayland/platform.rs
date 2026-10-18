//! The GNOME Shell bridge as a real [`pasteroll_app::Platform`].

use super::bridge::{ClickPollResult, Client, Context};
use super::protocol::{Error, MAX_FRAME_BYTES};
use pasteroll_app::{
    ClickPoll, Modifiers, OverlayInfo, PasteKey, Payload, Platform, SaveChoice, Target,
};
use pasteroll_core::keystore as ks;
use pasteroll_render::{Frame, HEIGHT, WIDTH};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PASTE_WINDOW: Duration = Duration::from_secs(3);
const CLICK_SETTLE: Duration = Duration::from_millis(750);
const SHIFT: u32 = 1;
const CONTROL: u32 = 4;
const ALT: u32 = 8;
const MOD4: u32 = 64;
const MOD5: u32 = 128;
const BUTTON1: u32 = 1 << 8;
const SUPER_HIGH: u32 = 1 << 26;

#[derive(Clone)]
struct AuthorizedPaste {
    generation: u64,
    target: Target,
    expires: Option<Instant>,
}

pub struct GnomePlatform {
    client: Client,
    db: PathBuf,
    size_percent: u32,
    keyboard_opt_in: bool,
    mouse_ticket: Option<u64>,
    generation: u64,
    overlay_generation: u64,
    active: bool,
    overlay_visible: bool,
    viewer_visible: bool,
    viewer_mode: bool,
    modal: bool,
    fast_ticks: bool,
    overlay: OverlayInfo,
    last_context: Option<Context>,
    session_target: Option<Target>,
    authorized: Option<AuthorizedPaste>,
    click_modifiers: Modifiers,
    click_started: Option<Instant>,
    start: Instant,
}

fn device(value: i32, scale: f32) -> i32 {
    (f64::from(value) * f64::from(scale))
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn logical(value: i32, scale: f32) -> i32 {
    (f64::from(value) / f64::from(scale.max(0.1))).round() as i32
}

fn device_rect((x, y, w, h): (i32, i32, u32, u32), scale: f32) -> (i32, i32, u32, u32) {
    (
        device(x, scale),
        device(y, scale),
        (f64::from(w) * f64::from(scale)).round() as u32,
        (f64::from(h) * f64::from(scale)).round() as u32,
    )
}

fn target_from_context(context: &Context) -> Option<Target> {
    if context.desktop {
        return Some(Target {
            id: context.id,
            pid: context.pid,
            exe: context.exe.to_ascii_lowercase(),
            class: context.class.clone(),
            desktop: true,
        });
    }
    if context.id == 0 || context.pid == 0 || context.pid == std::process::id() {
        return None;
    }
    Some(Target {
        id: context.id,
        pid: context.pid,
        exe: context.exe.to_ascii_lowercase(),
        class: context.class.clone(),
        desktop: false,
    })
}

fn chord(key: PasteKey) -> u32 {
    match key {
        PasteKey::CtrlV => 0,
        PasteKey::ShiftInsert => 1,
        PasteKey::CtrlShiftV => 2,
    }
}

fn unsafe_modifier(modifiers: u32) -> bool {
    modifiers & (ALT | MOD4 | MOD5 | BUTTON1 | SUPER_HIGH) != 0
}

fn local_tm(secs: i64) -> libc::tm {
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    tm
}

impl GnomePlatform {
    pub fn new(
        client: Client,
        db: PathBuf,
        size_percent: u32,
        keyboard_opt_in: bool,
    ) -> Result<Self, Error> {
        if !client.capabilities.paste()
            || !client.capabilities.viewer()
            || !client.capabilities.settings()
        {
            return Err(Error::Protocol);
        }
        Ok(Self {
            client,
            db,
            size_percent,
            keyboard_opt_in,
            mouse_ticket: None,
            generation: 0,
            overlay_generation: 0,
            active: false,
            overlay_visible: false,
            viewer_visible: false,
            viewer_mode: false,
            modal: false,
            fast_ticks: false,
            overlay: OverlayInfo::new(1.0, (WIDTH as u32, HEIGHT as u32)),
            last_context: None,
            session_target: None,
            authorized: None,
            click_modifiers: Modifiers::default(),
            click_started: None,
            start: Instant::now(),
        })
    }

    pub fn set_size_percent(&mut self, size: u32) {
        self.size_percent = size;
    }

    pub fn keyboard_opt_in(&self) -> bool {
        self.keyboard_opt_in
    }
    pub fn accept_mouse_ticket(&mut self, ticket: u64) -> bool {
        if self.active || self.keyboard_opt_in {
            return false;
        }
        match self.client.begin_mouse(ticket) {
            Ok(generation) => {
                self.generation = generation;
                self.mouse_ticket = Some(generation);
                true
            }
            Err(_) => false,
        }
    }
    pub fn finish_mouse_activation(&mut self) {
        // Empty history or an unsuitable target can make the controller decline
        // activation. Do not leave the preauthorized generation alive in that case.
        if self.mouse_ticket.take().is_some() {
            let _ = self.client.input_active(false);
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn active_generation(&self) -> u64 {
        self.generation
    }

    pub fn fast_ticks(&self) -> bool {
        self.fast_ticks
    }

    pub fn note_click_modifiers(&mut self, modifiers: u32) {
        self.click_modifiers = Modifiers {
            ctrl: modifiers & CONTROL != 0,
            shift: modifiers & SHIFT != 0,
            alt: modifiers & ALT != 0,
        };
    }

    fn remember_context(&mut self, context: Context) {
        self.last_context = Some(context);
    }

    fn sync_modes(&mut self) {
        if self.generation != 0 {
            if self
                .client
                .modes(self.generation, self.viewer_mode, self.modal)
                .is_err()
            {
                self.log("could not update the GNOME input mode");
            }
        }
    }

    fn suspend(&mut self, on: bool) -> bool {
        if self.generation == 0 {
            return false;
        }
        self.client.suspend(self.generation, on).is_ok()
    }

    fn with_dialog<T>(&mut self, unavailable: T, f: impl FnOnce() -> T) -> T {
        if !self.suspend(true) {
            self.log("the save dialog could not be opened while GNOME input is active");
            return unavailable;
        }
        let result = f();
        if !self.suspend(false) {
            self.log("GNOME input did not resume after the dialog");
        }
        self.sync_modes();
        result
    }

    fn mark_pending_expiry(&mut self) {
        if let Some(auth) = self.authorized.as_mut() {
            if auth.expires.is_none() {
                auth.expires = Some(Instant::now() + PASTE_WINDOW);
            }
        }
    }

    fn revoke_if(&mut self, id: u64) {
        if self.authorized.as_ref().is_some_and(|a| a.target.id == id) {
            self.authorized = None;
        }
    }
}

impl Platform for GnomePlatform {
    fn foreground(&mut self) -> Option<Target> {
        let context = match self.client.context() {
            Ok(context) => context,
            Err(_) => {
                self.log("could not read the GNOME foreground window");
                return None;
            }
        };
        self.remember_context(context.clone());
        let target = target_from_context(&context);
        self.session_target = target.clone();
        target
    }

    fn target_is_current(&mut self, target: &Target) -> bool {
        let current = if target.desktop {
            self.client.context().is_ok_and(|c| c.desktop)
        } else {
            self.client.current(target.id, target.pid).unwrap_or(false)
        };
        if !current {
            self.revoke_if(target.id);
        }
        current
    }

    fn target_privileged(&mut self, target: &Target) -> bool {
        if target.desktop {
            return false;
        }
        // An unknown answer is conservative: never send input to a target we could not check.
        self.client
            .privileged(target.id, target.pid)
            .unwrap_or(true)
    }

    fn cursor(&self) -> (i32, i32) {
        self.client
            .context()
            .or_else(|_| self.last_context.clone().ok_or(Error::Disconnected))
            .map(|c| {
                let scale = c.scale as f32;
                (device(c.pointer.0, scale), device(c.pointer.1, scale))
            })
            .unwrap_or((0, 0))
    }

    fn overlay_show(&mut self, at: (i32, i32)) -> OverlayInfo {
        let context = match self.client.context() {
            Ok(c) => c,
            Err(_) => {
                self.log("could not read the monitor geometry");
                return self.overlay;
            }
        };
        self.remember_context(context.clone());
        let scale = context.scale as f32;
        let monitor = device_rect(context.monitor, scale);
        let _work = device_rect(context.work, scale);
        let factor = pasteroll_core::settings::size_factor(self.size_percent);
        let max_w = monitor.2.max(200).min(4096);
        let max_h = monitor.3.max(150).min(4096);
        let mut width = ((WIDTH * factor * scale).round() as u32).clamp(200.min(max_w), max_w);
        let mut height = ((HEIGHT * factor * scale).round() as u32).clamp(150.min(max_h), max_h);
        let pixels = u64::from(width) * u64::from(height);
        let max_pixels = (MAX_FRAME_BYTES / 4) as u64;
        if pixels > max_pixels {
            let fit = (max_pixels as f64 / pixels as f64).sqrt();
            width = ((f64::from(width) * fit).floor() as u32).clamp(1, 4096);
            height = ((f64::from(height) * fit).floor() as u32).clamp(1, 4096);
        }
        let shown = if self.keyboard_opt_in {
            self.client.show(
                logical(at.0, scale),
                logical(at.1, scale),
                width,
                height,
                self.keyboard_opt_in,
            )
        } else {
            self.client.show_mouse(
                logical(at.0, scale),
                logical(at.1, scale),
                width,
                height,
                self.generation,
            )
        };
        let (generation, x, y, actual_scale) = match shown {
            Ok(v) => v,
            Err(e) => {
                self.log(&format!("could not show the GNOME carousel: {e}"));
                self.overlay_visible = false;
                return self.overlay;
            }
        };
        if self.active && generation != self.generation {
            self.log("the companion changed the input generation while a session was active");
            self.overlay_visible = false;
            return self.overlay;
        }
        self.overlay_generation = generation;
        self.overlay_visible = true;
        if !self.active {
            // A notification is not a new input session; it also revokes any old paste token at
            // the companion, so Rust must not keep a token the companion no longer honors.
            self.authorized = None;
        }
        let actual_scale = actual_scale as f32;
        self.overlay = OverlayInfo {
            scale: actual_scale,
            size_px: (width, height),
            origin_px: (device(x, actual_scale), device(y, actual_scale)),
            monitor_px: device_rect(context.monitor, actual_scale),
            work_px: device_rect(context.work, actual_scale),
        };
        self.overlay
    }

    fn overlay_present(&mut self, frame: &Frame) {
        if !self.overlay_visible && self.active {
            // Replay hides the Shell actor before the click. A desktop image editor can then stay
            // open after that click; when the carousel is needed again, re-show the same session.
            let at = self.cursor();
            self.overlay_show(at);
        }
        if !self.overlay_visible {
            return;
        }
        if let Err(e) = self.client.present(self.overlay_generation, frame) {
            self.log(&format!("could not draw the GNOME carousel: {e}"));
        }
    }

    fn overlay_hide(&mut self) {
        if self.overlay_visible && self.overlay_generation != 0 {
            if let Err(e) = self.client.hide(self.overlay_generation) {
                self.log(&format!("could not hide the GNOME carousel: {e}"));
            }
        }
        self.overlay_visible = false;
        self.mark_pending_expiry();
    }

    fn viewer_show(&mut self, rect: (i32, i32, u32, u32), rounded: bool) {
        if self.generation == 0 {
            return;
        }
        let scale = self.overlay.scale.max(0.1);
        match self.client.viewer_show(
            self.generation,
            logical(rect.0, scale),
            logical(rect.1, scale),
            rect.2,
            rect.3,
            rounded,
        ) {
            Ok(()) => self.viewer_visible = true,
            Err(e) => self.log(&format!("could not show the GNOME viewer: {e}")),
        }
    }

    fn viewer_present(&mut self, frame: &Frame) {
        if !self.viewer_visible || self.generation == 0 {
            return;
        }
        if let Err(e) = self.client.viewer_present(self.generation, frame) {
            self.log(&format!("could not draw the GNOME viewer: {e}"));
        }
    }

    fn viewer_hide(&mut self) {
        if self.viewer_visible && self.generation != 0 {
            if let Err(e) = self.client.viewer_hide(self.generation) {
                self.log(&format!("could not hide the GNOME viewer: {e}"));
            }
        }
        self.viewer_visible = false;
    }

    fn viewer_mode(&mut self, on: bool) {
        self.viewer_mode = on;
        self.sync_modes();
    }

    fn animating(&mut self, on: bool) {
        self.fast_ticks = on;
    }

    fn modal_clicks(&mut self, on: bool) {
        self.modal = on;
        self.sync_modes();
    }

    fn shift_held(&self) -> bool {
        self.client
            .context()
            .map(|c| c.modifiers & SHIFT != 0)
            .or_else(|_| {
                self.last_context
                    .as_ref()
                    .map(|c| c.modifiers & SHIFT != 0)
                    .ok_or(Error::Disconnected)
            })
            .unwrap_or(false)
    }

    fn save_dialog(&mut self, name: &str, _ext: &str, _target: &Target) -> SaveChoice {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let folder = self.desktop_folder().unwrap_or(home);
        let suggested = folder.join(name);
        self.with_dialog(SaveChoice::Unavailable, || {
            crate::dialogs::save_file(&suggested)
        })
    }

    fn folder_dialog(&mut self, start: Option<&Path>, _target: &Target) -> SaveChoice {
        self.with_dialog(SaveChoice::Unavailable, || {
            crate::dialogs::pick_folder(start)
        })
    }

    fn click_modifiers(&self) -> Modifiers {
        self.click_modifiers
    }

    fn load_presets(&self) -> String {
        std::fs::read_to_string(self.db.with_file_name("image-presets.txt")).unwrap_or_default()
    }

    fn save_presets(&mut self, text: &str) -> Result<(), String> {
        let path = self.db.with_file_name("image-presets.txt");
        let tmp = self.db.with_file_name("image-presets.txt.tmp");
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, &path))
            .map_err(|_| "the image presets could not be saved".to_string())
    }

    fn format_time(&self, secs: i64) -> String {
        let tm = local_tm(secs);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        )
    }

    fn input_active(&mut self, active: bool) {
        if active {
            if self
                .client
                .capabilities
                .allow_interaction(self.keyboard_opt_in)
                .is_err()
            {
                self.log("GNOME keyboard input was not enabled with --wayland-keyboard");
                return;
            }
            let begun = if self.keyboard_opt_in {
                self.client.input_active(true)
            } else {
                self.mouse_ticket.take().ok_or(Error::Stale)
            };
            match begun {
                Ok(generation) => {
                    self.generation = generation;
                    self.active = true;
                    self.click_started = None;
                    self.click_modifiers = Modifiers::default();
                    self.viewer_mode = false;
                    self.modal = false;
                    self.authorized = self.session_target.clone().map(|target| AuthorizedPaste {
                        generation,
                        target,
                        expires: None,
                    });
                }
                Err(e) => self.log(&format!("could not start GNOME input: {e}")),
            }
        } else {
            self.mouse_ticket = None;
            if self.active {
                if let Err(e) = self.client.input_active(false) {
                    self.log(&format!("could not release GNOME keyboard input: {e}"));
                }
            }
            self.active = false;
            self.viewer_mode = false;
            self.modal = false;
            self.click_started = None;
            self.mark_pending_expiry();
        }
    }

    fn input_generation(&self) -> u64 {
        self.generation
    }

    fn heartbeat(&mut self) {}

    fn modifiers_released(&mut self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if self
                .client
                .context()
                .map(|c| !unsafe_modifier(c.modifiers))
                .unwrap_or(false)
            {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn poll_click(&mut self, first: bool) -> ClickPoll {
        if self.generation == 0 {
            return ClickPoll::Rejected("no active Wayland session");
        }
        if first || self.click_started.is_none() {
            self.click_started = Some(Instant::now());
        }
        let result = match self.client.poll_click(self.generation, first) {
            Ok(r) => r,
            Err(Error::Stale) => return ClickPoll::Rejected("the Wayland session ended"),
            Err(_) => return ClickPoll::Rejected("the click could not be verified"),
        };
        match result {
            ClickPollResult::Pending => ClickPoll::Pending,
            ClickPollResult::Rejected => ClickPoll::Rejected("click was not on a paste target"),
            ClickPollResult::Desktop => {
                let Ok(context) = self.client.context() else {
                    return ClickPoll::Rejected("the desktop click could not be verified");
                };
                self.remember_context(context);
                self.overlay_visible = false;
                ClickPoll::Desktop
            }
            ClickPollResult::Target {
                id,
                pid,
                exe,
                class,
                desktop,
            } => {
                if !desktop {
                    if self.client.current(id, pid) != Ok(true) {
                        if self
                            .click_started
                            .is_some_and(|t| t.elapsed() < CLICK_SETTLE)
                        {
                            return ClickPoll::Pending;
                        }
                        return ClickPoll::Rejected("the clicked window did not keep focus");
                    }
                    if self.client.privileged(id, pid).unwrap_or(true) {
                        return ClickPoll::Rejected("the clicked window is privileged");
                    }
                }
                let Ok(context) = self.client.context() else {
                    return ClickPoll::Rejected("the clicked window could not be verified");
                };
                self.remember_context(context);
                let target = Target {
                    id,
                    pid,
                    exe: exe.to_ascii_lowercase(),
                    class,
                    desktop,
                };
                self.overlay_visible = false;
                self.session_target = Some(target.clone());
                self.authorized = Some(AuthorizedPaste {
                    generation: self.generation,
                    target: target.clone(),
                    expires: None,
                });
                ClickPoll::Target(target)
            }
        }
    }

    fn write_clipboard(&mut self, payload: &Payload) -> Result<(), String> {
        if !self.client.capabilities.paste() {
            return Err("the GNOME companion cannot paste".into());
        }
        self.client
            .write(payload)
            .map(|_| ())
            .map_err(|_| "the clipboard is busy or the companion is unavailable".into())
    }

    fn send_paste(&mut self, key: PasteKey) -> bool {
        let Some(auth) = self.authorized.take() else {
            self.log("paste refused: no authorized target");
            return false;
        };
        if auth.expires.is_none_or(|end| Instant::now() > end) {
            self.log("paste refused: the authorization expired");
            return false;
        }
        if !self.client.capabilities.paste() || !self.client.capabilities.keyboard_activation() {
            return false;
        }
        if self.client.current(auth.target.id, auth.target.pid) != Ok(true)
            || self
                .client
                .privileged(auth.target.id, auth.target.pid)
                .unwrap_or(true)
        {
            self.log("paste refused: the target changed");
            return false;
        }
        if self
            .client
            .context()
            .map_or(true, |c| unsafe_modifier(c.modifiers))
        {
            self.log("paste refused: Alt or Super is held");
            return false;
        }
        match self
            .client
            .send_paste(auth.generation, auth.target.id, auth.target.pid, chord(key))
        {
            Ok(true) => true,
            Ok(false) => {
                self.log("paste was rejected by the companion");
                false
            }
            Err(_) => {
                self.log("paste could not be delivered");
                false
            }
        }
    }

    fn desktop_folder(&self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)?;
        let dirs = std::fs::read_to_string(home.join(".config").join("user-dirs.dirs"))
            .unwrap_or_default();
        let from_file = dirs.lines().find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DESKTOP_DIR=")?;
            let value = value
                .trim_matches('"')
                .replace("$HOME", &home.to_string_lossy());
            Some(PathBuf::from(value))
        });
        let path = from_file.unwrap_or_else(|| home.join("Desktop"));
        path.is_dir().then_some(path)
    }

    fn local_timestamp(&self) -> String {
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        let tm = local_tm(now);
        format!(
            "{:04}-{:02}-{:02} {:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }

    fn file_created(&mut self, _path: &Path) {}

    fn now_secs(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn log(&self, msg: &str) {
        let mut clean = msg.to_owned();
        let mut private = Vec::new();
        for name in ["HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"] {
            if let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) {
                private.push(PathBuf::from(value));
            }
        }
        if let Some(parent) = self.db.parent() {
            private.push(parent.to_path_buf());
        }
        private.sort_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));
        for path in private {
            let text = path.to_string_lossy();
            if !text.is_empty() {
                clean = clean.replace(text.as_ref(), "[private]");
            }
        }
        eprintln!("[pasteroll] {clean}");
    }

    fn add_custom_filter(
        &mut self,
        name: &str,
        extensions: &[String],
    ) -> Result<Vec<pasteroll_core::config::CustomFilter>, String> {
        let text = ks::read_config_text(&self.db)
            .map_err(|_| "the custom filter could not be saved".to_string())?;
        let new_text = pasteroll_core::config::add_custom_filter(&text, name, extensions)
            .map_err(|problems| problems.join("; "))?;
        ks::write_config_text::<crate::sys::LinuxOs>(&self.db, &new_text)
            .map_err(|_| "the custom filter could not be saved".to_string())?;
        pasteroll_core::config::parse(&new_text)
            .map(|c| c.custom_filters)
            .map_err(|problems| problems.join("; "))
    }
}

impl Drop for GnomePlatform {
    fn drop(&mut self) {
        if self.viewer_visible && self.generation != 0 {
            let _ = self.client.viewer_hide(self.generation);
        }
        if self.active {
            let _ = self.client.input_active(false);
        }
        if self.overlay_visible && self.overlay_generation != 0 {
            let _ = self.client.hide(self.overlay_generation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_and_device_coordinates_convert_once() {
        assert_eq!(device(12, 2.0), 24);
        assert_eq!(logical(24, 2.0), 12);
        assert_eq!(device_rect((10, 20, 300, 400), 1.5), (15, 30, 450, 600));
    }

    #[test]
    fn only_the_three_fixed_paste_chords_are_mapped() {
        assert_eq!(chord(PasteKey::CtrlV), 0);
        assert_eq!(chord(PasteKey::ShiftInsert), 1);
        assert_eq!(chord(PasteKey::CtrlShiftV), 2);
    }

    #[test]
    fn alt_super_and_missing_targets_are_not_authorized() {
        assert!(unsafe_modifier(ALT));
        assert!(unsafe_modifier(MOD4));
        assert!(unsafe_modifier(MOD5));
        assert!(unsafe_modifier(BUTTON1));
        assert!(unsafe_modifier(SUPER_HIGH));
        assert!(!unsafe_modifier(CONTROL | SHIFT));
        let context = Context {
            id: 0,
            pid: 0,
            exe: String::new(),
            class: String::new(),
            desktop: false,
            pointer: (0, 0),
            modifiers: 0,
            monitor: (0, 0, 1920, 1080),
            work: (0, 0, 1920, 1040),
            scale: 1.0,
        };
        assert!(target_from_context(&context).is_none());
    }
}
