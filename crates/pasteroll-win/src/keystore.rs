//! Where the history lives and how its key is kept, on Windows.
//!
//! Layout (default `%LOCALAPPDATA%\Pasteroll\`, located through the shell, not the environment):
//!   history.db        encrypted clip bodies (SQLite, WAL)
//!   history.key       the 256-bit master key, protected by DPAPI for the signed-in user
//!   paste_keys.txt    optional per-app paste shortcuts
//!
//! The data folder is created locked to the current user, and is refused if it is a junction or
//! symlink or is owned by someone else (an attacker could otherwise pre-create it). The logic that
//! is the same on every platform (rotation, recovery, locking order) lives in
//! `pasteroll_core::keystore`; this file supplies the Windows parts.

use crate::winsys::{self, FileLock};
use pasteroll_core::keystore::{self as core_ks, LockError, Os};
#[cfg(test)]
use pasteroll_core::{keyring::KeyRing, vault::{generate_master, Vault}};
use pasteroll_core::{RotateReport, RotationState, Store};
use std::path::{Path, PathBuf};
use windows::core::w;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::*;

pub use pasteroll_core::keystore::{config_path, load_config, read_config_text, ROTATE_EVERY_DAYS};

const ENTROPY: &[u8] = b"carousel/v1/master-key";

pub fn default_db() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(db) = pasteroll_core::paths::portable_db(&exe) {
        return Ok(db);
    }
    Ok(default_db_in(&crate::winsys::local_app_data()?))
}

/// The history file under `base`. A history made while the app was still called Carousel stays
/// where it is: the old folder is used when it holds a history and the new one does not.
pub fn default_db_in(base: &Path) -> PathBuf {
    let new = base.join("Pasteroll").join("history.db");
    let old = base.join("Carousel").join("history.db");
    if !new.exists() && old.exists() {
        old
    } else {
        new
    }
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

fn take_blob(b: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let v = std::slice::from_raw_parts(b.pbData, b.cbData as usize).to_vec();
        // The system allocated this with the plaintext key in it: wipe before freeing.
        std::ptr::write_bytes(b.pbData, 0, b.cbData as usize);
        let _ = LocalFree(Some(HLOCAL(b.pbData as *mut _)));
        v
    }
}

fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            w!("Carousel master key"),
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .map_err(|e| format!("DPAPI protect failed: {e}"))?;
        Ok(take_blob(out))
    }
}

fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = blob(sealed);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            .map_err(|e| {
                format!(
                    "the history key could not be unlocked by this Windows account ({e}). If the \
                     history came from another PC, or an administrator reset your password, it \
                     cannot be recovered. History files must be moved together: history.db, \
                     history.db-wal, history.db-shm and history.key."
                )
            })?;
        Ok(take_blob(out))
    }
}

/// Makes sure `dir` exists, is a real folder we own, and is locked to the current user.
fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    if crate::winsys::is_reparse_point(dir) {
        return Err(format!("{} is a junction or symbolic link; refusing to use it", dir.display()));
    }
    if !crate::winsys::owned_by_current_user(dir)? {
        return Err(format!(
            "{} is not owned by the current user; refusing to store history there",
            dir.display()
        ));
    }
    // Idempotent: also tightens a folder that existed before this version created it.
    crate::winsys::restrict_to_current_user(dir, true)
        .map_err(|e| format!("could not lock {} to your account: {e}", dir.display()))?;
    // Defence in depth against confined (low-integrity) programs of the same user. Best effort:
    // the account-level lock above is the real boundary.
    if let Err(e) = crate::winsys::label_no_low_access(dir, true) {
        eprintln!("[pasteroll] note: {e}");
    }
    Ok(())
}

/// The Windows parts of the key store.
pub struct WinOs;

impl Os for WinOs {
    type Lock = FileLock;
    const ACCOUNT: &'static str = "Windows account";

    fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        protect(plain)
    }
    fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        unprotect(sealed)
    }
    fn private_dir(dir: &Path) -> Result<(), String> {
        private_dir(dir)
    }
    fn is_link(path: &Path) -> bool {
        winsys::is_reparse_point(path)
    }
    fn secure_file(path: &Path) -> Result<(), String> {
        winsys::restrict_to_current_user(path, false)?;
        let _ = winsys::label_no_low_access(path, false);
        Ok(())
    }
    fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
        winsys::replace_file(from, to)
    }
    fn lock_file(path: &Path, exclusive: bool) -> Result<FileLock, LockError> {
        winsys::lock_file(path, exclusive).map_err(|e| match e {
            winsys::LockError::Busy => LockError::Busy,
            winsys::LockError::Io(m) => LockError::Io(m),
        })
    }
    fn free_space(dir: &Path) -> Option<u64> {
        winsys::free_space(dir)
    }
}

#[cfg(test)]
fn load_or_create_ring(db: &Path) -> Result<pasteroll_core::keyring::KeyRing, String> {
    core_ks::load_or_create_ring::<WinOs>(db)
}

#[cfg(test)]
fn write_ring(db: &Path, ring: &pasteroll_core::keyring::KeyRing) -> Result<(), String> {
    core_ks::write_ring::<WinOs>(db, ring)
}

/// Opens (creating if this is a brand-new history) the vault for `db`.
#[cfg(test)]
pub fn vault_for(db: &Path) -> Result<Vault, String> {
    core_ks::vault_for::<WinOs>(db)
}

/// Opens the store at `db` with its key.
pub fn open_store(db: &Path) -> Result<Store, String> {
    core_ks::open_store::<WinOs>(db)
}

/// Takes the history's cross-process lock. `exclusive` is for anything that replaces the key.
pub fn lock_history(db: &Path, exclusive: bool) -> Result<FileLock, String> {
    core_ks::lock_history::<WinOs>(db, exclusive)
}

pub fn rotate_history(db: &Path, now: i64) -> Result<RotateReport, String> {
    core_ks::rotate_history::<WinOs>(db, now)
}

pub fn rotate_if_due(db: &Path, now: i64) -> Result<Option<RotateReport>, String> {
    core_ks::rotate_if_due::<WinOs>(db, now)
}

pub fn rotation_summary(db: &Path, now: i64) -> Result<RotationState, String> {
    core_ks::rotation_summary::<WinOs>(db, now)
}

pub fn write_config_text(db: &Path, text: &str) -> Result<(), String> {
    core_ks::write_config_text::<WinOs>(db, text)
}

static CONFIG_EDIT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Reads config.toml, lets `edit` change the text, and writes it back, all under one lock so two
/// edits made at the same moment (the tray menu and the update question, say) cannot overwrite each
/// other. Nothing is written when the text did not change.
pub fn edit_config(db: &Path, edit: impl FnOnce(&str) -> Result<String, String>) -> Result<(), String> {
    let _only_one = CONFIG_EDIT.lock().unwrap_or_else(|e| e.into_inner());
    let text = read_config_text(db)?;
    let new = edit(&text)?;
    if new == text {
        return Ok(());
    }
    write_config_text(db, &new)
}

pub fn export_contents(db: &Path) -> Result<pasteroll_core::export::ExportContents, String> {
    core_ks::export_contents::<WinOs>(db)
}

pub fn recovery_is_set_up(db: &Path) -> bool {
    core_ks::recovery_is_set_up::<WinOs>(db)
}

pub fn import_contents(
    db: &Path,
    contents: &pasteroll_core::export::ExportContents,
    replace: bool,
) -> Result<(), String> {
    core_ks::import_contents::<WinOs>(db, contents, replace)
}

