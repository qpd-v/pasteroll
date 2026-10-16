//! Low-level mouse and keyboard hooks on a dedicated thread.
//!
//! Hook callbacks must return fast or Windows drops the hook, so they only classify an event and
//! `PostMessage` a command to the app window. All decisions are made on the UI thread.
//!
//! Swallowing rules:
//! - the activation button's down *and* its matching up are swallowed, so the app underneath never
//!   sees a half click;
//! - while a session is open the wheel is swallowed everywhere;
//! - Enter / Esc / Up / Down are swallowed (down, repeat and up) while a session is open;
//! - so are the keys that type into the search box (letters, digits, punctuation, space, Backspace,
//!   Tab). They are translated to characters here and posted to the app window only; nothing is
//!   stored, logged or sent anywhere, and none of this exists outside a live session (the
//!   heartbeat and the 60-second cap bound it). Keys with Alt or Win held pass through untouched;
//!   with Ctrl held (usually the remains of the activation chord) they are swallowed and ignored,
//!   so they cannot fire a shortcut in the window underneath;
//! - clicks are never swallowed: they reach the target app, which is what places the caret.
//!
//! While the expanded viewer is open ("viewer mode") the wheel is posted raw (with Ctrl and Shift)
//! instead of as steps, Left / Right / Home / End / F3 / F11 are taken too, and a click outside the
//! viewer posts `ClickOutside` (it closes the viewer) instead of arming a paste-at-click. The viewer
//! extends the session limit with [`keepalive`] while it is in use.
//!
//! Injected events are ignored unless `accept_injected` is set (the self-test uses that).

use pasteroll_core::{Step, WheelAccumulator};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::Mutex;
use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_PASTEROLL: u32 = WM_APP + 220;
/// A typed character for the search box: `wParam` is the UTF-16 unit, `lParam` the generation.
pub const WM_PASTEROLL_CHAR: u32 = WM_APP + 222;
/// A wheel turn in viewer mode: `wParam` low 16 bits the delta (i16), bit 16 horizontal, bit 17
/// Ctrl, bit 18 Shift; `lParam` the generation.
pub const WM_PASTEROLL_WHEEL: u32 = WM_APP + 223;
const HOOK_KEYBOARD_ON: u32 = WM_APP + 230;
const HOOK_KEYBOARD_OFF: u32 = WM_APP + 231;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Command {
    Activate = 1,
    Older,
    Newer,
    Confirm,
    Cancel,
    PageUp,
    PageDown,
    Delete,
    CtrlDelete,
    TogglePin,
    Backspace,
    CtrlBackspace,
    Tab,
    /// Left click outside the overlay completed: paste there.
    ClickPaste,
    /// Right/other click outside the overlay: close without pasting.
    ClickCancel,
    Left,
    Right,
    Home,
    End,
    F3,
    ShiftF3,
    F11,
    ShiftDelete,
    CtrlA,
    CtrlC,
    CtrlE,
    CtrlF,
    CtrlI,
    CtrlS,
    CtrlZ,
    /// A click outside the expanded viewer (viewer mode only).
    ClickOutside,
}

impl Command {
    pub fn from_usize(v: usize) -> Option<Command> {
        use Command::*;
        [
            Activate, Older, Newer, Confirm, Cancel, PageUp, PageDown, Delete, CtrlDelete, TogglePin, Backspace,
            CtrlBackspace, Tab, ClickPaste, ClickCancel, Left, Right, Home, End, F3, ShiftF3, F11, ShiftDelete,
            CtrlA, CtrlC, CtrlE, CtrlF, CtrlI, CtrlS, CtrlZ, ClickOutside,
        ]
        .into_iter()
            .find(|c| *c as usize == v)
    }
}

