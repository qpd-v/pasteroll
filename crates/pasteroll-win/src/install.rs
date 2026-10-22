//! Installing an update that the person has chosen to install: download it, prove it is genuine,
//! then hand it to Setup (installed copies) or put it in place (portable and self-installed copies).
//!
//! Nothing here runs unless the person clicked "Download and install" and confirmed. Nothing is run
//! unless the signed manifest verifies (see `pasteroll_core::update_manifest`) and the file that was
//! downloaded is exactly the file the manifest names, checked again through a handle that nobody else
//! can write through, so it cannot be swapped between the check and the start.

use crate::fetch::{self, Dest};
use crate::sigcheck::CngVerifier;
use pasteroll_core::update::Version;
use pasteroll_core::update_manifest::{self as um, Asset, Hasher};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// How this copy of Pasteroll can update itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Installed by Setup: download the installer and run it silently. `all_users` copies live in
    /// Program Files, so Windows asks for administrator permission.
    Setup { all_users: bool },
    /// A portable or self-installed copy: replace this program file in its folder.
    Replace { exe: PathBuf },
    /// Not managed by anything: only say that an update exists. The text says why.
    NotifyOnly(String),
}

/// The facts the mode is chosen from, so the choice can be tested without touching the machine.
pub struct Facts<'a> {
    pub exe: &'a Path,
    pub portable: bool,
    pub installed_by_setup: bool,
    pub self_installed: bool,
    pub program_files: Vec<PathBuf>,
    pub folder_writable: bool,
    /// The folder Setup recorded for an install made for all users (machine-wide), if any.
    pub machine_install: Option<PathBuf>,
}

pub(crate) fn under(path: &Path, base: &Path) -> bool {
    let (p, b) = (path.to_string_lossy().to_ascii_lowercase(), base.to_string_lossy().to_ascii_lowercase());
    let b = b.trim_end_matches('\\');
    !b.is_empty() && p.starts_with(b) && p[b.len()..].starts_with('\\')
}

pub fn choose_mode(f: &Facts) -> Mode {
    if f.portable {
        return if f.folder_writable {
            Mode::Replace { exe: f.exe.to_path_buf() }
        } else {
            Mode::NotifyOnly("this portable copy's folder is not writable".to_owned())
        };
    }
    if f.installed_by_setup {
        if f.program_files.iter().any(|p| under(f.exe, p)) {
            return Mode::Setup { all_users: true };
        }
        // installed for all users into some other folder: the administrator helper only works from
        // Program Files, and a per-user Setup would make a second copy instead of updating this one
        let here = f.exe.parent().map(|p| p.to_string_lossy().to_ascii_lowercase());
        let recorded = f.machine_install.as_ref().map(|p| p.to_string_lossy().trim_end_matches('\\').to_ascii_lowercase());
        if here.is_some() && here == recorded {
            return Mode::NotifyOnly("this copy is installed for all users outside Program Files".to_owned());
        }
        return Mode::Setup { all_users: false };
    }
    if f.self_installed {
        return if f.folder_writable {
            Mode::Replace { exe: f.exe.to_path_buf() }
        } else {
            Mode::NotifyOnly("this copy's folder is not writable".to_owned())
        };
    }
    Mode::NotifyOnly("this copy is running from where it was downloaded, not from an install or a portable folder".to_owned())
}

/// Whether files can be created in `dir` (a program in a protected folder cannot replace itself).
fn writable(dir: &Path) -> bool {
    // a name that cannot be guessed, created only if it does not exist (a planted link is not followed)
    let probe = dir.join(format!(".pasteroll-write-test-{}", random_token()));
    let ok = OpenOptions::new().write(true).create_new(true).open(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// The Program Files folders, from Windows (not from environment variables a parent could set).
pub(crate) fn program_files_dirs() -> Vec<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX64, FOLDERID_ProgramFilesX86, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let mut dirs = Vec::new();
    for id in [&FOLDERID_ProgramFiles, &FOLDERID_ProgramFilesX86, &FOLDERID_ProgramFilesX64] {
        unsafe {
            if let Ok(p) = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) {
                if let Ok(s) = p.to_string() {
                    dirs.push(PathBuf::from(s));
                }
                CoTaskMemFree(Some(p.0 as *const _));
            }
        }
    }
    dirs
}

/// The folder Setup recorded for an install made for all users (its entry under HKEY_LOCAL_MACHINE).
fn machine_install_dir() -> Option<PathBuf> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::*;
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
    let sub = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{E04386D3-6242-420F-864E-35305B8A20FC}_is1");
    let name = wide("InstallLocation");
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(sub.as_ptr()), None, KEY_READ | KEY_WOW64_64KEY, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let mut size = 0u32;
        let mut ty = REG_NONE;
        let mut out = None;
        if RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, Some(&mut ty), None, Some(&mut size)) == ERROR_SUCCESS && ty == REG_SZ && size > 0 {
            let mut buf = vec![0u8; size as usize];
            if RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut size)) == ERROR_SUCCESS {
                let units: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                out = Some(PathBuf::from(String::from_utf16_lossy(&units).trim_end_matches('\0')));
            }
        }
        let _ = RegCloseKey(key);
        out
    }
}

