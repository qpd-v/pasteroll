//! Where the history lives and how its key is kept: everything that is the same on every
//! platform. A platform supplies the parts that are not through [`Os`]: protecting the key file
//! (DPAPI on Windows, a user-only file on Linux), locking the data folder to the user, atomic
//! replace, and the cross-process lock.
//!
//! Layout of a data folder:
//!   history.db        encrypted clip bodies (SQLite, WAL)
//!   history.key       the 256-bit master key ring, protected by the platform
//!   history.lock      cross-process lock (shared by the app, exclusive for key replacement)
//!   config.toml       the user's settings
//!   paste_keys.txt    optional per-app paste shortcuts
//!
//! The key file is never created while a database already exists next to it: a new key would
//! silently orphan the old history, so the app refuses and says why.

use crate::keyring::KeyRing;
use crate::recovery;
use crate::vault::{generate_master, Vault, MASTER_LEN};
use crate::{KeyStatus, RotateReport, RotationState, Store};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

/// How often the master key is replaced.
pub const ROTATE_EVERY_DAYS: i64 = 30;

#[derive(Debug)]
pub enum LockError {
    /// Another process holds a conflicting lock.
    Busy,
    Io(String),
}

/// What a platform provides.
pub trait Os {
    /// Held for as long as the lock is wanted.
    type Lock;
    /// "Windows account", "user account": used in messages.
    const ACCOUNT: &'static str;

    /// Seals the key ring for the current user.
    fn protect(plain: &[u8]) -> Result<Vec<u8>, String>;
    /// Opens what [`Os::protect`] sealed. The error is shown to the user as is.
    fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String>;
    /// Makes sure `dir` exists, is a real folder owned by the current user, and is private to them.
    fn private_dir(dir: &Path) -> Result<(), String>;
    /// A symbolic link, junction or similar: never followed for history files.
    fn is_link(path: &Path) -> bool;
    /// Makes a freshly written file private to the current user.
    fn secure_file(path: &Path) -> Result<(), String>;
    /// Atomically replaces `to` with `from` (durable).
    fn replace_file(from: &Path, to: &Path) -> Result<(), String>;
    fn lock_file(path: &Path, exclusive: bool) -> Result<Self::Lock, LockError>;
    /// Bytes free on the volume holding `dir`.
    fn free_space(dir: &Path) -> Option<u64>;
}

pub fn key_path(db: &Path) -> PathBuf {
    db.with_extension("key")
}

pub fn lock_path(db: &Path) -> PathBuf {
    db.with_file_name("history.lock")
}

pub fn config_path(db: &Path) -> PathBuf {
    db.with_file_name("config.toml")
}

fn private_parent<O: Os>(db: &Path) -> Result<(), String> {
    if let Some(dir) = db.parent() {
        if !dir.as_os_str().is_empty() {
            O::private_dir(dir)?;
        }
    }
    Ok(())
}

/// Reads the key ring, or creates one for a brand-new history. A history that already exists is
/// never given a new key: that would silently orphan it.
pub fn load_or_create_ring<O: Os>(db: &Path) -> Result<KeyRing, String> {
    private_parent::<O>(db)?;
    let kp = key_path(db);
    for f in [db, kp.as_path()] {
        if O::is_link(f) {
            return Err(format!("{} is a link, not a file; refusing to use it", f.display()));
        }
    }
    if kp.exists() {
        let sealed = std::fs::read(&kp).map_err(|e| format!("cannot read {}: {e}", kp.display()))?;
        let mut plain = O::unprotect(&sealed)?;
        let ring = KeyRing::decode(&plain).ok_or_else(|| "key file has the wrong format".to_string());
        plain.zeroize();
        return ring;
    }
    if db.exists() && std::fs::metadata(db).map(|m| m.len() > 0).unwrap_or(false) {
        return Err(format!(
            "{} exists but {} is missing. Refusing to create a new key, which would make the \
             existing history unreadable. Restore the key file, move the database aside, or run \
             `reset-history --yes` to start over.",
            db.display(),
            kp.display()
        ));
    }
    let mut master = generate_master().map_err(|e| format!("no random source: {e}"))?;
    let ring = KeyRing::new(master);
    master.zeroize();
    write_ring::<O>(db, &ring)?;
    Ok(ring)
}

