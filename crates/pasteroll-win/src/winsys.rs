//! Small process/security helpers: who is the user, is a process elevated, what is its exe, and
//! locking a file or folder down to the current user.

use std::path::Path;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Security::Authorization::*;
use windows::Win32::Security::*;
use windows::Win32::System::Threading::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// RAII for a process handle.
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn token_elevated(token: HANDLE) -> Option<bool> {
    unsafe {
        let mut e = TOKEN_ELEVATION::default();
        let mut got = 0u32;
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut e as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut got,
        )
        .ok()?;
        Some(e.TokenIsElevated != 0)
    }
}

pub fn self_elevated() -> bool {
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).is_err() {
            return false;
        }
        let t = Handle(tok);
        token_elevated(t.0).unwrap_or(false)
    }
}

/// Mandatory integrity level (RID) of a token: 0x1000 low, 0x2000 medium, 0x3000 high, 0x4000
/// system. Windows blocks input injection to a *higher* level, which is a wider set than "elevated".
fn token_integrity(token: HANDLE) -> Option<u32> {
    unsafe {
        let mut need = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut need);
        if need == 0 {
            return None;
        }
        let mut buf = vec![0u64; (need as usize).div_ceil(8)];
        GetTokenInformation(token, TokenIntegrityLevel, Some(buf.as_mut_ptr() as *mut _), need, &mut need)
            .ok()?;
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let n = *GetSidSubAuthorityCount(sid) as u32;
        if n == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, n - 1))
    }
}

pub fn process_integrity(pid: u32) -> Option<u32> {
    unsafe {
        let p = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => Handle(h),
            Err(e) if e.code() == E_ACCESSDENIED => return Some(u32::MAX),
            Err(_) => return None,
        };
        let mut tok = HANDLE::default();
        match OpenProcessToken(p.0, TOKEN_QUERY, &mut tok) {
            Ok(()) => token_integrity(Handle(tok).0),
            Err(e) if e.code() == E_ACCESSDENIED => Some(u32::MAX),
            Err(_) => None,
        }
    }
}

pub fn self_integrity() -> u32 {
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).is_err() {
            return 0x2000;
        }
        token_integrity(Handle(tok).0).unwrap_or(0x2000)
    }
}

/// True when the process runs at a higher integrity level than we do, so the system would block
/// our keystrokes to it. Unknown counts as "no": the paste is attempted and its failure reported.
pub fn outranks_us(pid: u32) -> bool {
    process_integrity(pid).is_some_and(|t| t > self_integrity())
}

/// Lower-cased exe file name of a process ("windowsterminal.exe").
pub fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let p = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()?;
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit(['\\', '/']).next().map(|s| s.to_ascii_lowercase())
    }
}

/// The current user's SID as a string ("S-1-5-21-...").
pub fn current_user_sid() -> Option<String> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).ok()?;
        let t = Handle(tok);
        let mut need = 0u32;
        let _ = GetTokenInformation(t.0, TokenUser, None, 0, &mut need);
        if need == 0 {
            return None;
        }
        // u64 elements: the TOKEN_USER we read out of this buffer needs 8-byte alignment.
        let mut buf = vec![0u64; (need as usize).div_ceil(8)];
        GetTokenInformation(t.0, TokenUser, Some(buf.as_mut_ptr() as *mut _), need, &mut need).ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        out
    }
}

