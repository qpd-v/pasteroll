//! The administrator half of an update, for a copy installed for all users (in Program Files).
//!
//! Setup must run with administrator rights there. An installer that an elevated process starts from a
//! folder the ordinary account can write to can be swapped for another file between the check and the
//! start, and Inno Setup reads its own file again after it has started. So the person's own Pasteroll
//! trusts nothing it downloaded itself: it only asks Windows to run `pasteroll apply-update VERSION`
//! with administrator rights, using the program in Program Files, which only administrators can change.
//!
//! That helper does the whole job again from the start, in a new folder that only Administrators and
//! the system account can open (under Windows' own `Temp`, with a random name): it downloads the signed
//! manifest and the installer, checks the signature and the checksum, points Setup's temporary files at
//! that folder too, and starts Setup from there. The ordinary account never owns a file that the
//! elevated code reads. What the helper accepts is exactly what the person's Pasteroll would accept,
//! so running it by hand can at most install a genuine, newer, signed release.

use crate::install::{self, Stage, Trust, Want, FOLDER_MARKER};
use pasteroll_core::update::Version;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The command-line word that starts the helper.
pub const COMMAND: &str = "apply-update";

/// What the helper's exit code means. It is all the helper can say: a file it wrote into a folder the
/// ordinary account controls would be a hazard.
pub mod exit {
    pub const STARTED: i32 = 0;
    pub const REFUSED: i32 = 40;
    pub const DOWNLOAD: i32 = 41;
    pub const VERIFY: i32 = 42;
    pub const FOLDER: i32 = 43;
    pub const START: i32 = 44;
}

/// Folders this code makes are named like this inside Windows' `Temp`.
const PREFIX: &str = "pasteroll-update-";

pub fn explain(code: i32) -> String {
    match code {
        exit::REFUSED => "the administrator part of the update refused to run (it must be the installed program, started with administrator rights)".to_owned(),
        exit::DOWNLOAD => "the administrator part of the update could not download the release (check the internet connection)".to_owned(),
        exit::VERIFY => "the administrator part of the update could not verify the release, so nothing was installed".to_owned(),
        exit::FOLDER => "the administrator part of the update could not prepare its protected folder".to_owned(),
        exit::START => "the administrator part of the update could not start Setup".to_owned(),
        other => format!("the administrator part of the update stopped unexpectedly (code {other})"),
    }
}

fn exit_code(stage: Stage) -> i32 {
    match stage {
        Stage::Download => exit::DOWNLOAD,
        Stage::Verify => exit::VERIFY,
        Stage::Folder => exit::FOLDER,
        Stage::Start => exit::START,
    }
}

/// The person's side: asks Windows for administrator permission to run the helper, and waits for it
/// to hand over to Setup. `Ok` means Setup was started (it closes this program itself).
pub fn run_helper(announced: &Version) -> Result<(), String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::*;
    // the real, long path that was checked to be in Program Files, not a link someone could retarget
    let exe = real_exe().ok_or("the program's own location is unknown")?;
    let file: Vec<u16> = exe.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let params: Vec<u16> = format!("{COMMAND} {announced}{}", test_arguments()).encode_utf16().chain(Some(0)).collect();
    // start in System32, not in whatever folder this program happens to be in
    let system = crate::winsys::system_dir().ok_or("Windows' system folder could not be found")?;
    let directory: Vec<u16> = system.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        nShow: 0, // SW_HIDE
        ..Default::default()
    };
    // ShellExecute wants a COM apartment on this thread
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let started = unsafe { ShellExecuteExW(&mut info) };
    if com.is_ok() {
        unsafe { CoUninitialize() };
    }
    started.map_err(|e| {
        if e.code().0 as u32 & 0xFFFF == 1223 {
            "Windows administrator permission was not given".to_owned()
        } else {
            format!("the administrator part of the update could not be started ({})", e.message())
        }
    })?;
    if info.hProcess.is_invalid() {
        return Err("the administrator part of the update could not be watched".to_owned());
    }
    // the download is capped at a few minutes; this is a generous limit on the whole helper
    let waited = unsafe { WaitForSingleObject(info.hProcess, 15 * 60 * 1000) };
    let mut code = 0u32;
    let got = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
    unsafe {
        let _ = CloseHandle(info.hProcess);
    }
    if waited != WAIT_OBJECT_0 {
        return Err("the administrator part of the update did not finish in time".to_owned());
    }
    if got.is_err() {
        return Err("the administrator part of the update did not report back".to_owned());
    }
    match code as i32 {
        exit::STARTED => Ok(()),
        other => Err(explain(other)),
    }
}

