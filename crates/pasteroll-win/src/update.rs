//! Looking for a newer version. This is the only place in Pasteroll that touches the network, and
//! it does nothing unless the person has said yes (`[updates] check_for_updates = true`) or asks for
//! a check themselves (the tray menu, or `pasteroll update-check`).
//!
//! One HTTPS request to `api.github.com`, using Windows' own client (WinHTTP: system certificates
//! and proxy settings, no extra library). GitHub sees the IP address and the program name and
//! version, which is what any download shows. Nothing is downloaded or installed: the person is told
//! that a newer version exists and the release page is opened in their browser when they ask.

use pasteroll_core::config::{self, UpdateCheck};
use pasteroll_core::update::{newest_release, UpdateState, Version, CHECK_EVERY_SECS, MAX_REPLY_BYTES};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Tells the tray window that a check finished (the result waits in [`DELIVERIES`]).
pub const WM_UPDATE_RESULT: u32 = WM_APP + 241;

/// How long a whole check may take, whatever the server does (a server that trickles bytes cannot
/// keep resetting the per-operation timeouts).
const DEADLINE: Duration = Duration::from_secs(45);
/// After a failed check, the next automatic one waits this long. Any two automatic checks are at
/// least this far apart in one run, whatever happens to the state file.
const RETRY_AFTER: Duration = Duration::from_secs(6 * 3600);

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A newer version exists.
    Newer(Version),
    /// This is the newest (the version named is the newest release found).
    UpToDate(Version),
    /// It could not be checked (offline, a proxy, GitHub's rate limit...). The text is for a person.
    Failed(String),
}

/// A finished check waiting for the tray window to show it.
pub struct Delivery {
    pub outcome: Outcome,
    /// The person asked for this check (so even "up to date" and failures are shown).
    pub manual: bool,
}

static DELIVERIES: Mutex<Vec<Delivery>> = Mutex::new(Vec::new());
/// The newer version found by a check (or remembered from an earlier run), for the menu entry.
static NEWER: Mutex<Option<Version>> = Mutex::new(None);
/// A check is running: a second one is not started on top of it.
static CHECKING: AtomicBool = AtomicBool::new(false);

pub fn take_deliveries() -> Vec<Delivery> {
    DELIVERIES.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default()
}

pub fn newer() -> Option<Version> {
    NEWER.lock().ok().and_then(|g| g.clone())
}

pub fn set_newer(v: Version) {
    if let Ok(mut n) = NEWER.lock() {
        *n = Some(v);
    }
}

fn deliver(window: isize, outcome: Outcome, manual: bool) {
    if let Ok(mut d) = DELIVERIES.lock() {
        d.push(Delivery { outcome, manual });
    }
    unsafe {
        let _ = PostMessageW(Some(HWND(window as *mut _)), WM_UPDATE_RESULT, WPARAM(0), LPARAM(0));
    }
}

/// Where the one request goes. The real one is fixed; tests point it at a local server.
struct Endpoint {
    host: String,
    port: u16,
    secure: bool,
    path: String,
}

impl Endpoint {
    fn github() -> Endpoint {
        // by the repository's number, so renaming or moving the repository does not break the
        // check in programs that are already installed (redirects are refused)
        Endpoint { host: "api.github.com".into(), port: 443, secure: true, path: "/repositories/1422347446/releases?per_page=10".into() }
    }
}

/// Closes a WinHTTP handle when it goes out of scope.
struct Handle(*mut c_void);

