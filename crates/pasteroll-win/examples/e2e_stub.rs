//! A stand-in for the new Pasteroll in the end-to-end update test (never shipped): it only records
//! that it was started, so the test can check that Setup relaunched it, and answers `stop` and
//! `--version` the way Setup expects.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--version") => println!("pasteroll 99.0.0-beta.1"),
        None | Some("run") => {
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    let _ = std::fs::write(dir.join("relaunched.txt"), args.join(" "));
                    // an all-users copy lives in Program Files, which an ordinary program cannot write to:
                    // also leave the note in the temp folder, named after the folder it ran from
                    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let admin = unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() };
                    let _ = std::fs::write(std::env::temp_dir().join(format!("pasteroll-e2e-relaunched-{folder}.txt")), format!("elevated={}", u8::from(admin)));
                }
            }
        }
        // used as a Setup that fails: the test serves this file as the installer to see what happens
        // when Setup does not succeed
        // (exit code 1, or what PASTEROLL_E2E_SETUP_EXIT says: -1 stands for a crashed or killed Setup)
        Some("/VERYSILENT") => std::process::exit(std::env::var("PASTEROLL_E2E_SETUP_EXIT").ok().and_then(|v| v.parse().ok()).unwrap_or(1)),
        // `stop --if-exe FILE` and anything else: nothing is running, so nothing to do
        _ => {}
    }
}