struct Shared {
    owner: AtomicIsize,
    overlay: AtomicIsize,
    active: AtomicBool,
    generation: AtomicUsize,
    accept_injected: AtomicBool,
    swallowed_buttons: AtomicU32,
    swallowed_keys: Mutex<[bool; 256]>,
    /// Keys whose press was let through to the target: their auto-repeats are not re-decided, so a
    /// key is never half swallowed (a down the target saw, with its up swallowed).
    passed_keys: Mutex<[bool; 256]>,
    click_x: AtomicI32,
    click_y: AtomicI32,
    click_hwnd: AtomicIsize,
    down_outside: AtomicBool,
    down_root: AtomicIsize,
    down_gen: AtomicUsize,
    wheel: Mutex<WheelAccumulator>,
}

static S: Shared = Shared {
    owner: AtomicIsize::new(0),
    overlay: AtomicIsize::new(0),
    active: AtomicBool::new(false),
    generation: AtomicUsize::new(0),
    accept_injected: AtomicBool::new(false),
    swallowed_buttons: AtomicU32::new(0),
    swallowed_keys: Mutex::new([false; 256]),
    passed_keys: Mutex::new([false; 256]),
    click_x: AtomicI32::new(0),
    click_y: AtomicI32::new(0),
    click_hwnd: AtomicIsize::new(0),
    down_outside: AtomicBool::new(false),
    down_root: AtomicIsize::new(0),
    down_gen: AtomicUsize::new(0),
    wheel: Mutex::new(WheelAccumulator::new()),
};

/// A session only swallows input while the UI thread keeps proving it is alive. If the UI thread
/// stalls (a huge clip, a busy database, a hung clipboard owner) the hooks stop swallowing within
/// `HEARTBEAT_MAX_MS` instead of leaving the wheel and Enter/Esc dead system-wide, and no session
/// can swallow for longer than `SESSION_MAX_MS` regardless.
const HEARTBEAT_MAX_MS: u64 = 500;
const SESSION_MAX_MS: u64 = 60_000;
static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static SESSION_STARTED: AtomicU64 = AtomicU64::new(0);
/// The expanded viewer's window, and whether it is open.
static VIEWER: AtomicIsize = AtomicIsize::new(0);
static VIEWER_MODE: AtomicBool = AtomicBool::new(false);

/// The viewer window (so clicks on it count as "inside").
pub fn set_viewer_window(h: HWND) {
    VIEWER.store(h.0 as isize, SeqCst);
}

/// The expanded viewer opened or closed.
pub fn set_viewer_mode(on: bool) {
    VIEWER_MODE.store(on, SeqCst);
    S.wheel.lock().map(|mut w| w.reset()).ok();
}

fn viewer_mode() -> bool {
    VIEWER_MODE.load(SeqCst)
}

/// The viewer is in use: the session's time limit starts again. Only the viewer calls this; the
/// heartbeat still has to prove the UI thread alive.
pub fn keepalive() {
    if S.active.load(SeqCst) {
        SESSION_STARTED.store(tick_count(), SeqCst);
    }
}

fn tick_count() -> u64 {
    unsafe { GetTickCount64() }
}

/// Called from the UI thread's timer.
pub fn heartbeat() {
    HEARTBEAT.store(tick_count(), SeqCst);
}

/// Whether a session is open *and* the UI thread is demonstrably alive.
fn session_live() -> bool {
    if !S.active.load(SeqCst) {
        return false;
    }
    let now = tick_count();
    now.saturating_sub(HEARTBEAT.load(SeqCst)) <= HEARTBEAT_MAX_MS
        && now.saturating_sub(SESSION_STARTED.load(SeqCst)) <= SESSION_MAX_MS
}

static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
static KEYBOARD_HOOK: AtomicIsize = AtomicIsize::new(0);

pub struct Hooks {
    thread: Option<std::thread::JoinHandle<()>>,
    thread_id: u32,
}