/// How this running copy can update.
pub fn mode() -> Mode {
    let Ok(exe) = std::env::current_exe() else {
        return Mode::NotifyOnly("the program's own location is unknown".to_owned());
    };
    // the long, real path (no 8.3 names, no link in the way), without the verbatim prefix
    let exe = std::fs::canonicalize(&exe)
        .ok()
        .and_then(|p| p.to_str().map(|s| s.strip_prefix(r"\\?\").unwrap_or(s).to_owned()))
        .map(PathBuf::from)
        .unwrap_or(exe);
    let program_files = program_files_dirs();
    let folder_writable = exe.parent().is_some_and(writable);
    choose_mode(&Facts {
        exe: &exe,
        portable: pasteroll_core::paths::is_portable(),
        installed_by_setup: crate::setup::installed_by_setup(),
        self_installed: crate::setup::running_installed(),
        program_files,
        folder_writable,
        machine_install: machine_install_dir(),
    })
}

/// Where updates are checked and from whom. The real one is fixed; the end-to-end test build can
/// point it at a local server with a key of its own.
pub struct Trust {
    pub keys: Vec<[u8; 64]>,
    /// The hosts a release file may come from (and be redirected to).
    pub hosts: Vec<String>,
    /// The address of a file of a given release.
    pub url: Box<dyn Fn(&Version, &str) -> String + Send + Sync>,
    /// The hosts the signed "newest version" file may come from.
    pub latest_hosts: Vec<String>,
    /// The address of the signed "newest version" file or of its signature.
    pub latest_url: Box<dyn Fn(&str) -> String + Send + Sync>,
}

impl Trust {
    pub fn github() -> Trust {
        #[cfg(feature = "e2e-test-server")]
        if let Some(t) = Trust::from_environment() {
            return t;
        }
        Trust {
            keys: um::configured_keys(),
            hosts: fetch::GITHUB_HOSTS.iter().map(|h| (*h).to_owned()).collect(),
            url: Box::new(|v, name| um::asset_url(v, name)),
            latest_hosts: um::LATEST_HOSTS.iter().map(|h| (*h).to_owned()).collect(),
            latest_url: Box::new(um::latest_url),
        }
    }

    /// For the end-to-end test build only (never compiled into a release):
    /// `PASTEROLL_UPDATE_BASE` is `http://127.0.0.1:PORT` and `PASTEROLL_UPDATE_TEST_KEY` the base64
    /// public key that signs the test manifest.
    #[cfg(feature = "e2e-test-server")]
    fn from_environment() -> Option<Trust> {
        let base = std::env::var("PASTEROLL_UPDATE_BASE").ok()?;
        let key = pasteroll_core::update_manifest::decode_signature(&std::env::var("PASTEROLL_UPDATE_TEST_KEY").ok()?)?;
        let latest_base = base.clone();
        Some(Trust {
            keys: vec![key],
            hosts: vec!["127.0.0.1".to_owned()],
            url: Box::new(move |v, name| format!("{base}/v{v}/{name}")),
            latest_hosts: vec!["127.0.0.1".to_owned()],
            latest_url: Box::new(move |name| format!("{latest_base}/{name}")),
        })
    }
}

/// Which file of the release is wanted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Want {
    Setup,
    Exe,
}

/// A downloaded file that has been checked and is held so that it cannot change.
pub struct Verified {
    pub path: PathBuf,
    /// Open for reading with no one else allowed to write to it or delete it.
    pub locked: File,
    pub asset: Asset,
}

impl Verified {
    /// Keeps the file locked until this program exits. Inno Setup (like most installers) opens its own
    /// file again after it has started, so the lock must outlive the start of the installer, and it
    /// does: this program is closed by Setup itself, much later.
    pub fn hold_until_exit(self) {
        std::mem::forget(self.locked);
    }
}

fn hash_file(f: &mut File) -> Result<(u64, [u8; 32]), String> {
    let mut hasher = Hasher::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("could not read the download ({e})"))?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((size, hasher.finish()))
}

/// The folder inside the data folder that downloads go in.
pub const UPDATE_DIR: &str = "pasteroll-update";
/// Present in a folder this code made; a folder without it is never touched.
pub(crate) const FOLDER_MARKER: &str = ".pasteroll-update-folder";
/// Written just before Setup is started, so the next start can say whether the update happened.
const PENDING: &str = "pending.txt";

fn random_token() -> String {
    // 64 random bits from Windows; `create_new` refuses a name that already exists
    crate::winsys::random_hex(8)
}

fn random_name() -> String {
    format!("update-{}.exe", random_token())
}

/// A fresh, private, empty update folder. An existing folder is removed only if it is one of ours
/// (it has the marker): `--db` can put the data folder anywhere, so a folder with this name may
/// belong to something else, and it is left alone.
fn fresh_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        if !dir.join(FOLDER_MARKER).is_file() {
            return Err(format!("{} exists and is not Pasteroll's update folder, so it was not touched", dir.display()));
        }
        std::fs::remove_dir_all(dir).map_err(|e| format!("could not clear the old update folder ({e})"))?;
    }
    crate::keystore::private_dir(dir)?;
    std::fs::write(dir.join(FOLDER_MARKER), b"Pasteroll's update folder. Safe to delete.\n")
        .map_err(|e| format!("could not prepare the update folder ({e})"))
}

/// Where a failed preparation stopped. The administrator helper cannot send words back to the
/// person's own process (a file it wrote into a folder the person controls would be a hazard), so it
/// reports this as its exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The release files could not be fetched.
    Download,
    /// What arrived is not a genuine, newer release (signature, version, size or checksum).
    Verify,
    /// The place to put the download could not be prepared.
    Folder,
    /// The verified installer could not be started.
    Start,
}

/// Fetches and checks the signed manifest for `announced`; returns it only if it is genuine, names
/// exactly that version, and is newer than `current`.
pub fn check_release(trust: &Trust, announced: &Version, current: &Version, say: &mut dyn FnMut(&str)) -> Result<um::Manifest, (Stage, String)> {
    let hosts: Vec<&str> = trust.hosts.iter().map(String::as_str).collect();
    say("Checking the update...");
    let small = |name: &str| fetch::download_small(&(trust.url)(announced, name), &hosts, um::MAX_SMALL_FILE_BYTES).map_err(|e| (Stage::Download, e));
    let (manifest_bytes, signature_bytes) = (small(um::MANIFEST_NAME)?, small(um::SIGNATURE_NAME)?);
    let manifest = um::verify_manifest(&manifest_bytes, &signature_bytes, &trust.keys, &CngVerifier).map_err(|e| (Stage::Verify, e))?;
    um::check_installable(&manifest, announced, current).map_err(|e| (Stage::Verify, e))?;
    Ok(manifest)
}

