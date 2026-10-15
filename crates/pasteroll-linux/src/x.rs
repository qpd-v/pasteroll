//! Small X11 helpers: atoms, the keyboard mapping, and facts about windows and their processes.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub type XResult<T> = Result<T, Box<dyn std::error::Error>>;

pub struct Atoms {
    pub clipboard: Atom,
    pub utf8_string: Atom,
    pub targets: Atom,
    pub text_plain_utf8: Atom,
    pub text_plain: Atom,
    pub uri_list: Atom,
    pub gnome_files: Atom,
    pub image_png: Atom,
    pub incr: Atom,
    pub timestamp: Atom,
    pub net_active_window: Atom,
    pub net_wm_pid: Atom,
    pub net_wm_window_type: Atom,
    pub window_type_desktop: Atom,
    pub resource_manager: Atom,
    pub password_hint: Atom,
    pub transfer: Atom,
}

impl Atoms {
    pub fn new(conn: &RustConnection) -> XResult<Atoms> {
        let names: [&[u8]; 16] = [
            b"CLIPBOARD",
            b"UTF8_STRING",
            b"TARGETS",
            b"text/plain;charset=utf-8",
            b"text/plain",
            b"text/uri-list",
            b"x-special/gnome-copied-files",
            b"image/png",
            b"INCR",
            b"TIMESTAMP",
            b"_NET_ACTIVE_WINDOW",
            b"_NET_WM_PID",
            b"_NET_WM_WINDOW_TYPE",
            b"_NET_WM_WINDOW_TYPE_DESKTOP",
            b"RESOURCE_MANAGER",
            b"x-kde-passwordManagerHint",
        ];
        let cookies: Vec<_> = names.iter().map(|n| conn.intern_atom(false, n)).collect::<Result<_, _>>()?;
        let mut a = Vec::with_capacity(names.len());
        for c in cookies {
            a.push(c.reply()?.atom);
        }
        let transfer = conn.intern_atom(false, b"PASTEROLL_TRANSFER")?.reply()?.atom;
        Ok(Atoms {
            clipboard: a[0],
            utf8_string: a[1],
            targets: a[2],
            text_plain_utf8: a[3],
            text_plain: a[4],
            uri_list: a[5],
            gnome_files: a[6],
            image_png: a[7],
            incr: a[8],
            timestamp: a[9],
            net_active_window: a[10],
            net_wm_pid: a[11],
            net_wm_window_type: a[12],
            window_type_desktop: a[13],
            resource_manager: a[14],
            password_hint: a[15],
            transfer,
        })
    }
}

/// keycode -> keysyms, as the server reports them.
pub struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_INSERT: u32 = 0xff63;

impl Keymap {
    #[cfg(test)]
    pub fn from_parts(min: u8, per: usize, syms: Vec<u32>) -> Keymap {
        Keymap { min, per, syms }
    }

    pub fn load(conn: &RustConnection) -> XResult<Keymap> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let r = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        Ok(Keymap { min, per: r.keysyms_per_keycode as usize, syms: r.keysyms })
    }

    /// The keysym at `index` (0 plain, 1 shift, 2 AltGr, 3 AltGr+shift) for `code`, or 0.
    pub fn sym(&self, code: u8, index: usize) -> u32 {
        if code < self.min || index >= self.per {
            return 0;
        }
        self.syms.get((code - self.min) as usize * self.per + index).copied().unwrap_or(0)
    }

    /// A keycode that produces `sym` unshifted.
    pub fn keycode_for(&self, sym: u32) -> Option<u8> {
        if self.per == 0 {
            return None;
        }
        self.syms
            .chunks(self.per)
            .position(|c| c.first() == Some(&sym))
            .map(|i| self.min.wrapping_add(i as u8))
    }

    /// A keycode with `sym` at any level (modifier keys sometimes only appear at level 0 anyway).
    pub fn keycode_any(&self, sym: u32) -> Option<u8> {
        self.keycode_for(sym).or_else(|| {
            self.syms
                .chunks(self.per.max(1))
                .position(|c| c.contains(&sym))
                .map(|i| self.min.wrapping_add(i as u8))
        })
    }
}

