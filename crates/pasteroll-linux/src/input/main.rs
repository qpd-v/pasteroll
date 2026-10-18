//! Optional host input service. It never opens application data or a session bus.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod device;
#[cfg(any(test, all(target_os = "linux", target_env = "gnu")))]
mod gesture;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod service;

fn main() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    let result = service::main();
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    let result: Result<(), &'static str> =
        Err("the input helper requires Linux with glibc and libevdev2");
    if let Err(message) = result {
        eprintln!("pasteroll-input: {message}");
        std::process::exit(1);
    }
}