/// The end-to-end test build only (never in a release): Windows does not pass environment variables to
/// an elevated program, so the settings of the local test server travel on the command line.
#[cfg(feature = "e2e-test-server")]
fn test_arguments() -> String {
    ["PASTEROLL_UPDATE_BASE", "PASTEROLL_UPDATE_TEST_KEY"]
        .iter()
        .filter_map(|n| std::env::var(n).ok().map(|v| format!(" {n}={v}")))
        .collect()
}

#[cfg(not(feature = "e2e-test-server"))]
fn test_arguments() -> String {
    String::new()
}

/// The helper itself (`pasteroll apply-update VERSION`), running with administrator rights. Returns the
/// process exit code.
pub fn apply_update(version: Option<&str>, extra: &[String]) -> i32 {
    #[cfg(feature = "e2e-test-server")]
    for pair in extra {
        if let Some((name, value)) = pair.split_once('=') {
            if name.starts_with("PASTEROLL_UPDATE_") {
                // SAFETY: nothing else is running yet
                unsafe { std::env::set_var(name, value) };
            }
        }
    }
    #[cfg(not(feature = "e2e-test-server"))]
    let _ = extra;
    let Some(announced) = version.and_then(Version::parse) else { return exit::REFUSED };
    // a hard limit on the whole helper, shorter than the wait on the person's side, so that a helper
    // the person has been told "did not finish" cannot go on to start Setup later
    let _ = std::thread::Builder::new().name("pasteroll-helper-deadline".into()).spawn(|| {
        std::thread::sleep(Duration::from_secs(10 * 60));
        std::process::exit(exit::DOWNLOAD);
    });
    if !crate::winsys::self_elevated() || !runs_from_program_files() {
        return exit::REFUSED;
    }
    let Some(base) = crate::winsys::windows_dir().map(|w| w.join("Temp")).filter(|t| t.is_dir() && !crate::winsys::is_reparse_point(t)) else {
        return exit::FOLDER;
    };
    match stage(&Trust::github(), &announced, &Version::current(), &base, &make_folder, &start_setup, SWEEP_AFTER) {
        Ok(()) => exit::STARTED,
        Err(s) => exit_code(s.0),
    }
}

/// This program's own file by its real long path (no 8.3 names, no link in the way).
fn real_exe() -> Option<PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    exe.to_str().map(|s| PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(s)))
}

/// Whether this running program is in a Program Files folder (a place only administrators can write
/// in a default Windows).
fn runs_from_program_files() -> bool {
    let Some(exe) = real_exe() else { return false };
    install::program_files_dirs().iter().any(|p| install::under(&exe, p))
}

/// Folders older than this are leftovers of an update that did not finish, and are removed.
const SWEEP_AFTER: Duration = Duration::from_secs(60 * 60);

fn make_folder(dir: &Path) -> Result<(), String> {
    crate::winsys::create_admin_only_dir(dir)?;
    // the owner can always change the access list: it must be Administrators or the system (a policy
    // can make the creator the owner instead)
    match crate::winsys::owner_sid(dir) {
        Ok(owner) if owner == "S-1-5-32-544" || owner == "S-1-5-18" => {}
        other => {
            let _ = std::fs::remove_dir(dir);
            return Err(format!("the update folder is not owned by Administrators ({other:?})"));
        }
    }
    std::fs::write(dir.join(FOLDER_MARKER), b"Pasteroll's update folder. Safe to delete.\n").map_err(|e| format!("could not prepare the update folder ({e})"))
}

