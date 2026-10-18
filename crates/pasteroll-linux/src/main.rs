//! Pasteroll for Linux (X11 and GNOME Wayland).
//!
//! On X11, `pasteroll run` opens the carousel with Ctrl+Shift+middle-click. Native GNOME Wayland
//! uses `pasteroll run --backend gnome` with the host mouse helper, or the explicit
//! `--wayland-keyboard` alternative that arms the companion's keyboard shortcut.
//! Explicit X11 mode remains
//! available for X11 and XWayland applications.

#[cfg(not(unix))]
fn main() {
    eprintln!("pasteroll-linux only runs on Linux");
    std::process::exit(2);
}

#[cfg(unix)]
mod clipboard;
#[cfg(unix)]
mod dialogs;
#[cfg(unix)]
mod install;
#[cfg(unix)]
mod keys;
#[cfg(unix)]
mod platform;
#[cfg(unix)]
mod run;
#[cfg(unix)]
mod shell;
#[cfg(unix)]
mod sni;
#[cfg(unix)]
mod sys;
#[cfg(unix)]
mod x;
// Foundations are portable enough for protocol tests on Windows; the transport is Unix-only.
#[allow(dead_code)]
mod wayland;

#[cfg(unix)]
mod cli {
    use crate::sys::{self, LinuxOs};
    use pasteroll_core::config;
    use pasteroll_core::keystore as ks;
    use pasteroll_core::limits::MB;
    use pasteroll_core::Store;
    use std::io::{IsTerminal, Write};
    use std::path::{Path, PathBuf};

    fn flag(args: &[String], name: &str) -> bool {
        args.iter().any(|a| a == name)
    }

