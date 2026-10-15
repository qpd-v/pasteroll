//! Programs where a pasted line break is a keypress: text with a newline can run a command.

use std::collections::HashSet;

/// Executable names (lower-case) known to be terminals or shells. VS Code's integrated terminal
/// cannot be told apart from the editor from outside, so it is not here.
/// Window classes that are terminals whatever program runs inside them.
pub const BUILTIN_CLASSES: &[&str] = &[
    "consolewindowclass",
    "cascadia_hosting_window_class",
    "mintty",
    "putty",
    "virtualconsoleclass",
    "pseudoconsolewindow",
];

pub const BUILTIN: &[&str] = &[
    // Windows
    "windowsterminal.exe",
    "wt.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "openconsole.exe",
    "mintty.exe",
    "putty.exe",
    "kitty.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "wsl.exe",
    "bash.exe",
    // macOS / Linux names, so a future backend only has to fill in the executable name
    "terminal",
    "iterm2",
    "gnome-terminal",
    "konsole",
    "xterm",
    "kitty",
    "alacritty",
    "wezterm",
    "tmux",
    "gnome-terminal-server",
    "xfce4-terminal",
    "tilix",
    "terminator",
    "uxterm",
    "foot",
    "lxterminal",
    "mate-terminal",
    "qterminal",
    "st",
    "urxvt",
];

#[derive(Debug, Clone)]
pub struct PastePolicy {
    /// Ask for a second Enter before pasting multi-line text into a terminal.
    pub warn_multiline_in_terminals: bool,
    terminals: HashSet<String>,
}

impl Default for PastePolicy {
    fn default() -> Self {
        PastePolicy::new(true, &[])
    }
}

impl PastePolicy {
    pub fn new(warn: bool, extra_terminals: &[String]) -> PastePolicy {
        let mut terminals: HashSet<String> = BUILTIN.iter().map(|s| (*s).to_owned()).collect();
        terminals.extend(extra_terminals.iter().map(|s| s.to_ascii_lowercase()));
        PastePolicy { warn_multiline_in_terminals: warn, terminals }
    }

    pub fn is_terminal(&self, exe: &str, class: &str) -> bool {
        self.terminals.contains(&exe.to_ascii_lowercase())
            || BUILTIN_CLASSES.contains(&class.to_ascii_lowercase().as_str())
    }

    /// Should this text give the user pause before it goes to `exe`? Any line break counts, a
    /// trailing one included: pasting "ls\n" into a shell runs it.
    pub fn needs_warning(&self, exe: &str, class: &str, text: &str) -> bool {
        self.warn_multiline_in_terminals && self.is_terminal(exe, class) && text.contains(['\n', '\r'])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_recognised_case_insensitively() {
        let p = PastePolicy::default();
        assert!(p.is_terminal("WindowsTerminal.exe", ""));
        assert!(p.is_terminal("powershell.exe", ""));
        assert!(!p.is_terminal("notepad.exe", "Notepad"));
        assert!(!p.is_terminal("code.exe", "Chrome_WidgetWin_1"));

    }

    #[test]
    fn a_terminal_window_class_counts_whatever_runs_inside() {
        let p = PastePolicy::default();
        // python.exe or ssh.exe running in a console window report themselves as the client
        assert!(p.is_terminal("python.exe", "ConsoleWindowClass"));
        assert!(p.needs_warning("ssh.exe", "CASCADIA_HOSTING_WINDOW_CLASS", "a\nb"));
        assert!(!p.needs_warning("python.exe", "Notepad", "a\nb"));
    }

    #[test]
    fn only_text_with_a_line_break_warns() {
        let p = PastePolicy::default();
        assert!(p.needs_warning("cmd.exe", "", "a\nb"));
        assert!(p.needs_warning("cmd.exe", "", "a\r\nb"));
        assert!(p.needs_warning("cmd.exe", "", "rm -rf build\n"), "a trailing newline runs the command");
        assert!(!p.needs_warning("cmd.exe", "", "single line"));
        assert!(!p.needs_warning("notepad.exe", "", "a\nb"));
    }

    #[test]
    fn it_can_be_switched_off_and_extended() {
        let off = PastePolicy::new(false, &[]);
        assert!(!off.needs_warning("cmd.exe", "", "a\nb"));
        let extra = PastePolicy::new(true, &["MyTerm.exe".to_owned()]);
        assert!(extra.needs_warning("myterm.exe", "", "a\nb"));
    }
}
