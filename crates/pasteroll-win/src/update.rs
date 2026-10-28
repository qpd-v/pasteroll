//! Looking for a newer version. This is the only place in Pasteroll that touches the network, and
//! it does nothing unless the person has said yes (`[updates] check_for_updates = true`) or asks for
//! a check themselves (the tray menu, or `pasteroll update-check`).
//!
//! Two small HTTPS requests to `raw.githubusercontent.com` (the signed "newest version" file and its
//! signature), using Windows' own client (WinHTTP: system certificates and proxy settings, no extra
//! library). GitHub sees the IP address and the program name and version, which is what any download
//! shows. The file is believed only if it is signed with the Pasteroll release key. Nothing is
//! downloaded or installed by a check: the person is told that a newer version exists, and installs
//! it (see `install`) or opens the release page only when they ask.

use pasteroll_core::config::{self, UpdateCheck};
use crate::fetch;
use crate::install::Trust;
use crate::sigcheck::CngVerifier;
use pasteroll_core::update::{UpdateState, Version, CHECK_EVERY_SECS};
use pasteroll_core::update_manifest as um;
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

/// After a failed check, the next automatic one waits this long. Any two automatic checks are at
/// least this far apart in one run, whatever happens to the state file.
const RETRY_AFTER: Duration = Duration::from_secs(6 * 3600);

/// A newer version, as the signed note describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub version: Version,
    /// Plain lines about it, shown before the person chooses to install it.
    pub notes: Vec<String>,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A newer version exists.
    Newer(Found),
    /// This is the newest (the version named is the newest release found).
    UpToDate(Version),
    /// It could not be checked (offline, a proxy, a note that is not genuine...). The text is for a person.
    Failed(String),
}

/// A finished check waiting for the tray window to show it.
pub struct Delivery {
    pub outcome: Outcome,
}

static DELIVERIES: Mutex<Vec<Delivery>> = Mutex::new(Vec::new());
/// The newer version found by a check (or remembered from an earlier run), for the menu entry.
static NEWER: Mutex<Option<Found>> = Mutex::new(None);
/// Held while the state file is read, changed and written, so that a check finishing and a skip
/// cannot lose each other's change.
static STATE_LOCK: Mutex<()> = Mutex::new(());
/// A check is running: a second one is not started on top of it.
static CHECKING: AtomicBool = AtomicBool::new(false);

pub fn take_deliveries() -> Vec<Delivery> {
    DELIVERIES.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default()
}

pub fn newer() -> Option<Version> {
    NEWER.lock().ok().and_then(|g| g.as_ref().map(|f| f.version.clone()))
}

/// The newer version with the plain lines its signed note carried, when a check in this run found it.
pub fn newer_found() -> Option<Found> {
    NEWER.lock().ok().and_then(|g| g.clone())
}

/// The plain lines of the signed note for `found`, fetched and verified now (only a note that names
/// exactly that version counts). Nothing shown to a person comes from a file on their own disk, so
/// there are none when the note cannot be verified. Blocks: call it from a worker thread.
pub fn verified_notes(found: &Version) -> Vec<String> {
    match read_latest(&Trust::github()) {
        Ok(latest) if latest.version == *found => latest.notes,
        _ => Vec::new(),
    }
}

pub fn set_newer(found: Found) {
    if let Ok(mut n) = NEWER.lock() {
        *n = Some(found);
    }
}

pub fn clear_newer() {
    if let Ok(mut n) = NEWER.lock() {
        *n = None;
    }
}

fn deliver(window: isize, outcome: Outcome) {
    if let Ok(mut d) = DELIVERIES.lock() {
        d.push(Delivery { outcome });
    }
    unsafe {
        let _ = PostMessageW(Some(HWND(window as *mut _)), WM_UPDATE_RESULT, WPARAM(0), LPARAM(0));
    }
}

/// Where the one request goes. The real one is fixed; tests point it at a local server.
pub(crate) struct Endpoint {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) secure: bool,
    pub(crate) path: String,
    /// The request headers after the user agent (each ends without a line break).
    pub(crate) headers: &'static str,
}

/// Closes a WinHTTP handle when it goes out of scope.
pub(crate) struct Handle(*mut c_void);

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