pub fn reset_history(db: &Path) -> Result<Vec<PathBuf>, String> {
    core_ks::reset_history::<WinOs>(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pasteroll-ks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn creates_a_key_then_reopens_with_the_same_one() {
        let d = temp("a");
        let db = d.join("h.db");
        let v1 = vault_for(&db).unwrap();
        let sealed = v1.seal(b"x", b"secret");
        assert!(db.with_extension("key").exists());
        let v2 = vault_for(&db).unwrap();
        assert_eq!(v2.open(b"x", &sealed).as_deref(), Some(&b"secret"[..]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_key_file_does_not_contain_the_key() {
        // DPAPI output is not the 32 raw bytes: it is a much longer blob.
        let d = temp("b");
        let db = d.join("h.db");
        vault_for(&db).unwrap();
        let blob = std::fs::read(db.with_extension("key")).unwrap();
        assert!(blob.len() > 100, "unexpectedly small key file: {}", blob.len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_temp_key_file_is_left_behind() {
        let d = temp("t");
        let db = d.join("h.db");
        vault_for(&db).unwrap();
        let names: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_to_invent_a_new_key_for_an_existing_history() {
        let d = temp("c");
        std::fs::create_dir_all(&d).unwrap();
        let db = d.join("h.db");
        std::fs::write(&db, b"not empty").unwrap();
        let e = vault_for(&db).err().expect("must refuse");
        assert!(e.contains("Refusing to create a new key"), "{e}");
        assert!(!db.with_extension("key").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_corrupt_key_file_is_an_error_not_a_new_key() {
        let d = temp("d");
        let db = d.join("h.db");
        vault_for(&db).unwrap();
        std::fs::write(db.with_extension("key"), b"garbage").unwrap();
        assert!(vault_for(&db).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn open_store_roundtrips_and_encrypts_on_disk() {
        let d = temp("e");
        let db = d.join("h.db");
        let id = {
            let mut s = open_store(&db).unwrap();
            s.add_text("needle-in-a-haystack-1234", 1).unwrap()
        };
        let s = open_store(&db).unwrap();
        assert_eq!(s.text(id).unwrap().as_deref(), Some("needle-in-a-haystack-1234"));
        drop(s);
        // The plaintext is in neither the database nor its write-ahead log.
        for f in std::fs::read_dir(&d).unwrap() {
            let bytes = std::fs::read(f.unwrap().path()).unwrap();
            assert!(!bytes.windows(9).any(|w| w == b"haystack-"));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn reset_history_removes_everything_and_allows_a_fresh_start() {
        let d = temp("r");
        let db = d.join("h.db");
        {
            let mut s = open_store(&db).unwrap();
            s.add_text("old", 1).unwrap();
        }
        let removed = reset_history(&db).unwrap();
        assert!(removed.len() >= 2, "{removed:?}");
        assert!(!db.exists() && !db.with_extension("key").exists());
        let mut s = open_store(&db).unwrap();
        assert_eq!(s.count().unwrap(), 0);
        s.add_text("new", 2).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---------------------------------------------------------------- rotation

    fn seeded(tag: &str) -> (PathBuf, PathBuf, i64) {
        let d = temp(tag);
        let db = d.join("h.db");
        let id = {
            let mut s = open_store(&db).unwrap();
            s.add_text("rotate me please", 1_700_000_000).unwrap()
        };
        (d, db, id)
    }

    #[test]
    fn rotation_replaces_the_key_and_keeps_every_clip() {
        let (d, db, id) = seeded("rot1");
        let before = std::fs::read(db.with_extension("key")).unwrap();
        let report = rotate_history(&db, 1_700_000_100).unwrap();
        assert_eq!(report.rows, 1);
        let after = std::fs::read(db.with_extension("key")).unwrap();
        assert_ne!(before, after, "the key file must change");
        let s = open_store(&db).unwrap();
        assert_eq!(s.text(id).unwrap().as_deref(), Some("rotate me please"));
        // the ring holds one key again
        let ring = load_or_create_ring(&db).unwrap();
        assert_eq!(ring.len(), 1);
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_previous_key_no_longer_opens_the_history() {
        let (d, db, id) = seeded("rot2");
        let old_key = *load_or_create_ring(&db).unwrap().current();
        rotate_history(&db, 1_700_000_100).unwrap();
        let s = Store::open(&db, Vault::new(&old_key)).unwrap();
        assert_eq!(s.text(id).unwrap(), None);
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_crash_after_the_new_key_is_written_but_before_the_commit_loses_nothing() {
        let (d, db, id) = seeded("rot3");
        let old = *load_or_create_ring(&db).unwrap().current();
        // simulate: step 1 done (ring has new + old), process died before the transaction
        let new = generate_master().unwrap();
        write_ring(&db, &KeyRing::rotating(new, old)).unwrap();
        let s = open_store(&db).unwrap();
        assert_eq!(s.text(id).unwrap().as_deref(), Some("rotate me please"), "still readable under the old key");
        drop(s);
        // and a later rotation completes cleanly
        rotate_history(&db, 1_700_000_200).unwrap();
        let s = open_store(&db).unwrap();
        assert_eq!(s.text(id).unwrap().as_deref(), Some("rotate me please"));
        assert_eq!(load_or_create_ring(&db).unwrap().len(), 1);
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_crash_after_the_commit_but_before_the_old_key_is_dropped_loses_nothing() {
        let (d, db, id) = seeded("rot4");
        let old = *load_or_create_ring(&db).unwrap().current();
        let new = generate_master().unwrap();
        write_ring(&db, &KeyRing::rotating(new, old)).unwrap();
        {
            // the database transaction happened...
            let mut st = Store::open(&db, Vault::new(&old)).unwrap();
            st.rotate(Vault::new(&new), 1_700_000_300).unwrap();
        }
        // ...and the process died before the ring was reduced to the new key
        assert_eq!(load_or_create_ring(&db).unwrap().len(), 2);
        let s = open_store(&db).unwrap();
        assert_eq!(s.text(id).unwrap().as_deref(), Some("rotate me please"));
        drop(s);
        assert_eq!(load_or_create_ring(&db).unwrap().len(), 1, "the stale previous key is dropped on open");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_history_that_matches_no_key_is_an_error_not_a_reset() {
        let (d, db, _) = seeded("rot5");
        write_ring(&db, &KeyRing::new(generate_master().unwrap())).unwrap();
        let e = open_store(&db).err().expect("must refuse");
        assert!(e.contains("cannot be opened with any key"), "{e}");
        assert!(db.exists(), "the history must not be touched");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rotation_happens_at_startup_only_when_due() {
        let (d, db, _) = seeded("rot6");
        let t = 1_700_000_000;
        assert!(rotate_if_due(&db, t).unwrap().is_none(), "first look starts the clock");
        assert!(rotate_if_due(&db, t + 10 * 86_400).unwrap().is_none());
        assert!(rotate_if_due(&db, t + 31 * 86_400).unwrap().is_some(), "older than 30 days");
        assert!(rotate_if_due(&db, t + 32 * 86_400).unwrap().is_none());
        // a request forces one
        {
            let s = open_store(&db).unwrap();
            s.request_rotation().unwrap();
        }
        assert!(rotate_if_due(&db, t + 33 * 86_400).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_first_version_bare_key_file_still_opens() {
        let d = temp("legacy");
        let db = d.join("h.db");
        std::fs::create_dir_all(&d).unwrap();
        let key = generate_master().unwrap();
        let sealed = protect(&key).unwrap();
        std::fs::write(db.with_extension("key"), sealed).unwrap();
        {
            let mut s = Store::open(&db, Vault::new(&key)).unwrap();
            s.add_text("old format", 1_700_000_000).unwrap();
        }
        let s = open_store(&db).unwrap();
        assert_eq!(s.count().unwrap(), 1);
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---------------------------------------------------------------- locking

    #[test]
    fn shared_locks_coexist_and_exclude_rotation() {
        let (d, db, _) = seeded("lock1");
        let a = lock_history(&db, false).unwrap();
        let b = lock_history(&db, false).unwrap();
        assert!(lock_history(&db, true).is_err(), "rotation must not start under a running app");
        let err = rotate_history(&db, 1_700_000_100).err().expect("must refuse");
        assert!(err.contains("another Pasteroll process"), "{err}");
        drop((a, b));
        assert!(rotate_history(&db, 1_700_000_100).is_ok(), "free again once the app is gone");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nothing_can_start_while_a_rotation_holds_the_exclusive_lock() {
        let (d, db, _) = seeded("lock2");
        let held = lock_history(&db, true).unwrap();
        let err = lock_history(&db, false).err().expect("must refuse");
        assert!(err.contains("re-encrypted"), "{err}");
        drop(held);
        assert!(lock_history(&db, false).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn start_up_rotation_gives_way_instead_of_failing_the_start() {
        let (d, db, _) = seeded("lock3");
        {
            let s = open_store(&db).unwrap();
            s.request_rotation().unwrap();
        }
        let _other = lock_history(&db, false).unwrap(); // some CLI command is running
        let r = rotate_if_due(&db, 1_700_000_100);
        assert!(r.is_err(), "reported, not panicked");
        assert!(open_store(&db).is_ok(), "the history still opens");
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---------------------------------------------------------------- recovery across rotation

    #[test]
    fn an_export_still_recovers_the_history_after_two_rotations() {
        let (d, db, id) = seeded("recov1");
        let contents = export_contents(&db).unwrap();
        // the key changes twice on its own, long after the export was made
        rotate_history(&db, 1_700_000_100).unwrap();
        rotate_history(&db, 1_700_100_000).unwrap();
        // the Windows account key is lost entirely
        std::fs::remove_file(db.with_extension("key")).unwrap();
        assert!(open_store(&db).is_err());
        import_contents(&db, &contents, false).unwrap();
        let s = open_store(&db).unwrap();
        assert_eq!(s.text(id).unwrap().as_deref(), Some("rotate me please"));
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_export_made_before_any_rotation_works_directly() {
        let (d, db, id) = seeded("recov2");
        let contents = export_contents(&db).unwrap();
        std::fs::remove_file(db.with_extension("key")).unwrap();
        import_contents(&db, &contents, false).unwrap();
        assert_eq!(open_store(&db).unwrap().text(id).unwrap().as_deref(), Some("rotate me please"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_second_export_replaces_the_first_one_recovery() {
        let (d, db, _) = seeded("recov3");
        let first = export_contents(&db).unwrap();
        let _second = export_contents(&db).unwrap();
        rotate_history(&db, 1_700_000_100).unwrap();
        std::fs::remove_file(db.with_extension("key")).unwrap();
        // the first export's key is stale and its recovery pair was replaced: it cannot open the history
        let e = import_contents(&db, &first, false).err().expect("old export must not work");
        assert!(e.contains("does not open that history"), "{e}");
        assert!(!db.with_extension("key").exists(), "nothing may be written on a failed import");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rotation_keeps_the_recovery_blob_current() {
        let (d, db, _) = seeded("recov4");
        export_contents(&db).unwrap();
        assert!(recovery_is_set_up(&db));
        let before = open_store(&db).unwrap().recovery_blob().unwrap().unwrap();
        rotate_history(&db, 1_700_000_100).unwrap();
        let after = open_store(&db).unwrap().recovery_blob().unwrap().unwrap();
        assert_ne!(before, after, "the new key must be sealed to the recovery key");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unreadable_config_is_never_treated_as_empty() {
        let (d, db, _) = seeded("cfg1");
        let _ = std::fs::remove_file(config_path(&db));
        assert_eq!(read_config_text(&db).unwrap(), "", "no file is just empty");
        std::fs::write(config_path(&db), [0xff, 0xfe, 0x00, 0x41]).unwrap();
        let e = read_config_text(&db).err().expect("must refuse");
        assert!(e.contains("Nothing was changed"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_default_location_is_under_the_shell_local_app_data() {
        let p = default_db().unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, "history.db");
        let folder = p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        assert!(folder == "Pasteroll" || folder == "Carousel", "{}", p.display());
    }

    #[test]
    fn an_old_carousel_history_is_found_until_a_new_one_exists() {
        let base = std::env::temp_dir().join(format!("pasteroll-ks-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("Carousel")).unwrap();
        assert!(default_db_in(&base).ends_with("Pasteroll\\history.db"), "nothing yet: the new name");
        std::fs::write(base.join("Carousel").join("history.db"), b"x").unwrap();
        assert!(default_db_in(&base).ends_with("Carousel\\history.db"), "the old history is used");
        std::fs::create_dir_all(base.join("Pasteroll")).unwrap();
        std::fs::write(base.join("Pasteroll").join("history.db"), b"y").unwrap();
        assert!(default_db_in(&base).ends_with("Pasteroll\\history.db"), "the new one wins");
        let _ = std::fs::remove_dir_all(&base);
    }
}