/// The whole job: a new protected folder, then the verified download in it, then Setup. `make` makes the
/// folder (protected), `start` starts Setup from it. The folder is removed again if anything fails.
pub(crate) fn stage(
    trust: &Trust,
    announced: &Version,
    current: &Version,
    base: &Path,
    make: &dyn Fn(&Path) -> Result<(), String>,
    start: &dyn Fn(&Path, &Path) -> Result<(), String>,
    sweep_after: Duration,
) -> Result<(), (Stage, String)> {
    sweep(base, sweep_after);
    let dir = base.join(format!("{PREFIX}{}", crate::winsys::random_hex(16)));
    let result = install::prepare_in(trust, announced, current, Want::Setup, &|| make(&dir).map(|()| dir.clone()), &mut |_| {}, &mut |_, _| {}).and_then(|v| {
        let started = start(&dir, &v.path).map_err(|e| (Stage::Start, e));
        drop(v.locked);
        started
    });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

/// Removes update folders of this program that are older than `age` and not links.
fn sweep(base: &Path, age: Duration) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = entry.file_name().to_string_lossy().starts_with(PREFIX);
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e >= age);
        if ours && old && !crate::winsys::is_reparse_point(&path) && std::fs::symlink_metadata(path.join(FOLDER_MARKER)).is_ok_and(|m| m.is_file()) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Characters a path may have for the command line below; nothing the command interpreter treats
/// specially. Windows' own folders and our random names always pass.
fn plain(path: &Path) -> bool {
    let s = path.to_string_lossy();
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | ':' | '\\'))
}

/// The command that runs Setup, waits for it, starts the new program as the person, then removes the
/// folder. Setup replaces `pasteroll.exe`, and the helper is that very program, so something else has
/// to wait: the system's own `cmd.exe`. `program` is started only if Setup succeeded, through
/// `explorer.exe`, which starts it the way the person would (without administrator rights, with their
/// own environment); it is left out if its name is not a plain one.
pub(crate) fn setup_command(setup: &Path, dir: &Path, program: &Path, explorer: &Path) -> Result<String, String> {
    if !plain(setup) || !plain(dir) {
        return Err("the update folder has an unexpected name".to_owned());
    }
    // by full path: nothing is looked up through PATH
    let relaunch = if plain(program) && plain(explorer) { format!(" && \"{}\" \"{}\"", explorer.display(), program.display()) } else { String::new() };
    // `start /wait` because cmd does not otherwise wait for a program that has windows
    Ok(format!("\"start \"\" /wait \"{}\" {}{relaunch} & rd /s /q \"{}\"\"", setup.display(), install::setup_args(true).join(" "), dir.display()))
}

