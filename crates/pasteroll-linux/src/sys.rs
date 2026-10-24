//! The Linux parts of the key store: where the data lives and how the key file is protected.
//!
//! Unlike Windows there is no per-user key service every desktop has, so the key ring file is
//! protected by file permissions only (a 0700 folder owned by the user, a 0600 file). Anything
//! running as the same user can read it. That is the same boundary as on Windows against
//! same-user malware, but there is no TPM/DPAPI binding, so a stolen disk image without disk
//! encryption exposes the history. Full-disk encryption is recommended.

use pasteroll_core::keystore::{LockError, Os};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub struct LinuxOs;

/// Held while the cross-process lock is wanted; the lock goes away with the file descriptor.
pub struct FileLock {
    _file: std::fs::File,
}

const KEY_MAGIC: &[u8] = b"PRK1";

fn euid() -> u32 {
    unsafe { libc::geteuid() }
}

impl Os for LinuxOs {
    type Lock = FileLock;
    const ACCOUNT: &'static str = "user account";

    fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let mut v = KEY_MAGIC.to_vec();
        v.extend_from_slice(plain);
        Ok(v)
    }

    fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        sealed
            .strip_prefix(KEY_MAGIC)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "the key file is not a Pasteroll key file".to_string())
    }

    fn private_dir(dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("cannot inspect {}: {e}", dir.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{} is a symbolic link; refusing to use it", dir.display()));
        }
        if !meta.is_dir() {
            return Err(format!("{} is not a folder", dir.display()));
        }
        if meta.uid() != euid() {
            return Err(format!("{} is not owned by the current user; refusing to store history there", dir.display()));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("could not make {} private: {e}", dir.display()))?;
        }
        Ok(())
    }

    fn is_link(path: &Path) -> bool {
        std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
    }

    fn secure_file(path: &Path) -> Result<(), String> {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("could not make {} private: {e}", path.display()))
    }

    fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| format!("cannot replace {}: {e}", to.display()))?;
        // make the rename itself durable
        if let Some(dir) = to.parent() {
            if let Ok(d) = std::fs::File::open(dir) {
                let _ = d.sync_all();
            }
        }
        Ok(())
    }

    fn lock_file(path: &Path, exclusive: bool) -> Result<FileLock, LockError> {
        use std::os::fd::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| LockError::Io(format!("cannot open {}: {e}", path.display())))?;
        let op = if exclusive { libc::LOCK_EX } else { libc::LOCK_SH } | libc::LOCK_NB;
        let rc = unsafe { libc::flock(file.as_raw_fd(), op) };
        if rc == 0 {
            Ok(FileLock { _file: file })
        } else {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                Err(LockError::Busy)
            } else {
                Err(LockError::Io(format!("cannot lock {}: {e}", path.display())))
            }
        }
    }

    fn free_space(dir: &Path) -> Option<u64> {
        use std::ffi::CString;
        let c = CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
        (rc == 0).then(|| (st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
}

/// `$XDG_DATA_HOME/pasteroll/history.db`, else `~/.local/share/pasteroll/history.db`.
pub fn default_db() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(db) = pasteroll_core::paths::portable_db(&exe) {
        return Ok(db);
    }
    let base = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => {
            let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).ok_or("HOME is not set")?;
            PathBuf::from(home).join(".local").join("share")
        }
    };
    Ok(base.join("pasteroll").join("history.db"))
}

/// The Desktop folder, if there is one: `XDG_DESKTOP_DIR` from the user-dirs file in
/// `$XDG_CONFIG_HOME` (else `~/.config`), else `~/Desktop`. Only an existing folder counts.
pub fn desktop_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from)?;
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    desktop_in(&home, config.as_deref())
}

