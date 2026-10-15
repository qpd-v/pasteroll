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
    let base = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => {
            let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).ok_or("HOME is not set")?;
            PathBuf::from(home).join(".local").join("share")
        }
    };
    Ok(base.join("pasteroll").join("history.db"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use pasteroll_core::keystore as ks;

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
