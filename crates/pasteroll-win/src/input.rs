//! Synthetic input helpers: key chords with modifier hygiene, and real mouse clicks for the
//! self-test. Real (non-injected) input is what exercises activation behaviour, so the probe uses
//! `SendInput` rather than posting window messages.

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

/// Navigation keys must carry the extended-key flag. Without it, Windows can read the event as a
/// numpad key, and with NumLock on, Shift+Numpad4 drops the Shift: Shift+Left would arrive as
/// plain Left.
fn is_extended(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_INSERT
            | VK_DELETE | VK_RCONTROL | VK_RMENU | VK_LWIN | VK_RWIN
    )
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    if is_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn send(inputs: &[INPUT]) -> u32 {
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) }
}

const MODIFIERS: [VIRTUAL_KEY; 5] = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];

fn is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

/// Modifiers the user is physically holding right now (for example the activation chord).
pub fn held_modifiers() -> Vec<VIRTUAL_KEY> {
    MODIFIERS.iter().copied().filter(|&m| is_down(m)).collect()
}

/// Sends `mods + key` as one chord. Ctrl and Shift that the user is holding are released first and
/// pressed again afterwards, so the target sees exactly the intended chord and the user's state is
/// left as it was.
///
/// Alt and Win are deliberately NOT toggled: a synthetic Alt-up with nothing between toggles the
/// target's menu bar, and releasing/re-pressing Win can open Start. Callers must `wait_released`
/// for them first (and give up if they stay down).
pub fn send_chord(mods: &[VIRTUAL_KEY], vk: VIRTUAL_KEY) -> bool {
    let held: Vec<VIRTUAL_KEY> =
        held_modifiers().into_iter().filter(|m| matches!(*m, VK_CONTROL | VK_SHIFT)).collect();
    let mut seq: Vec<INPUT> = Vec::new();
    for &m in &held {
        seq.push(key(m, true));
    }
    for &m in mods {
        seq.push(key(m, false));
    }
    seq.push(key(vk, false));
    seq.push(key(vk, true));
    for &m in mods.iter().rev() {
        seq.push(key(m, true));
    }
    for &m in &held {
        seq.push(key(m, false));
    }
    // `SendInput` returns how many events it accepted; fewer than we sent means input was blocked
    // (for example by UIPI when the target is elevated).
    send(&seq) as usize == seq.len()
}

/// Waits (pumping nothing) until Alt and Win are physically up. `false` if still held at timeout.
pub fn wait_released(timeout: std::time::Duration) -> bool {
    let end = std::time::Instant::now() + timeout;
    loop {
        if ![VK_MENU, VK_LWIN, VK_RWIN].iter().any(|&k| is_down(k)) {
            return true;
        }
        if std::time::Instant::now() >= end {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Pastes with the chord this app wants.
pub fn send_paste_key(key: pasteroll_app::PasteKey) -> bool {
    use pasteroll_app::PasteKey::*;
    match key {
        CtrlV => send_chord(&[VK_CONTROL], VIRTUAL_KEY(b'V' as u16)),
        ShiftInsert => send_chord(&[VK_SHIFT], VK_INSERT),
        CtrlShiftV => send_chord(&[VK_CONTROL, VK_SHIFT], VIRTUAL_KEY(b'V' as u16)),
    }
}

/// Types one ASCII character (self-test only).
pub fn send_char(c: char) -> bool {
    send_chord(&[], VIRTUAL_KEY(c.to_ascii_uppercase() as u16))
}

pub fn move_to(p: POINT) {
    unsafe {
        let _ = SetCursorPos(p.x, p.y);
    }
}

pub fn left_down() {
    send(&[mouse(MOUSEEVENTF_LEFTDOWN)]);
}

pub fn left_up() {
    send(&[mouse(MOUSEEVENTF_LEFTUP)]);
}