/// An answered request. Its handles close in the order request, connection, session.
pub(crate) struct Response {
    request: Handle,
    _connect: Handle,
    _session: Handle,
    pub status: u32,
}

/// Sends one GET and waits for the answer's headers. Redirects are never followed here (the caller
/// decides), TLS 1.2 and newer is required, certificates are checked (nothing here turns that off),
/// and the system's automatic sign-in to proxies and servers is off: if a setting that guards against
/// those cannot be applied, nothing is sent.
pub(crate) fn send(ep: &Endpoint, agent: &str) -> Result<Response, String> {
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
        let headers: Vec<u16> = ep.headers.encode_utf16().collect();
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
        Ok(Response { request, _connect: connect, _session: session, status })
    }
}

impl Response {
    /// The `Location` header, for a redirect.
    pub(crate) fn location(&self) -> Option<String> {
        unsafe {
            let mut size = 0u32;
            let _ = WinHttpQueryHeaders(self.request.0, WINHTTP_QUERY_LOCATION, PCWSTR::null(), None, &mut size, std::ptr::null_mut());
            if size == 0 || size > 8192 {
                return None;
            }
            let mut buf = vec![0u16; size as usize / 2 + 1];
            WinHttpQueryHeaders(
                self.request.0,
                WINHTTP_QUERY_LOCATION,
                PCWSTR::null(),
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut size,
                std::ptr::null_mut(),
            )
            .ok()?;
            String::from_utf16(&buf[..size as usize / 2]).ok()
        }
    }

    /// The `Content-Length` header, when there is one.
    pub(crate) fn content_length(&self) -> Option<u64> {
        unsafe {
            let mut value = 0u32;
            let mut size = size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                self.request.0,
                WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut value as *mut u32 as *mut c_void),
                &mut size,
                std::ptr::null_mut(),
            )
            .ok()?;
            Some(u64::from(value))
        }
    }

    /// Reads the next piece of the body into `buf` (up to its length); 0 means the end.
    pub(crate) fn read(&self, buf: &mut [u8]) -> Result<usize, String> {
        unsafe {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(self.request.0, &mut available).map_err(|e| format!("the answer was cut off ({})", e.message()))?;
            if available == 0 {
                return Ok(0);
            }
            let want = (available as usize).min(buf.len());
            let mut read = 0u32;
            WinHttpReadData(self.request.0, buf.as_mut_ptr() as *mut c_void, want as u32, &mut read)
                .map_err(|e| format!("the answer was cut off ({})", e.message()))?;
            Ok(read as usize)
        }
    }
}

/// What a status other than 200 means to a person.
pub(crate) fn status_message(status: u32) -> String {
    match status {
        403 | 429 => "GitHub is limiting requests from this address; try again later".to_owned(),
        407 => "a proxy on this network wants you to sign in".to_owned(),
        s => format!("the server answered {s}"),
    }
}

/// Reads the signed "newest version" note: both files are fetched (small, capped, from the
/// repository's own host) and the note is believed only if its signature verifies under one of
/// `trust`'s keys. A note and a signature fetched either side of a new release can disagree for a
/// moment, so a note that does not verify is fetched once more before giving up.
fn read_latest(trust: &Trust) -> Result<um::Latest, String> {
    let hosts: Vec<&str> = trust.latest_hosts.iter().map(String::as_str).collect();
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(if cfg!(test) { 20 } else { 2_000 }));
        }
        let get = |name: &str| fetch::download_small(&(trust.latest_url)(name), &hosts, um::MAX_SMALL_FILE_BYTES);
        let missing = |e: String| if e.contains("404") { "no newest-version note is published yet".to_owned() } else { e };
        let (file, signature) = (get(um::LATEST_NAME).map_err(missing)?, get(um::LATEST_SIGNATURE_NAME).map_err(missing)?);
        match um::verify_latest(&file, &signature, &trust.keys, &CngVerifier) {
            Ok(latest) => return Ok(latest),
            Err(e) => last = e,
        }
        // (a note and its signature fetched either side of a new release can disagree for a few minutes)
        last = format!("{last}; if a release was just published, try again in a few minutes");
    }
    Err(last)
}