/// Replaces the ACL on `path` with one that grants full control only to the current user and the
/// system account, and stops inheriting from the parent. For a folder the entries are inherited by
/// files created inside it later (the database's WAL and shared-memory files).
pub fn restrict_to_current_user(path: &Path, folder: bool) -> Result<(), String> {
    let sid = current_user_sid().ok_or("could not determine the current user")?;
    let inherit = if folder { "OICI" } else { "" };
    let sddl = format!("D:P(A;{inherit};FA;;;{sid})(A;{inherit};FA;;;SY)");
    unsafe {
        let mut psd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide(&sddl).as_ptr()),
            SDDL_REVISION_1,
            &mut psd,
            None,
        )
        .map_err(|e| format!("security descriptor: {e}"))?;
        let mut present = windows::core::BOOL(0);
        let mut defaulted = windows::core::BOOL(0);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let r = GetSecurityDescriptorDacl(psd, &mut present, &mut dacl, &mut defaulted)
            .map_err(|e| format!("dacl: {e}"));
        let result = r.and_then(|_| {
            use std::os::windows::ffi::OsStrExt;
            let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let err = SetNamedSecurityInfoW(
                PCWSTR(w.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(dacl),
                None,
            );
            if err == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(format!("SetNamedSecurityInfo failed ({})", err.0))
            }
        });
        let _ = LocalFree(Some(HLOCAL(psd.0)));
        result
    }
}


/// The user's local app-data folder from the shell (not from the environment, which a parent
/// process controls).
pub fn local_app_data() -> Result<std::path::PathBuf, String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None)
            .map_err(|e| format!("cannot locate Local AppData: {e}"))?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(std::path::PathBuf::from).map_err(|e| format!("bad Local AppData path: {e}"))
    }
}

/// A reparse point (junction, symlink, mount point) in place of the file or folder we expect.
pub fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & 0x400 != 0)
        .unwrap_or(false)
}

/// Whether `path` is owned by the current user (or, when we run elevated, by Administrators,
/// which owns files an elevated process creates).
pub fn owned_by_current_user(path: &Path) -> Result<bool, String> {
    use std::os::windows::ffi::OsStrExt;
    unsafe {
        let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut owner = PSID::default();
        let mut psd = PSECURITY_DESCRIPTOR::default();
        let err = GetNamedSecurityInfoW(
            PCWSTR(w.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut psd,
        );
        if err != ERROR_SUCCESS {
            return Err(format!("could not read the owner of {} ({})", path.display(), err.0));
        }
        let mut s = PWSTR::null();
        let conv = ConvertSidToStringSidW(owner, &mut s);
        let owner_str = if conv.is_ok() { s.to_string().ok() } else { None };
        if conv.is_ok() {
            let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        }
        let _ = LocalFree(Some(HLOCAL(psd.0)));
        let owner_str = owner_str.ok_or("could not read the owner SID")?;
        let me = current_user_sid().ok_or("could not determine the current user")?;
        Ok(owner_str == me || (self_elevated() && owner_str == "S-1-5-32-544"))
    }
}

/// Marks a file or folder so that programs running at a LOWER integrity level than normal user
/// programs (sandboxed browsers, document viewers, anything the system confines) can neither read
/// nor change it. This does not stop normal programs run by the same user: only an additional
/// secret can do that.
pub fn label_no_low_access(path: &Path, folder: bool) -> Result<(), String> {
    // ML = mandatory label, NW/NR/NX = no write/read/execute up, ME = medium integrity.
    let sddl = if folder { "S:(ML;OICI;NWNRNX;;;ME)" } else { "S:(ML;;NWNRNX;;;ME)" };
    use std::os::windows::ffi::OsStrExt;
    unsafe {
        let mut psd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide(sddl).as_ptr()),
            SDDL_REVISION_1,
            &mut psd,
            None,
        )
        .map_err(|e| format!("integrity label: {e}"))?;
        let mut present = windows::core::BOOL(0);
        let mut defaulted = windows::core::BOOL(0);
        let mut sacl: *mut ACL = std::ptr::null_mut();
        let got = GetSecurityDescriptorSacl(psd, &mut present, &mut sacl, &mut defaulted)
            .map_err(|e| format!("integrity label: {e}"));
        let result = got.and_then(|_| {
            let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let err = SetNamedSecurityInfoW(
                PCWSTR(w.as_ptr()),
                SE_FILE_OBJECT,
                LABEL_SECURITY_INFORMATION,
                None,
                None,
                None,
                Some(sacl),
            );
            if err == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(format!("could not set the integrity label ({})", err.0))
            }
        });
        let _ = LocalFree(Some(HLOCAL(psd.0)));
        result
    }
}

