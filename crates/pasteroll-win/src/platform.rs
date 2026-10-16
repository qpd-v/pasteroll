//! The Windows implementation of [`pasteroll_app::Platform`]: it senses the desktop and acts on it;
//! every decision is the controller's.

use crate::clipboard::{self, Formats};
use crate::hooks::Hooks;
use crate::overlay::OverlayWindow;
use crate::{input, winsys};
use pasteroll_app::{ClickPoll, OverlayInfo, PasteKey, Payload, Platform, SaveChoice, Target};
use pasteroll_render::Frame;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct WinPlatform {
    /// The hidden message window that owns the clipboard and receives our commands.
    pub hwnd: HWND,
    pub overlay: OverlayWindow,
    /// The expanded viewer's window.
    pub viewer: OverlayWindow,
    /// Ticks come every 10 ms (an animation is running) instead of every 25 ms.
    fast_ticks: bool,
    pub hooks: Option<Hooks>,
    formats: Formats,
    own_pid: u32,
    /// Self-tests only: let a window of this process be a paste target.
    pub allow_own_process: bool,
    last_focus: Option<HWND>,
    local_generation: u64,
    start: Instant,
    /// History file whose folder holds `config.toml` (custom filters are saved there).
    db: Option<PathBuf>,
}

pub fn root_of(h: HWND) -> HWND {
    unsafe { GetAncestor(h, GA_ROOT) }
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn is_taskbar(h: HWND) -> bool {
    matches!(class_of(h).as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

fn is_desktop(h: HWND) -> bool {
    matches!(class_of(h).as_str(), "Progman" | "WorkerW")
}

/// Two top-level windows are "the same place". The desktop has different Progman/WorkerW roots
/// across monitors and wallpaper modes, so any desktop window matches any other.
fn same_place(a: HWND, b: HWND) -> bool {
    a == b || (is_desktop(a) && is_desktop(b))
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
    }
    pid
}

fn target_of(root: HWND) -> Target {
    let pid = pid_of(root);
    Target {
        id: root.0 as usize as u64,
        pid,
        exe: winsys::exe_name(pid).unwrap_or_default(),
        class: class_of(root),
        desktop: is_desktop(root),
    }
}

impl WinPlatform {
    pub fn new(hwnd: HWND, overlay: OverlayWindow, viewer: OverlayWindow, hooks: Option<Hooks>) -> WinPlatform {
        WinPlatform {
            hwnd,
            overlay,
            viewer,
            fast_ticks: false,
            hooks,
            formats: Formats::register(),
            own_pid: std::process::id(),
            allow_own_process: false,
            last_focus: None,
            local_generation: 0,
            start: Instant::now(),
            db: None,
        }
    }

    pub fn with_db(mut self, db: &Path) -> WinPlatform {
        self.db = Some(db.to_path_buf());
        self
    }
}

impl Platform for WinPlatform {
    fn foreground(&mut self) -> Option<Target> {
        let root = root_of(unsafe { GetForegroundWindow() });
        if root.0.is_null()
            || root == self.overlay.hwnd()
            || root == self.viewer.hwnd()
            || is_taskbar(root)
            || (pid_of(root) == self.own_pid && !self.allow_own_process)
        {
            return None;
        }
        Some(target_of(root))
    }

    fn target_is_current(&mut self, t: &Target) -> bool {
        let fg = root_of(unsafe { GetForegroundWindow() });
        if t.desktop {
            // any desktop window counts as the desktop
            return is_desktop(fg);
        }
        fg == HWND(t.id as usize as *mut _) && pid_of(fg) == t.pid
    }

    fn target_privileged(&mut self, t: &Target) -> bool {
        winsys::outranks_us(t.pid)
    }

    fn cursor(&self) -> (i32, i32) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        (p.x, p.y)
    }

    fn overlay_show(&mut self, at: (i32, i32)) -> OverlayInfo {
        self.overlay.show(POINT { x: at.0, y: at.1 })
    }

    fn overlay_present(&mut self, frame: &Frame) {
        self.overlay.present(frame);
    }

    fn overlay_hide(&mut self) {
        self.overlay.hide();
    }

    fn viewer_show(&mut self, rect: (i32, i32, u32, u32), rounded: bool) {
        self.viewer.show_rect(rect, rounded);
    }

    fn viewer_present(&mut self, frame: &Frame) {
        self.viewer.present(frame);
    }

    fn viewer_hide(&mut self) {
        self.viewer.hide();
    }

    fn viewer_mode(&mut self, on: bool) {
        crate::hooks::set_viewer_mode(on);
    }

    fn animating(&mut self, on: bool) {
        if on != self.fast_ticks {
            self.fast_ticks = on;
            unsafe {
                // the same timer id: this replaces the period
                SetTimer(Some(self.hwnd), crate::app::TIMER_TICK, if on { 10 } else { 25 }, None);
            }
        }
    }

    fn shift_held(&self) -> bool {
        unsafe { (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0 }
    }

    fn save_dialog(&mut self, name: &str, ext: &str, target: &Target) -> SaveChoice {
        // the dialog needs the keyboard: stop swallowing it while the dialog is up
        self.input_active(false);
        // the keyboard goes back to the program the session is for
        let back = HWND(target.id as usize as *mut _);
        let choice = crate::dialogs::save_file(self.viewer.hwnd(), name, ext, back);
        self.input_active(true);
        choice
    }

    fn format_time(&self, secs: i64) -> String {
        crate::dialogs::local_time(secs).unwrap_or_else(|| pasteroll_app::controller::utc_date_time(secs))
    }

    fn input_keepalive(&mut self) {
        crate::hooks::keepalive();
    }

    fn input_active(&mut self, active: bool) {
        self.local_generation += 1;
        if let Some(h) = &self.hooks {
            h.set_active(active);
        }
    }

    fn input_generation(&self) -> u64 {
        match &self.hooks {
            Some(h) => h.generation() as u64,
            None => self.local_generation,
        }
    }

    fn heartbeat(&mut self) {
        crate::hooks::heartbeat();
    }

    fn modifiers_released(&mut self, timeout: Duration) -> bool {
        input::wait_released(timeout)
    }

    /// Paste-at-click: after the user's click has landed, wait until the app has settled and the
    /// click was on a client area.
    fn poll_click(&mut self, first: bool) -> ClickPoll {
        if first {
            self.last_focus = None;
        }
        let Some(hooks) = &self.hooks else {
            return ClickPoll::Rejected("no input hooks");
        };
        let (clicked, pt) = hooks.clicked();
        let root = root_of(clicked);
        if root.0.is_null() || unsafe { !IsWindow(Some(root)).as_bool() } || is_taskbar(root) {
            return ClickPoll::Rejected("click was not on an application window");
        }
        let desktop = is_desktop(root);
        let fg = root_of(unsafe { GetForegroundWindow() });
        if !same_place(fg, root) {
            return ClickPoll::Pending; // the click is still activating the window
        }
        // Hit-test the clicked window and its root: the title bar, borders and caption buttons
        // fail the root test; scrollbars and push buttons are refused by class.
        let lparam = LPARAM(((pt.y as i16 as u16 as isize) << 16) | (pt.x as i16 as u16 as isize));
        let hit = |h: HWND| -> Option<u32> {
            let mut r = 0usize;
            let ok = unsafe {
                SendMessageTimeoutW(h, WM_NCHITTEST, WPARAM(0), lparam, SMTO_ABORTIFHUNG, 50, Some(&mut r))
            };
            (ok.0 != 0).then_some(r as u32)
        };
        if hit(root) != Some(HTCLIENT) || (clicked != root && hit(clicked) != Some(HTCLIENT)) {
            return ClickPoll::Rejected("click was not on a client area");
        }
        if matches!(class_of(clicked).as_str(), "Button" | "ScrollBar") {
            return ClickPoll::Rejected("click was on a control, not a text area");
        }
        let down = |vk: VIRTUAL_KEY| unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 };
        if down(VK_LBUTTON) || !input::held_modifiers().is_empty() {
            return ClickPoll::Pending;
        }
        if desktop {
            return ClickPoll::Desktop;
        }
        let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        let tid = unsafe { GetWindowThreadProcessId(root, None) };
        let focus = unsafe {
            if GetGUIThreadInfo(tid, &mut info).is_ok() { info.hwndFocus } else { HWND::default() }
        };
        if self.last_focus != Some(focus) {
            self.last_focus = Some(focus); // focus moved: wait one more tick for it to settle
            return ClickPoll::Pending;
        }
        ClickPoll::Target(target_of(root))
    }

    fn write_clipboard(&mut self, payload: &Payload) -> Result<(), String> {
        let r = match payload {
            Payload::Text(t) => clipboard::set_text(self.hwnd, &self.formats, t),
            Payload::Files(f) => clipboard::set_files(self.hwnd, &self.formats, f),
            Payload::Image(png) => match pasteroll_core::image::decode_png(png) {
                Some(rgba) => clipboard::set_image(self.hwnd, &self.formats, png, &rgba),
                None => return Err("stored image could not be decoded".into()),
            },
        };
        r.map_err(|e| e.to_string())
    }

    fn send_paste(&mut self, key: PasteKey) -> bool {
        input::send_paste_key(key)
    }

    fn desktop_folder(&self) -> Option<PathBuf> {
        unsafe {
            match SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) {
                Ok(p) => {
                    let s = p.to_string().ok();
                    CoTaskMemFree(Some(p.0 as *const _));
                    s.map(PathBuf::from)
                }
                Err(_) => None,
            }
        }
    }

    fn local_timestamp(&self) -> String {
        let t = unsafe { GetLocalTime() };
        format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
    }

    fn file_created(&mut self, path: &Path) {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        unsafe {
            SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as *const _), None);
        }
    }

    fn now_secs(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn log(&self, msg: &str) {
        eprintln!("[pasteroll] {msg}");
    }

    fn add_custom_filter(
        &mut self,
        name: &str,
        extensions: &[String],
    ) -> Result<Vec<pasteroll_core::config::CustomFilter>, String> {
        let Some(db) = self.db.clone() else {
            return Err("Custom filters cannot be saved here.".to_owned());
        };
        let text = crate::keystore::read_config_text(&db)?;
        let new_text = pasteroll_core::config::add_custom_filter(&text, name, extensions)
            .map_err(|problems| problems.join("; "))?;
        crate::keystore::write_config_text(&db, &new_text)?;
        pasteroll_core::config::parse(&new_text)
            .map(|c| c.custom_filters)
            .map_err(|problems| problems.join("; "))
    }
}