    fn value(args: &[String], name: &str) -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    }

    fn fail(msg: impl AsRef<str>) -> i32 {
        eprintln!("{}", msg.as_ref());
        1
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn confirm(question: &str) -> bool {
        if !std::io::stdin().is_terminal() {
            return false;
        }
        eprint!("{question} [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).is_ok()
            && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }

    fn open(
        db: &Path,
    ) -> Result<
        (
            Store,
            config::Config,
            <LinuxOs as pasteroll_core::keystore::Os>::Lock,
        ),
        String,
    > {
        let lock = ks::lock_history::<LinuxOs>(db, false)?;
        let cfg = ks::load_config(db).map_err(|p| {
            format!(
                "{} is not valid:\n  - {}",
                ks::config_path(db).display(),
                p.join("\n  - ")
            )
        })?;
        let mut store = ks::open_store::<LinuxOs>(db)?;
        store.set_limits(cfg.limits).map_err(|e| e.to_string())?;
        Ok((store, cfg, lock))
    }

    fn app_is_running(db: &Path) -> bool {
        sys::single_instance(db).is_err()
    }

    fn seed(db: &Path) -> i32 {
        let clips = [
            "The quick brown fox jumps over the lazy dog. A second sentence follows it.",
            "fn greet(name: &str) -> String {\n    format!(\"Hello, {name}!\")\n}",
            "https://example.com/docs/getting-started",
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
        s.add_files(&["/demo/Documents/invoice-2026-10.pdf".into()], now)
            .ok();
        let mut px = Vec::new();
        for y in 0..120u32 {
            for x in 0..200u32 {
                px.extend_from_slice(&[(x * 255 / 200) as u8, (y * 255 / 120) as u8, 160, 255]);
            }
        }
        let img = pasteroll_core::image::Rgba {
            width: 200,
            height: 120,
            pixels: px,
        };
        if let Some(png) = pasteroll_core::image::encode_png(&img) {
            let _ = s.add_image_png(&png, now);
        }
        println!(
            "seeded {} clips, 1 file list and 1 image into {}",
            clips.len(),
            db.display()
        );
        0
    }

    fn status(db: &Path) -> i32 {
        let (s, cfg, _lock) = match open(db) {
            Ok(x) => x,
            Err(e) => return fail(e),
        };
        let st = s.stats();
        println!("history:        {}", db.display());
        println!("config:         {}", ks::config_path(db).display());
        println!(
            "stored:         {} clips, {} pinned, {:.1} MB",
            st.clips,
            st.pins,
            st.bytes as f64 / MB as f64
        );
        match s.count_unreadable() {
            Ok(0) => {}
            Ok(n) => println!(
                "DAMAGED:        {n} clip(s) cannot be opened. Run `repair --yes` to delete them."
            ),
            Err(e) => println!("damaged clips:  could not check ({e})"),
        }
        match ks::rotation_summary::<LinuxOs>(db, now_secs()) {
            Ok(pasteroll_core::RotationState::NotDue) => {
                println!(
                    "key rotation:   not due (every {} days)",
                    ks::ROTATE_EVERY_DAYS
                )
            }
            Ok(pasteroll_core::RotationState::DueByAge) => {
                println!("key rotation:   due (will run at the next start)")
            }
            Ok(pasteroll_core::RotationState::Requested) => {
                println!("key rotation:   requested (will run at the next start)")
            }
            Err(e) => println!("key rotation:   unknown ({e})"),
        }
        println!(
            "recovery:       {}",
            if ks::recovery_is_set_up::<LinuxOs>(db) {
                "an export exists (it stays valid across key rotations)"
            } else {
                "none: run `key-export` so losing the key file cannot lose the history"
            }
        );
        println!("{}", config::describe(&cfg));
        0
    }

    fn config_cmd(db: &Path, args: &[String]) -> i32 {
        let positional: Vec<&String> = args
            .iter()
            .skip(1)
            .filter(|a| !a.starts_with("--"))
            .collect();
        match positional.first().map(|s| s.as_str()) {
            None | Some("show") => status(db),
            Some("path") => {
                println!("{}", ks::config_path(db).display());
                0
            }
            Some("init") => {
                let path = ks::config_path(db);
                if path.exists() {
                    return fail(format!("{} already exists", path.display()));
                }
                match ks::write_config_text::<LinuxOs>(db, config::TEMPLATE) {
                    Ok(()) => {
                        println!("wrote {}", path.display());
                        0
                    }
                    Err(e) => fail(e),
                }
            }
            Some("set") => {
                let (Some(key), Some(val)) = (positional.get(1), positional.get(2)) else {
                    return fail("usage: config set KEY VALUE   (keys: max_clips, max_total_mb, max_age_days, max_clip_mb, max_pins, paused, warn_multiline_in_terminals, size)");
                };
                let text = match ks::read_config_text(db) {
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
                        "This would delete {} clip(s), {:.1} MB ({} of them only because of age). Pinned clips are kept.",
                        plan.clips,
                        plan.bytes as f64 / MB as f64,
                        plan.by_age
                    );
                    if !flag(args, "--yes") && !confirm("Delete them?") {
                        println!("Nothing changed.");
                        return 3;
                    }
                }
                if let Err(e) = ks::write_config_text::<LinuxOs>(db, &new_text) {
                    return fail(e);
                }
                store.set_limits(new_cfg.limits).expect("validated");
                match store.maintain(now_secs()) {
                    Ok(n) => {
                        if n > 0 {
                            let _ = store.request_rotation();
                        }
                        println!("Saved. {n} clip(s) removed. Restart Pasteroll for the running app to pick up the change.");
                    }
                    Err(e) => return fail(e.to_string()),
                }
                0
            }
            Some(other) => fail(format!("unknown config command {other}")),
        }
    }

    fn exclude(db: &Path, args: &[String]) -> i32 {
        let Some(exe) = args.iter().skip(1).find(|a| !a.starts_with("--")) else {
            return fail("usage: exclude PROGRAM   (for example: exclude keepassxc)");
        };
        let text = match ks::read_config_text(db) {
            Ok(t) => t,
            Err(e) => return fail(e),
        };
        match config::add_excluded_app(&text, exe)
            .and_then(|t| ks::write_config_text::<LinuxOs>(db, &t).map_err(|e| vec![e]))
        {
            Ok(()) => {
                println!(
                    "Copies made in {} will no longer be recorded. Restart Pasteroll to apply.",
                    exe.to_ascii_lowercase()
                );
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
            Ok(x) => x,
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
            return fail(format!(
                "{n} clip(s) cannot be opened and would be deleted. Run again with --yes."
            ));
        }
        match s.repair_unreadable() {
            Ok(n) => {
                println!("deleted {n} damaged clip(s)");
                0
            }
            Err(e) => fail(e.to_string()),
        }
    }

    fn rotate_key(db: &Path) -> i32 {
        if app_is_running(db) {
            return fail("Pasteroll is running. Quit it first: the running app holds the old key. (It rotates by itself at the next start when due.)");
        }
        match ks::rotate_history::<LinuxOs>(db, now_secs()) {
            Ok(r) => {
                println!(
                    "master key replaced; {} clip(s) re-sealed{}",
                    r.rows,
                    if r.durable {
                        ""
                    } else {
                        " (finishing at the next start)"
                    }
                );
                0
            }
            Err(e) => fail(e),
        }
    }

    /// Reads a passphrase without echo from the terminal, or from a file for automation (a file is
    /// never a good place for a passphrase; this exists for scripted use only).
    fn read_passphrase(args: &[String], confirm_twice: bool) -> Result<String, String> {
        use zeroize::Zeroize;
        if let Some(path) = value(args, "--passphrase-file") {
            let mut text =
                std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let out = text.trim_end_matches(['\r', '\n']).to_owned();
            text.zeroize();
            return Ok(out);
        }
        if !std::io::stdin().is_terminal() {
            return Err(
                "a passphrase is needed: run this in a terminal, or pass --passphrase-file".into(),
            );
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
            return fail(format!(
                "{} already exists; it will not be overwritten",
                out.display()
            ));
        }
        let mut pass = match read_passphrase(args, true) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let contents = match ks::export_contents::<LinuxOs>(db) {
            Ok(c) => c,
            Err(e) => {
                pass.zeroize();
                return fail(e);
            }
        };
        let mut plain = contents.encode();
        drop(contents);
        let blob = pasteroll_core::export::wrap(
            &plain,
            &pass,
            pasteroll_core::export::KdfParams::default(),
        );
        plain.zeroize();
        pass.zeroize();
        let blob = match blob {
            Ok(b) => b,
            Err(e) => return fail(e.to_string()),
        };
        // create_new: never replace an existing file, even one that appeared just now; 0600
        use std::os::unix::fs::OpenOptionsExt;
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&out)
            .and_then(|mut f| f.write_all(&blob).and_then(|_| f.sync_all()));
        match written {
            Ok(()) => {
                println!("Key exported to {}.", out.display());
                println!("It stays valid when the key is replaced automatically, as long as you also keep a recent");
                println!("copy of the history. Making a new export replaces the previous recovery, so keep only the");
                println!("latest one. Keep it and the passphrase away from this computer.");
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
        if app_is_running(db) {
            return fail("Pasteroll is running. Quit it first.");
        }
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
        match ks::import_contents::<LinuxOs>(db, &contents, flag(args, "--force")) {
            Ok(()) => {
                println!("Key imported. The history can be opened again.");
                0
            }
            Err(e) => fail(e),
        }
    }

    fn usage() -> i32 {
        eprintln!(
            "usage: pasteroll run | status | config [show|path|init|set KEY VALUE] | exclude PROGRAM | \
             clear --yes [--keep-pinned] | repair --yes | rotate-key | key-export --out FILE | \
             key-import --in FILE [--force] | reset-history --yes | seed | \
             stop | wayland-check | install | uninstall [--remove-data --yes] | autostart [on|off]\n\
             options: --db FILE   --no-capture   --max-seconds N   --yes\n\
             run options: --backend x11|gnome   --wayland-keyboard"
        );
        2
    }

    fn run_cmd(args: &[String], db: PathBuf) -> i32 {
        use crate::wayland::session::{self, Launch};
        if flag(args, "--backend") && value(args, "--backend").is_none() {
            return fail("--backend needs a value: x11 or gnome");
        }
        let requested = value(args, "--backend");
        let keyboard = flag(args, "--wayland-keyboard");
        match session::choose(requested.as_deref(), keyboard, session::current()) {
            Ok(Launch::X11) => crate::run::run(
                db,
                !flag(args, "--no-capture"),
                value(args, "--max-seconds").and_then(|v| v.parse().ok()),
            ),
            Ok(mode @ (Launch::GnomeKeyboard | Launch::GnomeMouse)) => crate::wayland::run::run(
                db,
                !flag(args, "--no-capture"),
                value(args, "--max-seconds").and_then(|v| v.parse().ok()),
                mode == Launch::GnomeKeyboard,
            ),
            Err(e) => fail(e),
        }
    }

    pub fn main() -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let cmd = args.first().map(String::as_str).unwrap_or("");
        // GNOME verifies /proc/PID/exe during its handshake. These two paths
        // authenticate before hardening, while no history or keys are open.
        // Both run backends harden before accessing private application data.
        if !matches!(cmd, "run" | "wayland-check") {
            sys::harden_process();
        }
        let db = match std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| {
                pasteroll_core::paths::database(
                    &exe,
                    value(&args, "--db").map(PathBuf::from),
                    sys::default_db,
                )
            }) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        match cmd {
            "wayland-check" => {
                use crate::wayland::session::{self, Backend};
                let result = match session::current() {
                    Backend::Gnome => match crate::wayland::bridge::check() {
                        Ok(()) => 0,
                        Err(e) => fail(e.to_string()),
                    },
                    Backend::UnsupportedWayland => {
                        fail("This Wayland desktop has no Pasteroll backend yet")
                    }
                    _ => fail("wayland-check requires a GNOME Wayland session"),
                };
                sys::harden_process();
                result
            }
            "stop" => match sys::stop_running(&db) {
                Ok(()) => 0,
                Err(e) => fail(e),
            },
            "run" => run_cmd(&args, db),
            "--version" | "-V" | "version" => {
                println!("pasteroll {}", env!("CARGO_PKG_VERSION"));
                0
            }
            "seed" => seed(&db),
            "status" => status(&db),
            "config" => config_cmd(&db, &args),
            "exclude" => exclude(&db, &args),
            "clear" => clear(&db, &args),
            "repair" => repair(&db, &args),
            "rotate-key" => rotate_key(&db),
            "key-export" => key_export(&db, &args),
            "key-import" => key_import(&db, &args),
            "install" => match crate::install::install() {
                Ok(m) => {
                    println!("{m}");
                    0
                }
                Err(e) => fail(e),
            },
            "uninstall" => {
                let remove = flag(&args, "--remove-data");
                if remove && !flag(&args, "--yes") {
                    eprintln!(
                        "--remove-data deletes your history and settings. Add --yes to confirm."
                    );
                    2
                } else {
                    match crate::install::uninstall(remove, db.parent()) {
                        Ok(m) => {
                            println!("{m}");
                            0
                        }
                        Err(e) => fail(e),
                    }
                }
            }
            "autostart" => match args.get(1).map(String::as_str) {
                Some("on" | "off")
                    if flag(&args, "--backend") && value(&args, "--backend").is_none() =>
                {
                    fail("--backend needs a value: x11 or gnome")
                }
                Some("on" | "off")
                    if value(&args, "--backend")
                        .as_deref()
                        .is_some_and(|b| b != "x11" && b != "gnome") =>
                {
                    fail("unknown backend; use x11 or gnome")
                }
                Some("on" | "off")
                    if value(&args, "--backend").as_deref() == Some("x11")
                        && flag(&args, "--wayland-keyboard") =>
                {
                    fail("--wayland-keyboard can only be used with --backend gnome")
                }
                Some("on") if value(&args, "--backend").as_deref() == Some("gnome") => {
                    match crate::install::set_wayland_autostart_mode(
                        true,
                        flag(&args, "--wayland-keyboard"),
                    ) {
                        Ok(()) => {
                            println!("Pasteroll will start with the GNOME Wayland backend when you sign in.");
                            0
                        }
                        Err(e) => fail(e),
                    }
                }
                Some("on") => match crate::install::set_autostart(true) {
                    Ok(()) => {
                        println!("Pasteroll will start when you sign in.");
                        0
                    }
                    Err(e) => fail(e),
                },
                Some("off") if value(&args, "--backend").as_deref() == Some("gnome") => {
                    match crate::install::set_wayland_autostart(false) {
                        Ok(()) => {
                            println!("Pasteroll will no longer start with the GNOME Wayland backend when you sign in.");
                            0
                        }
                        Err(e) => fail(e),
                    }
                }
                Some("off") => match crate::install::set_autostart(false) {
                    Ok(()) => {
                        println!("Pasteroll will no longer start when you sign in.");
                        0
                    }
                    Err(e) => fail(e),
                },
                _ => {
                    println!(
                        "start at sign-in: {}",
                        if crate::install::autostart_enabled() {
                            "on"
                        } else {
                            "off"
                        }
                    );
                    0
                }
            },
            "reset-history" => {
                if !flag(&args, "--yes") {
                    eprintln!(
                        "This deletes {} and its key. Run again with --yes to confirm.",
                        db.display()
                    );
                    2
                } else if app_is_running(&db) {
                    fail("Pasteroll is running. Quit it first.")
                } else {
                    match ks::reset_history::<LinuxOs>(&db) {
                        Ok(r) => {
                            println!("removed {} file(s)", r.len());
                            0
                        }
                        Err(e) => fail(e),
                    }
                }
            }
            _ => usage(),
        }
    }
}

#[cfg(unix)]
fn main() {
    std::process::exit(cli::main());
}