impl Hooks {
    pub fn start(owner: HWND, overlay: HWND, accept_injected: bool) -> Result<Hooks> {
        S.owner.store(owner.0 as isize, SeqCst);
        S.overlay.store(overlay.0 as isize, SeqCst);
        S.accept_injected.store(accept_injected, SeqCst);
        let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<u32, String>>();
        let thread = std::thread::spawn(move || unsafe {
            let hinst = match GetModuleHandleW(None) {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            // Force the message queue to exist before reporting the thread id.
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(hinst.into()), 0) {
                Ok(h) => MOUSE_HOOK.store(h.0 as isize, SeqCst),
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            }
            let _ = tx.send(Ok(GetCurrentThreadId()));
            let mut off_retries = 0u32;
            let mut off_timer = 0usize;
            let unhook_keyboard = || {
                let h = KEYBOARD_HOOK.swap(0, SeqCst);
                if h != 0 {
                    let _ = UnhookWindowsHookEx(HHOOK(h as *mut _));
                }
                if let Ok(mut k) = S.swallowed_keys.lock() {
                    *k = [false; 256];
                }
                if let Ok(mut k) = S.passed_keys.lock() {
                    *k = [false; 256];
                }
            };
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let mut try_off = false;
                match msg.message {
                    HOOK_KEYBOARD_ON => {
                        // a new session supersedes any pending removal
                        if off_timer != 0 {
                            let _ = KillTimer(None, off_timer);
                            off_timer = 0;
                        }
                        off_retries = 0;
                        if let Ok(mut k) = S.swallowed_keys.lock() {
                            *k = [false; 256];
                        }
                        if let Ok(mut k) = S.passed_keys.lock() {
                            *k = [false; 256];
                        }
                        if KEYBOARD_HOOK.load(SeqCst) == 0 {
                            if let Ok(h) =
                                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hinst.into()), 0)
                            {
                                KEYBOARD_HOOK.store(h.0 as isize, SeqCst);
                            }
                        }
                    }
                    HOOK_KEYBOARD_OFF => {
                        off_retries = 0;
                        try_off = true;
                    }
                    WM_TIMER if off_timer != 0 && msg.wParam.0 == off_timer => {
                        let _ = KillTimer(None, off_timer);
                        off_timer = 0;
                        try_off = true;
                    }
                    _ => {}
                }
                if try_off {
                    if S.active.load(SeqCst) {
                        // a new session started meanwhile: the hook must stay
                        off_retries = 0;
                    } else {
                        // keep swallowing until every swallowed key has been released
                        let pending =
                            S.swallowed_keys.lock().map(|k| k.iter().any(|&b| b)).unwrap_or(false);
                        if pending && off_retries < 100 {
                            off_retries += 1;
                            off_timer = SetTimer(None, 0, 10, None);
                        } else {
                            unhook_keyboard();
                        }
                    }
                }
            }
            for slot in [&KEYBOARD_HOOK, &MOUSE_HOOK] {
                let h = slot.swap(0, SeqCst);
                if h != 0 {
                    let _ = UnhookWindowsHookEx(HHOOK(h as *mut _));
                }
            }
        });
        match rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Ok(thread_id)) => Ok(Hooks { thread: Some(thread), thread_id }),
            Ok(Err(e)) => Err(windows::core::Error::new(E_FAIL, e)),
            Err(_) => Err(windows::core::Error::new(E_FAIL, "hook thread did not start")),
        }
    }

    /// Opens or closes the session as far as input is concerned.
    pub fn set_active(&self, active: bool) {
        S.generation.fetch_add(1, SeqCst);
        S.wheel.lock().map(|mut w| w.reset()).ok();
        if active {
            let now = tick_count();
            SESSION_STARTED.store(now, SeqCst);
            HEARTBEAT.store(now, SeqCst);
        }
        S.active.store(active, SeqCst);
        unsafe {
            let msg = if active { HOOK_KEYBOARD_ON } else { HOOK_KEYBOARD_OFF };
            let _ = PostThreadMessageW(self.thread_id, msg, WPARAM(0), LPARAM(0));
        }
    }

    pub fn generation(&self) -> usize {
        S.generation.load(SeqCst)
    }

    /// The window and screen point of the last completed outside click.
    pub fn clicked(&self) -> (HWND, POINT) {
        (
            HWND(S.click_hwnd.load(SeqCst) as *mut _),
            POINT { x: S.click_x.load(SeqCst), y: S.click_y.load(SeqCst) },
        )
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        S.active.store(false, SeqCst);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn post(cmd: Command) {
    let gen = if cmd == Command::Activate { 0 } else { S.generation.load(SeqCst) };
    unsafe {
        let _ = PostMessageW(
            Some(HWND(S.owner.load(SeqCst) as *mut _)),
            WM_PASTEROLL,
            WPARAM(cmd as usize),
            LPARAM(gen as isize),
        );
    }
}

fn post_wheel(delta: i16, horizontal: bool, ctrl: bool, shift: bool) {
    let flags = usize::from(horizontal) | (usize::from(ctrl) << 1) | (usize::from(shift) << 2);
    unsafe {
        let _ = PostMessageW(
            Some(HWND(S.owner.load(SeqCst) as *mut _)),
            WM_PASTEROLL_WHEEL,
            WPARAM(delta as u16 as usize | (flags << 16)),
            LPARAM(S.generation.load(SeqCst) as isize),
        );
    }
}

fn post_char(unit: u16) {
    unsafe {
        let _ = PostMessageW(
            Some(HWND(S.owner.load(SeqCst) as *mut _)),
            WM_PASTEROLL_CHAR,
            WPARAM(unit as usize),
            LPARAM(S.generation.load(SeqCst) as isize),
        );
    }
}

fn modifiers() -> (bool, bool, bool, bool) {
    let down = |vk: VIRTUAL_KEY| unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 };
    (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU), down(VK_LWIN) || down(VK_RWIN))
}

