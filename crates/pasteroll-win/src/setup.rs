//! Installing, removing and starting with Windows, without any installer framework: the program
//! installs itself for the current user (no administrator rights, nothing outside the user's
//! profile and registry), and the first run explains what it does.
//!
//! Layout of an installation:
//!   %LOCALAPPDATA%\Programs\Pasteroll\pasteroll.exe   the program
//!   Start menu shortcut                               "Pasteroll"
//!   HKCU\...\Uninstall\Pasteroll                      Settings > Apps entry
//!   HKCU\...\Run\Pasteroll                            only when "start with Windows" is on

use crate::winsys;
use std::path::{Path, PathBuf};
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::*;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Pasteroll";
const VALUE_NAME: &str = "Pasteroll";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pw(v: &[u16]) -> PCWSTR {
    PCWSTR(v.as_ptr())
}

// ------------------------------------------------------------------ paths

pub fn install_dir() -> Result<PathBuf, String> {
    Ok(winsys::local_app_data()?.join("Programs").join("Pasteroll"))
}

pub fn installed_exe() -> Result<PathBuf, String> {
    Ok(install_dir()?.join("pasteroll.exe"))
}

fn start_menu_link() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or("APPDATA is not set")?;
    Ok(PathBuf::from(appdata).join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Pasteroll.lnk"))
}

/// Is this program the installed copy?
pub fn running_installed() -> bool {
    if pasteroll_core::paths::is_portable() {
        return false;
    }
    if installed_by_setup() {
        return true;
    }
    match (std::env::current_exe(), installed_exe()) {
        (Ok(a), Ok(b)) => same_file(&a, &b),
        _ => false,
    }
}

pub fn installed_by_setup() -> bool {
    std::env::current_exe().is_ok_and(|p| pasteroll_core::paths::installed_by_setup(&p))
}

fn require_nonportable() -> Result<(), String> {
    if pasteroll_core::paths::is_portable() {
        Err("Installation and autostart changes are unavailable in portable mode. Remove portable.txt to install this copy.".into())
    } else {
        Ok(())
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

// ------------------------------------------------------------------ registry

fn set_string(subkey: &str, name: &str, value: &str) -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        let sk = wide(subkey);
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            pw(&sk),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        );
        if rc != ERROR_SUCCESS {
            return Err(format!("cannot open the registry key {subkey} (error {})", rc.0));
        }
        let v = wide(value);
        let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
        let n = wide(name);
        let rc = RegSetValueExW(key, pw(&n), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return Err(format!("cannot write {name} (error {})", rc.0));
        }
        Ok(())
    }
}

fn set_dword(subkey: &str, name: &str, value: u32) -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        let sk = wide(subkey);
        let rc = RegCreateKeyExW(HKEY_CURRENT_USER, pw(&sk), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None);
        if rc != ERROR_SUCCESS {
            return Err(format!("cannot open the registry key {subkey} (error {})", rc.0));
        }
        let n = wide(name);
        let rc = RegSetValueExW(key, pw(&n), None, REG_DWORD, Some(&value.to_le_bytes()));
        let _ = RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return Err(format!("cannot write {name} (error {})", rc.0));
        }
        Ok(())
    }
}

fn get_string(subkey: &str, name: &str) -> Option<String> {
    unsafe {
        let mut key = HKEY::default();
        let sk = wide(subkey);
        if RegOpenKeyExW(HKEY_CURRENT_USER, pw(&sk), None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let n = wide(name);
        let mut size = 0u32;
        let mut ty = REG_NONE;
        let rc = RegQueryValueExW(key, pw(&n), None, Some(&mut ty), None, Some(&mut size));
        if rc != ERROR_SUCCESS || ty != REG_SZ || size == 0 {
            let _ = RegCloseKey(key);
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let rc = RegQueryValueExW(key, pw(&n), None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut size));
        let _ = RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return None;
        }
        let units: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
    }
}

fn delete_value(subkey: &str, name: &str) {
    unsafe {
        let mut key = HKEY::default();
        let sk = wide(subkey);
        if RegOpenKeyExW(HKEY_CURRENT_USER, pw(&sk), None, KEY_WRITE, &mut key) == ERROR_SUCCESS {
            let n = wide(name);
            let _ = RegDeleteValueW(key, pw(&n));
            let _ = RegCloseKey(key);
        }
    }
}

fn delete_key(subkey: &str) {
    unsafe {
        let sk = wide(subkey);
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, pw(&sk));
    }
}

