//! Pasteroll: Windows front end.
//!
//! pasteroll     run [--db FILE] [options]    the app (global hooks; clipboard capture and paste)
//!     --no-capture        do not listen to the system clipboard
//!     --accept-injected   honour synthetic input (debug builds only; for self-tests)
//!     --max-seconds N     exit after N seconds
//! pasteroll     status                       what is stored, and the limits in force
//! pasteroll     config [show|path|init]      the settings file
//! pasteroll     config set KEY VALUE [--yes] change a setting (asks first if it would delete clips)
//! pasteroll     exclude EXE                  never record copies made in EXE
//! pasteroll     clear --yes [--keep-pinned]  delete history
//! pasteroll     repair --yes                 delete clips that can no longer be opened
//! pasteroll     key-export --out FILE        save the key, protected by a passphrase, for recovery
//! pasteroll     key-import --in FILE         restore it (after a reinstall or password reset)
//! pasteroll     rotate-key                   replace the master key now (Pasteroll must not be running)
//! pasteroll     reset-history --yes          delete history and its key
//! pasteroll     seed                         add synthetic demo clips
//! pasteroll     update-check                 ask GitHub once whether a newer version exists
//! pasteroll     update [--yes]               the same, and with --yes download, verify and install it
//! pasteroll     probe                        self-test of the overlay with real input (no clipboard)
//! pasteroll     clipboard-selftest           round-trips synthetic items through the real clipboard
//!                                           (overwrites the clipboard)
//!
//! The history defaults to %LOCALAPPDATA%\Pasteroll\history.db with its key and config.toml beside it.
//! The folder is locked to the current user and the clip bodies are encrypted. A file that is not a
//! Pasteroll history is refused, so `--db` cannot be pointed at another app's database by accident.


mod about;
mod app;
mod clipboard;
mod dialogs;
mod elevated;
mod fetch;
mod hooks;
mod input;
mod install;
mod installui;
mod keystore;
mod ocr;
mod overlay;
mod platform;
mod probe;
mod rollkeys;
mod selftest;
mod settings;
mod settingsui;
mod setup;
mod sigcheck;
#[cfg(test)]
mod testsupport;
mod themeddlg;
mod tray;
mod update;
mod updatewin;
mod winsys;

use pasteroll_core::config;
use pasteroll_core::limits::MB;
use pasteroll_core::Store;
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

/// A failed allocation ends the process immediately. Rust's own out-of-memory path is a
/// fast-fail, which Windows Error Reporting can capture as a memory dump; with clips of tens of
/// megabytes in memory that is a realistic way to leak clipboard text into a crash report.
struct DieOnOom;