/// Downloads the manifest and the file for `announced`, checks the signature, the version and the
/// file, and returns the file locked. `make_dir` prepares the (empty) folder the download goes in and
/// is called only after the manifest has checked out. `say` is told what is happening; `progress` how
/// much has arrived.
pub fn prepare_in(
    trust: &Trust,
    announced: &Version,
    current: &Version,
    want: Want,
    make_dir: &dyn Fn() -> Result<PathBuf, String>,
    say: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Verified, (Stage, String)> {
    let hosts: Vec<&str> = trust.hosts.iter().map(String::as_str).collect();
    let manifest = check_release(trust, announced, current, say)?;
    let asset = match want {
        Want::Setup => manifest.setup,
        Want::Exe => manifest.exe,
    };

    // a fresh private folder: nothing left over from an earlier attempt
    let dir = make_dir().map_err(|e| (Stage::Folder, e))?;
    let path = dir.join(random_name());
    say("Downloading the update...");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0) // nobody else opens it while it is written
            .open(&path)
            .map_err(|e| (Stage::Folder, format!("could not create the download file ({e})")))?;
        let got = fetch::download(&(trust.url)(announced, &asset.name), &hosts, asset.size, Dest::File(&mut file), progress);
        let got = match got {
            Ok(g) => g,
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(&path);
                return Err((Stage::Download, e));
            }
        };
        if let Err(e) = um::check_download(&asset, got.size, &got.sha256) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err((Stage::Verify, e));
        }
    }
    // Hold the file so that no one can write to or delete it, and check it once more through that
    // very handle: whatever happened between the download and now, what is held is what was verified.
    say("Verifying the update...");
    let mut locked = OpenOptions::new()
        .read(true)
        .share_mode(1) // FILE_SHARE_READ only
        .open(&path)
        .map_err(|e| (Stage::Folder, format!("could not open the download ({e})")))?;
    let verified = hash_file(&mut locked).and_then(|(size, digest)| um::check_download(&asset, size, &digest));
    if let Err(e) = verified {
        drop(locked);
        let _ = std::fs::remove_file(&path);
        return Err((Stage::Verify, e));
    }
    Ok(Verified { path, locked, asset })
}

/// [`prepare_in`] for a download into `dir`, a private folder inside the person's data folder.
pub fn prepare(
    trust: &Trust,
    announced: &Version,
    current: &Version,
    want: Want,
    dir: &Path,
    say: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Verified, String> {
    prepare_in(trust, announced, current, want, &|| fresh_dir(dir).map(|()| dir.to_path_buf()), say, progress).map_err(|(_, e)| e)
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// The command-line switches of the silent update: no windows, no restart, keep the person's data,
/// and start the new version again. `/ALLUSERS` or `/CURRENTUSER` must match how this copy was installed.
pub fn setup_args(all_users: bool) -> Vec<String> {
    let mut args = vec!["/VERYSILENT".to_owned(), "/NORESTART".to_owned(), "/SUPPRESSMSGBOXES".to_owned()];
    if all_users {
        // an elevated Setup would start the program with administrator rights, so the administrator
        // helper starts it again as the person instead (see elevated.rs)
        args.push("/ALLUSERS".to_owned());
    } else {
        args.push("/RELAUNCH=1".to_owned());
        args.push("/CURRENTUSER".to_owned());
    }
    args
}

/// Starts the verified Setup silently for a copy installed for this account only. It asks the running
/// Pasteroll to quit, keeps your data, and (with `/RELAUNCH=1`) starts the new version. (Copies
/// installed for all users go through `elevated`, which never trusts a file this account can change.)
///
/// Setup is run through the system's `cmd.exe`, which waits for it: if Setup does not succeed after it
/// has closed this program, `program` (this very program, as it was) is started again, and says at its
/// start that the update did not finish. (Without that, a failed Setup would leave nothing running.)
pub fn run_setup(v: &Verified, log: &Path, program: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut args = setup_args(false);
    args.push(format!("/LOG={}", quoted(log)));
    let started = match (setup_line(&v.path, &args, program), crate::winsys::system_dir()) {
        (Some(line), Some(system)) => std::process::Command::new(system.join("cmd.exe"))
            // /d: no AutoRun; /e:on /v:off: the same command-processor behaviour whatever the account's settings say
            .raw_arg("/d /e:on /v:off /s /c")
            .raw_arg(line)
            .current_dir(&system)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x0800_0000 | 0x0000_0200) // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
            .spawn(),
        // a name the command interpreter could misread: Setup alone, without the restart on failure
        _ => {
            let mut command = std::process::Command::new(&v.path);
            for a in &args {
                // the log path is already quoted: pass it through exactly as written
                command.raw_arg(a);
            }
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x0000_0008 | 0x0000_0200) // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
                .spawn()
        }
    };
    started.map(|_| ()).map_err(|e| format!("Setup could not be started ({e})"))
}

/// The `cmd.exe` command line that runs Setup, waits for it and, if it did not succeed, starts
/// `program` again. `None` when a path has a character the command interpreter treats specially.
pub(crate) fn setup_line(setup: &Path, args: &[String], program: &Path) -> Option<String> {
    let log_path = args.iter().find_map(|a| a.strip_prefix("/LOG=\"")).and_then(|a| a.strip_suffix('"'));
    let plain_paths = [Some(setup), Some(program), log_path.map(Path::new)];
    if plain_paths.iter().flatten().any(|p| !crate::elevated::plain(p)) {
        return None;
    }
    // `start /wait` because cmd does not otherwise wait for a program that has windows. `if errorlevel 1`
    // is "the exit code is 1 or more", so a Setup that crashed or was killed (a negative code such as
    // 0xC0000005) needs `if not errorlevel 0` as well. Neither fires after a success, and the first
    // one's `start` leaves the code alone for the second only when it did not fire.
    Some(format!(
        "\"start \"\" /wait \"{}\" {} & (if errorlevel 1 (if exist \"{p}\" start \"\" \"{p}\")) & (if not errorlevel 0 (if exist \"{p}\" start \"\" \"{p}\"))\"",
        setup.display(),
        args.join(" "),
        p = program.display()
    ))
}

