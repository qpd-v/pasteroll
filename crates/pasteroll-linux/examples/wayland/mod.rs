//! Narrow include boundary for the integration probe.
//!
//! The production module also contains the full controller runtime, which
//! deliberately depends on the binary crate root. The probe needs only the
//! wire client and protocol, so it must not pull that runtime into an example
//! crate.

#[allow(dead_code)]
#[path = "../../src/wayland/bridge.rs"]
pub mod bridge;
#[path = "../../src/wayland/protocol.rs"]
pub mod protocol;
