//! A crash must end the process immediately and must not leave a memory dump behind.

use std::path::PathBuf;
use std::process::Command;

const CRASH_EXIT_CODE: i32 = 0xC0DE_0001u32 as i32;

fn run(kind: &str) -> (std::process::Output, i32) {
    let exe = env!("CARGO_BIN_EXE_pasteroll");
    let out = Command::new(exe).args(["crash-test", kind]).output().expect("spawn");
    let code = out.status.code().unwrap_or(-1);
    (out, code)
}

fn dump_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("CrashDumps"))
}

fn pasteroll_dumps() -> usize {
    dump_dir()
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|it| {
            it.filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("pasteroll"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn a_panic_ends_the_process_without_a_dump_and_without_printing_the_message() {
    let before = pasteroll_dumps();
    let (out, code) = run("panic");
    assert_eq!(code, CRASH_EXIT_CODE, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("internal error at"), "location should be reported: {err}");
    assert!(!err.contains("hunter2"), "the panic message (which may hold data) must not be printed: {err}");
    assert_eq!(pasteroll_dumps(), before, "no crash dump may be written");
}

#[test]
fn an_access_violation_ends_the_process_without_a_dump() {
    let before = pasteroll_dumps();
    let (_out, code) = run("access");
    assert_eq!(code, CRASH_EXIT_CODE);
    assert_eq!(pasteroll_dumps(), before, "no crash dump may be written");
}

#[test]
fn running_out_of_memory_ends_the_process_without_a_dump() {
    let before = pasteroll_dumps();
    let (_out, code) = run("oom");
    assert_eq!(code, CRASH_EXIT_CODE, "an allocation failure must not become a Windows fast-fail");
    assert_eq!(pasteroll_dumps(), before, "no crash dump may be written");
}
