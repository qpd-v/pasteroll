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
    match (std::env::current_exe(), installed_exe()) {
        (Ok(a), Ok(b)) => same_file(&a, &b),
        _ => false,
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
    get_string(RUN_KEY, VALUE_NAME).is_some()
}

/// Turns "start when I sign in" on or off for the current user. Off by default; only the person
/// asks for it (the settings window or `pasteroll autostart on`).
pub fn set_autostart(on: bool) -> Result<(), String> {
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

/// Installs this program for the current user. Returns a line for the person to read.
pub fn install() -> Result<String, String> {
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

/// Removes the installation. The history and settings are kept unless `remove_data`.
pub fn uninstall(remove_data: bool, data_dir: Option<&Path>) -> Result<String, String> {
    if !stop_running() {
        return Err("Pasteroll is running and did not quit. Quit it from its tray icon and try again.".into());
    }
    delete_value(RUN_KEY, VALUE_NAME);
    delete_key(UNINSTALL_KEY);
    if let Ok(link) = start_menu_link() {
        let _ = std::fs::remove_file(link);
    }
    let mut notes = Vec::new();
    if remove_data {
        if let Some(d) = data_dir {
            // only the files Pasteroll itself makes: the folder may have been chosen with --db
            let mut removed = 0;
            for name in [
                "history.db",
                "history.db-wal",
                "history.db-shm",
                "history.key",
                "history.key.tmp",
                "history.lock",
                "config.toml",
                "config.toml.tmp",
                "paste_keys.txt",
                "first-run.txt",
            ] {
                if std::fs::remove_file(d.join(name)).is_ok() {
                    removed += 1;
                }
            }
            let _ = std::fs::remove_dir(d);
            notes.push(format!("deleted {removed} file(s) in {}", d.display()));
        }
    } else if let Some(d) = data_dir {
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