fn check_with(trust: &Trust, current: &Version, state: &UpdateState) -> Outcome {
    let latest = match read_latest(trust) {
        Ok(l) => l,
        Err(e) => return Outcome::Failed(e),
    };
    // an old genuine note served again must not move anyone back
    if state.is_replay(&latest.version) {
        return Outcome::Failed("the newest-version note is older than one seen before, so it was ignored".to_owned());
    }
    if !current.is_pre() && latest.version.is_pre() {
        // a stable build is not told about betas
        return Outcome::UpToDate(current.clone());
    }
    if latest.version > *current {
        Outcome::Newer(Found { version: latest.version, notes: latest.notes })
    } else {
        Outcome::UpToDate(latest.version)
    }
}

/// Asks GitHub now. Blocks for up to a few minutes: call it from a worker thread. `state` is what was
/// remembered by earlier checks (it makes a replayed old note fail).
pub fn check(state: &UpdateState) -> Outcome {
    check_with(&Trust::github(), &Version::current(), state)
}

fn state_path(db: &Path) -> Option<PathBuf> {
    db.parent().map(|d| d.join("update-check.txt"))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The state with a finished check applied: when it ran, which newer version the person has now been
/// told about, and the newest version any signed note has named.
fn apply(mut state: UpdateState, outcome: &Outcome, at: i64) -> UpdateState {
    let raise = |state: &mut UpdateState, v: &Version| {
        if state.highest.as_ref().is_none_or(|h| v > h) {
            state.highest = Some(v.clone());
        }
    };
    match outcome {
        // not again at once, and not only tomorrow
        Outcome::Failed(_) => state.last_check = at - (CHECK_EVERY_SECS - RETRY_AFTER.as_secs() as i64),
        Outcome::UpToDate(v) => {
            state.last_check = at;
            state.last_ok = at;
            raise(&mut state, v);
        }
        Outcome::Newer(found) => {
            state.last_check = at;
            state.last_ok = at;
            state.announced = Some(found.version.clone());
            raise(&mut state, &found.version);
        }
    }
    state
}

/// Remembers a finished check.
fn remember(db: &Path, outcome: &Outcome) {
    let Some(path) = state_path(db) else { return };
    let _guard = STATE_LOCK.lock();
    let _ = apply(UpdateState::load(&path), outcome, now()).save(&path);
}

/// What the person chose not to hear about: the version stays out of announcements and the menu.
/// (A check they ask for still reports it, and any newer version is announced as usual.)
pub fn skip(db: &Path, version: &Version) {
    clear_newer();
    let Some(path) = state_path(db) else { return };
    let _guard = STATE_LOCK.lock();
    let mut state = UpdateState::load(&path);
    state.skipped = Some(version.clone());
    let _ = state.save(&path);
}

/// When the last check that got an answer was made, in seconds since 1970 (0 for never).
pub fn last_checked(db: &Path) -> i64 {
    state_path(db).map(|p| UpdateState::load(&p).last_ok).unwrap_or(0)
}

/// "Last checked 3 hours ago", for the Settings window.
pub fn last_checked_text(last_ok: i64, now: i64) -> String {
    if last_ok <= 0 {
        return "Not checked yet".to_owned();
    }
    let ago = now - last_ok;
    if ago < 0 {
        return "Last checked a moment ago".to_owned();
    }
    let (n, unit) = match ago {
        0..=89 => return "Last checked just now".to_owned(),
        90..=3_569 => ((ago + 30) / 60, "minute"),
        3_570..=84_599 => ((ago + 1_800) / 3_600, "hour"),
        _ => ((ago + 43_200) / 86_400, "day"),
    };
    format!("Last checked {n} {unit}{} ago", if n == 1 { "" } else { "s" })
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

/// Whether a check is running right now.
pub fn checking() -> bool {
    CHECKING.load(Ordering::Acquire)
}

/// Runs one check unless one is already running. `None` when one was.
fn check_guarded(state: &UpdateState) -> Option<Outcome> {
    if CHECKING.swap(true, Ordering::AcqRel) {
        return None;
    }
    let outcome = check(state);
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
                        if let Some(outcome) = check_guarded(&state) {
                            remember(&db, &outcome);
                            not_before = Instant::now()
                                + match outcome {
                                    Outcome::Failed(_) => RETRY_AFTER,
                                    _ => Duration::from_secs(CHECK_EVERY_SECS as u64),
                                };
                            if let Outcome::Newer(found) = &outcome {
                                if state.should_announce(&found.version, &Version::current()) {
                                    deliver(window, outcome);
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
                let state = UpdateState::load(&path);
                if let Some(v) = state.announced.clone().filter(|v| *v > current && state.skipped.as_ref() != Some(v)) {
                    // (no notes: nothing shown to a person comes from a file on their own disk)
                    set_newer(Found { version: v, notes: Vec::new() });
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
    // "Never check" has the keyboard: the answer that uses no network is one Enter away, and a stray
    // key typed while this appears cannot say yes
    let spec = crate::themeddlg::Spec {
        title: "Pasteroll updates",
        heading: "Let Pasteroll check for updates?",
        body: QUESTION,
        buttons: &["Check for updates", "Never check"],
        focus: 1,
    };
    match crate::themeddlg::show(&spec) {
        Some(0) => Answer::Yes,
        Some(1) => Answer::No,
        _ => Answer::Undecided,
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

/// A check the person asked for, run on the calling (worker) thread and remembered like any other. A
/// newer version is also kept for the tray menu. `None` when another check is already running.
pub fn check_now_blocking(db: Option<&Path>) -> Option<Outcome> {
    let state = db.and_then(state_path).map(|p| UpdateState::load(&p)).unwrap_or_default();
    let outcome = check_guarded(&state)?;
    if let Some(db) = db {
        remember(db, &outcome);
    }
    if let Outcome::Newer(found) = &outcome {
        set_newer(found.clone());
    }
    Some(outcome)
}

/// `pasteroll update-check`: one check from the command line, printed. Exit code 0 when it could
/// be checked (whatever the answer), 1 when it could not.
pub fn command() -> i32 {
    match check(&UpdateState::default()) {
        Outcome::Newer(found) => {
            println!("Pasteroll {} is available (this is {}).\n{}", found.version, env!("CARGO_PKG_VERSION"), found.version.release_page());
            for note in &found.notes {
                println!("  - {note}");
            }
            0
        }
        Outcome::UpToDate(newest) => {
            println!("Pasteroll {} is the newest version (the signed note names {newest}).", env!("CARGO_PKG_VERSION"));
            0
        }
        Outcome::Failed(e) => {
            eprintln!("pasteroll: could not check for updates: {e}");
            1
        }
    }
}

/// `pasteroll update [--yes]`: one check, printed; with `--yes` the update is downloaded, verified
/// and installed the way the tray menu does it (without the confirmation window, since the flag is
/// the confirmation). Exit code 0 when it worked or there was nothing to do, 1 when it did not.
pub fn install_command(db: &Path, yes: bool) -> i32 {
    use crate::install::{self, Done, Trust};
    let state = state_path(db).map(|p| UpdateState::load(&p)).unwrap_or_default();
    #[cfg(feature = "e2e-test-server")]
    let announced = match std::env::var("PASTEROLL_UPDATE_ANNOUNCE").ok().and_then(|v| Version::parse(&v)) {
        // the end-to-end test build names the release itself
        Some(v) => Outcome::Newer(Found { version: v, notes: Vec::new() }),
        None => check(&state),
    };
    #[cfg(not(feature = "e2e-test-server"))]
    let announced = check(&state);
    remember(db, &announced);
    match announced {
        Outcome::Failed(e) => {
            eprintln!("pasteroll: could not check for updates: {e}");
            1
        }
        Outcome::UpToDate(_) => {
            println!("Pasteroll {} is the newest version.", env!("CARGO_PKG_VERSION"));
            0
        }
        Outcome::Newer(Found { version: v, notes }) => {
            println!("Pasteroll {v} is available (this is {}).\n{}", env!("CARGO_PKG_VERSION"), v.release_page());
            for note in &notes {
                println!("  - {note}");
            }
            if !yes {
                println!("Run `pasteroll update --yes` to download, verify and install it.");
                return 0;
            }
            let mode = install::mode();
            let Some(dir) = db.parent() else {
                eprintln!("pasteroll: the data folder is unknown");
                return 1;
            };
            match install::run_update(&Trust::github(), &mode, &v, &Version::current(), dir, &mut |s| println!("{s}"), &mut |_, _| {}) {
                Ok(Done::SetupStarted) => {
                    println!("The installer was started; it closes Pasteroll and starts the new version.");
                    0
                }
                Ok(Done::Replaced(exe)) => {
                    println!("Replaced {}. Start Pasteroll again to use the new version.", exe.display());
                    0
                }
                Err(e) => {
                    eprintln!("pasteroll: the update was not installed: {e}");
                    1
                }
            }
        }
    }
}

/// `pasteroll update-verify MANIFEST SIGNATURE`: checks a release's manifest against the keys built
/// into this program, the way the updater will. Used by the release process to prove that the real
/// signature works with the real keys before anything is published.
pub fn verify_command(manifest: &Path, signature: &Path, note: Option<(&Path, &Path)>) -> i32 {
    use pasteroll_core::update_manifest::{configured_keys, verify_latest, verify_manifest};
    let (Ok(m), Ok(s)) = (std::fs::read(manifest), std::fs::read(signature)) else {
        eprintln!("pasteroll: could not read the manifest or the signature");
        return 1;
    };
    let keys = configured_keys();
    let parsed = match verify_manifest(&m, &s, &keys, &crate::sigcheck::CngVerifier) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return 1;
        }
    };
    println!("PASS: signed by a Pasteroll release key; version {}, installer {}, program {}", parsed.version, parsed.setup.name, parsed.exe.name);
    // the "newest version" note of the same release, when given
    if let Some((note, note_signature)) = note {
        let (Ok(n), Ok(ns)) = (std::fs::read(note), std::fs::read(note_signature)) else {
            eprintln!("pasteroll: could not read the newest-version note or its signature");
            return 1;
        };
        match verify_latest(&n, &ns, &keys, &crate::sigcheck::CngVerifier) {
            Ok(latest) if latest.version == parsed.version => {
                println!("PASS: the newest-version note is signed by a Pasteroll release key and names {} ({} plain lines)", latest.version, latest.notes.len());
            }
            Ok(latest) => {
                eprintln!("FAIL: the note names {} but the manifest is for {}", latest.version, parsed.version);
                return 1;
            }
            Err(e) => {
                eprintln!("FAIL: {e}");
                return 1;
            }
        }
    }
    0
}

/// The wording of the one-time question.
pub const QUESTION: &str = "Once a day Pasteroll would ask GitHub whether a newer version exists. \
GitHub sees your IP address and the Pasteroll version you run (plus the usual request headers), \
nothing else: no clipboard content, no history. You are only told: nothing is downloaded \
or installed unless you later choose to install an update.\n\n\
If you choose \"Never check\", Pasteroll never uses the internet. You can change this any time in \
Settings, or check by hand from the tray menu.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sigcheck::TestSigner;
    use crate::testsupport::{b64, Server};

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn note_text(version: &str, notes: &[&str]) -> String {
        let mut t = format!("pasteroll-update-latest 1\nversion {version}\n");
        for n in notes {
            t.push_str(&format!("note {n}\n"));
        }
        t
    }

    /// The two files of the newest-version note, signed by `signer`, as a server hands them out.
    fn note_files(text: &str, signer: &TestSigner) -> Vec<(String, Vec<u8>)> {
        vec![
            ("/latest.txt".to_owned(), text.as_bytes().to_vec()),
            ("/latest.txt.sig".to_owned(), b64(&signer.sign(text.as_bytes())).into_bytes()),
        ]
    }

    fn trust(base: &str, key: [u8; 64]) -> Trust {
        let release_base = base.to_owned();
        let note_base = base.to_owned();
        Trust {
            keys: vec![key],
            hosts: vec!["127.0.0.1".into()],
            url: Box::new(move |v, name| format!("{release_base}/v{v}/{name}")),
            latest_hosts: vec!["127.0.0.1".into()],
            latest_url: Box::new(move |name| format!("{note_base}/{name}")),
        }
    }

    /// One check against a local server serving `files`; returns what it found and the paths asked for.
    fn check_files(files: Vec<(String, Vec<u8>)>, key: [u8; 64], current: &str, state: &UpdateState) -> (Outcome, Vec<String>) {
        let server = Server::start(files);
        let outcome = check_with(&trust(&server.base, key), &v(current), state);
        (outcome, server.finish())
    }

    #[test]
    fn a_newer_signed_release_is_reported_with_its_notes() {
        let signer = TestSigner::new();
        let text = note_text("99.0.0-beta.1", &["First thing.", "Second thing."]);
        let (out, asked) = check_files(note_files(&text, &signer), signer.public(), "0.1.0-beta.9", &UpdateState::default());
        assert_eq!(out, Outcome::Newer(Found { version: v("99.0.0-beta.1"), notes: vec!["First thing.".into(), "Second thing.".into()] }));
        assert_eq!(asked, vec!["/latest.txt".to_owned(), "/latest.txt.sig".to_owned()]);
    }

    #[test]
    fn the_same_or_an_older_version_is_up_to_date() {
        let signer = TestSigner::new();
        for newest in ["0.1.0-beta.9", "0.1.0-beta.8"] {
            let text = note_text(newest, &[]);
            let (out, _) = check_files(note_files(&text, &signer), signer.public(), "0.1.0-beta.9", &UpdateState::default());
            assert_eq!(out, Outcome::UpToDate(v(newest)));
        }
    }

    #[test]
    fn a_stable_build_is_not_told_about_a_beta() {
        let signer = TestSigner::new();
        let text = note_text("0.2.0-beta.1", &[]);
        let server = Server::start(note_files(&text, &signer));
        let out = check_with(&trust(&server.base, signer.public()), &v("0.1.0"), &UpdateState::default());
        server.finish();
        assert_eq!(out, Outcome::UpToDate(v("0.1.0")));
    }

    #[test]
    fn a_note_that_is_not_signed_by_the_release_key_is_refused() {
        let signer = TestSigner::new();
        let impostor = TestSigner::new();
        let text = note_text("99.0.0-beta.1", &["Install this."]);
        // signed by someone else
        let (out, _) = check_files(note_files(&text, &impostor), signer.public(), "0.1.0-beta.9", &UpdateState::default());
        assert!(matches!(&out, Outcome::Failed(e) if e.contains("not signed")), "{out:?}");
        // changed after it was signed
        let mut files = note_files(&text, &signer);
        files[0].1 = note_text("99.0.0-beta.2", &["Install this."]).into_bytes();
        let (out, _) = check_files(files, signer.public(), "0.1.0-beta.9", &UpdateState::default());
        assert!(matches!(&out, Outcome::Failed(e) if e.contains("not signed")), "{out:?}");
        // no signature at all
        let mut files = note_files(&text, &signer);
        files.pop();
        let (out, _) = check_files(files, signer.public(), "0.1.0-beta.9", &UpdateState::default());
        assert!(matches!(&out, Outcome::Failed(e) if e.contains("no newest-version note")), "{out:?}");
        // garbage where the signature should be
        let mut files = note_files(&text, &signer);
        files[1].1 = b"not a signature".to_vec();
        let (out, _) = check_files(files, signer.public(), "0.1.0-beta.9", &UpdateState::default());
        assert!(matches!(&out, Outcome::Failed(_)), "{out:?}");
    }

    #[test]
    fn a_genuine_note_that_is_older_than_one_already_seen_is_ignored() {
        let signer = TestSigner::new();
        let text = note_text("99.0.0-beta.1", &[]);
        let seen = UpdateState { highest: Some(v("99.0.0-beta.5")), ..Default::default() };
        let (out, _) = check_files(note_files(&text, &signer), signer.public(), "0.1.0-beta.9", &seen);
        assert!(matches!(&out, Outcome::Failed(e) if e.contains("older")), "{out:?}");
        // the same version again is fine
        let same = UpdateState { highest: Some(v("99.0.0-beta.1")), ..Default::default() };
        let (out, _) = check_files(note_files(&text, &signer), signer.public(), "0.1.0-beta.9", &same);
        assert!(matches!(out, Outcome::Newer(_)));
    }

    #[test]
    fn no_note_published_or_nobody_listening_is_a_plain_failure() {
        let signer = TestSigner::new();
        let (out, _) = check_files(Vec::new(), signer.public(), "0.1.0-beta.9", &UpdateState::default());
        assert!(matches!(&out, Outcome::Failed(e) if e.contains("no newest-version note is published yet")), "{out:?}");
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let out = check_with(&trust(&format!("http://127.0.0.1:{port}"), signer.public()), &v("0.1.0-beta.9"), &UpdateState::default());
        assert!(matches!(out, Outcome::Failed(_)));
    }

    #[test]
    fn the_real_note_is_read_from_the_repository_over_https_with_the_release_keys() {
        let t = Trust::github();
        assert_eq!(t.latest_hosts, vec!["raw.githubusercontent.com".to_owned()]);
        assert_eq!((t.latest_url)("latest.txt"), "https://raw.githubusercontent.com/qpd-v/pasteroll/main/updates/latest.txt");
        assert_eq!((t.latest_url)("latest.txt.sig"), "https://raw.githubusercontent.com/qpd-v/pasteroll/main/updates/latest.txt.sig");
        assert_eq!(t.keys, um::configured_keys());
    }

    #[test]
    fn a_finished_check_is_remembered_and_a_failed_one_is_retried_sooner() {
        let at = 1_800_000_000;
        let failed = apply(UpdateState::default(), &Outcome::Failed("x".into()), at);
        assert!(!failed.due(at + 3_600), "not within the hour");
        assert!(failed.due(at + RETRY_AFTER.as_secs() as i64 + 5), "but after the retry wait");
        assert_eq!(failed.last_ok, 0, "a failure is not a check that worked");
        let newer = Found { version: v("99.0.0"), notes: vec!["A note.".into()] };
        let found = apply(failed, &Outcome::Newer(newer.clone()), at + 10);
        assert_eq!((found.announced.clone(), found.highest.clone()), (Some(v("99.0.0")), Some(v("99.0.0"))));
        assert_eq!(found.last_ok, at + 10);
        assert!(!found.due(at + 60));
        // the highest note ever seen only goes up
        let again = apply(found, &Outcome::UpToDate(v("98.0.0")), at + 20);
        assert_eq!(again.highest, Some(v("99.0.0")));
    }

    #[test]
    fn a_check_is_written_to_the_state_file() {
        let dir = std::env::temp_dir().join(format!("pasteroll-update-win-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db");
        remember(&db, &Outcome::Newer(Found { version: v("99.0.0"), notes: vec!["Hi.".into()] }));
        let s = UpdateState::load(&dir.join("update-check.txt"));
        assert_eq!(s.announced, Some(v("99.0.0")));
        assert!(last_checked(&db) > 0);
        skip(&db, &v("99.0.0"));
        let s = UpdateState::load(&dir.join("update-check.txt"));
        assert_eq!(s.skipped, Some(v("99.0.0")));
        assert!(!s.should_announce(&v("99.0.0"), &v("0.1.0")));
        assert!(s.should_announce(&v("99.0.1"), &v("0.1.0")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_last_check_is_described_in_plain_words() {
        let now = 1_800_000_000;
        assert_eq!(last_checked_text(0, now), "Not checked yet");
        assert_eq!(last_checked_text(now - 10, now), "Last checked just now");
        assert_eq!(last_checked_text(now - 600, now), "Last checked 10 minutes ago");
        assert_eq!(last_checked_text(now - 100, now), "Last checked 2 minutes ago");
        assert_eq!(last_checked_text(now - 3_600, now), "Last checked 1 hour ago");
        assert_eq!(last_checked_text(now - 3 * 3_600, now), "Last checked 3 hours ago");
        assert_eq!(last_checked_text(now - 86_400, now), "Last checked 1 day ago");
        assert_eq!(last_checked_text(now - 5 * 86_400, now), "Last checked 5 days ago");
        assert_eq!(last_checked_text(now + 100, now), "Last checked a moment ago", "a clock that went backwards");
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
        assert!(check_guarded(&UpdateState::default()).is_none(), "a second one is not started");
        CHECKING.store(false, Ordering::SeqCst);
    }
}