/// Protects and writes the ring beside the history, atomically: a temp file in the same folder,
/// then a rename, so a crash can never leave a partial key file behind.
pub fn write_ring<O: Os>(db: &Path, ring: &KeyRing) -> Result<(), String> {
    let kp = key_path(db);
    let mut plain = ring.encode();
    let sealed = O::protect(&plain);
    plain.zeroize();
    let sealed = sealed?;
    let tmp = kp.with_extension("key.tmp");
    let _ = std::fs::remove_file(&tmp);
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
        f.write_all(&sealed).and_then(|_| f.sync_all()).map_err(|e| format!("cannot write key: {e}"))?;
    }
    O::secure_file(&tmp)?;
    O::replace_file(&tmp, &kp)
}

/// Opens (creating if this is a brand-new history) the vault for `db`.
pub fn vault_for<O: Os>(db: &Path) -> Result<Vault, String> {
    let ring = load_or_create_ring::<O>(db)?;
    Ok(Vault::new(ring.current()))
}

/// Opens the store with whichever key in the ring matches the data, and reports which one. During
/// a rotation there are two; a crash at any point leaves exactly one of them matching.
pub fn open_matching<O: Os>(db: &Path) -> Result<(Store, KeyRing, usize), String> {
    let ring = load_or_create_ring::<O>(db)?;
    let mut found: Option<(Store, usize)> = None;
    for (i, key) in ring.iter().enumerate() {
        let mut store = Store::open(db, Vault::new(key)).map_err(|e| format!("cannot open {}: {e}", db.display()))?;
        match store.verify_key().map_err(|e| e.to_string())? {
            KeyStatus::Ok | KeyStatus::Initialized => {
                found = Some((store, i));
                break;
            }
            KeyStatus::Wrong => continue,
        }
    }
    if let Some((store, i)) = found {
        return Ok((store, ring, i));
    }
    Err(format!(
        "{} cannot be opened with any key in {}. It was written by another {} or with a different \
         key file.",
        db.display(),
        key_path(db).display(),
        O::ACCOUNT
    ))
}

/// Opens the store at `db` with its key.
pub fn open_store<O: Os>(db: &Path) -> Result<Store, String> {
    let (store, ring, used) = open_matching::<O>(db)?;
    if ring.len() > 1 {
        if used == 0 {
            // A rotation finished but the old key was not dropped yet. Failing to tidy up (an
            // antivirus holding the file, say) must not stop the app: the data opens fine.
            let _ = write_ring::<O>(db, &ring.without_previous());
        } else {
            // A rotation was interrupted before it committed: the data is still under the old key.
            // Keep working with it and try again at the next start.
            store.request_rotation().map_err(|e| e.to_string())?;
        }
    }
    Ok(store)
}

/// Takes the history's cross-process lock. `exclusive` is for anything that replaces the key.
pub fn lock_history<O: Os>(db: &Path, exclusive: bool) -> Result<O::Lock, String> {
    private_parent::<O>(db)?;
    O::lock_file(&lock_path(db), exclusive).map_err(|e| match e {
        LockError::Busy if exclusive => {
            "another Pasteroll process is using the history (or the app is running); the key cannot be replaced right now"
                .to_string()
        }
        LockError::Busy => "the history is being re-encrypted by another Pasteroll process; try again in a moment".to_string(),
        LockError::Io(m) => m,
    })
}

/// Replaces the master key and re-seals every clip under it. Crash-safe: the new key is written
/// beside the old one *before* anything changes, and the old one is dropped only after the database
/// has committed and been fully written out. Needs the exclusive lock: the running app holds the
/// old key in memory, and two rotators would overwrite each other's key file.
pub fn rotate_history<O: Os>(db: &Path, now: i64) -> Result<RotateReport, String> {
    let _lock = lock_history::<O>(db, true)?;
    rotate_locked::<O>(db, now)
}

