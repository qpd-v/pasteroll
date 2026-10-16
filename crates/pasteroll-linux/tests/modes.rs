#![cfg(unix)]

use std::process::Command;

#[test]
fn portable_cli_keeps_data_local_and_refuses_installation_changes() {
    let d = std::env::temp_dir().join(format!("pasteroll-portable-cli-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let exe = d.join("pasteroll");
    std::fs::copy(env!("CARGO_BIN_EXE_pasteroll"), &exe).unwrap();
    std::fs::write(d.join("portable.txt"), b"").unwrap();
    let out = Command::new(&exe).args(["config", "path"]).output().unwrap();
    assert!(out.status.success());
    assert!(std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()) == d.join("data/config.toml"),
        "the config path must stay beside the copied executable");
    for args in [&["install"][..], &["uninstall"], &["autostart", "on"], &["autostart", "off"], &["status", "--db", "elsewhere.db"]] {
        let out = Command::new(&exe).args(args).output().unwrap();
        assert!(!out.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("portable mode"), "{args:?}");
    }
    let out = Command::new(&exe).arg("stop").output().unwrap();
    assert!(out.status.success());
    assert!(!d.join("data").exists());
    std::fs::remove_dir_all(d).unwrap();
}
