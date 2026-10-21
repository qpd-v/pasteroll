//! The update check: version numbers, the release list and what is remembered between checks.
//!
//! There is no network code here. A platform fetches the list of releases (when the person has
//! allowed it, see [`crate::config::UpdateCheck`]) and hands the text to [`newest_release`]. Pasteroll
//! only ever tells the person that a newer version exists and where to read about it; it never
//! downloads or installs anything. The page it opens is built here from a version that was parsed
//! strictly, never taken from the server's reply.

use std::cmp::Ordering;
use std::fmt;
use std::path::Path;

/// The GitHub repository the releases come from.
pub const REPO: &str = "qpd-v/pasteroll";

/// How often a check may run on its own, in seconds.
pub const CHECK_EVERY_SECS: i64 = 24 * 60 * 60;

/// The longest reply that is looked at; anything larger is refused.
pub const MAX_REPLY_BYTES: usize = 1 << 20;

/// One part of a pre-release name (`beta`, `7`). Numbers sort before words, as in semver.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Id {
    Num(u64),
    Text(String),
}

/// A version such as `0.1.0` or `0.1.0-beta.7`. A pre-release sorts before its release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<Vec<Id>>,
}

fn number(s: &str) -> Option<u64> {
    // digits only, no sign and no leading zero, short enough that nothing overflows
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

impl Version {
    /// Parses `0.1.0`, `v0.1.0` or `0.1.0-beta.7`. Anything else (build metadata, spaces, signs,
    /// odd characters, a second `v`) is refused.
    pub fn parse(text: &str) -> Option<Version> {
        Version::parse_plain(text.strip_prefix('v').unwrap_or(text))
    }

    /// A release tag as this project writes them: `v`, three numbers, and optionally `-alpha.N`,
    /// `-beta.N` or `-rc.N`. Nothing else is a release (so a stray tag such as
    /// `v0.1.0-beta.7-linux` can never be announced).
    pub fn from_tag(tag: &str) -> Option<Version> {
        let v = Version::parse_plain(tag.strip_prefix('v')?)?;
        match &v.pre {
            None => Some(v),
            Some(p) => match p.as_slice() {
                [Id::Text(name), Id::Num(_)] if matches!(name.as_str(), "alpha" | "beta" | "rc") => Some(v),
                _ => None,
            },
        }
    }

    /// Whether this is a pre-release (a beta).
    pub fn is_pre(&self) -> bool {
        self.pre.is_some()
    }

    fn parse_plain(text: &str) -> Option<Version> {
        if text.is_empty() || text.len() > 40 {
            return None;
        }
        let (core, pre) = match text.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let (major, minor, patch) = (number(parts.next()?)?, number(parts.next()?)?, number(parts.next()?)?);
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => None,
            Some(p) => {
                let ids: Option<Vec<Id>> = p
                    .split('.')
                    .map(|id| {
                        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                            None
                        } else if id.bytes().all(|b| b.is_ascii_digit()) {
                            number(id).map(Id::Num)
                        } else {
                            Some(Id::Text(id.to_owned()))
                        }
                    })
                    .collect();
                Some(ids?)
            }
        };
        Some(Version { major, minor, patch, pre })
    }

    /// The version of this program.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is a valid version")
    }

    /// The page that describes this release.
    pub fn release_page(&self) -> String {
        format!("https://github.com/{REPO}/releases/tag/v{self}")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            let ids: Vec<String> = pre
                .iter()
                .map(|id| match id {
                    Id::Num(n) => n.to_string(),
                    Id::Text(t) => t.clone(),
                })
                .collect();
            write!(f, "-{}", ids.join("."))?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The newest version named by any `"tag_name"` in the reply to GitHub's list-releases request.
/// The reply is not parsed as JSON: each `"tag_name"` string is read on its own and kept only if it
/// is a release tag as [`Version::from_tag`] defines it. Pre-releases are skipped unless
/// `include_pre` (a stable build is not told about betas). `None` when there is none, or the
/// reply is too large.
pub fn newest_release(reply: &str, include_pre: bool) -> Option<Version> {
    if reply.len() > MAX_REPLY_BYTES {
        return None;
    }
    let key = "\"tag_name\"";
    let mut best: Option<Version> = None;
    let mut rest = reply;
    while let Some(at) = rest.find(key) {
        rest = &rest[at + key.len()..];
        let Some(value) = rest.trim_start().strip_prefix(':') else { continue };
        let Some(value) = value.trim_start().strip_prefix('"') else { continue };
        let Some(end) = value.find('"') else { break };
        let tag = &value[..end];
        // only the tags this project uses: `v` and a version, nothing escaped or odd
        if let Some(v) = Version::from_tag(tag).filter(|v| include_pre || !v.is_pre()) {
            if best.as_ref().is_none_or(|b| v > *b) {
                best = Some(v);
            }
        }
    }
    best
}

/// What is remembered between checks, in a small file in the data folder.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateState {
    /// When the last check was made (seconds since 1970), 0 for never.
    pub last_check: i64,
    /// The newest version the person was already told about, so it is announced once.
    pub announced: Option<Version>,
}

