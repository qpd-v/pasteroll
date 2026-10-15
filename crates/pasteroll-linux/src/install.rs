//! Installing for the current user, "start when I sign in", and the first-run notice marker.
//! Everything lives under the user's home: no root, nothing system-wide.
//!
//!   ~/.local/bin/pasteroll                                  the program
//!   ~/.local/share/applications/pasteroll.desktop           the application menu entry
//!   ~/.local/share/icons/hicolor/256x256/apps/pasteroll.png the icon
//!   ~/.config/autostart/pasteroll.desktop                   only when "start with login" is on

use std::path::{Path, PathBuf};

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from).ok_or_else(|| "HOME is not set".to_string())
}

fn data_home() -> Result<PathBuf, String> {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(p) => Ok(PathBuf::from(p)),
        None => Ok(home()?.join(".local").join("share")),
    }
}

fn config_home() -> Result<PathBuf, String> {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(p) => Ok(PathBuf::from(p)),
        None => Ok(home()?.join(".config")),
    }
}

pub fn bin_path() -> Result<PathBuf, String> {
    Ok(home()?.join(".local").join("bin").join("pasteroll"))
}

fn desktop_path() -> Result<PathBuf, String> {
    Ok(data_home()?.join("applications").join("pasteroll.desktop"))
}

fn icon_path() -> Result<PathBuf, String> {
    Ok(data_home()?.join("icons").join("hicolor").join("256x256").join("apps").join("pasteroll.png"))
}

fn autostart_path() -> Result<PathBuf, String> {
    Ok(config_home()?.join("autostart").join("pasteroll.desktop"))
}

/// A freedesktop desktop entry that starts `exe run`.
pub fn desktop_entry(exe: &Path, autostart: bool) -> String {
    // a path with spaces must be quoted in Exec
    let exec = if exe.to_string_lossy().contains(' ') {
        format!("\"{}\" run", exe.display())
    } else {
        format!("{} run", exe.display())
    };
    let mut s = String::from("[Desktop Entry]\nType=Application\nName=Pasteroll\n");
    s.push_str("Comment=Clipboard history you can scroll through and paste from\n");
    s.push_str(&format!("Exec={exec}\nIcon=pasteroll\nTerminal=false\nCategories=Utility;\nStartupNotify=false\n"));
    if autostart {
        s.push_str("X-GNOME-Autostart-enabled=true\n");
    }
    s
}

fn write_file(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("new");
    std::fs::write(&tmp, data).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot place {}: {e}", path.display()))
}

pub fn autostart_enabled() -> bool {
    autostart_path().map(|p| p.exists()).unwrap_or(false)
}

/// Turns "start when I sign in" on or off. Off by default; only the person asks for it.
pub fn set_autostart(on: bool) -> Result<(), String> {
    let path = autostart_path()?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        write_file(&path, desktop_entry(&exe, true).as_bytes(), 0o644)
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
        }
    }
}

pub fn install() -> Result<String, String> {
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dst = bin_path()?;
    if std::fs::canonicalize(&src).ok() != std::fs::canonicalize(&dst).ok() {
        let bytes = std::fs::read(&src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
        write_file(&dst, &bytes, 0o755)?;
    }
    write_file(&desktop_path()?, desktop_entry(&dst, false).as_bytes(), 0o644)?;
    write_file(&icon_path()?, pasteroll_core::logo::LOGO_PNG, 0o644)?;
    // an existing autostart entry follows the program to its new place
    if autostart_enabled() {
        write_file(&autostart_path()?, desktop_entry(&dst, true).as_bytes(), 0o644)?;
    }
    Ok(format!(
        "Installed to {}. Find Pasteroll in your application menu, or run `{} run`. \
         (If the command is not found, add ~/.local/bin to your PATH.)",
        dst.display(),
        dst.display()
    ))
}

/// Removes the installation. The history and settings stay unless `remove_data`.
pub fn uninstall(remove_data: bool, data_dir: Option<&Path>) -> Result<String, String> {
    for p in [autostart_path()?, desktop_path()?, icon_path()?, bin_path()?] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {e}", p.display())),
        }
    }
    let mut note = String::new();
    if let Some(d) = data_dir {
        if remove_data {
            let mut n = 0;
            // only the files Pasteroll itself makes: the folder may have been chosen with --db
            for name in [
                "history.db",
                "history.db-wal",
                "history.db-shm",
                "history.key",
                "history.key.tmp",
                "history.lock",
                "app.lock",
                "config.toml",
                "config.toml.tmp",
                "paste_keys.txt",
                "first-run.txt",
            ] {
                if std::fs::remove_file(d.join(name)).is_ok() {
                    n += 1;
                }
            }
            let _ = std::fs::remove_dir(d);
            note = format!(" Deleted {n} file(s) in {}.", d.display());
        } else {
            note = format!(" Your history and settings were kept in {}.", d.display());
        }
    }
    Ok(format!("Pasteroll was removed.{note}"))
}