fn prop_u32(conn: &RustConnection, win: Window, prop: Atom) -> Option<Vec<u32>> {
    let r = conn.get_property(false, win, prop, AtomEnum::ANY, 0, 64).ok()?.reply().ok()?;
    r.value32().map(|it| it.collect())
}

pub fn pid_of(conn: &RustConnection, atoms: &Atoms, win: Window) -> Option<u32> {
    prop_u32(conn, win, atoms.net_wm_pid).and_then(|v| v.first().copied())
}

/// Lower-case executable name of a process, from /proc.
pub fn exe_of_pid(pid: u32) -> Option<String> {
    let p = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let name = p.file_name()?.to_string_lossy().into_owned();
    Some(name.trim_end_matches(" (deleted)").to_ascii_lowercase())
}

/// Does the process run as another user than we do (so our keystrokes could not be trusted there)?
pub fn pid_is_other_user(pid: u32) -> bool {
    let me = unsafe { libc::geteuid() };
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else { return false };
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().nth(1)) // effective uid
        .and_then(|s| s.parse::<u32>().ok())
        .is_some_and(|uid| uid != me)
}

/// WM_CLASS class part, lower-case ("firefox", "xterm").
pub fn class_of(conn: &RustConnection, win: Window) -> Option<String> {
    let r = conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256).ok()?.reply().ok()?;
    let parts: Vec<&[u8]> = r.value.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    let class = parts.get(1).or(parts.first())?;
    Some(String::from_utf8_lossy(class).to_ascii_lowercase())
}

fn has_wm_class(conn: &RustConnection, win: Window) -> bool {
    conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|r| r.value_len > 0)
}

/// The application window `win` belongs to: `win` or its nearest ancestor that has a class. With a
/// reparenting window manager the pointer's window is deep inside the client.
pub fn client_of(conn: &RustConnection, root: Window, win: Window) -> Window {
    let mut cur = win;
    for _ in 0..32 {
        if cur == root || cur == 0 {
            return win;
        }
        if has_wm_class(conn, cur) {
            return cur;
        }
        match conn.query_tree(cur).ok().and_then(|c| c.reply().ok()) {
            Some(t) if t.parent != 0 && t.parent != root => cur = t.parent,
            _ => return cur,
        }
    }
    win
}

pub fn is_desktop(conn: &RustConnection, atoms: &Atoms, win: Window) -> bool {
    prop_u32(conn, win, atoms.net_wm_window_type).is_some_and(|v| v.contains(&atoms.window_type_desktop))
}

/// Device pixels per DIP from `Xft.dpi`, 1.0 when it is not set.
pub fn scale_from_xft(conn: &RustConnection, atoms: &Atoms, root: Window) -> f32 {
    let Ok(cookie) = conn.get_property(false, root, atoms.resource_manager, AtomEnum::STRING, 0, 16384) else {
        return 1.0;
    };
    let Ok(r) = cookie.reply() else { return 1.0 };
    let text = String::from_utf8_lossy(&r.value);
    text.lines()
        .find_map(|l| l.strip_prefix("Xft.dpi:"))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|dpi| (dpi / 96.0).clamp(1.0, 4.0))
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_process_is_not_another_user() {
        assert!(!pid_is_other_user(std::process::id()));
    }

    #[test]
    fn exe_of_pid_names_this_test_binary() {
        let n = exe_of_pid(std::process::id()).unwrap();
        assert!(!n.is_empty() && n == n.to_ascii_lowercase());
    }

    #[test]
    fn the_keymap_finds_keycodes_and_levels() {
        let km = Keymap { min: 8, per: 2, syms: vec![0x61, 0x41, 0x62, 0x42] };
        assert_eq!(km.sym(8, 0), 0x61);
        assert_eq!(km.sym(8, 1), 0x41);
        assert_eq!(km.sym(9, 1), 0x42);
        assert_eq!(km.sym(7, 0), 0);
        assert_eq!(km.keycode_for(0x62), Some(9));
        assert_eq!(km.keycode_for(0x42), None, "only unshifted");
        assert_eq!(km.keycode_any(0x42), Some(9));
    }
}