fn rotate_locked<O: Os>(db: &Path, now: i64) -> Result<RotateReport, String> {
    let (mut store, ring, used) = open_matching::<O>(db)?;
    let mut old: [u8; MASTER_LEN] = *ring.iter().nth(used).expect("matched key exists");
    let mut new = generate_master().map_err(|e| format!("no random source: {e}"))?;
    let result = (|| {
        write_ring::<O>(db, &KeyRing::rotating(new, old))?;
        // If recovery is set up, the new key is sealed to the recovery public key in the same
        // transaction, so an old export keeps working.
        let blob = match store.recovery_public().map_err(|e| e.to_string())? {
            Some(public) => Some(recovery::seal_to(&public, &new).map_err(|e| e.to_string())?),
            None => None,
        };
        let report = store
            .rotate_with_recovery(Vault::new(&new), now, blob)
            .map_err(|e| e.to_string())?;
        if report.durable {
            write_ring::<O>(db, &KeyRing::new(new))?;
        }
        // not durable: the ring keeps [new, old]; open_matching copes and the next start tidies up
        Ok(report)
    })();
    new.zeroize();
    old.zeroize();
    result
}

/// Rotates if the key is older than [`ROTATE_EVERY_DAYS`] or a rotation was requested. Returns the
/// report when it did. Call before any other connection to the history exists. It never blocks
/// start-up: if the lock is busy, or the disk is too full, it says so and tries again next start.
pub fn rotate_if_due<O: Os>(db: &Path, now: i64) -> Result<Option<RotateReport>, String> {
    let _lock = match lock_history::<O>(db, true) {
        Ok(l) => l,
        Err(e) => return Err(format!("not rotating now: {e}")),
    };
    let state = {
        let (mut store, _, _) = open_matching::<O>(db)?;
        store.rotation_state(now, ROTATE_EVERY_DAYS).map_err(|e| e.to_string())?
    };
    if state == RotationState::NotDue {
        return Ok(None);
    }
    // A rotation briefly needs about the history's size again (the write-ahead log, then the copy
    // VACUUM makes). Do not start one that would fill the disk.
    let size = std::fs::metadata(db).map(|m| m.len()).unwrap_or(0);
    if let Some(free) = db.parent().and_then(O::free_space) {
        if free < size.saturating_mul(3) + (64 << 20) {
            return Err(format!(
                "not rotating now: only {} MB free, about {} MB needed",
                free >> 20,
                (size.saturating_mul(3) + (64 << 20)) >> 20
            ));
        }
    }
    rotate_locked::<O>(db, now).map(Some)
}

/// When the key was last replaced (day number) and whether a rotation is pending, for `status`.
pub fn rotation_summary<O: Os>(db: &Path, now: i64) -> Result<RotationState, String> {
    let (mut store, _, _) = open_matching::<O>(db)?;
    store.rotation_state(now, ROTATE_EVERY_DAYS).map_err(|e| e.to_string())
}

/// Loads the user's config. A missing file means defaults. An invalid file is an error carrying
/// every problem: starting with limits you did not ask for would be worse than not starting.
pub fn load_config(db: &Path) -> Result<crate::config::Config, Vec<String>> {
    match std::fs::read_to_string(config_path(db)) {
        Ok(text) => crate::config::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(crate::config::Config::default()),
        Err(e) => Err(vec![format!("cannot read {}: {e}", config_path(db).display())]),
    }
}

/// The config file's text. A missing file is empty text; a file that exists but cannot be read
/// (locked by another program, not UTF-8) is an error, so nothing is ever written over it.
pub fn read_config_text(db: &Path) -> Result<String, String> {
    match std::fs::read_to_string(config_path(db)) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!(
            "cannot read {}: {e}. Nothing was changed.",
            config_path(db).display()
        )),
    }
}