/// Replaces the program file `exe` with the verified one. Windows lets a running program be
/// renamed but not overwritten, so the old file is renamed to `<name>.old` (removed at the next
/// start), the new one copied into place and checked, and on any failure the old one is put back.
pub fn replace_program(v: &Verified, exe: &Path) -> Result<(), String> {
    let (old, new) = (old_name(exe), new_name(exe));
    let _ = std::fs::remove_file(&new);
    // 1. the new program beside the old one, complete, flushed and checked, before anything moves
    let staged = std::fs::copy(&v.path, &new)
        .map_err(|e| e.to_string())
        .and_then(|_| OpenOptions::new().write(true).open(&new).and_then(|f| f.sync_all()).map_err(|e| e.to_string()))
        .and_then(|()| File::open(&new).map_err(|e| e.to_string()))
        .and_then(|mut f| hash_file(&mut f))
        .and_then(|(size, digest)| um::check_download(&v.asset, size, &digest));
    if let Err(e) = staged {
        let _ = std::fs::remove_file(&new);
        return Err(format!("the new program could not be prepared ({e}); the old program was not touched"));
    }
    // 2. the old program aside (a running program can be renamed, not overwritten)
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("could not move the old program aside ({e}); nothing was changed"));
    }
    // 3. the new program into its place
    if let Err(e) = std::fs::rename(&new, exe) {
        return match std::fs::rename(&old, exe) {
            Ok(()) => {
                let _ = std::fs::remove_file(&new);
                Err(format!("could not put the new program in place ({e}); the old program was kept"))
            }
            Err(e2) => Err(format!(
                "could not put the new program in place ({e}), and the old one could not be put back ({e2}). Your program is saved as {} and the new one as {}: rename one of them to {}",
                old.display(),
                new.display(),
                exe.display()
            )),
        };
    }
    Ok(())
}

/// The name the new program has while it is being prepared.
pub fn new_name(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".new");
    PathBuf::from(name)
}

/// The name a program file is moved to while it is being replaced.
pub fn old_name(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Removes the leftover of a replacement, at start-up. Failure is fine: it is tried again next time.
pub fn remove_leftover() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_name(&exe));
        let _ = std::fs::remove_file(new_name(&exe));
    }
}

/// Remembers that Setup is being started for `target`, so the next start can say whether it worked.
fn mark_pending(dir: &Path, target: &Version) {
    let _ = std::fs::write(dir.join(PENDING), target.to_string());
}

/// What an update that was started earlier came to, found at start-up.
#[derive(Debug, PartialEq, Eq)]
pub enum Startup {
    /// It worked: this is the new version.
    Updated(Version),
    /// It was started but this is still the old version (Setup failed or was stopped). The text is for
    /// a person.
    DidNotFinish(String),
}

/// At start-up: if an update was started earlier, say whether it happened. `None` when there was
/// nothing pending. Reported once.
pub fn startup_report(data_dir: &Path) -> Option<Startup> {
    let dir = data_dir.join(UPDATE_DIR);
    let marker = dir.join(PENDING);
    let target = Version::parse(std::fs::read_to_string(&marker).ok()?.trim())?;
    let _ = std::fs::remove_file(&marker);
    let current = Version::current();
    if current >= target {
        // it worked: the downloaded installer is not needed any more
        if dir.join(FOLDER_MARKER).is_file() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        return Some(Startup::Updated(current));
    }
    Some(Startup::DidNotFinish(format!(
        "Pasteroll {target} could not be installed, so you are still on {current}. Nothing of yours was changed. You can try again from the tray menu, or install {target} by hand from its release page. (For a copy installed for one account, an update-setup.log in the data folder says what Setup did; a copy installed for all users keeps no log.)"
    )))
}

/// What happened, for the person.
#[derive(Debug, PartialEq, Eq)]
pub enum Done {
    /// Setup was started; it will close this program and start the new version.
    SetupStarted,
    /// The program file was replaced; this copy must restart into it.
    Replaced(PathBuf),
}