// ------------------------------------------------------------------ start with Windows

/// What the Run entry would be: this program, `run`.
fn run_command(exe: &Path) -> String {
    format!("\"{}\" run", exe.display())
}

pub fn autostart_enabled() -> bool {
    if pasteroll_core::paths::is_portable() { return false; }
    get_string(RUN_KEY, VALUE_NAME).is_some()
}

/// Turns "start when I sign in" on or off for the current user. Off by default; only the person
/// asks for it (the settings window or `pasteroll autostart on`).
pub fn set_autostart(on: bool) -> Result<(), String> {
    require_nonportable()?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        set_string(RUN_KEY, VALUE_NAME, &run_command(&exe))
    } else {
        delete_value(RUN_KEY, VALUE_NAME);
        Ok(())
    }
}

// ------------------------------------------------------------------ install / uninstall

fn make_shortcut(link: &Path, target: &Path) -> Result<(), String> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| -> windows::core::Result<()> {
            let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let t = wide(&target.to_string_lossy());
            sl.SetPath(pw(&t))?;
            sl.SetArguments(w!("run"))?;
            sl.SetIconLocation(pw(&t), 0)?;
            sl.SetDescription(w!("Pasteroll clipboard history"))?;
            sl.SetShowCmd(SW_SHOWMINNOACTIVE)?;
            let dir = wide(&target.parent().unwrap_or(Path::new("")).to_string_lossy());
            sl.SetWorkingDirectory(pw(&dir))?;
            let file: IPersistFile = sl.cast()?;
            let l = wide(&link.to_string_lossy());
            file.Save(pw(&l), true)?;
            Ok(())
        })();
        if hr.is_ok() {
            CoUninitialize();
        }
        result.map_err(|e| format!("cannot create the Start menu shortcut: {e}"))
    }
}

