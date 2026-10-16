//! The system's Save dialog, and local time, for the expanded viewer.

use pasteroll_app::SaveChoice;
use std::path::PathBuf;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// An invisible window that takes the foreground for the dialog. Pasteroll never has the
/// foreground (the program being pasted into keeps it), and Windows does not let a background
/// program bring a dialog to the front; joining the foreground thread's input for a moment does.
struct DialogOwner(HWND);

impl DialogOwner {
    fn new(near: HWND) -> Option<DialogOwner> {
        unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(near, &mut r);
            let (x, y) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
            let hinst = GetModuleHandleW(None).ok()?;
            let h = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
                w!("STATIC"),
                w!("Pasteroll"),
                WS_POPUP,
                x,
                y,
                1,
                1,
                None,
                None,
                Some(hinst.into()),
                None,
            )
            .ok()?;
            let _ = SetLayeredWindowAttributes(h, COLORREF(0), 0, LWA_ALPHA);
            let _ = ShowWindow(h, SW_SHOW);
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let me = GetCurrentThreadId();
            let joined = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
            let _ = SetForegroundWindow(h);
            if joined {
                let _ = AttachThreadInput(me, fg_thread, false);
            }
            Some(DialogOwner(h))
        }
    }
}

impl Drop for DialogOwner {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// Asks where to save `name` (with extension `ext`), above `owner`. The dialog asks before
/// replacing an existing file. Afterwards `give_back` gets the foreground again.
pub fn save_file(owner: HWND, name: &str, ext: &str, give_back: HWND) -> SaveChoice {
    unsafe {
        // the UI thread; harmless if COM is already set up on it
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileSaveDialog = match CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) {
            Ok(d) => d,
            Err(_) => return SaveChoice::Unavailable,
        };
        let (label, spec) = match ext {
            "png" => (w!("PNG picture"), w!("*.png")),
            _ => (w!("Text file"), w!("*.txt")),
        };
        let types = [
            COMDLG_FILTERSPEC { pszName: label, pszSpec: spec },
            COMDLG_FILTERSPEC { pszName: w!("All files"), pszSpec: w!("*.*") },
        ];
        let _ = dlg.SetFileTypes(&types);
        let _ = dlg.SetDefaultExtension(&HSTRING::from(ext));
        let _ = dlg.SetFileName(&HSTRING::from(name));
        let _ = dlg.SetTitle(w!("Save clip as"));
        if let Ok(opts) = dlg.GetOptions() {
            let _ = dlg.SetOptions(opts | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST);
        }
        let host = DialogOwner::new(owner);
        let shown = dlg.Show(Some(host.as_ref().map_or(owner, |h| h.0)));
        // while this program still holds the foreground, hand it back
        if IsWindow(Some(give_back)).as_bool() {
            let _ = SetForegroundWindow(give_back);
        }
        drop(host);
        if shown.is_err() {
            return SaveChoice::Cancelled; // cancelled (or could not be shown: nothing was written)
        }
        let Ok(item) = dlg.GetResult() else { return SaveChoice::Cancelled };
        let Ok(path) = item.GetDisplayName(SIGDN_FILESYSPATH) else { return SaveChoice::Cancelled };
        let s = PCWSTR(path.0).to_string().ok();
        CoTaskMemFree(Some(path.0 as *const _));
        match s {
            Some(s) if !s.is_empty() => SaveChoice::Path(PathBuf::from(s)),
            _ => SaveChoice::Cancelled,
        }
    }
}

/// "YYYY-MM-DD HH:MM" in the computer's time zone.
pub fn local_time(secs: i64) -> Option<String> {
    // Unix seconds to FILETIME (100 ns since 1601)
    let t = (secs.checked_add(11_644_473_600)?).checked_mul(10_000_000)? as u64;
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    unsafe {
        let mut utc = SYSTEMTIME::default();
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        let mut local = SYSTEMTIME::default();
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
        Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_formats_a_date() {
        let s = local_time(1_700_000_000).unwrap();
        // 2023-11-14 22:13 UTC: any time zone keeps it on the 14th or 15th
        assert!(s.starts_with("2023-11-14") || s.starts_with("2023-11-15"), "{s}");
        assert_eq!(s.len(), 16);
    }
}
