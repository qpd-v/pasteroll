//! Which keystroke pastes in which app.
//!
//! Ctrl+V is right for almost everything, including Windows Terminal and VS Code's terminal. A few
//! programs bind Ctrl+V to something else (Emacs scrolls, mintty and PuTTY expect Shift+Insert),
//! so those get a different chord. Users can override or extend this with `paste_keys.txt`:
//!
//! ```text
//! # exe name = chord      (ctrl+v | shift+insert | ctrl+shift+v)
//! myterminal.exe = ctrl+shift+v
//! ```

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteKey {
    CtrlV,
    ShiftInsert,
    CtrlShiftV,
}

impl PasteKey {
    fn parse(s: &str) -> Option<PasteKey> {
        let norm: String = s.to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
        match norm.as_str() {
            "ctrl+v" | "control+v" => Some(PasteKey::CtrlV),
            "shift+insert" | "shift+ins" => Some(PasteKey::ShiftInsert),
            "ctrl+shift+v" | "shift+ctrl+v" => Some(PasteKey::CtrlShiftV),
            _ => None,
        }
    }
}

/// Programs known to want Shift+Insert.
const SHIFT_INSERT: &[&str] = &[
    // Linux: the xterm family pastes the PRIMARY selection on Shift+Insert and binds nothing to
    // Ctrl+Shift+V out of the box
    "xterm",
    "uxterm",
    "urxvt",
    "st",
    "mintty.exe",
    "putty.exe",
    "kitty.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "emacs.exe",
    "runemacs.exe",
    "gvim.exe",
];

/// Linux terminal emulators: Ctrl+V is a literal control character there, Ctrl+Shift+V pastes.
const CTRL_SHIFT_V: &[&str] = &[
    "gnome-terminal-server",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "tilix",
    "terminator",
    "kitty",
    "alacritty",
    "wezterm-gui",
    "foot",
    "lxterminal",
    "mate-terminal",
    "qterminal",
];

#[derive(Debug, Default)]
pub struct PasteKeys {
    table: HashMap<String, PasteKey>,
}

impl PasteKeys {
    pub fn builtin() -> PasteKeys {
        let mut table: HashMap<String, PasteKey> =
            SHIFT_INSERT.iter().map(|e| ((*e).to_owned(), PasteKey::ShiftInsert)).collect();
        for e in CTRL_SHIFT_V {
            table.insert((*e).to_owned(), PasteKey::CtrlShiftV);
        }
        PasteKeys { table }
    }

    /// Built-in table plus the user's overrides. A missing or unreadable file is not an error;
    /// unparseable lines are reported in the returned warnings.
    pub fn load(path: &Path) -> (PasteKeys, Vec<String>) {
        let mut keys = PasteKeys::builtin();
        let mut warnings = Vec::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            warnings = keys.apply(&text);
        }
        (keys, warnings)
    }

    pub fn apply(&mut self, text: &str) -> Vec<String> {
        let mut warnings = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            match line.split_once('=') {
                Some((exe, chord)) => match PasteKey::parse(chord) {
                    Some(k) => {
                        self.table.insert(exe.trim().to_ascii_lowercase(), k);
                    }
                    None => warnings.push(format!("line {}: unknown chord {:?}", n + 1, chord.trim())),
                },
                None => warnings.push(format!("line {}: expected `exe = chord`", n + 1)),
            }
        }
        warnings
    }

    pub fn for_exe(&self, exe: &str) -> PasteKey {
        self.table.get(&exe.to_ascii_lowercase()).copied().unwrap_or(PasteKey::CtrlV)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_ctrl_v() {
        let k = PasteKeys::builtin();
        assert_eq!(k.for_exe("notepad.exe"), PasteKey::CtrlV);
        assert_eq!(k.for_exe("windowsterminal.exe"), PasteKey::CtrlV);
    }

    #[test]
    fn known_programs_use_shift_insert() {
        let k = PasteKeys::builtin();
        assert_eq!(k.for_exe("mintty.exe"), PasteKey::ShiftInsert);
        assert_eq!(k.for_exe("PuTTY.exe"), PasteKey::ShiftInsert);
    }

    #[test]
    fn user_file_overrides_and_extends() {
        let mut k = PasteKeys::builtin();
        let w = k.apply(
            "# comment\nmyterm.exe = Ctrl+Shift+V\nmintty.exe = ctrl+v   # I changed it\n\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(k.for_exe("myterm.exe"), PasteKey::CtrlShiftV);
        assert_eq!(k.for_exe("mintty.exe"), PasteKey::CtrlV);
    }

    #[test]
    fn bad_lines_warn_and_do_not_break_the_rest() {
        let mut k = PasteKeys::builtin();
        let w = k.apply("nonsense\nfoo.exe = hyper+v\nbar.exe = shift+insert\n");
        assert_eq!(w.len(), 2);
        assert_eq!(k.for_exe("bar.exe"), PasteKey::ShiftInsert);
    }
}
