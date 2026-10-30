//! Pasteroll application logic, independent of any operating system.
//!
//! A platform backend implements [`Platform`], turns OS input into [`Event`]s and drives a
//! [`Controller`]. The controller decides; the platform only senses and acts.

pub mod controller;
pub mod event;
pub mod files;
pub mod filters;
pub mod pastekeys;
pub mod platform;
pub mod search;
pub mod terminals;
pub mod workers;

pub use controller::Controller;
pub use event::{Event, Key, MouseEvent, MouseKind, Wheel};
pub use pastekeys::{PasteKey, PasteKeys};
pub use terminals::PastePolicy;
pub use platform::{
    Captured, ClickPoll, ClipboardSource, Modifiers, OverlayInfo, OverlayMoved, Payload, Platform, SaveChoice, Skip, Target,
    TextRecognizer, Waker,
};
pub use workers::{IngestPolicy, Logger, StoreFactory};

#[cfg(test)]
mod tests;
