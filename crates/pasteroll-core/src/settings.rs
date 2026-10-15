//! What a settings window offers and how its choices become `config.toml` edits. The windows
//! themselves are platform code; this part is the same everywhere.

use crate::config::{self, Config};
use crate::limits::{Limits, MB};

const CLIP_STEPS: [u64; 10] = [100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000];
const SIZE_STEPS_MB: [u64; 8] = [50, 100, 250, 500, 1_024, 2_048, 5_120, 10_240];
const AGE_STEPS: [u32; 5] = [7, 30, 90, 180, 365];
const CLIP_SIZE_STEPS_MB: [u64; 5] = [1, 4, 16, 32, 64];
const SIZE_PERCENT_STEPS: [u32; 11] = [50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200];

/// The carousel sizes a slider offers, 100% in the middle (plus a value that was set in the file
/// and is not one of them, so saving never changes it).
pub fn size_choices(current: u32) -> Vec<u32> {
    let mut p = SIZE_PERCENT_STEPS.to_vec();
    if !p.contains(&current) {
        p.push(current);
        p.sort_unstable();
    }
    p
}

pub fn fmt_size(percent: u32) -> String {
    format!("{percent}%")
}

/// The factor applied on top of the display scale to the carousel's normal size.
pub fn size_factor(percent: u32) -> f32 {
    percent as f32 / 100.0
}

/// `base` plus `current` (kept exactly when it is not one of the steps, so opening the window and
/// saving never changes a value the person set in the file).
fn with_current(base: &[u64], current: u64) -> Vec<u64> {
    let mut v = base.to_vec();
    if !v.contains(&current) {
        v.push(current);
        v.sort_unstable();
    }
    v
}

fn age_steps(current: Option<u32>) -> Vec<Option<u32>> {
    let mut days: Vec<u32> = AGE_STEPS.to_vec();
    if let Some(c) = current {
        if !days.contains(&c) {
            days.push(c);
            days.sort_unstable();
        }
    }
    let mut v: Vec<Option<u32>> = days.into_iter().map(Some).collect();
    v.push(None);
    v
}

/// The values the four sliders move through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steps {
    pub clips: Vec<u64>,
    pub sizes_mb: Vec<u64>,
    pub ages: Vec<Option<u32>>,
    pub clip_mb: Vec<u64>,
}

impl Steps {
    pub fn for_limits(l: &Limits) -> Steps {
        Steps {
            clips: with_current(&CLIP_STEPS, l.max_clips),
            sizes_mb: with_current(&SIZE_STEPS_MB, l.max_bytes / MB),
            ages: age_steps(l.max_age_days),
            clip_mb: with_current(&CLIP_SIZE_STEPS_MB, l.max_clip_bytes / MB),
        }
    }

    /// Slider positions for the limits these steps were made from.
    pub fn positions(&self, l: &Limits) -> [usize; 4] {
        [
            self.clips.iter().position(|v| *v == l.max_clips).unwrap_or(0),
            self.sizes_mb.iter().position(|v| *v == l.max_bytes / MB).unwrap_or(0),
            self.ages.iter().position(|v| *v == l.max_age_days).unwrap_or(0),
            self.clip_mb.iter().position(|v| *v == l.max_clip_bytes / MB).unwrap_or(0),
        ]
    }

    pub fn counts(&self) -> [usize; 4] {
        [self.clips.len(), self.sizes_mb.len(), self.ages.len(), self.clip_mb.len()]
    }

    /// The values at slider positions (clamped).
    pub fn at(&self, p: [usize; 4]) -> (u64, u64, Option<u32>, u64) {
        let c = |v: &Vec<u64>, i: usize| v[i.min(v.len() - 1)];
        (
            c(&self.clips, p[0]),
            c(&self.sizes_mb, p[1]),
            self.ages[p[2].min(self.ages.len() - 1)],
            c(&self.clip_mb, p[3]),
        )
    }
}

pub fn thousands(n: u64) -> String {
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

pub fn fmt_mb(mb: u64) -> String {
    if mb >= 1024 && mb % 1024 == 0 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{} MB", thousands(mb))
    }
}

pub fn fmt_age(a: Option<u32>) -> String {
    match a {
        None => "Never".to_owned(),
        Some(1) => "1 day".to_owned(),
        Some(d) => format!("{d} days"),
    }
}

/// Everything a settings window lets the person change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub clips: u64,
    pub size_mb: u64,
    pub age_days: Option<u32>,
    pub clip_mb: u64,
    pub paused: bool,
    pub warn_multiline: bool,
    pub size_percent: u32,
    pub exclude_apps: Vec<String>,
    pub filter_names: Vec<String>,
}

impl Choice {
    /// The window's starting point: the config as it is.
    pub fn from_config(c: &Config) -> Choice {
        Choice {
            clips: c.limits.max_clips,
            size_mb: c.limits.max_bytes / MB,
            age_days: c.limits.max_age_days,
            clip_mb: c.limits.max_clip_bytes / MB,
            paused: c.paused,
            warn_multiline: c.warn_multiline_in_terminals,
            size_percent: c.size_percent,
            exclude_apps: c.exclude_apps.clone(),
            filter_names: c.custom_filters.iter().map(|f| f.name.clone()).collect(),
        }
    }
}