fn desktop_in(home: &Path, config_home: Option<&Path>) -> Option<PathBuf> {
    let config = config_home.map(Path::to_path_buf).unwrap_or_else(|| home.join(".config"));
    let dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let from_file = dirs.lines().find_map(|line| {
        let value = line.trim().strip_prefix("XDG_DESKTOP_DIR=")?;
        let value = value.trim_matches('"').replace("$HOME", &home.to_string_lossy());
        Some(PathBuf::from(value))
    });
    // a relative value is not a folder we can name; `XDG_DESKTOP_DIR="$HOME/"` is how the user-dirs
    // file says "there is no Desktop", and pictures must not then land in the home folder itself
    let path = from_file
        .filter(|p| p.is_absolute() && p.as_path() != home)
        .unwrap_or_else(|| home.join("Desktop"));
    path.is_dir().then_some(path)
}

/// Core dumps would put the decrypted history on disk: turn them off for this process.
pub fn harden_process() {
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        let zero = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
    }
}

/// One instance per user: two would grab the same input twice.
pub fn single_instance(db: &Path) -> Result<FileLock, String> {
    let path = db.with_file_name("app.lock");
    if let Some(dir) = path.parent() {
        LinuxOs::private_dir(dir)?;
    }
    match LinuxOs::lock_file(&path, true) {
        Ok(l) => Ok(l),
        Err(LockError::Busy) => Err("Pasteroll is already running".to_string()),
        Err(LockError::Io(e)) => Err(e),
    }
}

/// Local-only control pipe. Its reader belongs to the process holding app.lock; no PID is
/// stored and no signal is sent to a possibly reused PID. Create only after taking that lock.
pub struct StopListener {
    file: std::fs::File,
    path: PathBuf,
}

impl StopListener {
    pub fn new(db: &Path) -> Result<Self, String> {
        use std::ffi::CString;
        use std::os::unix::fs::FileTypeExt;
        let path = db.with_file_name("app.stop");
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_fifo() && m.uid() == euid() => {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            Ok(_) => return Err("app.stop is not our control pipe; refusing to replace it".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let c = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
        if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
            return Err(format!("cannot create the stop pipe: {}", std::io::Error::last_os_error()));
        }
        let file = std::fs::OpenOptions::new().read(true).write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(&path).map_err(|e| e.to_string())?;
        Ok(Self { file, path })
    }

    pub fn requested(&mut self) -> bool {
        use std::io::Read;
        let mut buf = [0u8; 64];
        self.file.read(&mut buf).is_ok_and(|n| buf[..n].split(|b| *b == b'\n').any(|s| s == b"stop"))
    }
}

impl Drop for StopListener {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
}

/// Read the existing instance lock without creating a data folder or lock file.
fn instance_running(db: &Path) -> Result<bool, String> {
    use std::os::fd::AsRawFd;
    let file = match std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NOFOLLOW)
        .open(db.with_file_name("app.lock")) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(false);
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) { Ok(true) } else { Err(err.to_string()) }
}

