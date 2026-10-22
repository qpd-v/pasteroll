//! What the person sees when they choose to install an update: the confirmation, and progress or
//! failure reported through the notification icon. The work itself is in `install`.

use crate::install::{self, Done, Mode, Trust};
use pasteroll_core::update::Version;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Tells the tray window that something happened while installing (see [`take_events`]).
pub const WM_INSTALL_EVENT: u32 = WM_APP + 242;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A step, for the notification.
    Say(String),
    /// It stopped; nothing was changed. The text is for a person.
    Failed(String),
    /// Setup was started; it will close this program and start the new version.
    SetupStarted,
    /// The program file was replaced; restart into it.
    Replaced(PathBuf),
}

static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());
/// An update is being offered or installed: a second one is not started on top of it.
static BUSY: AtomicBool = AtomicBool::new(false);

pub fn take_events() -> Vec<Event> {
    EVENTS.lock().map(|mut e| std::mem::take(&mut *e)).unwrap_or_default()
}

fn post(window: isize, event: Event) {
    if let Ok(mut e) = EVENTS.lock() {
        e.push(event);
    }
    unsafe {
        let _ = PostMessageW(Some(HWND(window as *mut _)), WM_INSTALL_EVENT, WPARAM(0), LPARAM(0));
    }
}

/// A message for the person from outside an update they are running (start-up findings).
pub fn report(window: isize, text: String) {
    post(window, Event::Failed(text));
}

/// How this copy updates; it does not change while the program runs.
pub fn mode() -> &'static Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    MODE.get_or_init(install::mode)
}

/// Whether "Download and install" can be offered for this copy.
pub fn installable() -> bool {
    !matches!(mode(), Mode::NotifyOnly(_))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The words of the confirmation, for this kind of copy.
pub fn confirmation_text(mode: &Mode, current: &Version, found: &Version) -> String {
    let what = match mode {
        Mode::Setup { all_users: false } => String::new(),
        Mode::Setup { all_users: true } => "\n\nThis copy is installed for all users, so Windows will ask for administrator permission.".to_owned(),
        Mode::Replace { exe } => format!(
            "\n\nPasteroll will replace its own program file ({}). Your history and settings are not touched.",
            exe.display()
        ),
        Mode::NotifyOnly(_) => String::new(),
    };
    format!(
        "You have Pasteroll {current}. Pasteroll {found} is available.\n\nPasteroll will download it from GitHub, check that it is signed with the Pasteroll release key, and install it. Your history and settings are kept. Pasteroll closes and starts again by itself.{what}"
    )
}

#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Install,
    Notes,
    NotNow,
}

fn ask(text: &str, found: &Version) -> Answer {
    use windows::Win32::UI::Controls::*;
    const INSTALL: i32 = 101;
    const NOTES: i32 = 102;
    let title = wide("Pasteroll update");
    let heading = wide(&format!("Install Pasteroll {found} now?"));
    let body = wide(text);
    // buttons with no keyboard shortcuts, and "Not now" as the default: a stray key cannot install
    let (install_text, notes_text) = (wide("Download and install"), wide("Release notes"));
    let buttons = [
        TASKDIALOG_BUTTON { nButtonID: INSTALL, pszButtonText: PCWSTR(install_text.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: NOTES, pszButtonText: PCWSTR(notes_text.as_ptr()) },
    ];
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(heading.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: 2, // IDCANCEL
        ..Default::default()
    };
    let mut pressed = 0i32;
    match unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) } {
        Ok(()) => match pressed {
            INSTALL => Answer::Install,
            NOTES => Answer::Notes,
            _ => Answer::NotNow,
        },
        Err(_) => Answer::NotNow,
    }
}

/// Offers the update `found` for installation: asks, then (only on "Download and install") downloads,
/// verifies and installs it. Runs on its own thread; the notification icon shows progress.
pub fn offer(window: isize, db: PathBuf, found: Version) {
    if BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new().name("pasteroll-update-install".into()).spawn(move || {
        let current = Version::current();
        let mode = mode().clone();
        if let Mode::NotifyOnly(why) = &mode {
            post(window, Event::Failed(format!("This copy cannot update itself: {why}.")));
            BUSY.store(false, Ordering::Release);
            return;
        }
        match ask(&confirmation_text(&mode, &current, &found), &found) {
            Answer::Install => {}
            Answer::Notes => {
                open_page(&found);
                BUSY.store(false, Ordering::Release);
                return;
            }
            Answer::NotNow => {
                BUSY.store(false, Ordering::Release);
                return;
            }
        }
        let Some(data_dir) = db.parent().map(PathBuf::from) else {
            post(window, Event::Failed("Pasteroll's data folder is unknown.".to_owned()));
            BUSY.store(false, Ordering::Release);
            return;
        };
        let mut last_percent = u64::MAX;
        let result = install::run_update(
            &Trust::github(),
            &mode,
            &found,
            &current,
            &data_dir,
            &mut |s| post(window, Event::Say(s.to_owned())),
            &mut |got, total| {
                // a message at most every 25 percent
                if let Some(t) = total.filter(|t| *t > 0) {
                    let pct = got * 100 / t / 25 * 25;
                    if pct != last_percent && pct > 0 && pct < 100 {
                        last_percent = pct;
                        post(window, Event::Say(format!("Downloading the update... {pct}%")));
                    }
                }
            },
        );
        match result {
            Ok(Done::SetupStarted) => post(window, Event::SetupStarted),
            Ok(Done::Replaced(exe)) => post(window, Event::Replaced(exe)),
            Err(e) => post(window, Event::Failed(format!("The update was not installed: {e}."))),
        }
        // a started Setup closes this program; if that does not happen, a new offer is allowed
        BUSY.store(false, Ordering::Release);
    });
    if spawned.is_err() {
        BUSY.store(false, Ordering::Release);
    }
}

/// The confirmation window on its own, for looking at it in the end-to-end test build.
#[cfg(feature = "e2e-test-server")]
pub fn preview_confirmation() -> i32 {
    let (cur, found) = (Version::current(), Version::parse("99.0.0-beta.1").expect("version"));
    let answer = ask(&confirmation_text(mode(), &cur, &found), &found);
    println!("{answer:?}");
    0
}

/// Opens the release page of `v` in the browser.
pub fn open_page(v: &Version) {
    use windows::core::w;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
    let url = wide(&v.release_page());
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), None, None, SW_SHOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_confirmation_says_what_will_happen_to_this_kind_of_copy() {
        let (cur, new) = (Version::parse("0.1.0-beta.8").unwrap(), Version::parse("0.1.0-beta.9").unwrap());
        let setup = confirmation_text(&Mode::Setup { all_users: false }, &cur, &new);
        assert!(setup.contains("0.1.0-beta.8") && setup.contains("0.1.0-beta.9") && setup.contains("signed"));
        assert!(setup.contains("history and settings are kept"));
        assert!(!setup.contains("administrator"));
        let machine = confirmation_text(&Mode::Setup { all_users: true }, &cur, &new);
        assert!(machine.contains("administrator permission"));
        let portable = confirmation_text(&Mode::Replace { exe: PathBuf::from(r"D:\tools\pasteroll.exe") }, &cur, &new);
        assert!(portable.contains(r"D:\tools\pasteroll.exe") && portable.contains("replace its own program file"));
    }
}