pub const NOTICE_TITLE: &str = "Welcome to Pasteroll";

pub const NOTICE: &str = "Pasteroll keeps a history of what you copy so you can paste it again later.\n\
- It is stored only on this computer, encrypted, in your home folder. Nothing is sent anywhere.\n\
- Open it with Ctrl+Shift+middle-click. Pick a clip with the wheel and press Enter, or select part of it first to paste just that part.\n\
- Pause recording, or exclude programs such as password managers, from the tray icon: Settings.\n\
- Pasteroll does not start when you sign in unless you turn that on in Settings.\n\
You will only see this once.";

/// True the first time (and records that it was shown).
pub fn first_run_pending(db: &Path) -> bool {
    let Some(dir) = db.parent() else { return false };
    !dir.join("first-run.txt").exists()
}

pub fn mark_first_run_done(db: &Path) {
    if let Some(dir) = db.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join("first-run.txt"), format!("shown by version {}\n", env!("CARGO_PKG_VERSION")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_entry_runs_the_program_and_quotes_spaces() {
        let e = desktop_entry(Path::new("/home/me/.local/bin/pasteroll"), false);
        assert!(e.contains("Exec=/home/me/.local/bin/pasteroll run\n"));
        assert!(e.contains("Icon=pasteroll") && e.contains("Terminal=false"));
        assert!(!e.contains("X-GNOME-Autostart-enabled"));
        let q = desktop_entry(Path::new("/home/a b/pasteroll"), true);
        assert!(q.contains("Exec=\"/home/a b/pasteroll\" run\n"));
        assert!(q.contains("X-GNOME-Autostart-enabled=true"));
    }

    #[test]
    fn install_autostart_and_uninstall_round_trip_in_a_scratch_home() {
        let home = std::env::temp_dir().join(format!("pasteroll-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // these tests run single-threaded for the environment change
        let _g = ENV.lock().unwrap();
        let (h, x, c) = (std::env::var_os("HOME"), std::env::var_os("XDG_DATA_HOME"), std::env::var_os("XDG_CONFIG_HOME"));
        std::env::set_var("HOME", &home);
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");

        let msg = install().unwrap();
        assert!(msg.contains("Installed"));
        assert!(bin_path().unwrap().exists());
        assert!(desktop_path().unwrap().exists());
        assert!(icon_path().unwrap().exists());
        assert!(!autostart_enabled());
        set_autostart(true).unwrap();
        assert!(autostart_enabled());
        let entry = std::fs::read_to_string(autostart_path().unwrap()).unwrap();
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
        set_autostart(false).unwrap();
        assert!(!autostart_enabled());
        set_autostart(false).unwrap(); // already off is fine

        let data = home.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("history.db"), b"x").unwrap();
        std::fs::write(data.join("other-file.txt"), b"keep me").unwrap();
        let kept = uninstall(false, Some(&data)).unwrap();
        assert!(kept.contains("kept"));
        assert!(!bin_path().unwrap().exists() && !desktop_path().unwrap().exists());
        assert!(data.join("history.db").exists());
        install().unwrap();
        uninstall(true, Some(&data)).unwrap();
        assert!(!data.join("history.db").exists());
        assert!(data.join("other-file.txt").exists(), "files that are not ours stay");

        for (k, v) in [("HOME", h), ("XDG_DATA_HOME", x), ("XDG_CONFIG_HOME", c)] {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn the_first_run_marker_is_per_data_folder() {
        let d = std::env::temp_dir().join(format!("pasteroll-firstrun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let db = d.join("history.db");
        assert!(first_run_pending(&db));
        mark_first_run_done(&db);
        assert!(!first_run_pending(&db));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_notice_says_what_matters() {
        for needle in ["only on this computer", "encrypted", "Nothing is sent", "does not start when you sign in"] {
            assert!(NOTICE.contains(needle), "{needle}");
        }
    }
}