unsafe impl GlobalAlloc for DieOnOom {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if p.is_null() {
            winsys::die();
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if p.is_null() {
            winsys::die();
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if p.is_null() {
            winsys::die();
        }
        p
    }
}

#[global_allocator]
static ALLOC: DieOnOom = DieOnOom;

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn usage() -> ! {
    eprintln!(
        "usage: pasteroll     run | status | config [show|path|init|set KEY VALUE] | exclude EXE | \
         clear --yes [--keep-pinned] | repair --yes | key-export --out FILE | key-import --in FILE | \
         rotate-key | reset-history --yes | seed | probe | clipboard-selftest | stop | install | \
         uninstall [--remove-data --yes] | autostart [on|off] | about [--text] | update-check | update [--yes] | update-verify MANIFEST SIGNATURE\n\
         options: --db FILE   --no-capture   --max-seconds N   --yes   --no-notice"
    );
    std::process::exit(2);
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn fail(msg: impl AsRef<str>) -> i32 {
    eprintln!("{}", msg.as_ref());
    1
}

/// The store with the user's limits applied, and the config they came from. Holds the shared
/// history lock for as long as the returned guard lives: a key replacement cannot start under it.
fn open(db: &Path) -> Result<(Store, config::Config, winsys::FileLock), String> {
    let lock = keystore::lock_history(db, false)?;
    let cfg = keystore::load_config(db).map_err(|p| {
        format!("{} is not valid:\n  - {}", keystore::config_path(db).display(), p.join("\n  - "))
    })?;
    let mut store = keystore::open_store(db)?;
    store.set_limits(cfg.limits).map_err(|e| e.to_string())?;
    Ok((store, cfg, lock))
}

fn seed(db: &Path, args: &[String]) -> i32 {
    // Synthetic content only; the same kind of clips the demo used.
    let clips = [
        "The quick brown fox jumps over the lazy dog. A second sentence follows it.",
        "fn greet(name: &str) -> String {\n    format!(\"Hello, {name}!\")\n}",
        "https://example.com/docs/getting-started?ref=carousel",
        "Capture a thought before it slips away.",
        "Meeting notes: ship the spike, measure paste reliability, decide.",
    ];
    let (mut s, _, _lock) = match open(db) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let now = now_secs();
    for c in clips {
        if let Err(e) = s.add_text(c, now) {
            return fail(format!("seed failed: {e}"));
        }
    }
    s.add_files(&["C:\\Demo\\Documents\\invoice-2026-10.pdf".into()], now).ok();
    // A small synthetic image too, so image preview and image-to-desktop can be tried.
    let mut px = Vec::new();
    for y in 0..120u32 {
        for x in 0..200u32 {
            px.extend_from_slice(&[(x * 255 / 200) as u8, (y * 255 / 120) as u8, 160, 255]);
        }
    }
    let img = pasteroll_core::image::Rgba { width: 200, height: 120, pixels: px };
    if let Some(png) = pasteroll_core::image::encode_png(&img) {
        let _ = s.add_image_png(&png, now);
    }
    // A long text (for scrolling and finding in the expanded viewer) and a picture of text (for
    // zooming and Extract text): Pasteroll's own carousel, drawn at twice the size.
    let long: String = (1..=400)
        .map(|i| format!("Line {i:03}: the expanded viewer scrolls long clips; find any word, such as needle {i}.\n"))
        .collect();
    let _ = s.add_text_from(&long, now, Some("notepad.exe"));
    let mut r = pasteroll_render::Renderer::new();
    let vm = pasteroll_render::ViewModel {
        rows: ["Invoice 2026-10 total 1,284.50 EUR", "Meeting notes: ship the spike", "Quarterly report draft"]
            .iter()
            .map(|t| pasteroll_render::RowView {
                kind: pasteroll_render::RowKind::Text,
                pinned: false,
                title: (*t).to_owned(),
                meta: String::new(),
                highlights: vec![],
                multi: 0,
                ratio: 0.0,
                thumb: None,
            })
            .collect(),
        active: 0,
        body: pasteroll_render::Body::Text {
            display: "Invoice 2026-10\nTotal due: 1,284.50 EUR\nPay by 31 October".into(),
            selection: None,
            scroll: 0.0,
        },
        ..pasteroll_render::ViewModel::default()
    };
    let f = r.render(&vm, (pasteroll_render::WIDTH, pasteroll_render::HEIGHT), 2.0);
    let mut px = f.bgra;
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let shot = pasteroll_core::image::Rgba { width: f.width, height: f.height, pixels: px };
    if let Some(png) = pasteroll_core::image::encode_png(&shot) {
        let _ = s.add_image_png_from(&png, now, Some("snippingtool.exe"));
    }
    // `--image FILE` (repeatable) adds pictures of your own, for trying the viewer on them
    let mut extra = 0;
    for w in args.windows(2).filter(|w| w[0] == "--image") {
        match std::fs::read(&w[1]) {
            Ok(png) if pasteroll_core::image::decode_png(&png).is_some() => {
                if s.add_image_png_from(&png, now, Some("seed")).is_ok() {
                    extra += 1;
                }
            }
            _ => eprintln!("not a PNG picture: {}", w[1]),
        }
    }
    println!("seeded {} clips, 1 file list and {} images into {}", clips.len() + 1, 2 + extra, db.display());
    0
}

fn status(db: &Path) -> i32 {
    let (s, cfg, _lock) = match open(db) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let st = s.stats();
    println!("history:        {}", db.display());
    println!("config:         {}", keystore::config_path(db).display());
    println!(
        "stored:         {} clips, {} pinned, {:.1} MB",
        st.clips,
        st.pins,
        st.bytes as f64 / MB as f64
    );
    match s.count_unreadable() {
        Ok(0) => {}
        Ok(n) => println!("DAMAGED:        {n} clip(s) cannot be opened (this blocks key rotation). Run `repair --yes` to delete them."),
        Err(e) => println!("damaged clips:  could not check ({e})"),
    }
    if let Some(dir) = db.parent() {
        let on = winsys::has_no_low_access_label(dir);
        println!("access label:   {}", if on { "set (confined programs cannot read the history)" } else { "not set" });
    }
    match keystore::rotation_summary(db, now_secs()) {
        Ok(pasteroll_core::RotationState::NotDue) => {
            println!("key rotation:   not due (every {} days)", keystore::ROTATE_EVERY_DAYS)
        }
        Ok(pasteroll_core::RotationState::DueByAge) => println!("key rotation:   due (will run at the next start)"),
        Ok(pasteroll_core::RotationState::Requested) => {
            println!("key rotation:   requested (will run at the next start)")
        }
        Err(e) => println!("key rotation:   unknown ({e})"),
    }
    println!(
        "recovery:       {}",
        if keystore::recovery_is_set_up(db) {
            "an export exists (it stays valid across key rotations)"
        } else {
            "none: run `key-export` so a reinstall or password reset cannot lose the history"
        }
    );
    println!("{}", config::describe(&cfg));
    0
}

fn rotate_key(db: &Path) -> i32 {
    if winsys::app_is_running() {
        return fail("Pasteroll is running. Quit it first: the running app holds the old key. (It rotates by itself at the next start when due.)");
    }
    match keystore::rotate_history(db, now_secs()) {
        Ok(r) => {
            println!(
                "master key replaced; {} clip(s) re-sealed{}",
                r.rows,
                if r.durable { "" } else { " (finishing at the next start)" }
            );
            0
        }
        Err(e) => fail(e),
    }
}

fn confirm(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok() && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn config_cmd(db: &Path, args: &[String]) -> i32 {
    let positional: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    match positional.first().map(|s| s.as_str()) {
        None | Some("show") => status(db),
        Some("path") => {
            println!("{}", keystore::config_path(db).display());
            0
        }
        Some("init") => {
            let path = keystore::config_path(db);
            if path.exists() {
                return fail(format!("{} already exists", path.display()));
            }
            match keystore::write_config_text(db, config::TEMPLATE) {
                Ok(()) => {
                    println!("wrote {}", path.display());
                    0
                }
                Err(e) => fail(e),
            }
        }
        Some("set") => {
            let (Some(key), Some(val)) = (positional.get(1), positional.get(2)) else {
                return fail("usage: config set KEY VALUE   (keys: max_clips, max_total_mb, max_age_days, max_clip_mb, max_pins, paused, size, mode)");
            };
            let text = match keystore::read_config_text(db) {
                Ok(t) => t,
                Err(e) => return fail(e),
            };
            let new_text = match config::set_value(&text, key, val) {
                Ok(t) => t,
                Err(problems) => return fail(problems.join("\n")),
            };
            let new_cfg = config::parse(&new_text).expect("validated by set_value");
            let (mut store, _old, _lock) = match open(db) {
                Ok(x) => x,
                Err(e) => return fail(e),
            };
            let plan = match store.plan_prune(&new_cfg.limits, now_secs()) {
                Ok(p) => p,
                Err(e) => return fail(e.to_string()),
            };
            if plan.clips > 0 {
                println!(
                    "This would delete {} clip(s), {:.1} MB ({} of them only because of age). Favorite clips are kept.",
                    plan.clips,
                    plan.bytes as f64 / MB as f64,
                    plan.by_age
                );
                if !flag(args, "--yes") && !confirm("Delete them?") {
                    println!("Nothing changed.");
                    return 3;
                }
            }
            if let Err(e) = keystore::write_config_text(db, &new_text) {
                return fail(e);
            }
            store.set_limits(new_cfg.limits).expect("validated");
            match store.maintain(now_secs()) {
                Ok(n) => {
                    if n > 0 {
                        // Deleted data stays decryptable under the old key until it is replaced.
                        let _ = store.request_rotation();
                    }
                    println!("Saved. {n} clip(s) removed. Restart Pasteroll for the running app to pick up the change.");
                    if n > 0 {
                        println!("The master key will be replaced at the next start, so what was removed cannot be recovered from old copies.");
                    }
                }
                Err(e) => return fail(e.to_string()),
            }
            for k in new_cfg.limits.above_ui_maximum() {
                println!("note: {k} is above what a settings window will offer; it is allowed here.");
            }
            0
        }
        Some(other) => fail(format!("unknown config command {other}")),
    }
}

fn exclude(db: &Path, args: &[String]) -> i32 {
    let Some(exe) = args.iter().skip(1).find(|a| !a.starts_with("--")) else {
        return fail("usage: exclude EXE   (for example: exclude keepass.exe)");
    };
    let text = match keystore::read_config_text(db) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    match config::add_excluded_app(&text, exe).and_then(|t| keystore::write_config_text(db, &t).map_err(|e| vec![e])) {
        Ok(()) => {
            println!("Copies made in {} will no longer be recorded. Restart Pasteroll to apply.", exe.to_ascii_lowercase());
            0
        }
        Err(p) => fail(p.join("\n")),
    }
}

fn clear(db: &Path, args: &[String]) -> i32 {
    if !flag(args, "--yes") {
        return fail("This deletes history. Run again with --yes (and --keep-pinned to keep pinned clips).");
    }
    let (mut s, _, _lock) = match open(db) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    match s.clear(flag(args, "--keep-pinned")) {
        Ok(n) => {
            println!("deleted {n} clip(s)");
            0
        }
        Err(e) => fail(e.to_string()),
    }
}

fn repair(db: &Path, args: &[String]) -> i32 {
    let (mut s, _, _lock) = match open(db) {
        Ok(x) => x, // opened with a key that verified, so "unreadable" really means damaged
        Err(e) => return fail(e),
    };
    let n = match s.count_unreadable() {
        Ok(n) => n,
        Err(e) => return fail(e.to_string()),
    };
    if n == 0 {
        println!("Nothing to repair: every clip opens.");
        return 0;
    }
    if !flag(args, "--yes") {
        return fail(format!("{n} clip(s) cannot be opened and would be deleted. Run again with --yes."));
    }
    match s.repair_unreadable() {
        Ok(n) => {
            println!("deleted {n} damaged clip(s)");
            0
        }
        Err(e) => fail(e.to_string()),
    }
}

/// Reads a passphrase without echo from the console, or from a file for automation (a file is
/// never a good place for a passphrase; this exists for scripted use only).
fn read_passphrase(args: &[String], confirm_twice: bool) -> Result<String, String> {
    use zeroize::Zeroize;
    if let Some(path) = value(args, "--passphrase-file") {
        let mut text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let out = text.trim_end_matches(['\r', '\n']).to_owned();
        text.zeroize();
        return Ok(out);
    }
    if !std::io::stdin().is_terminal() {
        return Err("a passphrase is needed: run this in a terminal, or pass --passphrase-file".into());
    }
    let mut first = rpassword::prompt_password("Passphrase: ").map_err(|e| e.to_string())?;
    if confirm_twice {
        let mut second = rpassword::prompt_password("Again: ").map_err(|e| e.to_string())?;
        let same = first == second;
        second.zeroize();
        if !same {
            first.zeroize();
            return Err("the two passphrases differ".into());
        }
    }
    Ok(first)
}

fn key_export(db: &Path, args: &[String]) -> i32 {
    use zeroize::Zeroize;
    let Some(out) = value(args, "--out") else {
        return fail("usage: key-export --out FILE");
    };
    let out = PathBuf::from(out);
    if out.exists() {
        return fail(format!("{} already exists; it will not be overwritten", out.display()));
    }
    let mut pass = match read_passphrase(args, true) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let contents = match keystore::export_contents(db) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let mut plain = contents.encode();
    drop(contents);
    let blob = pasteroll_core::export::wrap(&plain, &pass, pasteroll_core::export::KdfParams::default());
    plain.zeroize();
    pass.zeroize();
    let blob = match blob {
        Ok(b) => b,
        Err(e) => return fail(e.to_string()),
    };
    // create_new: never replace an existing file, even one that appeared just now
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
        .and_then(|mut f| f.write_all(&blob).and_then(|_| f.sync_all()));
    match written {
        Ok(()) => {
            println!("Key exported to {}.", out.display());
            println!("It stays valid when the key is replaced automatically, as long as you also keep a recent");
            println!("copy of the history. Making a new export replaces the previous recovery, so keep only the");
            println!("latest one. Keep it and the passphrase away from this PC (BitLocker also helps).");
            0
        }
        Err(e) => fail(format!("cannot write {}: {e}", out.display())),
    }
}

fn key_import(db: &Path, args: &[String]) -> i32 {
    use zeroize::Zeroize;
    let Some(input) = value(args, "--in") else {
        return fail("usage: key-import --in FILE [--force]");
    };
    let blob = match std::fs::read(&input) {
        Ok(b) => b,
        Err(e) => return fail(format!("cannot read {input}: {e}")),
    };
    let mut pass = match read_passphrase(args, false) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let plain = pasteroll_core::export::unwrap(&blob, &pass);
    pass.zeroize();
    let mut plain = match plain {
        Ok(p) => p,
        Err(e) => return fail(e.to_string()),
    };
    let contents = pasteroll_core::export::ExportContents::decode(&plain);
    plain.zeroize();
    let Some(contents) = contents else {
        return fail("the export is damaged");
    };
    match keystore::import_contents(db, &contents, flag(args, "--force")) {
        Ok(()) => {
            println!("Key imported. The history can be opened again.");
            0
        }
        Err(e) => fail(e),
    }
}

fn run_app(db: PathBuf, args: &[String]) -> i32 {
    app::run(app::Options {
        db,
        capture: !flag(args, "--no-capture"),
        // Synthetic input can drive the whole app, so only debug builds will honour it.
        accept_injected: cfg!(debug_assertions) && flag(args, "--accept-injected"),
        max_seconds: value(args, "--max-seconds").and_then(|v| v.parse().ok()),
        hooks: true,
        tray: true,
        notice: !flag(args, "--no-notice"),
    })
}

fn main() {
    // The end-to-end test build, started again after a failed Setup: leave a note and stop, so that
    // the test never runs a second real tray program.
    #[cfg(feature = "e2e-test-server")]
    if let Ok(note) = std::env::var("PASTEROLL_E2E_RESTART_NOTE") {
        if std::env::args().nth(1).is_none() {
            let _ = std::fs::write(note, "restarted");
            return;
        }
    }
    // started from the Start menu or Explorer, the app would sit behind an empty console window:
    // let go of a console that is ours alone (a terminal's console is shared, so it stays)
    if matches!(std::env::args().nth(1).as_deref(), None | Some("run"))
        || (std::env::args().nth(1).as_deref() == Some("about")
            && !std::env::args().any(|a| a == "--text"))
    {
        setup::detach_console_if_owned();
    }
    winsys::harden_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let db = match std::env::current_exe().map_err(|e| e.to_string()).and_then(|exe| {
        pasteroll_core::paths::database(&exe, value(&args, "--db").map(PathBuf::from), keystore::default_db)
    }) {
        Ok(p) => p,
        Err(e) => std::process::exit(fail(e)),
    };
    let code = match cmd {
        "--version" | "-V" | "version" => {
            println!("pasteroll {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "about" => {
            if flag(&args, "--text") {
                println!("{}", about::text());
            } else {
                about::show(None);
            }
            0
        }
        "probe" => probe::run(),
        "update-check" => update::command(),
        "update" => update::install_command(&db, flag(&args, "--yes")),
        // the administrator half of an update for a copy in Program Files (see elevated.rs)
        "apply-update" => elevated::apply_update(args.get(1).map(String::as_str), args.get(2..).unwrap_or(&[])),
        "update-verify" => match (args.get(1), args.get(2), args.get(3), args.get(4)) {
            (Some(m), Some(s), None, None) => update::verify_command(Path::new(m), Path::new(s), None),
            (Some(m), Some(s), Some(n), Some(ns)) => update::verify_command(Path::new(m), Path::new(s), Some((Path::new(n), Path::new(ns)))),
            _ => fail("usage: update-verify MANIFEST SIGNATURE [NOTE NOTE_SIGNATURE]"),
        },
        // debug aid: a picture of the settings window
        "settings-snapshot" => {
            unsafe {
                let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                    windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                );
            }
            match value(&args, "--out") {
                Some(out) => match settings::snapshot(&db, Path::new(&out)) {
                    Ok(()) => 0,
                    Err(e) => fail(e),
                },
                None => fail("usage: settings-snapshot --out FILE.png [--db FILE]"),
            }
        }
        #[cfg(feature = "e2e-test-server")]
        "update-window-e2e" => updatewin::e2e(&db),
        // debug aid: a picture of the About window or the update question
        "dialog-snapshot" => {
            unsafe {
                let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                    windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                );
            }
            match (value(&args, "--what"), value(&args, "--out")) {
                (Some(what), Some(out)) => match themeddlg::snapshot(&what, Path::new(&out)) {
                    Ok(()) => 0,
                    Err(e) => fail(e),
                },
                _ => fail("usage: dialog-snapshot --what about|question --out FILE.png"),
            }
        }
        // debug aid: a picture of the update window on one of its pages (no network)
        "update-window-snapshot" => {
            unsafe {
                let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                    windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                );
            }
            match (value(&args, "--page"), value(&args, "--out")) {
                (Some(page), Some(out)) => match updatewin::snapshot(&page, Path::new(&out)) {
                    Ok(()) => 0,
                    Err(e) => fail(e),
                },
                _ => fail("usage: update-window-snapshot --page PAGE --out FILE.png"),
            }
        }
        "clipboard-selftest" => selftest::run(),
        "seed" => seed(&db, &args),
        "status" => status(&db),
        "config" => config_cmd(&db, &args),
        "exclude" => exclude(&db, &args),
        "clear" => clear(&db, &args),
        "repair" => repair(&db, &args),
        "rotate-key" => rotate_key(&db),
        "key-export" => key_export(&db, &args),
        "key-import" => key_import(&db, &args),
        "reset-history" => {
            if !flag(&args, "--yes") {
                eprintln!("This deletes {} and its key. Run again with --yes to confirm.", db.display());
                2
            } else {
                match keystore::reset_history(&db) {
                    Ok(r) => {
                        println!("removed {} file(s)", r.len());
                        0
                    }
                    Err(e) => fail(e),
                }
            }
        }
        "run" => run_app(db, &args),
        // started by double-click: offer to install, or just run when already installed
        "" => {
            if pasteroll_core::paths::is_portable() || setup::running_installed() || cfg!(debug_assertions) || !setup::prompt_install() {
                run_app(db, &args)
            } else {
                match setup::install() {
                    Ok(msg) => {
                        eprintln!("{msg}");
                        match setup::installed_exe() {
                            Ok(exe) => {
                                use std::os::windows::process::CommandExt;
                                let _ = std::process::Command::new(exe)
                                    .arg("run")
                                    .creation_flags(0x0000_0008)
                                    .spawn();
                                0
                            }
                            Err(e) => fail(e),
                        }
                    }
                    Err(e) => fail(e),
                }
            }
        }
        "stop" => {
            let expected = value(&args, "--if-exe");
            if flag(&args, "--if-exe") && expected.is_none() {
                fail("--if-exe requires an executable path.")
            } else if expected.as_deref().map_or_else(setup::stop_running, |p| setup::stop_installation(Path::new(p))) {
                0
            } else {
                fail("Pasteroll did not quit. Quit it from the tray icon and try again.")
            }
        },
        "install" => match if flag(&args, "--setup-adopt") { setup::adopt_setup() } else { setup::install() } {
            Ok(msg) => {
                println!("{msg}");
                0
            }
            Err(e) => fail(e),
        },
        "uninstall" => {
            let remove = flag(&args, "--remove-data");
            let data_only = flag(&args, "--data-only");
            if data_only && (!remove || !flag(&args, "--yes")) {
                fail("--data-only requires --remove-data --yes.")
            } else if data_only {
                match setup::remove_user_data(&db) {
                    Ok(n) => { println!("Removed {n} data file(s). The program was kept."); 0 }
                    Err(e) => fail(e),
                }
            } else if remove && !flag(&args, "--yes") {
                eprintln!("--remove-data deletes your history and settings in {}. Add --yes to confirm.", db.parent().map(|p| p.display().to_string()).unwrap_or_default());
                2
            } else {
                match setup::uninstall(remove, &db) {
                    Ok(msg) => {
                        println!("{msg}");
                        0
                    }
                    Err(e) => fail(e),
                }
            }
        }
        "autostart" => match args.get(1).map(String::as_str) {
            Some("on") => match setup::set_autostart(true) {
                Ok(()) => {
                    println!("Pasteroll will start when you sign in to Windows.");
                    0
                }
                Err(e) => fail(e),
            },
            Some("off") => match setup::set_autostart(false) {
                Ok(()) => {
                    println!("Pasteroll will no longer start with Windows.");
                    0
                }
                Err(e) => fail(e),
            },
            _ => {
                println!("start with Windows: {}", if setup::autostart_enabled() { "on" } else { "off" });
                0
            }
        },
        // Test support, debug builds only: end this process in a controlled way (tests/crash.rs).
        #[cfg(debug_assertions)]
        "crash-test" => match args.get(1).map(String::as_str) {
            // the crash note goes where the app writes it: the history folder's pasteroll.log
            _ if { if let Some(d) = db.parent() { winsys::set_crash_log(d.join("pasteroll.log")) } false } => 0,
            Some("panic") => panic!("simulated failure containing clip text: hunter2"),
            Some("access") => unsafe {
                std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1);
                0
            },
            Some("oom") => {
                // 64 TiB: the allocation fails, and must end the process without a dump
                let v: Vec<u8> = Vec::with_capacity(1usize << 46);
                std::hint::black_box(&v);
                0
            }
            _ => usage(),
        },
        _ => usage(),
    };
    std::process::exit(code);
}
