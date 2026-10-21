//! Pasteroll core: everything that is not Win32.
//!
//! - [`wheel`]   wheel-delta accumulation (high-resolution wheels step correctly)
//! - [`source`]  the history interface the carousel session reads through
//! - [`session`] the cycle state machine: snapshot, batches, sliding window, wrap, skip-deleted
//! - [`select`]  text selection over UTF-16 code units (what DirectWrite hit-testing returns)
//! - [`store`]   SQLite history (encrypted padded bodies, keyed dedupe, pins, limits, newest-first)
//! - [`limits`] / [`config`]  what is kept, and the user's config file
//! - [`padding`] / [`record`]  length hiding, and the small searchable description of a clip
//! - [`vault`]   encryption at rest (XChaCha20-Poly1305, HMAC fingerprints)
//! - [`image`]   DIB / PNG / RGBA conversion and thumbnails
//! - [`imgedit`] / [`imgfx`] / [`imgout`]  the Advanced Image Paste editor: adjustments, pixel operations, file formats

pub mod config;
pub mod export;
pub mod hdrop;
pub mod image;
pub mod imgedit;
pub mod imgfx;
pub mod imgout;
pub mod keyring;
pub mod keystore;
pub mod limits;
pub mod logo;
pub mod ocr_regions;
pub mod padding;
pub mod paths;
pub mod preview;
pub mod record;
pub mod recovery;
pub mod select;
pub mod settings;
pub mod session;
pub mod source;
pub mod store;
pub mod update;
pub mod vault;
pub mod wheel;

pub use select::Selection;
pub use session::Session;
pub use source::{Dir, HistorySource, Item, Snapshot};
pub use limits::Limits;
pub use store::{
    ClipInfo, KeyStatus, Kind, PinResult, Preview, PrunePlan, RotateReport, RotationState, SearchRow, Stats, Store,
};
pub use vault::Vault;
pub use wheel::{Step, WheelAccumulator};
