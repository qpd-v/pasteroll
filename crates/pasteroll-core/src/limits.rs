//! How much history is kept.
//!
//! Whichever limit is hit first prunes the oldest *unpinned* clips. There is deliberately no
//! "unlimited": an app that silently grows to tens of gigabytes of encrypted clipboard data is a
//! surprise nobody wants. A settings UI offers values up to the "UI maximum"; the config file may go
//! up to the hard maximum, and anything beyond that is rejected when the file is loaded.

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub const DEFAULT_CLIPS: u64 = 1_000;
pub const UI_MAX_CLIPS: u64 = 100_000;
pub const HARD_MAX_CLIPS: u64 = 1_000_000;

pub const DEFAULT_BYTES: u64 = 500 * MB;
pub const UI_MAX_BYTES: u64 = 10 * GB;
pub const HARD_MAX_BYTES: u64 = 100 * GB;

pub const DEFAULT_AGE_DAYS: u32 = 90;
pub const HARD_MAX_AGE_DAYS: u32 = 36_500;

pub const DEFAULT_CLIP_BYTES: u64 = 16 * MB;
pub const UI_MAX_CLIP_BYTES: u64 = 64 * MB;
/// 256 MB needs chunked sealing (a clip is sealed in one piece, so peak memory is ~3x its size).
pub const HARD_MAX_CLIP_BYTES: u64 = 64 * MB;

pub const DEFAULT_PINS: u32 = 200;
pub const UI_MAX_PINS: u32 = 1_000;
pub const HARD_MAX_PINS: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Unpinned clips kept.
    pub max_clips: u64,
    /// Total stored size of unpinned clips, in bytes.
    pub max_bytes: u64,
    /// `None` = never expire.
    pub max_age_days: Option<u32>,
    /// Largest single clip.
    pub max_clip_bytes: u64,
    /// Pinned clips allowed (pins are never auto-deleted and do not count toward the other limits).
    pub max_pins: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_clips: DEFAULT_CLIPS,
            max_bytes: DEFAULT_BYTES,
            max_age_days: Some(DEFAULT_AGE_DAYS),
            max_clip_bytes: DEFAULT_CLIP_BYTES,
            max_pins: DEFAULT_PINS,
        }
    }
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn fmt_bytes(n: u64) -> String {
    if n >= GB && n % GB == 0 {
        format!("{} GB", n / GB)
    } else {
        format!("{} MB", n / MB)
    }
}

impl Limits {
    /// Every problem with these values, as sentences a person can act on. Empty = valid.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.max_clips < 1 || self.max_clips > HARD_MAX_CLIPS {
            p.push(format!(
                "max_clips must be between 1 and {} (got {})",
                fmt_int(HARD_MAX_CLIPS),
                fmt_int(self.max_clips)
            ));
        }
        if self.max_bytes < MB || self.max_bytes > HARD_MAX_BYTES {
            p.push(format!(
                "max_total_mb must be between 1 MB and {} (got {})",
                fmt_bytes(HARD_MAX_BYTES),
                fmt_bytes(self.max_bytes)
            ));
        }
        if let Some(d) = self.max_age_days {
            if d < 1 || d > HARD_MAX_AGE_DAYS {
                p.push(format!("max_age_days must be between 1 and {HARD_MAX_AGE_DAYS}, or \"never\" (got {d})"));
            }
        }
        if self.max_clip_bytes < MB || self.max_clip_bytes > HARD_MAX_CLIP_BYTES {
            p.push(format!(
                "max_clip_mb must be between 1 and {} (got {})",
                HARD_MAX_CLIP_BYTES / MB,
                self.max_clip_bytes / MB
            ));
        }
        if self.max_pins > HARD_MAX_PINS {
            p.push(format!("max_pins must be at most {} (got {})", fmt_int(u64::from(HARD_MAX_PINS)), self.max_pins));
        }
        if self.max_clip_bytes > self.max_bytes {
            p.push("max_clip_mb cannot be larger than max_total_mb".to_owned());
        }
        p
    }

    /// Settings above what a settings UI would offer (allowed in the config file).
    pub fn above_ui_maximum(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.max_clips > UI_MAX_CLIPS {
            v.push("max_clips");
        }
        if self.max_bytes > UI_MAX_BYTES {
            v.push("max_total_mb");
        }
        if self.max_clip_bytes > UI_MAX_CLIP_BYTES {
            v.push("max_clip_mb");
        }
        if self.max_pins > UI_MAX_PINS {
            v.push("max_pins");
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_inside_the_ui_range() {
        let l = Limits::default();
        assert!(l.problems().is_empty());
        assert!(l.above_ui_maximum().is_empty());
        assert_eq!(l.max_clips, 1_000);
        assert_eq!(l.max_bytes, 500 * MB);
        assert_eq!(l.max_age_days, Some(90));
        assert_eq!(l.max_clip_bytes, 16 * MB);
    }

    #[test]
    fn hard_maxima_are_accepted_and_one_past_is_rejected() {
        let ok = Limits { max_clips: HARD_MAX_CLIPS, max_bytes: HARD_MAX_BYTES, max_age_days: None, ..Limits::default() };
        assert!(ok.problems().is_empty(), "{:?}", ok.problems());
        let bad = Limits { max_clips: HARD_MAX_CLIPS + 1, ..Limits::default() };
        assert!(bad.problems()[0].contains("max_clips"));
        let bad = Limits { max_bytes: HARD_MAX_BYTES + MB, ..Limits::default() };
        assert!(bad.problems()[0].contains("max_total_mb"));
        let bad = Limits { max_clip_bytes: HARD_MAX_CLIP_BYTES + MB, ..Limits::default() };
        assert!(bad.problems().iter().any(|p| p.contains("max_clip_mb")));
    }

    #[test]
    fn zero_is_never_valid() {
        assert!(!Limits { max_clips: 0, ..Limits::default() }.problems().is_empty());
        assert!(!Limits { max_age_days: Some(0), ..Limits::default() }.problems().is_empty());
        assert!(!Limits { max_bytes: 0, ..Limits::default() }.problems().is_empty());
    }

    #[test]
    fn a_clip_cannot_exceed_the_whole_budget() {
        let l = Limits { max_bytes: 8 * MB, max_clip_bytes: 16 * MB, ..Limits::default() };
        assert!(l.problems().iter().any(|p| p.contains("cannot be larger")));
    }

    #[test]
    fn values_between_ui_and_hard_maximum_are_flagged_but_valid() {
        let l = Limits { max_clips: 500_000, max_bytes: 50 * GB, ..Limits::default() };
        assert!(l.problems().is_empty());
        assert_eq!(l.above_ui_maximum(), vec!["max_clips", "max_total_mb"]);
    }

    #[test]
    fn messages_use_readable_numbers() {
        let p = Limits { max_clips: 2_000_000, ..Limits::default() }.problems();
        assert!(p[0].contains("1,000,000") && p[0].contains("2,000,000"), "{}", p[0]);
    }
}