/// Does `path` carry the no-access-from-lower-integrity label set by [`label_no_low_access`]?
pub fn has_no_low_access_label(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    unsafe {
        let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut sacl: *mut ACL = std::ptr::null_mut();
        let mut psd = PSECURITY_DESCRIPTOR::default();
        let err = GetNamedSecurityInfoW(
            PCWSTR(w.as_ptr()),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            None,
            None,
            None,
            Some(&mut sacl),
            &mut psd,
        );
        if err != ERROR_SUCCESS || sacl.is_null() {
            return false;
        }
        let mut found = false;
        for i in 0..(*sacl).AceCount as u32 {
            let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
            if GetAce(sacl, i, &mut ace).is_err() || ace.is_null() {
                continue;
            }
            let label = &*(ace as *const SYSTEM_MANDATORY_LABEL_ACE);
            // 0x11 = SYSTEM_MANDATORY_LABEL_ACE_TYPE; mask bits: no-write-up 1, no-read-up 2, no-exec-up 4
            if label.Header.AceType == 0x11 && label.Mask & 0x7 == 0x7 {
                found = true;
            }
        }
        let _ = LocalFree(Some(HLOCAL(psd.0)));
        found
    }
}

/// A held lock on a file in the data folder. Dropping it releases the lock.
pub struct FileLock {
    _file: std::fs::File,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LockError {
    /// Another process holds a conflicting lock.
    Busy,
    Io(String),
}

/// Takes a lock on `path` (created if missing) without waiting. Many *shared* holders may coexist;
/// an *exclusive* holder excludes everyone. The app holds a shared lock for as long as it runs, so
/// anything that replaces the history key (which needs the exclusive lock) cannot do so under it,
/// and nothing can start writing while a key replacement is in progress.
pub fn lock_file(path: &Path, exclusive: bool) -> Result<FileLock, LockError> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LOCK_FILE_FLAGS};
    use windows::Win32::System::IO::OVERLAPPED;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| LockError::Io(format!("cannot open {}: {e}", path.display())))?;
    let mut ov = OVERLAPPED::default();
    let flags = LOCKFILE_FAIL_IMMEDIATELY
        | if exclusive { LOCKFILE_EXCLUSIVE_LOCK } else { LOCK_FILE_FLAGS(0) };
    let r = unsafe { LockFileEx(HANDLE(file.as_raw_handle()), flags, None, 1, 0, &mut ov) };
    match r {
        Ok(()) => Ok(FileLock { _file: file }),
        Err(e) if e.code() == ERROR_LOCK_VIOLATION.to_hresult() => Err(LockError::Busy),
        Err(e) => Err(LockError::Io(e.to_string())),
    }
}

/// Renames `from` over `to` and does not return until the rename is on disk. A plain rename may
/// still be in the cache when the power fails, which is exactly the window key replacement must
/// not have.
pub fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
    let f: Vec<u16> = from.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let t: Vec<u16> = to.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        MoveFileExW(PCWSTR(f.as_ptr()), PCWSTR(t.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
            .map_err(|e| format!("cannot replace {}: {e}", to.display()))
    }
}

/// Free bytes on the volume holding `path` (for the current user).
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut avail = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut avail), None, None).ok()? };
    Some(avail)
}

/// Exit code of a process that ended itself on a crash. Distinct, so tests can tell.
pub const CRASH_EXIT_CODE: u32 = 0xC0DE_0001;

pub fn die() -> ! {
    unsafe {
        let _ = TerminateProcess(GetCurrentProcess(), CRASH_EXIT_CODE);
    }
    // TerminateProcess does not return for the current process; this is for the type system.
    loop {
        std::thread::park();
    }
}