/// The whole update for `mode`, from download to hand-over. `data_dir` is the history folder (the
/// download goes in a private `update` folder inside it, and Setup's log next to it).
pub fn run_update(
    trust: &Trust,
    mode: &Mode,
    announced: &Version,
    current: &Version,
    data_dir: &Path,
    say: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Done, String> {
    let dir = data_dir.join(UPDATE_DIR);
    match mode {
        Mode::NotifyOnly(why) => Err(format!("this copy cannot update itself: {why}")),
        Mode::Setup { all_users: true } => {
            // Installed in Program Files: the download, the check and the start all happen in an
            // administrator-only folder, by the elevated helper. Nothing in a folder this account can
            // write to is ever trusted, so there is nothing to swap while Windows asks for permission.
            // The signature is still checked here first, so a bad release never causes a prompt.
            check_release(trust, announced, current, say).map_err(|(_, e)| e)?;
            fresh_dir(&dir)?;
            mark_pending(&dir, announced);
            say("Asking Windows for administrator permission...");
            match crate::elevated::run_helper(announced) {
                Ok(()) => Ok(Done::SetupStarted),
                Err(e) => {
                    let _ = std::fs::remove_file(dir.join(PENDING));
                    Err(e)
                }
            }
        }
        Mode::Setup { all_users: false } => {
            let v = prepare(trust, announced, current, Want::Setup, &dir, say, progress)?;
            mark_pending(&dir, announced);
            say("Starting the installer...");
            let program = std::env::current_exe().map_err(|e| format!("the program's own location is unknown ({e})"))?;
            match run_setup(&v, &data_dir.join("update-setup.log"), &program) {
                Ok(()) => {
                    // Setup re-reads its own file after it has started: keep it locked until we exit
                    v.hold_until_exit();
                    Ok(Done::SetupStarted)
                }
                Err(e) => {
                    drop(v.locked);
                    let _ = std::fs::remove_file(&v.path);
                    let _ = std::fs::remove_file(dir.join(PENDING));
                    Err(e)
                }
            }
        }
        Mode::Replace { exe } => {
            let v = prepare(trust, announced, current, Want::Exe, &dir, say, progress)?;
            say("Replacing the program...");
            // the restarted program says "updated" (and tidies the download folder) at its start
            mark_pending(&dir, announced);
            let result = replace_program(&v, exe);
            drop(v.locked);
            let _ = std::fs::remove_file(&v.path);
            if result.is_err() {
                let _ = std::fs::remove_file(dir.join(PENDING));
            }
            result.map(|()| Done::Replaced(exe.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sigcheck::TestSigner;
    use crate::testsupport::{b64, Server};

    fn facts<'a>(exe: &'a Path) -> Facts<'a> {
        Facts {
            exe,
            portable: false,
            installed_by_setup: false,
            self_installed: false,
            program_files: vec![PathBuf::from(r"C:\Program Files"), PathBuf::from(r"C:\Program Files (x86)")],
            folder_writable: true,
            machine_install: None,
        }
    }

    #[test]
    fn each_kind_of_copy_updates_the_way_it_can() {
        let per_user = Path::new(r"C:\Users\me\AppData\Local\Programs\Pasteroll\pasteroll.exe");
        let machine = Path::new(r"C:\Program Files\Pasteroll\pasteroll.exe");
        let lookalike = Path::new(r"C:\Program Files Extra\Pasteroll\pasteroll.exe");
        let portable = Path::new(r"D:\tools\pasteroll\pasteroll.exe");
        assert_eq!(choose_mode(&Facts { installed_by_setup: true, ..facts(per_user) }), Mode::Setup { all_users: false });
        assert_eq!(choose_mode(&Facts { installed_by_setup: true, ..facts(machine) }), Mode::Setup { all_users: true });
        assert_eq!(choose_mode(&Facts { installed_by_setup: true, ..facts(lookalike) }), Mode::Setup { all_users: false }, "a folder that only starts with the same letters");
        // installed for all users somewhere else: not updated through a per-user Setup
        let elsewhere = Path::new(r"D:\Apps\Pasteroll\pasteroll.exe");
        let recorded = Some(PathBuf::from(r"D:\Apps\Pasteroll\"));
        assert!(matches!(choose_mode(&Facts { installed_by_setup: true, machine_install: recorded.clone(), ..facts(elsewhere) }), Mode::NotifyOnly(w) if w.contains("all users")));
        assert_eq!(choose_mode(&Facts { installed_by_setup: true, machine_install: Some(PathBuf::from(r"E:\Other\")), ..facts(elsewhere) }), Mode::Setup { all_users: false }, "the all-users record belongs to another folder");
        assert_eq!(choose_mode(&Facts { portable: true, ..facts(portable) }), Mode::Replace { exe: portable.to_path_buf() });
        assert!(matches!(choose_mode(&Facts { portable: true, folder_writable: false, ..facts(portable) }), Mode::NotifyOnly(_)));
        assert_eq!(choose_mode(&Facts { self_installed: true, ..facts(per_user) }), Mode::Replace { exe: per_user.to_path_buf() });
        assert!(matches!(choose_mode(&facts(portable)), Mode::NotifyOnly(w) if w.contains("downloaded")));
        // a portable copy is never treated as a Setup install, whatever else is true
        assert_eq!(choose_mode(&Facts { portable: true, installed_by_setup: true, ..facts(machine) }), Mode::Replace { exe: machine.to_path_buf() });
    }

    // ---- a local release server with its own throwaway signing key

    struct Release {
        version: Version,
        setup: Vec<u8>,
        exe: Vec<u8>,
    }

    fn manifest_for(r: &Release) -> String {
        format!(
            "pasteroll-update-manifest 1\nversion {}\nsetup {} {} {}\nexe {} {} {}\n",
            r.version,
            um::setup_name(&r.version),
            r.setup.len(),
            um::hex(&um::sha256(&r.setup)),
            um::exe_name(&r.version),
            r.exe.len(),
            um::hex(&um::sha256(&r.exe))
        )
    }

    /// The files of a release, signed by `signer`, as the server will hand them out.
    fn files(r: &Release, signer: &TestSigner) -> Vec<(String, Vec<u8>)> {
        let m = manifest_for(r);
        let sig = b64(&signer.sign(m.as_bytes()));
        let dir = format!("/v{}/", r.version);
        vec![
            (format!("{dir}{}", um::MANIFEST_NAME), m.into_bytes()),
            (format!("{dir}{}", um::SIGNATURE_NAME), sig.into_bytes()),
            (format!("{dir}{}", um::setup_name(&r.version)), r.setup.clone()),
            (format!("{dir}{}", um::exe_name(&r.version)), r.exe.clone()),
        ]
    }

    fn trust(base: &str, key: [u8; 64]) -> Trust {
        let base = base.to_owned();
        let latest_base = base.clone();
        Trust {
            keys: vec![key],
            hosts: vec!["127.0.0.1".into()],
            url: Box::new(move |v, name| format!("{base}/v{v}/{name}")),
            latest_hosts: vec!["127.0.0.1".into()],
            latest_url: Box::new(move |name| format!("{latest_base}/{name}")),
        }
    }

    fn release() -> Release {
        Release {
            version: Version::parse("99.0.0-beta.1").unwrap(),
            setup: (0..300_000u32).map(|i| (i % 253) as u8).collect(),
            exe: (0..120_000u32).map(|i| (i % 241) as u8).collect(),
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pasteroll-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn prepare_with(server_files: Vec<(String, Vec<u8>)>, key: [u8; 64], r: &Release, announced: &str, want: Want, tag: &str) -> (Result<Verified, String>, PathBuf, Vec<String>) {
        let server = Server::start(server_files);
        let dir = scratch(tag);
        let t = trust(&server.base, key);
        let result = prepare(&t, &Version::parse(announced).unwrap(), &Version::current(), want, &dir.join("update"), &mut |_| {}, &mut |_, _| {});
        let _ = r;
        (result, dir, server.finish())
    }

    #[test]
    fn a_genuine_update_is_downloaded_verified_and_held() {
        let signer = TestSigner::new();
        let r = release();
        let (result, dir, asked) = prepare_with(files(&r, &signer), signer.public(), &r, "99.0.0-beta.1", Want::Setup, "ok");
        let v = result.unwrap();
        assert_eq!(std::fs::read(&v.path).unwrap(), r.setup);
        assert_eq!(v.asset.name, um::setup_name(&r.version));
        assert!(asked.iter().any(|p| p.ends_with("update-manifest.txt")) && asked.iter().any(|p| p.ends_with("setup.exe")));
        // held: nobody can write to it, replace it or delete it while it is held
        assert!(OpenOptions::new().write(true).open(&v.path).is_err(), "cannot be written");
        assert!(std::fs::remove_file(&v.path).is_err(), "cannot be deleted");
        assert!(std::fs::rename(&v.path, v.path.with_extension("moved")).is_err(), "cannot be renamed");
        assert!(File::open(&v.path).is_ok(), "but can be read, so it can be run");
        drop(v);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_portable_file_is_the_one_asked_for() {
        let signer = TestSigner::new();
        let r = release();
        let (result, dir, _) = prepare_with(files(&r, &signer), signer.public(), &r, "99.0.0-beta.1", Want::Exe, "exe");
        assert_eq!(std::fs::read(&result.unwrap().path).unwrap(), r.exe);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_unsigned_or_wrongly_signed_is_ever_used() {
        let signer = TestSigner::new();
        let r = release();
        // signed by a key the program does not trust
        let impostor = TestSigner::new();
        let (result, dir, asked) = prepare_with(files(&r, &impostor), signer.public(), &r, "99.0.0-beta.1", Want::Setup, "impostor");
        assert!(result.err().unwrap().contains("not signed"), "wrong key");
        assert!(!asked.iter().any(|p| p.ends_with("setup.exe")), "the installer is not even downloaded");
        let _ = std::fs::remove_dir_all(dir);
        // a manifest changed after signing
        let mut f = files(&r, &signer);
        let manifest = String::from_utf8(f[0].1.clone()).unwrap().replace("setup ", "setup ").replace(&um::hex(&um::sha256(&r.setup)), &um::hex(&um::sha256(b"other")));
        f[0].1 = manifest.into_bytes();
        let (result, dir, _) = prepare_with(f, signer.public(), &r, "99.0.0-beta.1", Want::Setup, "tampered");
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
        // no signature file at all
        let mut f = files(&r, &signer);
        f.remove(1);
        let (result, dir, _) = prepare_with(f, signer.public(), &r, "99.0.0-beta.1", Want::Setup, "nosig");
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_not_what_was_signed_is_deleted() {
        let signer = TestSigner::new();
        let r = release();
        // right size, wrong content
        let mut f = files(&r, &signer);
        let name = um::setup_name(&r.version);
        let bad = vec![0u8; r.setup.len()];
        f.iter_mut().find(|(p, _)| p.ends_with(&name)).unwrap().1 = bad;
        let (result, dir, _) = prepare_with(f, signer.public(), &r, "99.0.0-beta.1", Want::Setup, "badfile");
        assert!(result.err().unwrap().contains("checksum"));
        let left: Vec<_> = std::fs::read_dir(dir.join("update")).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name() != FOLDER_MARKER).collect();
        assert!(left.is_empty(), "the bad download is not left behind");
        let _ = std::fs::remove_dir_all(dir);
        // too short
        let mut f = files(&r, &signer);
        f.iter_mut().find(|(p, _)| p.ends_with(&name)).unwrap().1 = vec![1u8; 10];
        let (result, dir, _) = prepare_with(f, signer.public(), &r, "99.0.0-beta.1", Want::Setup, "short");
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_the_announced_newer_release_is_installed() {
        let signer = TestSigner::new();
        let r = release();
        // announced version differs from the signed manifest's
        let server = Server::start(files(&r, &signer));
        let dir = scratch("version");
        let t = trust(&server.base, signer.public());
        let ann = Version::parse("99.0.0-beta.1").unwrap();
        // an older or equal build is refused: pretend we already are 99.0.0-beta.1
        let err = prepare(&t, &ann, &ann, Want::Setup, &dir.join("update"), &mut |_| {}, &mut |_, _| {}).err().unwrap();
        assert!(err.contains("not newer"), "{err}");
        let newer = Version::parse("99.0.0-beta.2").unwrap();
        let err = prepare(&t, &newer, &Version::current(), Want::Setup, &dir.join("update"), &mut |_| {}, &mut |_, _| {}).err().unwrap();
        assert!(!err.is_empty(), "a release that was not signed is not found");
        server.finish();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replacing_the_program_keeps_the_old_one_if_anything_goes_wrong() {
        let signer = TestSigner::new();
        let r = release();
        let server = Server::start(files(&r, &signer));
        let dir = scratch("replace");
        let t = trust(&server.base, signer.public());
        let exe = dir.join("pasteroll.exe");
        std::fs::write(&exe, b"the old program").unwrap();
        let done = run_update(
            &t,
            &Mode::Replace { exe: exe.clone() },
            &r.version,
            &Version::current(),
            &dir,
            &mut |_| {},
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(done, Done::Replaced(exe.clone()));
        assert_eq!(std::fs::read(&exe).unwrap(), r.exe, "the new program is in place");
        assert_eq!(std::fs::read(old_name(&exe)).unwrap(), b"the old program", "the old one waits to be removed");
        let left: Vec<_> = std::fs::read_dir(dir.join(UPDATE_DIR)).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name() != FOLDER_MARKER && e.file_name() != PENDING).collect();
        assert!(left.is_empty(), "the download is removed");
        assert_eq!(std::fs::read_to_string(dir.join(UPDATE_DIR).join(PENDING)).unwrap(), r.version.to_string(), "the restarted program will say it updated");
        assert!(!new_name(&exe).exists(), "no half-made new program is left");
        // a file that is not a verified download is never copied over the program
        let v = Verified { path: dir.join("nope.exe"), locked: File::open(&exe).unwrap(), asset: Asset { name: "x".into(), size: 1, sha256: [0; 32] } };
        assert!(replace_program(&v, &exe).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), r.exe, "the program is unchanged after a failed replacement");
        server.finish();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_copy_that_cannot_update_says_so_and_touches_nothing() {
        let signer = TestSigner::new();
        let t = trust("http://127.0.0.1:1", signer.public());
        let dir = scratch("notify");
        let err = run_update(&t, &Mode::NotifyOnly("because".into()), &Version::parse("99.0.0").unwrap(), &Version::current(), &dir, &mut |_| {}, &mut |_, _| {}).unwrap_err();
        assert!(err.contains("because"));
        assert!(!dir.join(UPDATE_DIR).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_setup_command_for_one_account_restarts_the_old_program_only_if_setup_fails() {
        let setup = Path::new(r"C:\Users\me\AppData\Local\Pasteroll\pasteroll-update\update-0123456789abcdef.exe");
        let program = Path::new(r"C:\Users\me\AppData\Local\Programs\Pasteroll\pasteroll.exe");
        let log = Path::new(r"C:\Users\me\AppData\Local\Pasteroll\update-setup.log");
        let mut args = setup_args(false);
        args.push(format!("/LOG={}", quoted(log)));
        let line = setup_line(setup, &args, program).unwrap();
        assert!(line.starts_with("\"start \"\" /wait \"C:\\Users\\me\\AppData\\Local\\Pasteroll\\pasteroll-update\\update-"), "{line}");
        assert!(line.contains("/VERYSILENT") && line.contains("/RELAUNCH=1") && line.contains("/CURRENTUSER") && !line.contains("/ALLUSERS"), "{line}");
        assert!(line.contains("/LOG=\"C:\\Users\\me\\AppData\\Local\\Pasteroll\\update-setup.log\""), "{line}");
        // after Setup: a failure (a non-zero exit code) starts the old program again, success does nothing more
        let start = "if exist \"C:\\Users\\me\\AppData\\Local\\Programs\\Pasteroll\\pasteroll.exe\" start \"\" \"C:\\Users\\me\\AppData\\Local\\Programs\\Pasteroll\\pasteroll.exe\"";
        assert!(line.ends_with(&format!(" & (if errorlevel 1 ({start})) & (if not errorlevel 0 ({start}))\"")), "{line}");
        assert!(!line.contains("&&"), "{line}");
        // a name the command interpreter could misread means no command line at all (Setup is started alone)
        for odd in [r"C:\a&b\x.exe", r"C:\a%PATH%\x.exe", r"C:\a^b\x.exe", r"C:\a(b)\x.exe", "C:\\a\nb\\x.exe"] {
            assert!(setup_line(Path::new(odd), &args, program).is_none(), "{odd:?}");
            assert!(setup_line(setup, &args, Path::new(odd)).is_none(), "{odd:?}");
            let mut odd_log = setup_args(false);
            odd_log.push(format!("/LOG={}", quoted(Path::new(odd))));
            assert!(setup_line(setup, &odd_log, program).is_none(), "{odd:?}");
        }
    }

    #[test]
    fn leftovers_are_named_beside_the_program() {
        assert_eq!(old_name(Path::new(r"C:\x\pasteroll.exe")), PathBuf::from(r"C:\x\pasteroll.exe.old"));
        assert_eq!(new_name(Path::new(r"C:\x\pasteroll.exe")), PathBuf::from(r"C:\x\pasteroll.exe.new"));
    }

    #[test]
    fn the_verified_file_stays_locked_after_the_installer_is_started() {
        // Setup opens its own file again after it starts, so the lock must outlive the hand-over
        let signer = TestSigner::new();
        let r = release();
        let (result, _dir, _) = prepare_with(files(&r, &signer), signer.public(), &r, "99.0.0-beta.1", Want::Setup, "hold");
        let v = result.unwrap();
        let path = v.path.clone();
        v.hold_until_exit();
        assert!(std::fs::rename(&path, path.with_extension("moved")).is_err(), "cannot be swapped for another file");
        assert!(std::fs::remove_file(&path).is_err(), "cannot be deleted");
        assert!(OpenOptions::new().write(true).open(&path).is_err(), "cannot be written");
        // (the handle is closed when the test program exits)
    }

    #[test]
    fn a_folder_that_is_not_ours_is_never_deleted() {
        let dir = scratch("notours");
        let theirs = dir.join("somebody-elses-update-folder");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("precious.txt"), b"keep me").unwrap();
        let err = fresh_dir(&theirs).unwrap_err();
        assert!(err.contains("not Pasteroll's"), "{err}");
        assert_eq!(std::fs::read(theirs.join("precious.txt")).unwrap(), b"keep me");
        // one of ours is cleared and made again
        let ours = dir.join("ours");
        fresh_dir(&ours).unwrap();
        std::fs::write(ours.join("old-download.exe"), b"x").unwrap();
        fresh_dir(&ours).unwrap();
        assert!(!ours.join("old-download.exe").exists() && ours.join(FOLDER_MARKER).is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn verified_exe(dir: &Path, name: &str, bytes: &[u8]) -> Verified {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let locked = OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        Verified { path, locked, asset: Asset { name: "x".into(), size: bytes.len() as u64, sha256: um::sha256(bytes) } }
    }

    #[test]
    fn a_replacement_that_cannot_happen_leaves_the_old_program_and_says_so() {
        let dir = scratch("replfail");
        let exe = dir.join("pasteroll.exe");
        std::fs::write(&exe, b"the old program").unwrap();
        let v = verified_exe(&dir, "good.exe", b"the new program");
        // the .old name is taken by a folder with something in it: the old program cannot be moved aside
        std::fs::create_dir_all(old_name(&exe)).unwrap();
        std::fs::write(old_name(&exe).join("x"), b"x").unwrap();
        let err = replace_program(&v, &exe).unwrap_err();
        assert!(err.contains("nothing was changed"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"the old program");
        assert!(!new_name(&exe).exists(), "no half-made program is left");
        std::fs::remove_dir_all(old_name(&exe)).unwrap();
        // a download that does not match what was verified is never put in place
        let mut bad = verified_exe(&dir, "bad.exe", b"the new program");
        bad.asset.sha256 = um::sha256(b"something else");
        let err = replace_program(&bad, &exe).unwrap_err();
        assert!(err.contains("old program was not touched"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"the old program");
        // and the good case, for contrast
        replace_program(&v, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"the new program");
        assert_eq!(std::fs::read(old_name(&exe)).unwrap(), b"the old program");
        assert!(!new_name(&exe).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_update_that_did_not_finish_is_reported_once_at_the_next_start() {
        let data = scratch("pending");
        let dir = data.join(UPDATE_DIR);
        fresh_dir(&dir).unwrap();
        // nothing pending: nothing to say
        assert_eq!(startup_report(&data), None);
        // started for a newer version, and this is still the old one: say so, once
        mark_pending(&dir, &Version::parse("99.0.0-beta.1").unwrap());
        let Some(Startup::DidNotFinish(said)) = startup_report(&data) else { panic!("it did not finish") };
        assert!(said.contains("99.0.0-beta.1") && said.contains("could not be installed"), "{said}");
        assert_eq!(startup_report(&data), None, "reported once");
        // started for a version that is now running: it worked, and the download is cleaned up
        std::fs::write(dir.join("update-1.exe"), b"x").unwrap();
        mark_pending(&dir, &Version::current());
        assert_eq!(startup_report(&data), Some(Startup::Updated(Version::current())));
        assert_eq!(startup_report(&data), None, "said once");
        assert!(!dir.exists(), "the installer is not kept after a successful update");
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn the_install_kind_is_not_fooled_by_paths_that_only_look_like_program_files() {
        let machine = program_files_dirs();
        assert!(!machine.is_empty(), "Windows names its Program Files folders");
        let pf = machine[0].clone();
        let inside = pf.join("Pasteroll").join("pasteroll.exe");
        let beside = PathBuf::from(format!("{}x", pf.display())).join("pasteroll.exe");
        let kind = |exe: &Path| choose_mode(&Facts { installed_by_setup: true, program_files: machine.clone(), ..facts(exe) });
        assert_eq!(kind(&inside), Mode::Setup { all_users: true });
        assert_eq!(kind(&beside), Mode::Setup { all_users: false });
    }

    // ---- the administrator half (elevated.rs): the same checks, in a folder that is made for it

    fn staged_folders(base: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(base).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("pasteroll-update-")).collect()
    }

    #[test]
    fn the_administrator_half_downloads_verifies_and_starts_setup_from_its_own_folder() {
        let signer = TestSigner::new();
        let r = release();
        let server = Server::start(files(&r, &signer));
        let base = scratch("staged-ok");
        let started = std::cell::RefCell::new(Vec::new());
        let result = crate::elevated::stage(
            &trust(&server.base, signer.public()),
            &r.version,
            &Version::current(),
            &base,
            &|dir| std::fs::create_dir(dir).and_then(|()| std::fs::write(dir.join(FOLDER_MARKER), b"")).map_err(|e| e.to_string()),
            &|dir, setup| {
                // at the moment Setup would start: the verified installer, inside the new folder, held
                started.borrow_mut().push((dir.to_path_buf(), setup.to_path_buf(), std::fs::read(setup).unwrap()));
                assert!(OpenOptions::new().write(true).open(setup).is_err(), "cannot be written while held");
                assert!(std::fs::remove_file(setup).is_err(), "cannot be deleted while held");
                Ok(())
            },
            std::time::Duration::from_secs(3600),
        );
        assert!(result.is_ok(), "{result:?}");
        let started = started.into_inner();
        assert_eq!(started.len(), 1);
        let (dir, setup, bytes) = &started[0];
        assert_eq!(bytes, &r.setup);
        assert!(setup.starts_with(dir) && dir.starts_with(&base));
        assert!(dir.file_name().unwrap().to_string_lossy().len() >= "pasteroll-update-".len() + 32, "an unguessable name");
        // nothing at all was written in the person's data folder or anywhere but the new folder
        assert_eq!(staged_folders(&base), vec![dir.clone()]);
        let asked = server.finish();
        assert!(asked.iter().any(|p| p.ends_with("setup.exe")));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_administrator_half_refuses_what_is_not_genuine_and_leaves_nothing() {
        let signer = TestSigner::new();
        let impostor = TestSigner::new();
        let r = release();
        let base = scratch("staged-bad");
        let never_started = |_: &Path, _: &Path| -> Result<(), String> { panic!("Setup must not be started") };
        let make = |dir: &Path| std::fs::create_dir(dir).and_then(|()| std::fs::write(dir.join(FOLDER_MARKER), b"")).map_err(|e| e.to_string());
        // signed by the wrong key
        let server = Server::start(files(&r, &impostor));
        let result = crate::elevated::stage(&trust(&server.base, signer.public()), &r.version, &Version::current(), &base, &make, &never_started, std::time::Duration::from_secs(3600));
        assert_eq!(result.err().map(|e| e.0), Some(Stage::Verify));
        assert!(!server.finish().iter().any(|p| p.ends_with("setup.exe")), "the installer is not even fetched");
        assert!(staged_folders(&base).is_empty());
        // the right signature but not newer than what is running
        let server = Server::start(files(&r, &signer));
        let same = Version::parse("99.0.0-beta.1").unwrap();
        let result = crate::elevated::stage(&trust(&server.base, signer.public()), &r.version, &same, &base, &make, &never_started, std::time::Duration::from_secs(3600));
        assert_eq!(result.err().map(|e| e.0), Some(Stage::Verify));
        let _ = server.finish();
        // an installer that is not what was signed
        let mut f = files(&r, &signer);
        f[2].1 = vec![7u8; r.setup.len()];
        let server = Server::start(f);
        let result = crate::elevated::stage(&trust(&server.base, signer.public()), &r.version, &Version::current(), &base, &make, &never_started, std::time::Duration::from_secs(3600));
        assert_eq!(result.err().map(|e| e.0), Some(Stage::Verify));
        let _ = server.finish();
        assert!(staged_folders(&base).is_empty(), "a failed update leaves no folder behind");
        // nothing to download
        let server = Server::start(Vec::new());
        let result = crate::elevated::stage(&trust(&server.base, signer.public()), &r.version, &Version::current(), &base, &make, &never_started, std::time::Duration::from_secs(3600));
        assert_eq!(result.err().map(|e| e.0), Some(Stage::Download));
        let _ = server.finish();
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_administrator_half_cleans_up_when_setup_cannot_be_started() {
        let signer = TestSigner::new();
        let r = release();
        let server = Server::start(files(&r, &signer));
        let base = scratch("staged-nostart");
        let result = crate::elevated::stage(
            &trust(&server.base, signer.public()),
            &r.version,
            &Version::current(),
            &base,
            &|dir| std::fs::create_dir(dir).map_err(|e| e.to_string()),
            &|_, _| Err("no".to_owned()),
            std::time::Duration::from_secs(3600),
        );
        assert_eq!(result.err().map(|e| e.0), Some(Stage::Start));
        assert!(staged_folders(&base).is_empty());
        let _ = server.finish();
        // and a folder that cannot be made is a folder failure, not a crash
        let server = Server::start(files(&r, &signer));
        let result = crate::elevated::stage(&trust(&server.base, signer.public()), &r.version, &Version::current(), &base, &|_| Err("denied".to_owned()), &|_, _| Ok(()), std::time::Duration::from_secs(3600));
        assert_eq!(result.err().map(|e| e.0), Some(Stage::Folder));
        let _ = server.finish();
        let _ = std::fs::remove_dir_all(base);
    }
}
