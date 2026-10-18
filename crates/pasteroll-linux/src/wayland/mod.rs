//! GNOME Wayland backend: the Shell companion, with an optional host mouse helper.
#[cfg(unix)]
pub mod bridge;
#[cfg(unix)]
pub mod platform;
pub mod protocol;
#[cfg(unix)]
pub mod run;
pub mod session;
#[cfg(unix)]
pub mod settings;