impl UpdateState {
    /// Reads the file. A missing or damaged file is "never checked": it only ever means one extra check.
    pub fn load(path: &Path) -> UpdateState {
        let Ok(text) = std::fs::read_to_string(path) else { return UpdateState::default() };
        UpdateState::parse(&text)
    }

    pub fn parse(text: &str) -> UpdateState {
        let mut s = UpdateState::default();
        for line in text.lines().take(16) {
            match line.split_once('=') {
                Some(("last_check", v)) => s.last_check = v.trim().parse().unwrap_or(0).max(0),
                Some(("announced", v)) => s.announced = Version::parse(v.trim()),
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let mut t = format!("last_check={}\n", self.last_check);
        if let Some(v) = &self.announced {
            t.push_str(&format!("announced={v}\n"));
        }
        t
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_text())
    }

    /// Whether a check on its own is due. A clock that went backwards counts as due, so a wrong
    /// date cannot silence the check for years.
    pub fn due(&self, now: i64) -> bool {
        self.last_check <= 0 || now < self.last_check || now - self.last_check >= CHECK_EVERY_SECS
    }

    /// Whether `found` is worth telling the person about: newer than `current` and not the one they
    /// were already told about. (Different, not merely higher: a mistaken tag that was announced
    /// once must not silence the real releases that follow it.)
    pub fn should_announce(&self, found: &Version, current: &Version) -> bool {
        found > current && self.announced.as_ref() != Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn versions_order_the_way_releases_do() {
        assert!(v("0.1.0-beta.7") > v("0.1.0-beta.6"));
        assert!(v("0.1.0-beta.10") > v("0.1.0-beta.9"), "numbers, not text");
        assert!(v("0.1.0") > v("0.1.0-beta.99"), "a release follows its betas");
        assert!(v("0.1.1-beta.1") > v("0.1.0"));
        assert!(v("0.2.0-alpha") > v("0.1.9"));
        assert!(v("0.1.0-beta.1") > v("0.1.0-alpha.9"));
        assert!(v("0.1.0-beta.1.1") > v("0.1.0-beta.1"));
        assert_eq!(v("v0.1.0-beta.6"), v("0.1.0-beta.6"));
        assert_eq!(v("0.1.0-beta.6").to_string(), "0.1.0-beta.6");
        assert_eq!(v("0.1.0-beta.6").cmp(&v("0.1.0-beta.6")), Ordering::Equal);
    }

    #[test]
    fn only_plain_versions_parse() {
        for bad in [
            "", "v", "1", "1.2", "1.2.3.4", "01.2.3", "1.2.-3", "1.2.3-", "1.2.3-beta..1", "1.2.3+build", "1.2.3 ",
            " 1.2.3", "1.2.3-be ta", "latest", "1.2.99999999999", "1.2.3-é", "vv1.2.3",
        ] {
            assert!(Version::parse(bad).is_none(), "{bad:?}");
        }
        assert!(Version::parse(&format!("1.2.3-{}", "a".repeat(80))).is_none(), "too long");
    }

    #[test]
    fn this_programs_version_parses() {
        let c = Version::current();
        assert_eq!(c.to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_release_page_comes_from_the_parsed_version() {
        assert_eq!(
            v("0.1.0-beta.7").release_page(),
            "https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.7"
        );
    }

    const REPLY: &str = r#"[
      {"url":"https://api.github.com/x","tag_name":"v0.1.0-beta.6","name":"Pasteroll 0.1.0-beta.6 (Windows)","draft":false,"prerelease":true,
       "body":"Fix. Mentions \"tag_name\": \"v9.9.9\" in prose."},
      {"tag_name" : "v0.1.0-beta.7", "prerelease": true},
      {"tag_name":"v0.1.0-beta.4","prerelease":true}
    ]"#;

    #[test]
    fn the_newest_tag_in_a_release_list_is_found() {
        assert_eq!(newest_release(REPLY, true), Some(v("0.1.0-beta.7")));
        assert_eq!(newest_release("[]", true), None);
        assert_eq!(newest_release("", true), None);
        assert_eq!(newest_release(r#"{"message":"API rate limit exceeded"}"#, true), None);
    }

    #[test]
    fn odd_tags_and_escaped_text_are_ignored() {
        // a tag that is not v + version, escaped quotes inside prose, a truncated reply
        let r = r#"[{"tag_name":"nightly"},{"tag_name":"v1.0.0+x"},{"tag_name":"v0.2.0\"","x":1},{"tag_name":"v0.1.0-beta.6"},{"tag_name":"v0.3"#;
        assert_eq!(newest_release(r, true), Some(v("0.1.0-beta.6")));
        assert_eq!(newest_release(r#"{"tag_name": 5}"#, true), None);
        assert_eq!(newest_release(r#"{"tag_name"  "v1.0.0"}"#, true), None, "no colon");
        // only this project's tag shapes: no second v, no suffixes, no stray words
        for odd in ["vv0.2.0", "v0.1.0-beta.7-linux", "v0.1.0-nightly.3", "v0.1.0-beta", "v0.1.0-beta.1.2", "0.2.0", "V0.2.0"] {
            let reply = format!(r#"[{{"tag_name":"{odd}"}},{{"tag_name":"v0.1.0-beta.6"}}]"#);
            assert_eq!(newest_release(&reply, true), Some(v("0.1.0-beta.6")), "{odd}");
        }
        assert!(Version::from_tag("v0.1.0").is_some() && Version::from_tag("v0.1.0-rc.2").is_some());
    }

    #[test]
    fn a_stable_build_is_not_told_about_betas() {
        let reply = r#"[{"tag_name":"v0.2.0-beta.1"},{"tag_name":"v0.1.5"},{"tag_name":"v0.1.4"}]"#;
        assert_eq!(newest_release(reply, false), Some(v("0.1.5")));
        assert_eq!(newest_release(reply, true), Some(v("0.2.0-beta.1")));
        assert!(v("0.1.0-beta.6").is_pre() && !v("0.1.0").is_pre());
    }

    #[test]
    fn a_huge_reply_is_refused() {
        let big = format!("{}{}", " ".repeat(MAX_REPLY_BYTES), r#"{"tag_name":"v9.0.0"}"#);
        assert_eq!(newest_release(&big, true), None);
    }

    #[test]
    fn state_round_trips_and_survives_damage() {
        let s = UpdateState { last_check: 1_700_000_000, announced: Some(v("0.1.0-beta.7")) };
        assert_eq!(UpdateState::parse(&s.to_text()), s);
        assert_eq!(UpdateState::parse(""), UpdateState::default());
        assert_eq!(UpdateState::parse("last_check=abc\nannounced=zzz\nnoise"), UpdateState::default());
        assert_eq!(UpdateState::parse("last_check=-5").last_check, 0, "never negative");
        let dir = std::env::temp_dir().join(format!("pasteroll-update-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("update-check.txt");
        assert_eq!(UpdateState::load(&file), UpdateState::default(), "missing file");
        s.save(&file).unwrap();
        assert_eq!(UpdateState::load(&file), s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let now = 1_800_000_000;
        assert!(UpdateState::default().due(now), "never checked");
        let s = UpdateState { last_check: now - 60, announced: None };
        assert!(!s.due(now));
        assert!(!UpdateState { last_check: now - CHECK_EVERY_SECS + 1, announced: None }.due(now));
        assert!(UpdateState { last_check: now - CHECK_EVERY_SECS, announced: None }.due(now));
        assert!(s.due(now - 3_600), "the clock went backwards");
    }

    #[test]
    fn a_version_is_announced_once() {
        let cur = v("0.1.0-beta.6");
        let none = UpdateState::default();
        assert!(none.should_announce(&v("0.1.0-beta.7"), &cur));
        assert!(!none.should_announce(&v("0.1.0-beta.6"), &cur), "same version");
        assert!(!none.should_announce(&v("0.1.0-beta.5"), &cur), "older version");
        let told = UpdateState { last_check: 1, announced: Some(v("0.1.0-beta.7")) };
        assert!(!told.should_announce(&v("0.1.0-beta.7"), &cur), "already told");
        assert!(told.should_announce(&v("0.1.0-beta.8"), &cur), "a newer one is news again");
        // a mistaken or hostile huge tag was announced once; the real release after it still is news
        let bogus = UpdateState { last_check: 1, announced: Some(v("999.0.0")) };
        assert!(bogus.should_announce(&v("0.1.0-beta.7"), &cur));
        assert!(!bogus.should_announce(&v("999.0.0"), &cur), "but the same one is not repeated");
    }
}
