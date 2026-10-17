//! Wayland backend foundations. The normal controller loop is integrated separately.
#[cfg(unix)]
pub mod bridge;
pub mod protocol;
pub mod session;