/// Replaces the config file atomically (temp file + rename, inside the private data folder).
pub fn write_config_text<O: Os>(db: &Path, text: &str) -> Result<(), String> {
    let path = config_path(db);
    if let Some(dir) = path.parent() {
        O::private_dir(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.write_all(text.as_bytes()).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.sync_all().map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    }
    O::replace_file(&tmp, &path)
}

/// What `key-export` writes (before the passphrase wrapping): the key that actually opens the
/// history right now, and the private half of a recovery key pair that keeps working after the key
/// rotates. Making an export creates a NEW recovery pair, so it replaces any earlier recovery.
pub fn export_contents<O: Os>(db: &Path) -> Result<crate::export::ExportContents, String> {
    let _lock = lock_history::<O>(db, false)?;
    let (store, ring, used) = open_matching::<O>(db)?;
    let key: [u8; MASTER_LEN] = *ring.iter().nth(used).expect("matched key exists");
    let (secret, public) = recovery::generate().map_err(|e| e.to_string())?;
    let blob = recovery::seal_to(&public, &key).map_err(|e| e.to_string())?;
    store.set_recovery(&public, &blob).map_err(|e| e.to_string())?;
    Ok(crate::export::ExportContents { ring: KeyRing::new(key).encode(), recovery_secret: Some(secret) })
}

/// Is a recovery key pair set up for this history (has an export ever been made)?
pub fn recovery_is_set_up<O: Os>(db: &Path) -> bool {
    open_matching::<O>(db).ok().and_then(|(s, _, _)| s.recovery_public().ok().flatten()).is_some()
}

/// Installs a key from an export. The key must actually open the history (when one exists):
/// importing the wrong key must never replace a working one. Tries the exported key first, then, if
/// the history has moved on (the key rotated since the export), the recovery blob inside the
/// history, opened with the exported private half. Without `replace` an existing key file stays.
pub fn import_contents<O: Os>(
    db: &Path,
    contents: &crate::export::ExportContents,
    replace: bool,
) -> Result<(), String> {
    let _lock = lock_history::<O>(db, true)?;
    let ring = KeyRing::decode(&contents.ring).ok_or_else(|| "the export does not contain a valid key".to_string())?;
    let kp = key_path(db);
    if kp.exists() && !replace {
        return Err(format!(
            "{} already exists. Importing would replace it; pass --force if that is what you want.",
            kp.display()
        ));
    }
    private_parent::<O>(db)?;
    let has_history = db.exists() && std::fs::metadata(db).map(|m| m.len() > 0).unwrap_or(false);
    let mut install = KeyRing::decode(&contents.ring).expect("decoded above");
    if has_history {
        let mut store = Store::open(db, Vault::new(ring.current())).map_err(|e| e.to_string())?;
        if store.verify_key().map_err(|e| e.to_string())? == KeyStatus::Wrong {
            // the key has rotated since this export was made: follow the recovery blob
            let recovered = contents
                .recovery_secret
                .as_ref()
                .zip(store.recovery_blob().map_err(|e| e.to_string())?)
                .and_then(|(secret, blob)| recovery::open_with(secret, &blob))
                .and_then(|mut k| {
                    let arr: Option<[u8; MASTER_LEN]> = k.as_slice().try_into().ok();
                    k.zeroize();
                    arr
                });
            let Some(mut key) = recovered else {
                return Err("this export does not open that history; nothing was changed".to_string());
            };
            let mut check = Store::open(db, Vault::new(&key)).map_err(|e| e.to_string())?;
            if check.verify_key().map_err(|e| e.to_string())? == KeyStatus::Wrong {
                key.zeroize();
                return Err("this export does not open that history; nothing was changed".to_string());
            }
            install = KeyRing::new(key);
            key.zeroize();
        }
    }
    write_ring::<O>(db, &install.without_previous())
}

/// Deletes a history and its key (the explicit way out of "key missing" or "wrong account").
pub fn reset_history<O: Os>(db: &Path) -> Result<Vec<PathBuf>, String> {
    let _lock = lock_history::<O>(db, true)?;
    let mut removed = Vec::new();
    let candidates = [
        db.to_path_buf(),
        db.with_extension("db-wal"),
        db.with_extension("db-shm"),
        key_path(db),
        key_path(db).with_extension("key.tmp"),
    ];
    for p in candidates {
        if O::is_link(&p) {
            return Err(format!("{} is a link; not touching it", p.display()));
        }
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("cannot delete {}: {e}", p.display()))?;
            removed.push(p);
        }
    }
    Ok(removed)
}