pub fn stop_running(db: &Path) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::FileTypeExt;
    let path = db.with_file_name("app.stop");
    let mut sent = false;
    for _ in 0..50 {
        if !instance_running(db)? { return Ok(()); }
        if !sent {
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                if !meta.file_type().is_fifo() || meta.uid() != euid() {
                    return Err("app.stop is not our control pipe; refusing to use it".into());
                }
                match std::fs::OpenOptions::new().write(true)
                    .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(&path) {
                    Ok(mut f) => {
                        f.write_all(b"stop\n").map_err(|e| e.to_string())?;
                        sent = true;
                    }
                    Err(e) if matches!(e.raw_os_error(), Some(libc::ENXIO) | Some(libc::ENOENT)) => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !instance_running(db)? { return Ok(()); }
    Err("Pasteroll did not quit. Quit it from the tray icon and try again. Older versions may not support stop.".into())
}

#[cfg(test)]
mod desktop_tests {
    use super::*;

    #[test]
    fn the_desktop_comes_from_the_user_dirs_file_in_the_config_folder() {
        let dir = std::env::temp_dir().join(format!("pasteroll-desktop-{}", std::process::id()));
        let (home, config) = (dir.join("home"), dir.join("cfg"));
        std::fs::create_dir_all(home.join("Schreibtisch")).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        assert_eq!(desktop_in(&home, Some(&config)), None, "no Desktop folder and no file");
        std::fs::write(config.join("user-dirs.dirs"), "XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n").unwrap();
        assert_eq!(desktop_in(&home, Some(&config)), Some(home.join("Schreibtisch")));
        std::fs::create_dir_all(home.join("Desktop")).unwrap();
        assert_eq!(desktop_in(&home, None), Some(home.join("Desktop")), "~/.config is not read when the config folder is elsewhere");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pasteroll_core::keystore as ks;

    #[test]
    fn stop_without_an_instance_creates_no_data() {
        let d = temp("stop-absent");
        stop_running(&d.join("history.db")).unwrap();
        assert!(!d.exists());
    }

    #[test]
    fn stop_requests_a_graceful_exit_and_waits_for_the_instance_lock() {
        let d = temp("stop-running");
        let db = d.join("history.db");
        let lock = single_instance(&db).unwrap();
        let mut listener = StopListener::new(&db).unwrap();
        let peer_db = db.clone();
        let peer = std::thread::spawn(move || stop_running(&peer_db));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !listener.requested() {
            assert!(std::time::Instant::now() < deadline, "stop request never arrived");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(instance_running(&db).unwrap());
        drop(listener);
        drop(lock);
        peer.join().unwrap().unwrap();
        assert!(!db.with_file_name("app.stop").exists());
        stop_running(&db).unwrap();
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn stop_never_replaces_or_writes_an_unrelated_file() {
        let d = temp("stop-foreign");
        let db = d.join("history.db");
        let lock = single_instance(&db).unwrap();
        let path = d.join("app.stop");
        std::fs::write(&path, b"keep").unwrap();
        assert!(StopListener::new(&db).is_err());
        assert!(stop_running(&db).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        drop(lock);
        std::fs::remove_dir_all(d).unwrap();
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pasteroll-linux-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_new_history_gets_a_private_folder_and_key_file() {
        let d = temp("a");
        let db = d.join("history.db");
        let store = ks::open_store::<LinuxOs>(&db).unwrap();
        drop(store);
        let dir_mode = std::fs::metadata(&d).unwrap().permissions().mode() & 0o777;
        let key_mode = std::fs::metadata(ks::key_path(&db)).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(key_mode, 0o600);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_key_survives_a_reopen_and_a_rotation() {
        let d = temp("b");
        let db = d.join("history.db");
        {
            let mut s = ks::open_store::<LinuxOs>(&db).unwrap();
            s.add_text("hello", 1_700_000_000).unwrap();
        }
        ks::rotate_history::<LinuxOs>(&db, 1_700_000_100).unwrap();
        let s = ks::open_store::<LinuxOs>(&db).unwrap();
        assert_eq!(s.search_rows().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_key_is_refused_not_replaced() {
        let d = temp("c");
        let db = d.join("history.db");
        {
            let mut s = ks::open_store::<LinuxOs>(&db).unwrap();
            s.add_text("hello", 1_700_000_000).unwrap();
        }
        std::fs::remove_file(ks::key_path(&db)).unwrap();
        let e = ks::open_store::<LinuxOs>(&db).err().expect("must refuse");
        assert!(e.contains("Refusing to create a new key"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn locks_are_shared_for_the_app_and_exclusive_for_rotation() {
        let d = temp("d");
        let db = d.join("history.db");
        let a = ks::lock_history::<LinuxOs>(&db, false).unwrap();
        let b = ks::lock_history::<LinuxOs>(&db, false).unwrap();
        assert!(ks::lock_history::<LinuxOs>(&db, true).is_err());
        drop((a, b));
        assert!(ks::lock_history::<LinuxOs>(&db, true).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_symlinked_folder_is_refused() {
        let d = temp("e");
        std::fs::create_dir_all(&d).unwrap();
        let real = d.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = d.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(LinuxOs::private_dir(&link).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_second_instance_is_refused() {
        let d = temp("f");
        let db = d.join("history.db");
        let first = single_instance(&db).unwrap();
        assert!(single_instance(&db).is_err());
        drop(first);
        assert!(single_instance(&db).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