/// Asks a running copy to quit and waits for it.
pub fn stop_running() -> bool {
    if !winsys::app_is_running() {
        return true;
    }
    unsafe {
        if let Ok(h) = FindWindowExW(Some(HWND_MESSAGE), None, w!("PasterollApp"), None) {
            let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    for _ in 0..50 {
        if !winsys::app_is_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// An installer must not stop an unrelated copy running from another installation folder.
/// Enumerate message-only windows, verify the owning executable, then wait for that process.
pub fn stop_installation(expected: &Path) -> bool {
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    let mut previous = None;
    let mut unreadable = false;
    loop {
        let window = unsafe { FindWindowExW(Some(HWND_MESSAGE), previous, w!("PasterollApp"), None) };
        let Ok(window) = window else { return !unreadable };
        previous = Some(window);
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut pid)); }
        let Some(actual) = winsys::exe_path(pid) else { unreadable = true; continue };
        if !same_file(&actual, expected) { continue; }
        let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else { return false };
        let stopped = unsafe {
            PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
                && WaitForSingleObject(process, 5_000) == WAIT_OBJECT_0
        };
        unsafe { let _ = CloseHandle(process); }
        return stopped;
    }
}

/// Installs this program for the current user. Returns a line for the person to read.
pub fn install() -> Result<String, String> {
    require_nonportable()?;
    if installed_by_setup() {
        return Ok("Pasteroll is already installed by Setup. Use Setup to upgrade it.".into());
    }
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir()?;
    let dst = installed_exe()?;
    if !stop_running() {
        return Err("Pasteroll is running and did not quit. Quit it from its tray icon and try again.".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    if !same_file(&src, &dst) {
        let tmp = dst.with_extension("exe.new");
        std::fs::copy(&src, &tmp).map_err(|e| format!("cannot copy the program to {}: {e}", dir.display()))?;
        let _ = std::fs::remove_file(&dst);
        std::fs::rename(&tmp, &dst).map_err(|e| format!("cannot place {}: {e}", dst.display()))?;
    }
    let exe = dst.to_string_lossy().into_owned();
    set_string(UNINSTALL_KEY, "DisplayName", "Pasteroll")?;
    set_string(UNINSTALL_KEY, "DisplayVersion", env!("CARGO_PKG_VERSION"))?;
    set_string(UNINSTALL_KEY, "Publisher", "Pasteroll")?;
    set_string(UNINSTALL_KEY, "InstallLocation", &dir.to_string_lossy())?;
    set_string(UNINSTALL_KEY, "DisplayIcon", &exe)?;
    set_string(UNINSTALL_KEY, "UninstallString", &format!("\"{exe}\" uninstall"))?;
    set_dword(UNINSTALL_KEY, "NoModify", 1)?;
    set_dword(UNINSTALL_KEY, "NoRepair", 1)?;
    let link = start_menu_link()?;
    if let Some(parent) = link.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    make_shortcut(&link, &dst)?;
    // an existing "start with Windows" entry must follow the program to its new place
    if autostart_enabled() {
        set_string(RUN_KEY, VALUE_NAME, &run_command(&dst))?;
    }
    Ok(format!("Installed to {}. Start it from the Start menu.", dir.display()))
}

/// Adopt a self-install's per-user registration after Setup has placed its marker.
pub fn adopt_setup() -> Result<String, String> {
    require_nonportable()?;
    if !installed_by_setup() { return Err("This command requires a Setup-owned copy.".into()); }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let previous = installed_exe()?;
    // Only retire the old self-installer's Apps entry when Setup upgraded that same folder.
    if let Some(dir) = get_string(UNINSTALL_KEY, "InstallLocation") {
        if exe.parent().is_some_and(|p| same_file(p, Path::new(&dir))) {
            if get_string(RUN_KEY, VALUE_NAME).as_deref() == Some(&run_command(&previous)) {
                set_string(RUN_KEY, VALUE_NAME, &run_command(&exe))?;
            }
            delete_key(UNINSTALL_KEY);
        }
    }
    Ok("Setup registration synchronized.".into())
}

/// Removes a legacy self-install. Setup itself owns removal of Setup installations.
pub fn uninstall(remove_data: bool, db: &Path) -> Result<String, String> {
    require_nonportable()?;
    if installed_by_setup() {
        return Err("Pasteroll was installed by Setup. Uninstall it through Settings > Apps > Pasteroll.".into());
    }
    if !stop_running() {
        return Err("Pasteroll is running and did not quit. Quit it from its tray icon and try again.".into());
    }
    let removed = if remove_data { Some(remove_user_data(db)?) } else { None };
    delete_value(RUN_KEY, VALUE_NAME);
    delete_key(UNINSTALL_KEY);
    if let Ok(link) = start_menu_link() {
        let _ = std::fs::remove_file(link);
    }
    let mut notes = Vec::new();
    if let Some(removed) = removed {
        notes.push(format!("deleted {removed} data file(s)"));
    } else if let Some(d) = db.parent() {
        notes.push(format!("your history and settings were kept in {}", d.display()));
    }
    // the program file: if this is the running copy it cannot delete itself, so a detached
    // command line does it a moment after this process exits
    let dir = install_dir()?;
    let exe = installed_exe()?;
    if running_installed() {
        use std::os::windows::process::CommandExt;
        let script = format!(
            "/C \"ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"\"",
            exe.display(),
            dir.display()
        );
        // raw_arg: the quoting above is for cmd.exe and must not be escaped again
        let _ = std::process::Command::new("cmd")
            .raw_arg(script)
            .creation_flags(0x0800_0000 | 0x0000_0008) // CREATE_NO_WINDOW | DETACHED_PROCESS
            .spawn();
    } else {
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(format!("Pasteroll was removed. {}", notes.join("; ")))
}

/// Setup's data-only removal: no program, shortcut or registry changes, and no key decryption.
/// The exclusive history lock refuses removal while an app or maintenance operation uses it.
pub fn remove_user_data(db: &Path) -> Result<usize, String> {
    require_nonportable()?;
    let dir = db.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    if !dir.exists() { return Ok(0); }
    let lock = crate::keystore::lock_history(db, true)?;
    let sidecar = |suffix: &str| {
        let mut name = db.as_os_str().to_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    let mut files = vec![db.to_path_buf(), sidecar("-wal"), sidecar("-shm"), sidecar("-journal"),
        db.with_extension("key"), db.with_extension("key.tmp")];
    for name in ["config.toml", "config.toml.tmp", "paste_keys.txt", "image-presets.txt",
        "image-presets.txt.tmp", "first-run.txt", "pasteroll.log"] {
        files.push(dir.join(name));
    }
    // Refuse links before deleting anything, just as reset-history does.
    for p in &files {
        if winsys::is_reparse_point(p) {
            return Err("A data file is a link; refusing to remove data.".into());
        }
    }
    let mut removed = 0;
    for p in files {
        match std::fs::remove_file(p) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("could not remove a data file: {e}")),
        }
    }
    drop(lock);
    let _ = std::fs::remove_file(dir.join("history.lock"));
    let _ = std::fs::remove_dir(dir); // only succeeds if empty; unrelated files always stay
    Ok(removed)
}

// ------------------------------------------------------------------ first run

pub const NOTICE: &str = "Pasteroll keeps a history of what you copy so you can paste it again later.\n\n\
- It is stored only on this PC, encrypted for your Windows account. Nothing is sent anywhere.\n\
- Open it with Ctrl+Shift+middle-click. Pick a clip with the wheel and press Enter, or select part of it first to paste just that part.\n\
- Pause recording, or exclude programs such as password managers, from the tray icon: Settings.\n\
- Pasteroll does not start with Windows unless you turn that on in Settings.\n\n\
You will only see this once.";

/// Shows the privacy notice the first time the app runs (a marker file in the data folder).
pub fn first_run_notice(db: &Path) {
    let Some(dir) = db.parent() else { return };
    let marker = dir.join("first-run.txt");
    if marker.exists() {
        return;
    }
    let (t, x) = (wide("Welcome to Pasteroll"), wide(NOTICE));
    unsafe {
        MessageBoxW(None, pw(&x), pw(&t), MB_OK | MB_ICONINFORMATION | MB_TOPMOST);
    }
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&marker, format!("shown by version {}\n", env!("CARGO_PKG_VERSION")));
}

/// Double-clicking the downloaded program: offer to install it.
pub fn prompt_install() -> bool {
    let (t, x) = (
        wide("Pasteroll"),
        wide("Install Pasteroll for your account?\n\nIt will be placed in your user folder and added to the Start menu. No administrator rights are needed, and it will not start with Windows unless you turn that on.\n\nChoose No to run it once without installing."),
    );
    unsafe { MessageBoxW(None, pw(&x), pw(&t), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

/// If this process is the only one on its console (it was started from the Start menu or
/// Explorer, not from a terminal), let go of the console so no empty window stays open. Output that
/// was going to that console goes to NUL instead of failing; redirected output (a file, a pipe)
/// is left alone.
pub fn detach_console_if_owned() {
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Console::*;
    unsafe {
        let mut pids = [0u32; 4];
        if GetConsoleProcessList(&mut pids) != 1 {
            return;
        }
        let is_console = |which: STD_HANDLE| {
            GetStdHandle(which).ok().is_some_and(|h| {
                let mut mode = CONSOLE_MODE(0);
                GetConsoleMode(h, &mut mode).is_ok()
            })
        };
        let (out_c, err_c) = (is_console(STD_OUTPUT_HANDLE), is_console(STD_ERROR_HANDLE));
        if FreeConsole().is_err() {
            return;
        }
        if out_c || err_c {
            if let Ok(nul) = CreateFileW(
                w!("NUL"),
                0x4000_0000, // GENERIC_WRITE
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) {
                if out_c {
                    let _ = SetStdHandle(STD_OUTPUT_HANDLE, nul);
                }
                if err_c {
                    let _ = SetStdHandle(STD_ERROR_HANDLE, nul);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_removal_refuses_an_in_use_history_before_deleting_anything() {
        let dir = std::env::temp_dir().join(format!("pasteroll-remove-locked-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db");
        std::fs::write(&db, b"scratch").unwrap();
        let lock = crate::keystore::lock_history(&db, false).unwrap();
        assert!(remove_user_data(&db).is_err());
        assert_eq!(std::fs::read(&db).unwrap(), b"scratch");
        drop(lock);
        remove_user_data(&db).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn the_run_entry_quotes_the_path() {
        assert_eq!(run_command(Path::new(r"C:\Users\A B\pasteroll.exe")), r#""C:\Users\A B\pasteroll.exe" run"#);
    }

    #[test]
    fn registry_values_round_trip_and_delete() {
        let key = r"Software\PasterollTest\roundtrip";
        set_string(key, "v", "hello ünïcode").unwrap();
        assert_eq!(get_string(key, "v").as_deref(), Some("hello ünïcode"));
        delete_value(key, "v");
        assert_eq!(get_string(key, "v"), None);
        delete_key(key);
        delete_key(r"Software\PasterollTest");
    }

    #[test]
    fn the_installed_location_is_under_local_app_data() {
        let p = installed_exe().unwrap();
        assert!(p.ends_with(r"Programs\Pasteroll\pasteroll.exe"), "{}", p.display());
    }

    #[test]
    fn the_notice_says_what_matters() {
        for needle in ["only on this PC", "encrypted", "Nothing is sent", "does not start with Windows"] {
            assert!(NOTICE.contains(needle), "{needle}");
        }
    }
}
