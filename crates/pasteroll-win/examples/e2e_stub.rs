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
        // `stop --if-exe FILE` and anything else: nothing is running, so nothing to do
        _ => {}
    }
}