impl Handle {
    fn open(raw: *mut c_void, what: &str) -> Result<Handle, String> {
        if raw.is_null() {
            Err(format!("{what} failed ({})", windows::core::Error::from_thread().message()))
        } else {
            Ok(Handle(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Sets a WinHTTP option that must not silently fail.
unsafe fn require(handle: *mut c_void, option: u32, value: u32, what: &str) -> Result<(), String> {
    unsafe {
        WinHttpSetOption(Some(handle), option, Some(&value.to_ne_bytes()))
            .map_err(|e| format!("could not set {what} ({})", e.message()))
    }
}

/// One GET. Redirects are refused, TLS 1.2 and newer is required, certificates are checked (nothing
/// here turns that off), the system's automatic sign-in to proxies and servers is off, the reply and
/// the total time are capped, and a status other than 200 is an error.
fn get(ep: &Endpoint, agent: &str) -> Result<String, String> {
    let started = Instant::now();
    unsafe {
        let session = Handle::open(
            WinHttpOpen(PCWSTR(wide(agent).as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0),
            "opening the connection",
        )?;
        if ep.secure {
            // Valid on the session only. TLS 1.2 and 1.3; a Windows that does not know 1.3 is
            // offered 1.2 alone. If neither can be set the request is not made.
            const TLS12: u32 = 0x0800;
            const TLS13: u32 = 0x2000;
            if require(session.0, WINHTTP_OPTION_SECURE_PROTOCOLS, TLS12 | TLS13, "the security protocols").is_err() {
                require(session.0, WINHTTP_OPTION_SECURE_PROTOCOLS, TLS12, "the security protocols")?;
            }
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 20_000).map_err(|e| e.message())?;
        let connect = Handle::open(
            WinHttpConnect(session.0, PCWSTR(wide(&ep.host).as_ptr()), ep.port, 0),
            "reaching the server",
        )?;
        let flags = if ep.secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let request = Handle::open(
            WinHttpOpenRequest(connect.0, w!("GET"), PCWSTR(wide(&ep.path).as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags),
            "preparing the request",
        )?;
        // a redirect could lead somewhere else, and Windows must not sign in to a proxy or server
        // with your account on its own: both are refused, and failing to refuse them stops the check
        require(request.0, WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER, "the redirect policy")?;
        require(request.0, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_DISABLE_AUTHENTICATION, "the sign-in policy")?;
        let headers: Vec<u16> = "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28".encode_utf16().collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).map_err(|e| format!("could not send the request ({})", e.message()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|e| format!("no answer ({})", e.message()))?;

        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("no status ({})", e.message()))?;
        if status != 200 {
            return Err(match status {
                403 | 429 => "GitHub is limiting requests from this address; try again later".to_owned(),
                407 => "a proxy on this network wants you to sign in".to_owned(),
                s => format!("the server answered {s}"),
            });
        }

        let mut body: Vec<u8> = Vec::new();
        loop {
            if started.elapsed() > DEADLINE {
                return Err("the server answered too slowly".to_owned());
            }
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut available).map_err(|e| format!("the answer was cut off ({})", e.message()))?;
            if available == 0 {
                break;
            }
            let mut chunk = vec![0u8; (available as usize).min(16 * 1024)];
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr() as *mut c_void, chunk.len() as u32, &mut read)
                .map_err(|e| format!("the answer was cut off ({})", e.message()))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MAX_REPLY_BYTES {
                return Err("the answer was unexpectedly large".to_owned());
            }
        }
        String::from_utf8(body).map_err(|_| "the answer was not text".to_owned())
    }
}

fn check_at(ep: &Endpoint) -> Outcome {
    let agent = format!("Pasteroll/{}", env!("CARGO_PKG_VERSION"));
    let current = Version::current();
    match get(ep, &agent) {
        Err(e) => Outcome::Failed(e),
        Ok(reply) => match newest_release(&reply, current.is_pre()) {
            None => Outcome::Failed("no release was found in the answer".to_owned()),
            Some(newest) if newest > current => Outcome::Newer(newest),
            Some(newest) => Outcome::UpToDate(newest),
        },
    }
}

/// Asks GitHub now. Blocks for up to a minute: call it from a worker thread.
pub fn check() -> Outcome {
    check_at(&Endpoint::github())
}

fn state_path(db: &Path) -> Option<PathBuf> {
    db.parent().map(|d| d.join("update-check.txt"))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Remembers a finished check: when it ran and which newer version the person has now been told about.
fn remember(db: &Path, outcome: &Outcome) {
    let Some(path) = state_path(db) else { return };
    let mut state = UpdateState::load(&path);
    match outcome {
        // not again at once, and not only tomorrow
        Outcome::Failed(_) => state.last_check = now() - (CHECK_EVERY_SECS - RETRY_AFTER.as_secs() as i64),
        Outcome::UpToDate(_) => state.last_check = now(),
        Outcome::Newer(v) => {
            state.last_check = now();
            state.announced = Some(v.clone());
        }
    }
    let _ = state.save(&path);
}

/// What the settings file says about the check right now.
#[derive(Debug, PartialEq, Eq)]
enum Setting {
    On,
    /// Off, or not decided: no automatic check.
    NotOn,
    /// The file could not be read this time (another program has it open): try again later.
    Unknown,
}

fn setting(db: &Path) -> Setting {
    match crate::keystore::read_config_text(db).ok().and_then(|t| config::parse(&t).ok()) {
        Some(c) if c.updates == UpdateCheck::On => Setting::On,
        Some(_) => Setting::NotOn,
        None => Setting::Unknown,
    }
}

/// Runs one check unless one is already running. `None` when one was.
fn check_guarded() -> Option<Outcome> {
    if CHECKING.swap(true, Ordering::AcqRel) {
        return None;
    }
    let outcome = check();
    CHECKING.store(false, Ordering::Release);
    Some(outcome)
}

/// Starts the once-a-day check for as long as the program runs. A worker thread wakes every hour,
/// looks at the settings again (so turning the check off takes effect), and asks GitHub only when a
/// day has passed since the last check, never twice within a few hours, and never while another
/// check is running. Call it only when the setting is on.
pub fn start_daily(window: isize, db: PathBuf) {
    let spawned = std::thread::Builder::new().name("pasteroll-update-check".into()).spawn(move || {
        // let the program settle first, and never delay its start
        std::thread::sleep(Duration::from_secs(30));
        let mut not_before = Instant::now();
        loop {
            match setting(&db) {
                Setting::NotOn => return,
                Setting::Unknown => {}
                Setting::On => {
                    let state = state_path(&db).map(|p| UpdateState::load(&p)).unwrap_or_default();
                    if Instant::now() >= not_before && state.due(now()) {
                        if let Some(outcome) = check_guarded() {
                            remember(&db, &outcome);
                            not_before = Instant::now()
                                + match outcome {
                                    Outcome::Failed(_) => RETRY_AFTER,
                                    _ => Duration::from_secs(CHECK_EVERY_SECS as u64),
                                };
                            if let Outcome::Newer(v) = &outcome {
                                if state.should_announce(v, &Version::current()) {
                                    deliver(window, outcome, false);
                                }
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(3_600));
        }
    });
    if let Err(e) = spawned {
        eprintln!("[pasteroll] could not start the update check: {e}");
    }
}

/// What to do at start-up with the person's choice: run the daily check when it is on, or ask
/// the one-time question when it is not decided yet (`may_ask` is false for self-tests and
/// `--no-notice`, which never ask). The question is shown from a worker thread so it never holds
/// up the program, and nothing is requested until the answer is yes.
pub fn begin(window: isize, db: &Path, choice: UpdateCheck, may_ask: bool) {
    match choice {
        UpdateCheck::On => {
            // the menu entry for a version found in an earlier run, in case its notification was missed
            if let Some(path) = state_path(db) {
                let current = Version::current();
                if let Some(v) = UpdateState::load(&path).announced.filter(|v| *v > current) {
                    set_newer(v);
                }
            }
            start_daily(window, db.to_path_buf());
        }
        UpdateCheck::Ask if may_ask => ask_in_background(window, db.to_path_buf()),
        _ => {}
    }
}

/// The answer to the question: only a definite choice is remembered. Closing the window (or a
/// dialog that could not be shown) decides nothing, and the question comes again next time.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
    Undecided,
}

fn ask() -> Answer {
    use windows::Win32::UI::Controls::*;
    const YES: i32 = 101;
    const NO: i32 = 102;
    let (title, heading, body) = (wide("Pasteroll updates"), wide("Let Pasteroll check for updates?"), wide(QUESTION));
    // buttons with no keyboard shortcuts: a stray key typed while this appears cannot say yes
    let (yes_text, no_text) = (wide("Check for updates"), wide("Never check"));
    let buttons = [
        TASKDIALOG_BUTTON { nButtonID: YES, pszButtonText: PCWSTR(yes_text.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: NO, pszButtonText: PCWSTR(no_text.as_ptr()) },
    ];
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(heading.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        // "Never check" is the default: the answer that uses no network is one Enter away
        nDefaultButton: NO,
        ..Default::default()
    };
    let mut pressed = 0i32;
    match unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) } {
        Ok(()) => match pressed {
            YES => Answer::Yes,
            NO => Answer::No,
            _ => Answer::Undecided,
        },
        Err(e) => {
            eprintln!("[pasteroll] could not show the update question: {}", e.message());
            Answer::Undecided
        }
    }
}

fn ask_in_background(window: isize, db: PathBuf) {
    let spawned = std::thread::Builder::new().name("pasteroll-update-question".into()).spawn(move || {
        std::thread::sleep(Duration::from_secs(8));
        // not on top of the Settings window: wait until it is closed
        while crate::settings::is_open() {
            std::thread::sleep(Duration::from_secs(3));
        }
        // the person may have decided in Settings meanwhile
        let undecided = crate::keystore::read_config_text(&db)
            .ok()
            .and_then(|t| config::parse(&t).ok())
            .is_some_and(|c| c.updates == UpdateCheck::Ask);
        if !undecided {
            return;
        }
        let answer = ask();
        let yes = match answer {
            Answer::Yes => true,
            Answer::No => false,
            Answer::Undecided => return,
        };
        let saved = crate::keystore::edit_config(&db, |text| {
            config::set_value(text, "check_for_updates", if yes { "true" } else { "false" }).map_err(|p| p.join("; "))
        });
        if let Err(e) = saved {
            eprintln!("[pasteroll] could not remember the update choice: {e}");
        } else if yes {
            start_daily(window, db);
        }
    });
    if let Err(e) = spawned {
        eprintln!("[pasteroll] could not show the update question: {e}");
    }
}

/// A check the person asked for: the result is always shown. A second click while one is running
/// does nothing.
pub fn check_now(window: isize, db: Option<PathBuf>) {
    let spawned = std::thread::Builder::new().name("pasteroll-update-now".into()).spawn(move || {
        let Some(outcome) = check_guarded() else { return };
        if let Some(db) = &db {
            remember(db, &outcome);
        }
        deliver(window, outcome, true);
    });
    if spawned.is_err() {
        deliver(window, Outcome::Failed("could not start the check".to_owned()), true);
    }
}

/// `pasteroll update-check`: one check from the command line, printed. Exit code 0 when it could
/// be checked (whatever the answer), 1 when it could not.
pub fn command() -> i32 {
    match check() {
        Outcome::Newer(v) => {
            println!("Pasteroll {v} is available (this is {}).\n{}", env!("CARGO_PKG_VERSION"), v.release_page());
            0
        }
        Outcome::UpToDate(_) => {
            println!("Pasteroll {} is the newest version.", env!("CARGO_PKG_VERSION"));
            0
        }
        Outcome::Failed(e) => {
            eprintln!("pasteroll: could not check for updates: {e}");
            1
        }
    }
}

/// The wording of the one-time question.
pub const QUESTION: &str = "Once a day Pasteroll would ask GitHub whether a newer version exists. \
GitHub sees your IP address and the Pasteroll version you run (plus the usual request headers), \
nothing else: no clipboard content, no history. Nothing is downloaded or installed; you are only \
told, and the download is up to you.\n\n\
If you choose \"Never check\", Pasteroll never uses the internet. You can change this any time in \
Settings, or check by hand from the tray menu.";

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    /// A one-shot local server: reads the request, answers `status` and `body`, hands back the request.
    fn serve(status: &str, body: String) -> (Endpoint, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status = status.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut s, _): (TcpStream, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(body.as_bytes());
            String::from_utf8_lossy(&req).into_owned()
        });
        (Endpoint { host: "127.0.0.1".into(), port, secure: false, path: "/repos/x/releases".into() }, handle)
    }

    fn list(newest: &str) -> String {
        format!(r#"[{{"tag_name":"{newest}","prerelease":true}},{{"tag_name":"v0.0.1"}}]"#)
    }

    #[test]
    fn a_newer_release_is_reported_and_the_request_says_what_it_is() {
        let (ep, server) = serve("200 OK", list("v99.0.0"));
        let out = check_at(&ep);
        assert_eq!(out, Outcome::Newer(Version::parse("99.0.0").unwrap()));
        let req = server.join().unwrap();
        assert!(req.starts_with("GET /repos/x/releases HTTP/1.1"), "{req}");
        assert!(req.contains(&format!("User-Agent: Pasteroll/{}", env!("CARGO_PKG_VERSION"))), "{req}");
        assert!(!req.to_ascii_lowercase().contains("cookie") && !req.to_ascii_lowercase().contains("authorization"), "{req}");
    }

    #[test]
    fn the_same_or_an_older_version_is_up_to_date() {
        let (ep, server) = serve("200 OK", list(&format!("v{}", env!("CARGO_PKG_VERSION"))));
        assert!(matches!(check_at(&ep), Outcome::UpToDate(_)));
        server.join().unwrap();
        let (ep, server) = serve("200 OK", list("v0.0.2"));
        assert!(matches!(check_at(&ep), Outcome::UpToDate(_)));
        server.join().unwrap();
    }

    #[test]
    fn errors_are_reported_not_hidden() {
        let (ep, server) = serve("403 Forbidden", r#"{"message":"rate limit"}"#.into());
        match check_at(&ep) {
            Outcome::Failed(e) => assert!(e.contains("limiting"), "{e}"),
            other => panic!("{other:?}"),
        }
        server.join().unwrap();
        let (ep, server) = serve("500 Internal Server Error", String::new());
        assert!(matches!(check_at(&ep), Outcome::Failed(e) if e.contains("500")));
        server.join().unwrap();
        let (ep, server) = serve("200 OK", "[]".into());
        assert!(matches!(check_at(&ep), Outcome::Failed(e) if e.contains("no release")));
        server.join().unwrap();
        let (ep, server) = serve("200 OK", "not json at all".into());
        assert!(matches!(check_at(&ep), Outcome::Failed(_)));
        server.join().unwrap();
    }

    #[test]
    fn a_proxy_that_wants_a_sign_in_fails_closed_without_signing_in() {
        // the 407 asks for Windows (NTLM/Negotiate) credentials; the request must not carry any
        let (ep, server) = serve("407 Proxy Authentication Required", String::new());
        assert!(matches!(check_at(&ep), Outcome::Failed(e) if e.contains("proxy")));
        let req = server.join().unwrap().to_ascii_lowercase();
        assert!(!req.contains("authorization"), "{req}");
    }

    #[test]
    fn a_redirect_is_not_followed() {
        // the server answers a redirect; nothing at the other address is ever contacted
        let (ep, server) = serve("302 Found", String::new());
        assert!(matches!(check_at(&ep), Outcome::Failed(e) if e.contains("302")));
        server.join().unwrap();
    }

    #[test]
    fn a_huge_answer_is_refused() {
        let big = format!("[{}]", "0,".repeat(MAX_REPLY_BYTES));
        let (ep, server) = serve("200 OK", big);
        assert!(matches!(check_at(&ep), Outcome::Failed(e) if e.contains("large") || e.contains("cut off")));
        let _ = server.join();
    }

    #[test]
    fn nothing_is_listening() {
        // a closed port: a plain error, no hang
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let ep = Endpoint { host: "127.0.0.1".into(), port, secure: false, path: "/".into() };
        assert!(matches!(check_at(&ep), Outcome::Failed(_)));
    }

    #[test]
    fn the_real_endpoint_is_github_over_https() {
        let e = Endpoint::github();
        assert_eq!((e.host.as_str(), e.port, e.secure), ("api.github.com", 443, true));
        assert!(e.path.starts_with("/repositories/") && e.path.contains("/releases?per_page=10"), "{}", e.path);
    }

    #[test]
    fn a_secure_request_to_a_server_that_does_not_speak_tls_fails_cleanly() {
        // the TLS options are accepted (the request is attempted) and a server that answers plain
        // HTTP on the HTTPS port is an error, not a hang or an accepted reply
        let (mut ep, server) = serve("200 OK", list("v99.0.0"));
        ep.secure = true;
        assert!(matches!(check_at(&ep), Outcome::Failed(_)));
        let _ = server.join();
    }

    #[test]
    fn a_failed_check_is_retried_later_and_news_is_remembered() {
        let dir = std::env::temp_dir().join(format!("pasteroll-update-win-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db");
        remember(&db, &Outcome::Failed("x".into()));
        let s = UpdateState::load(&dir.join("update-check.txt"));
        assert!(!s.due(now() + 3_600), "not within the hour");
        assert!(s.due(now() + RETRY_AFTER.as_secs() as i64 + 5), "but after the retry wait");
        let newer = Version::parse("99.0.0").unwrap();
        remember(&db, &Outcome::Newer(newer.clone()));
        let s = UpdateState::load(&dir.join("update-check.txt"));
        assert_eq!(s.announced, Some(newer));
        assert!(!s.due(now() + 60));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_file_is_read_as_on_off_or_unknown() {
        let dir = std::env::temp_dir().join(format!("pasteroll-update-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db");
        let cfg = dir.join("config.toml");
        std::fs::write(&cfg, "[updates]\ncheck_for_updates = true\n").unwrap();
        assert_eq!(setting(&db), Setting::On);
        std::fs::write(&cfg, "[updates]\ncheck_for_updates = false\n").unwrap();
        assert_eq!(setting(&db), Setting::NotOn);
        std::fs::write(&cfg, "").unwrap();
        assert_eq!(setting(&db), Setting::NotOn, "undecided is not on");
        std::fs::write(&cfg, "[updates\nbroken").unwrap();
        assert_eq!(setting(&db), Setting::Unknown, "a damaged file does not end the check for good");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_one_check_runs_at_a_time() {
        CHECKING.store(true, Ordering::SeqCst);
        assert!(check_guarded().is_none(), "a second one is not started");
        CHECKING.store(false, Ordering::SeqCst);
    }
}