/// Applies `choice` to the config text. Only values that changed are written, so everything else
/// in the file (comments, limits the window cannot show exactly) is left alone.
pub fn build_text(orig: &Config, choice: &Choice, text: &str) -> Result<String, Vec<String>> {
    let mut t = text.to_owned();
    let lim = orig.limits;
    if choice.clips != lim.max_clips {
        t = config::set_value(&t, "max_clips", &choice.clips.to_string())?;
    }
    if choice.size_mb * MB != lim.max_bytes {
        t = config::set_value(&t, "max_total_mb", &choice.size_mb.to_string())?;
    }
    if choice.age_days != lim.max_age_days {
        t = config::set_value(&t, "max_age_days", &choice.age_days.map_or("never".to_owned(), |d| d.to_string()))?;
    }
    if choice.clip_mb * MB != lim.max_clip_bytes {
        t = config::set_value(&t, "max_clip_mb", &choice.clip_mb.to_string())?;
    }
    if choice.paused != orig.paused {
        t = config::set_value(&t, "paused", &choice.paused.to_string())?;
    }
    if choice.warn_multiline != orig.warn_multiline_in_terminals {
        t = config::set_value(&t, "warn_multiline_in_terminals", &choice.warn_multiline.to_string())?;
    }
    if choice.size_percent != orig.size_percent {
        t = config::set_value(&t, "size", &choice.size_percent.to_string())?;
    }
    for old in &orig.exclude_apps {
        if !choice.exclude_apps.contains(old) {
            t = config::remove_excluded_app(&t, old)?;
        }
    }
    for new in &choice.exclude_apps {
        if !orig.exclude_apps.contains(new) {
            t = config::add_excluded_app(&t, new)?;
        }
    }
    for f in &orig.custom_filters {
        if !choice.filter_names.iter().any(|n| n.eq_ignore_ascii_case(&f.name)) {
            t = config::remove_custom_filter(&t, &f.name)?;
        }
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_keep_a_value_that_is_not_one_of_them() {
        let l = Limits::default();
        assert_eq!(Steps::for_limits(&l).clips, CLIP_STEPS.to_vec());
        let odd = Limits { max_clips: 1_234, max_age_days: Some(45), ..l };
        let s = Steps::for_limits(&odd);
        assert!(s.clips.contains(&1_234));
        assert!(s.clips.windows(2).all(|w| w[0] < w[1]), "stays sorted");
        assert!(s.ages.contains(&Some(45)));
        assert_eq!(s.ages.last(), Some(&None), "never is last");
        let p = s.positions(&odd);
        assert_eq!(s.at(p), (1_234, odd.max_bytes / MB, Some(45), odd.max_clip_bytes / MB));
    }

    #[test]
    fn values_read_naturally() {
        assert_eq!(fmt_mb(500), "500 MB");
        assert_eq!(fmt_mb(2_048), "2 GB");
        assert_eq!(fmt_mb(1_536), "1,536 MB");
        assert_eq!(fmt_age(None), "Never");
        assert_eq!(fmt_age(Some(90)), "90 days");
        assert_eq!(thousands(100_000), "100,000");
    }

    #[test]
    fn only_changed_values_are_written() {
        let orig = config::parse("").unwrap();
        let same = Choice::from_config(&orig);
        let text = "# my notes\n[limits]\nmax_clips = 1000\n";
        assert_eq!(build_text(&orig, &same, text).unwrap(), text, "nothing changed, nothing rewritten");
        let changed = Choice {
            clips: 250,
            size_mb: 100,
            age_days: None,
            clip_mb: 4,
            paused: true,
            warn_multiline: false,
            ..same
        };
        let out = build_text(&orig, &changed, text).unwrap();
        let c = config::parse(&out).unwrap();
        assert_eq!(c.limits.max_clips, 250);
        assert_eq!(c.limits.max_bytes, 100 * MB);
        assert_eq!(c.limits.max_age_days, None);
        assert_eq!(c.limits.max_clip_bytes, 4 * MB);
        assert!(c.paused && !c.warn_multiline_in_terminals);
        assert!(out.contains("# my notes"), "comments survive");
    }

    #[test]
    fn the_size_is_chosen_written_and_scaled() {
        let orig = config::parse("").unwrap();
        assert_eq!(orig.size_percent, 100);
        let ch = Choice { size_percent: 125, ..Choice::from_config(&orig) };
        let out = build_text(&orig, &ch, "").unwrap();
        assert_eq!(config::parse(&out).unwrap().size_percent, 125);
        assert!(config::set_value("", "size", "250").is_err());
        assert!(config::set_value("", "size", "40").is_err());
        assert_eq!(config::parse(&config::set_value("", "appearance.size", "auto").unwrap()).unwrap().size_percent, 100);
        // a file written by the earlier version still loads
        assert_eq!(config::parse("[appearance]\nsize = \"auto\"\n").unwrap().size_percent, 100);
        let c = size_choices(100);
        assert_eq!(c.len() % 2, 1);
        assert_eq!(c[c.len() / 2], 100, "100% sits in the middle of the slider");
        assert_eq!((c[0], c[c.len() - 1]), (50, 200));
        assert!(size_choices(135).contains(&135));
        assert_eq!(fmt_size(150), "150%");
        assert_eq!(size_factor(150), 1.5);
        assert_eq!(size_factor(50), 0.5);
    }

    #[test]
    fn list_edits_become_adds_and_removes() {
        let base = config::add_excluded_app("", "keepass.exe").unwrap();
        let base = config::add_custom_filter(&base, "Design", &["psd".into()]).unwrap();
        let orig = config::parse(&base).unwrap();
        let choice = Choice {
            exclude_apps: vec!["bitwarden.exe".into()],
            filter_names: vec![],
            ..Choice::from_config(&orig)
        };
        let c = config::parse(&build_text(&orig, &choice, &base).unwrap()).unwrap();
        assert_eq!(c.exclude_apps, vec!["bitwarden.exe".to_string()]);
        assert!(c.custom_filters.is_empty());
    }
}