fn start_setup(dir: &Path, setup: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let system = crate::winsys::system_dir().ok_or("Windows' system folder could not be found")?;
    let program = std::env::current_exe().map_err(|e| format!("the program's own location is unknown ({e})"))?;
    let windows = crate::winsys::windows_dir().ok_or("Windows' folder could not be found")?;
    let line = setup_command(setup, dir, &program, &windows.join("explorer.exe"))?;
    // Setup's own temporary files (it copies itself and unpacks into %TEMP%) go in the protected
    // folder too, not into the ordinary account's temp folder
    let tmp = dir.join("tmp");
    std::fs::create_dir(&tmp).map_err(|e| format!("could not prepare the temporary folder ({e})"))?;
    std::process::Command::new(system.join("cmd.exe"))
        // /d: no AutoRun; /e:on /v:off: the same command-processor behaviour whatever the account's settings say
        .raw_arg("/d /e:on /v:off /s /c")
        .raw_arg(line)
        .current_dir(&system)
        // nothing is looked up through a PATH that comes from the account's own registry settings
        .env("PATH", format!("{};{}", system.display(), windows.display()))
        .env("TEMP", &tmp)
        .env("TMP", &tmp)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000 | 0x0000_0200) // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Setup could not be started ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setup_command_waits_starts_the_program_as_the_person_and_cleans_up() {
        let dir = PathBuf::from(r"C:\WINDOWS\Temp\pasteroll-update-0123456789abcdef0123456789abcdef");
        let setup = dir.join("update-0123456789abcdef.exe");
        let program = PathBuf::from(r"C:\Program Files\Pasteroll\pasteroll.exe");
        let explorer = PathBuf::from(r"C:\WINDOWS\explorer.exe");
        let line = setup_command(&setup, &dir, &program, &explorer).unwrap();
        assert!(line.starts_with("\"start \"\" /wait \"C:\\WINDOWS\\Temp\\pasteroll-update-"), "{line}");
        assert!(line.contains("/VERYSILENT") && line.contains("/ALLUSERS") && !line.contains("/CURRENTUSER"), "{line}");
        assert!(!line.contains("/RELAUNCH"), "an elevated Setup must not start the program itself: {line}");
        assert!(line.contains(" && \"C:\\WINDOWS\\explorer.exe\" \"C:\\Program Files\\Pasteroll\\pasteroll.exe\" & rd /s /q \"C:\\WINDOWS\\Temp\\pasteroll-update-"), "{line}");
        assert!(line.ends_with("\"\""), "{line}");
        // an odd program name only means no restart; an odd folder name means nothing is run
        let odd = setup_command(&setup, &dir, Path::new(r"C:\a&b\pasteroll.exe"), &explorer).unwrap();
        assert!(!odd.contains("explorer") && odd.contains("/ALLUSERS"), "{odd}");
        for bad in [r"C:\a&b\x", r"C:\a%PATH%\x", r#"C:\a"b\x"#, r"C:\a^b\x", r"C:\a|b\x", r"C:\a(b)\x", "C:\\a\nb\\x", ""] {
            assert!(setup_command(&dir.join("s.exe"), Path::new(bad), &program, &explorer).is_err(), "{bad:?}");
            assert!(setup_command(Path::new(bad), &dir, &program, &explorer).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn every_failure_has_its_own_words() {
        let codes = [exit::REFUSED, exit::DOWNLOAD, exit::VERIFY, exit::FOLDER, exit::START, 9999];
        let texts: Vec<String> = codes.iter().map(|c| explain(*c)).collect();
        for (i, t) in texts.iter().enumerate() {
            assert!(t.starts_with("the administrator part of the update"), "{t}");
            assert!(texts.iter().enumerate().all(|(j, u)| i == j || u != t));
        }
        assert!(texts[5].contains("9999"));
        assert_eq!(exit_code(Stage::Verify), exit::VERIFY);
        assert_ne!(exit::STARTED, exit_code(Stage::Start));
    }

    #[test]
    fn the_helper_refuses_what_it_is_not_asked_properly() {
        // not elevated (tests run as an ordinary program), no version, a version that is not one
        assert_eq!(apply_update(None, &[]), exit::REFUSED);
        assert_eq!(apply_update(Some("not a version"), &[]), exit::REFUSED);
        assert_eq!(apply_update(Some("99.0.0-beta.1; calc"), &[]), exit::REFUSED);
        if !crate::winsys::self_elevated() {
            assert_eq!(apply_update(Some("99.0.0-beta.1"), &[]), exit::REFUSED);
        }
    }

    #[test]
    fn a_folder_made_for_administrators_is_closed_to_the_ordinary_account() {
        let base = std::env::temp_dir().join(format!("pasteroll-admin-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dir = base.join("locked");
        crate::winsys::create_admin_only_dir(&dir).unwrap();
        assert!(dir.is_dir());
        assert!(crate::winsys::create_admin_only_dir(&dir).is_err(), "an existing folder is never reused");
        if !crate::winsys::self_elevated() {
            // an ordinary account has no access: it cannot add a file, list it or open it
            assert!(std::fs::write(dir.join("planted.txt"), b"x").is_err(), "cannot plant a file");
            assert!(std::fs::read_dir(&dir).is_err(), "cannot list it");
        }
        let _ = std::fs::remove_dir(&dir);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn old_update_folders_are_swept_and_other_things_are_not() {
        let base = std::env::temp_dir().join(format!("pasteroll-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ours = base.join(format!("{PREFIX}aaaa"));
        let no_marker = base.join(format!("{PREFIX}bbbb"));
        let others = base.join("somebody-elses");
        for d in [&ours, &no_marker, &others] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(ours.join(FOLDER_MARKER), b"").unwrap();
        std::fs::write(others.join(FOLDER_MARKER), b"").unwrap();
        sweep(&base, Duration::from_secs(3600));
        assert!(ours.exists(), "a recent folder may belong to an update that is running");
        sweep(&base, Duration::ZERO);
        assert!(!ours.exists(), "an old folder of ours goes");
        assert!(no_marker.exists(), "a folder without the marker is not ours");
        assert!(others.exists(), "a folder with another name is not ours");
        let _ = std::fs::remove_dir_all(&base);
    }
}
