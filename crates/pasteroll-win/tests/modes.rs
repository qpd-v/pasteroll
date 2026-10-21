//! Run only harmless commands on copies with scratch marker files. Never start capture or stop
//! the user's running app, and never point a command at the user's history.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Copy(PathBuf);
impl Copy {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("pasteroll-modes-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_pasteroll"), d.join("pasteroll.exe")).unwrap();
        Self(d)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(self.0.join("pasteroll.exe")).args(args).output().unwrap()
    }
    fn marker(&self, name: &str) { std::fs::write(self.0.join(name), b"").unwrap(); }
}
impl Drop for Copy {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn portable_paths_and_installation_guards_work_through_the_cli() {
    let c = Copy::new();
    c.marker("portable.txt");
    c.marker("setup.txt"); // portable takes precedence
    let out = c.run(&["config", "path"]);
    assert!(out.status.success());
    assert!(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()) == c.0.join("data").join("config.toml"),
        "the config path must stay beside the copied executable");
    for args in [&["install"][..], &["uninstall"], &["autostart", "on"], &["autostart", "off"], &["status", "--db", "elsewhere.db"]] {
        let out = c.run(args);
        assert!(!out.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("portable mode"), "{args:?}");
    }
    assert!(!c.0.join("data").exists(), "these commands must not create a history");
}

#[test]
fn setup_owned_copies_delegate_install_and_uninstall() {
    let c = Copy::new();
    c.marker("setup.txt");
    let out = c.run(&["install"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("already installed by Setup"));
    let out = c.run(&["uninstall"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Settings > Apps"));
    assert!(c.0.join("pasteroll.exe").exists());
}

#[test]
fn setup_data_only_removal_requires_consent_and_keeps_the_program_and_unrelated_files() {
    let c = Copy::new();
    c.marker("setup.txt");
    let data = c.0.join("data");
    std::fs::create_dir_all(&data).unwrap();
    for name in ["custom.sqlite", "custom.sqlite-wal", "custom.sqlite-shm", "custom.sqlite-journal", "custom.key", "config.toml", "image-presets.txt",
        "image-presets.txt.tmp", "update-check.txt", "pasteroll.log", "unrelated.txt"] {
        std::fs::write(data.join(name), b"scratch fixture").unwrap();
    }
    let db = data.join("custom.sqlite");
    let db_arg = db.to_str().unwrap();
    for args in [vec!["uninstall", "--data-only", "--db", db_arg],
        vec!["uninstall", "--data-only", "--remove-data", "--db", db_arg]] {
        assert!(!c.run(&args).status.success());
        assert!(db.exists(), "missing consent must preserve the database");
    }
    let out = c.run(&["uninstall", "--data-only", "--remove-data", "--yes", "--db", db_arg]);
    assert!(out.status.success(), "data-only removal failed");
    assert!(c.0.join("pasteroll.exe").exists());
    assert!(c.0.join("setup.txt").exists());
    assert_eq!(std::fs::read_dir(&data).unwrap().count(), 1);
    assert!(data.join("unrelated.txt").exists());
}
