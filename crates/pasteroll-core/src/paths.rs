//! Executable-relative portable data and the Windows Setup ownership marker.

use std::path::{Path, PathBuf};

pub const PORTABLE_MARKER: &str = "portable.txt";
/// Setup creates this beside the program; portable mode always takes precedence.
pub const SETUP_MARKER: &str = "setup.txt";
pub const PORTABLE_AUTOSTART: &str = "Start at sign-in is unavailable in portable mode.";

pub fn portable_db(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    dir.join(PORTABLE_MARKER).is_file().then(|| dir.join("data").join("history.db"))
}

pub fn is_portable() -> bool {
    std::env::current_exe().ok().and_then(|p| portable_db(&p)).is_some()
}

pub fn installed_by_setup(exe: &Path) -> bool {
    portable_db(exe).is_none() && exe.parent().is_some_and(|p| p.join(SETUP_MARKER).is_file())
}

/// Resolve portable mode before evaluating any user-profile paths. Explicit databases remain
/// supported outside portable mode; a portable copy must never write its data elsewhere.
pub fn database(
    exe: &Path,
    explicit: Option<PathBuf>,
    default: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    if let Some(db) = portable_db(exe) {
        if explicit.is_some() {
            return Err("--db is unavailable in portable mode; data stays beside the program in data/.".into());
        }
        return Ok(db);
    }
    explicit.map(Ok).unwrap_or_else(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("pasteroll-paths-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn exe(&self) -> PathBuf { self.0.join("pasteroll") }
    }
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn portable_data_is_beside_the_executable_without_consulting_the_profile() {
        let d = Scratch::new();
        std::fs::write(d.0.join(PORTABLE_MARKER), b"").unwrap();
        let db = database(&d.exe(), None, || panic!("must not resolve the profile")).unwrap();
        assert_eq!(db, d.0.join("data/history.db"));
        assert!(!d.0.join("data").exists(), "resolving paths does not create data");
        for name in ["config.toml", "paste_keys.txt", "image-presets.txt", "pasteroll.log", "first-run.txt"] {
            assert_eq!(db.with_file_name(name).parent(), Some(d.0.join("data").as_path()));
        }
        assert_eq!(crate::keystore::key_path(&db), d.0.join("data/history.key"));
    }

    #[test]
    fn portable_mode_refuses_an_explicit_database() {
        let d = Scratch::new();
        std::fs::write(d.0.join(PORTABLE_MARKER), b"").unwrap();
        assert!(database(&d.exe(), Some(d.0.join("elsewhere.db")), || panic!()).unwrap_err().contains("--db"));
    }

    #[test]
    fn ordinary_copies_keep_explicit_and_default_paths() {
        let d = Scratch::new();
        let custom = d.0.join("custom.db");
        assert_eq!(database(&d.exe(), Some(custom.clone()), || panic!()).unwrap(), custom);
        assert_eq!(database(&d.exe(), None, || Ok(custom.clone())).unwrap(), custom);
        assert_eq!(database(&d.exe(), None, || Err("no profile".into())).unwrap_err(), "no profile");
    }

    #[test]
    fn setup_detection_works_in_any_install_folder_and_portable_wins() {
        let d = Scratch::new();
        assert!(!installed_by_setup(&d.exe()));
        std::fs::write(d.0.join(SETUP_MARKER), b"").unwrap();
        assert!(installed_by_setup(&d.exe()));
        std::fs::write(d.0.join(PORTABLE_MARKER), b"").unwrap();
        assert!(!installed_by_setup(&d.exe()));
    }

    #[test]
    fn marker_directories_do_not_enable_either_mode() {
        let d = Scratch::new();
        std::fs::create_dir(d.0.join(PORTABLE_MARKER)).unwrap();
        std::fs::create_dir(d.0.join(SETUP_MARKER)).unwrap();
        assert!(portable_db(&d.exe()).is_none());
        assert!(!installed_by_setup(&d.exe()));
    }
}