fn inside_overlay(pt: POINT) -> bool {
    let h = unsafe { WindowFromPoint(pt).0 as isize };
    h == S.overlay.load(SeqCst) || (h != 0 && h == VIEWER.load(SeqCst))
}

/// In viewer mode only the viewer itself counts: the dimmed carousel behind it is backdrop.
fn inside_viewer(pt: POINT) -> bool {
    let h = unsafe { WindowFromPoint(pt).0 as isize };
    h != 0 && h == VIEWER.load(SeqCst)
}

/// Activation: Ctrl+Shift+middle-click (a middle-click chord that is rarely used by programs).
fn is_activation(msg: u32) -> bool {
    msg == WM_MBUTTONDOWN && modifiers() == (true, true, false, false)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let ev = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let injected = ev.flags & LLMHF_INJECTED != 0;
            if !injected || S.accept_injected.load(SeqCst) {
                if mouse_event(wparam.0 as u32, ev) {
                    return LRESULT(1);
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// `true` = swallow.
fn mouse_event(msg: u32, ev: &MSLLHOOKSTRUCT) -> bool {
    const MBUTTON: u32 = 1;
    // matching up for a swallowed activation
    if msg == WM_MBUTTONUP && S.swallowed_buttons.load(SeqCst) & MBUTTON != 0 {
        S.swallowed_buttons.fetch_and(!MBUTTON, SeqCst);
        return true;
    }
    let active = session_live();
    if is_activation(msg) {
        S.swallowed_buttons.fetch_or(MBUTTON, SeqCst);
        if !active {
            post(Command::Activate);
        }
        return true;
    }
    // A stale "swallow the next middle-up" flag (the activation's up was consumed by another hook)
    // must not eat an ordinary middle click later.
    if msg == WM_MBUTTONDOWN {
        S.swallowed_buttons.fetch_and(!MBUTTON, SeqCst);
    }
    if !active {
        return false;
    }
    if viewer_mode() {
        return viewer_mouse_event(msg, ev);
    }
    match msg {
        WM_MOUSEWHEEL => {
            let delta = (ev.mouseData >> 16) as u16 as i16;
            if let Ok(mut w) = S.wheel.lock() {
                w.push(delta, |step| {
                    post(match step {
                        Step::Newer => Command::Newer,
                        Step::Older => Command::Older,
                    })
                });
            }
            true
        }
        WM_LBUTTONDOWN => {
            let outside = !inside_overlay(ev.pt);
            S.down_outside.store(outside, SeqCst);
            let root = unsafe { GetAncestor(WindowFromPoint(ev.pt), GA_ROOT) };
            S.down_root.store(root.0 as isize, SeqCst);
            S.down_gen.store(S.generation.load(SeqCst), SeqCst);
            false
        }
        WM_LBUTTONUP => {
            if S.down_outside.swap(false, SeqCst) {
                let clicked = unsafe { WindowFromPoint(ev.pt) };
                let root = unsafe { GetAncestor(clicked, GA_ROOT) };
                if S.down_gen.load(SeqCst) == S.generation.load(SeqCst)
                    && !inside_overlay(ev.pt)
                    && root.0 as isize == S.down_root.load(SeqCst)
                {
                    S.click_hwnd.store(clicked.0 as isize, SeqCst);
                    S.click_x.store(ev.pt.x, SeqCst);
                    S.click_y.store(ev.pt.y, SeqCst);
                    post(Command::ClickPaste);
                }
            }
            false
        }
        WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            if !inside_overlay(ev.pt) {
                post(Command::ClickCancel);
            }
            false
        }
        _ => false,
    }
}

/// Mouse handling while the viewer is open. `true` = swallow.
fn viewer_mouse_event(msg: u32, ev: &MSLLHOOKSTRUCT) -> bool {
    match msg {
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = (ev.mouseData >> 16) as u16 as i16;
            let (ctrl, shift, _, _) = modifiers();
            post_wheel(delta, msg == WM_MOUSEHWHEEL, ctrl, shift);
            true
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            // never a paste-at-click while the viewer is open: a click elsewhere closes it
            S.down_outside.store(false, SeqCst);
            if !inside_viewer(ev.pt) {
                post(Command::ClickOutside);
            }
            false
        }
        _ => false,
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let ev = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let injected = ev.flags.0 & LLKHF_INJECTED.0 != 0;
            if (!injected || S.accept_injected.load(SeqCst)) && key_event(wparam.0 as u32, ev) {
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// What a key means to the carousel while a session is live.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Out {
    Cmd(Command),
    Char(u16),
    /// Swallow the key and do nothing (a dead key, or Ctrl+letter during a session).
    Ignore,
}

/// The text a key produces in the foreground window's layout, without disturbing dead-key state.
/// `Some(Some(c))` a character, `Some(None)` a dead key, `None` not a text key.
fn translate(vk: u32, scan: u32, shift: bool, altgr: bool) -> Option<Option<u16>> {
    let mut state = [0u8; 256];
    if shift {
        state[VK_SHIFT.0 as usize] = 0x80;
    }
    if altgr {
        state[VK_CONTROL.0 as usize] = 0x80;
        state[VK_MENU.0 as usize] = 0x80;
    }
    unsafe {
        if GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 {
            state[VK_CAPITAL.0 as usize] = 1;
        }
        if GetKeyState(VK_NUMLOCK.0 as i32) & 1 != 0 {
            state[VK_NUMLOCK.0 as usize] = 1;
        }
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let layout = GetKeyboardLayout(thread);
        let mut buf = [0u16; 8];
        // flag 4: do not change the keyboard state (a dead key stays pending for the real app)
        let n = ToUnicodeEx(vk, scan, &state, &mut buf, 4, Some(layout));
        match n {
            0 => None,
            n if n < 0 => Some(None),
            _ => {
                let c = buf[0];
                (c >= 0x20 && c != 0x7f).then_some(Some(c)).or(Some(None))
            }
        }
    }
}

fn classify(vk: usize, scan: u32) -> Option<Out> {
    let (ctrl, shift, alt, win) = modifiers();
    let plain = !ctrl && !shift && !alt && !win;
    let ctrl_only = ctrl && !shift && !alt && !win;
    let direct = match VIRTUAL_KEY(vk as u16) {
        VK_ESCAPE => Some(Command::Cancel),
        VK_RETURN => Some(Command::Confirm),
        VK_UP if plain => Some(Command::Newer),
        VK_DOWN if plain => Some(Command::Older),
        VK_PRIOR if plain => Some(Command::PageUp),
        VK_NEXT if plain => Some(Command::PageDown),
        VK_DELETE if plain => Some(Command::Delete),
        VK_DELETE if ctrl_only => Some(Command::CtrlDelete),
        VK_TAB if plain => Some(Command::Tab),
        VK_BACK if plain || (shift && !ctrl && !alt && !win) => Some(Command::Backspace),
        VK_BACK if ctrl_only => Some(Command::CtrlBackspace),
        VK_DELETE if shift && !ctrl && !alt && !win => Some(Command::ShiftDelete),
        VIRTUAL_KEY(0x50) if ctrl_only => Some(Command::TogglePin), // Ctrl+P
        VIRTUAL_KEY(0x41) if ctrl_only => Some(Command::CtrlA),
        VIRTUAL_KEY(0x43) if ctrl_only => Some(Command::CtrlC),
        VIRTUAL_KEY(0x45) if ctrl_only => Some(Command::CtrlE),
        VIRTUAL_KEY(0x46) if ctrl_only => Some(Command::CtrlF),
        VIRTUAL_KEY(0x49) if ctrl_only => Some(Command::CtrlI),
        VIRTUAL_KEY(0x53) if ctrl_only => Some(Command::CtrlS),
        VIRTUAL_KEY(0x5A) if ctrl_only => Some(Command::CtrlZ),
        // the viewer's navigation keys belong to the application underneath otherwise
        VK_LEFT if plain && viewer_mode() => Some(Command::Left),
        VK_RIGHT if plain && viewer_mode() => Some(Command::Right),
        VK_HOME if plain && viewer_mode() => Some(Command::Home),
        VK_END if plain && viewer_mode() => Some(Command::End),
        VK_F3 if plain && viewer_mode() => Some(Command::F3),
        VK_F3 if shift && !ctrl && !alt && !win && viewer_mode() => Some(Command::ShiftF3),
        VK_F11 if plain && viewer_mode() => Some(Command::F11),
        _ => None,
    };
    if let Some(c) = direct {
        return Some(Out::Cmd(c));
    }
    if win || (alt && !ctrl) {
        return None; // Alt+key and Win+key belong to the system and the target app
    }
    let altgr = ctrl && alt;
    match translate(vk as u32, scan, shift, altgr) {
        None => None,
        Some(Some(c)) if !ctrl || altgr => Some(Out::Char(c)),
        Some(_) => Some(Out::Ignore),
    }
}

fn key_event(msg: u32, ev: &KBDLLHOOKSTRUCT) -> bool {
    let vk = ev.vkCode as usize;
    if vk >= 256 {
        return false;
    }
    let Ok(mut swallowed) = S.swallowed_keys.lock() else { return false };
    let Ok(mut passed) = S.passed_keys.lock() else { return false };
    if msg == WM_KEYUP || msg == WM_SYSKEYUP {
        passed[vk] = false;
        if swallowed[vk] {
            swallowed[vk] = false;
            return true;
        }
        return false;
    }
    if msg != WM_KEYDOWN && msg != WM_SYSKEYDOWN {
        return false;
    }
    if swallowed[vk] {
        // auto-repeat of a swallowed key: keep going for the keys that should repeat, drain the rest
        if session_live() {
            match classify(vk, ev.scanCode) {
                Some(Out::Cmd(
                    c @ (Command::Newer
                    | Command::Older
                    | Command::Backspace
                    | Command::CtrlBackspace
                    | Command::Left
                    | Command::Right
                    | Command::PageUp
                    | Command::PageDown
                    | Command::F3
                    | Command::ShiftF3),
                )) => post(c),
                Some(Out::Char(c)) => post_char(c),
                _ => {}
            }
        }
        return true;
    }
    // Decide once per physical press: a repeat of a key we let through is let through again, even
    // if a modifier changed meanwhile, so the target never sees a down whose up we swallowed.
    if passed[vk] {
        return false;
    }
    if !session_live() {
        passed[vk] = true;
        return false;
    }
    match classify(vk, ev.scanCode) {
        Some(out) => {
            swallowed[vk] = true;
            match out {
                Out::Cmd(c) => post(c),
                Out::Char(c) => post_char(c),
                Out::Ignore => {}
            }
            true
        }
        None => {
            passed[vk] = true;
            false
        }
    }
}