unsafe extern "system" fn on_unhandled_exception(
    _info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    die()
}

/// Crash handling for a program that holds clipboard text in memory.
///
/// A crash dump written by Windows Error Reporting contains the process's memory, clip text
/// included, and may be uploaded or left on disk. So: no error dialogs, and any panic or unhandled
/// exception ends the process immediately, before WER gets to capture it. The panic hook prints the
/// source location only: a panic message could contain data.
pub fn harden_process() {
    use windows::Win32::System::Diagnostics::Debug::{
        SetErrorMode, SetUnhandledExceptionFilter, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
        SEM_NOOPENFILEERRORBOX,
    };
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
        SetUnhandledExceptionFilter(Some(on_unhandled_exception));
    }
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        if let Some(l) = info.location() {
            // `let _` on purpose: if stderr is a closed pipe, reporting must not panic again.
            let _ = writeln!(std::io::stderr(), "[pasteroll] internal error at {}:{}", l.file(), l.line());
        }
        die();
    }));
}

/// Is a Pasteroll instance running for this user? (Its single-instance mutex exists.) Anything that
/// replaces the history key must refuse while it is: the running app holds the old key.
pub fn app_is_running() -> bool {
    use windows::core::w;
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, w!("Local\\PasterollApp")) {
            Ok(h) => {
                let _ = CloseHandle(h);
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_process_is_known() {
        let pid = std::process::id();
        assert!(!outranks_us(pid));
        assert_eq!(process_integrity(pid), Some(self_integrity()));
        let exe = exe_name(pid).unwrap();
        assert!(exe.ends_with(".exe") && exe.contains("pasteroll"), "{exe}");
    }

    #[test]
    fn the_current_user_has_a_sid() {
        assert!(current_user_sid().unwrap().starts_with("S-1-5-"));
    }

    #[test]
    fn a_restricted_folder_stays_usable_by_us() {
        let d = std::env::temp_dir().join(format!("pasteroll-acl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        restrict_to_current_user(&d, true).unwrap();
        let f = d.join("inside.txt");
        std::fs::write(&f, b"ok").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"ok");
        // files created later inherit the restriction and remain ours
        restrict_to_current_user(&f, false).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"ok");
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod more_tests {
    #[test]
    fn the_low_integrity_label_is_set_and_inherited() {
        let d = std::env::temp_dir().join(format!("pasteroll-label-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(!has_no_low_access_label(&d));
        label_no_low_access(&d, true).unwrap();
        assert!(has_no_low_access_label(&d));
        // files created later inherit it, and we (medium integrity) can still use them
        let f = d.join("inside.txt");
        std::fs::write(&f, b"ok").unwrap();
        assert!(has_no_low_access_label(&f), "new files must inherit the label");
        assert_eq!(std::fs::read(&f).unwrap(), b"ok");
        // an existing file can be labelled directly
        let g = std::env::temp_dir().join(format!("pasteroll-label-file-{}", std::process::id()));
        std::fs::write(&g, b"x").unwrap();
        label_no_low_access(&g, false).unwrap();
        assert!(has_no_low_access_label(&g));
        let _ = std::fs::remove_file(&g);
        let _ = std::fs::remove_dir_all(&d);
    }

    use super::*;

    #[test]
    fn local_app_data_is_found_and_owned_by_us() {
        let p = local_app_data().unwrap();
        assert!(p.is_dir(), "{}", p.display());
        assert!(owned_by_current_user(&p).unwrap());
    }

    #[test]
    fn ordinary_folders_are_not_reparse_points() {
        assert!(!is_reparse_point(&std::env::temp_dir()));
        assert!(!is_reparse_point(Path::new("C:\\definitely\\not\\here")));
    }

    #[test]
    fn integrity_levels_are_plausible() {
        let me = self_integrity();
        assert!((0x1000..=0x4000).contains(&me), "{me:#x}");
    }
}
