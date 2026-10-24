//! The user's config file (`config.toml`).
//!
//! Strict on purpose: an unknown key or a wrong type is an error with a line the user can fix,
//! never silently ignored (a typo in `max_clips` must not leave you with a different limit than you
//! think). Editing through [`set_value`] keeps the user's comments and layout.

use crate::limits::{Limits, GB, MB};
use toml_edit::{value, Array, DocumentMut, Item, Table};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFilter {
    pub name: String,
    /// Lower-case, without the dot.
    pub extensions: Vec<String>,
}

/// The key that, held while clicking the desktop or a window with an image chosen, opens the
/// Advanced Image Paste window instead of saving or pasting the picture at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickModifier {
    Ctrl,
    Shift,
    Alt,
}

impl ClickModifier {
    pub const ALL: [ClickModifier; 3] = [ClickModifier::Ctrl, ClickModifier::Shift, ClickModifier::Alt];

    pub fn label(self) -> &'static str {
        match self {
            ClickModifier::Ctrl => "Ctrl",
            ClickModifier::Shift => "Shift",
            ClickModifier::Alt => "Alt",
        }
    }

    pub fn parse(text: &str) -> Option<ClickModifier> {
        match text.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(ClickModifier::Ctrl),
            "shift" => Some(ClickModifier::Shift),
            "alt" => Some(ClickModifier::Alt),
            _ => None,
        }
    }
}

/// How the carousel looks. Every feature, the recording, the limits and the stored history are the
/// same in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiMode {
    /// Everything on show: clip list, search bar, filter button, buttons on the card.
    #[default]
    Complete,
    /// Only the current clip, in a smaller window. The search bar and filter menu appear while
    /// they are used; the rest is reached by keyboard (Ctrl+E, Ctrl+P, Delete, Ctrl+Z...).
    NoBloat,
}

impl UiMode {
    pub const ALL: [UiMode; 2] = [UiMode::Complete, UiMode::NoBloat];

    /// The name shown in menus and settings windows.
    pub fn label(self) -> &'static str {
        match self {
            UiMode::Complete => "Complete",
            UiMode::NoBloat => "No Bloat",
        }
    }

    /// The value written to `config.toml`.
    pub fn key(self) -> &'static str {
        match self {
            UiMode::Complete => "complete",
            UiMode::NoBloat => "no-bloat",
        }
    }

    /// Case, spaces, `-` and `_` do not matter: "No Bloat", "no_bloat" and "nobloat" are one mode.
    pub fn parse(text: &str) -> Option<UiMode> {
        let flat: String = text
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | ' '))
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match flat.as_str() {
            "complete" => Some(UiMode::Complete),
            "nobloat" => Some(UiMode::NoBloat),
            _ => None,
        }
    }
}

/// Whether Pasteroll may look for a newer version on the internet. This is the only thing in the
/// program that ever makes a network request, so nothing happens until the person says yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateCheck {
    /// Not decided yet: no request is made, and the person is asked once.
    #[default]
    Ask,
    /// Once a day, ask GitHub whether a newer version exists and say so. Nothing is downloaded.
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub limits: Limits,
    /// Executable names (lower-case) whose copies are never recorded.
    pub exclude_apps: Vec<String>,
    /// Capture is paused: nothing new is recorded.
    pub paused: bool,
    pub custom_filters: Vec<CustomFilter>,
    /// Ask for a second Enter before pasting text with line breaks into a terminal.
    pub warn_multiline_in_terminals: bool,
    /// Extra executable names (lower-case) to treat as terminals, besides the built-in list.
    pub terminal_apps: Vec<String>,
    /// Size of the carousel as a percentage of its normal size.
    pub size_percent: u32,
    /// Hold this while clicking where an image should go, to edit it before it is saved or pasted.
    pub advanced_image_modifier: ClickModifier,
    /// The folder Paste Path and the image editor save pictures into. Empty: the Desktop.
    pub image_folder: String,
    /// Which interface the carousel shows.
    pub mode: UiMode,
    /// Whether to look for a newer version (see [`UpdateCheck`]).
    pub updates: UpdateCheck,
}

/// The range the carousel size may take, in percent.
pub const SIZE_MIN: u32 = 50;
pub const SIZE_DEFAULT: u32 = 100;
pub const SIZE_MAX: u32 = 200;

impl Default for Config {
    fn default() -> Self {
        Config {
            limits: Limits::default(),
            exclude_apps: Vec::new(),
            paused: false,
            custom_filters: Vec::new(),
            warn_multiline_in_terminals: true,
            terminal_apps: Vec::new(),
            size_percent: SIZE_DEFAULT,
            advanced_image_modifier: ClickModifier::Ctrl,
            image_folder: String::new(),
            mode: UiMode::Complete,
            updates: UpdateCheck::Ask,
        }
    }
}

pub const TEMPLATE: &str = r#"# Pasteroll configuration. Lines starting with # are comments.

[limits]
# Whichever limit is reached first removes the oldest unpinned clips. Pinned clips are never removed
# automatically and do not count toward these limits.
max_clips = 1000          # 1 .. 1,000,000   (a settings window offers up to 100,000)
max_total_mb = 500        # 1 MB .. 100,000 MB (a settings window offers up to 10 GB)
max_age_days = 90         # 1 .. 36500, or "never"
max_clip_mb = 16          # 1 .. 64: the largest single clip that is recorded
max_pins = 200            # 0 .. 10,000

[capture]
paused = false
# Copies made in these programs are never recorded (executable names).
exclude_apps = []

[paste]
# Pasting text that contains line breaks into a terminal can run commands. With this on, the first
# Enter shows a warning and a second Enter pastes.
warn_multiline_in_terminals = true
# More programs to treat as terminals (the usual ones are built in).
terminal_apps = []
# Holding this key while clicking the desktop or any window with an image chosen opens the image
# editor (outline, resize, crop, shadow and more) instead of saving or pasting the picture right away.
# "ctrl", "shift" or "alt".
advanced_image_modifier = "ctrl"
# The folder the image editor and Paste Path save pictures into. "" means the Desktop. Settings has a
# Browse button for it.
image_folder = ""

[appearance]
# "complete" shows everything. "no-bloat" shows only the current clip: no list and no buttons on the
# card (search, filters and the other tools still work and appear when used). Switch it from the
# tray menu or Settings.
mode = "complete"
# Size of the carousel as a percentage of its normal size, 50 .. 200.
size = 100

[updates]
# Pasteroll never uses the internet unless you allow it here. true: once a day it asks GitHub
# whether a newer version exists (GitHub sees your IP address and the version you run, nothing
# else) and tells you if there is one. It never downloads or installs anything. false: never.
# Left out: Windows asks you once.
# check_for_updates = false

# Add your own filters for the search dropdown, for example:
# [[filters]]
# name = "Design"
# extensions = ["psd", "ai", "sketch"]
"#;

fn int(item: &Item, key: &str) -> Result<u64, String> {
    match item.as_integer() {
        Some(n) if n >= 0 => Ok(n as u64),
        Some(_) => Err(format!("{key} must not be negative")),
        None => Err(format!("{key} must be a whole number")),
    }
}

/// The longest folder path accepted (Windows' long-path limit is 32,767; no real folder needs this).
const FOLDER_MAX: usize = 1024;

/// A folder path as written in the file: trimmed, no control characters, not absurdly long. Empty is
/// allowed and means the Desktop. Whether the folder exists is checked when a picture is saved.
pub fn clean_folder(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.chars().count() > FOLDER_MAX {
        return Err(format!("image_folder is longer than {FOLDER_MAX} characters"));
    }
    if t.chars().any(char::is_control) {
        return Err("image_folder must not contain control characters".into());
    }
    Ok(t.to_owned())
}

fn strings(item: &Item, key: &str) -> Result<Vec<String>, String> {
    let arr = item.as_array().ok_or_else(|| format!("{key} must be a list of strings"))?;
    arr.iter()
        .map(|v| v.as_str().map(str::to_owned).ok_or_else(|| format!("{key} must contain only strings")))
        .collect()
}

/// Parses and validates a config file. `Err` carries every problem found.
pub fn parse(text: &str) -> Result<Config, Vec<String>> {
    let doc: DocumentMut = text.parse().map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?;
    let mut cfg = Config::default();
    let mut problems: Vec<String> = Vec::new();

    for (section, item) in doc.iter() {
        match section {
            "limits" => {
                let Some(t) = item.as_table() else {
                    problems.push("[limits] must be a table".into());
                    continue;
                };
                for (k, v) in t.iter() {
                    let r: Result<(), String> = (|| {
                        match k {
                            "max_clips" => cfg.limits.max_clips = int(v, k)?,
                            "max_total_mb" => cfg.limits.max_bytes = int(v, k)?.saturating_mul(MB),
                            "max_clip_mb" => cfg.limits.max_clip_bytes = int(v, k)?.saturating_mul(MB),
                            "max_pins" => cfg.limits.max_pins = int(v, k)?.min(u64::from(u32::MAX)) as u32,
                            "max_age_days" => {
                                if v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("never")) {
                                    cfg.limits.max_age_days = None;
                                } else {
                                    cfg.limits.max_age_days = Some(int(v, k)?.min(u64::from(u32::MAX)) as u32);
                                }
                            }
                            other => return Err(format!("unknown setting [limits] {other}")),
                        }
                        Ok(())
                    })();
                    if let Err(e) = r {
                        problems.push(e);
                    }
                }
            }
            "capture" => {
                let Some(t) = item.as_table() else {
                    problems.push("[capture] must be a table".into());
                    continue;
                };
                for (k, v) in t.iter() {
                    match k {
                        "paused" => match v.as_bool() {
                            Some(b) => cfg.paused = b,
                            None => problems.push("paused must be true or false".into()),
                        },
                        "exclude_apps" => match strings(v, "exclude_apps") {
                            Ok(list) => {
                                cfg.exclude_apps = list.iter().map(|s| s.trim().to_ascii_lowercase()).collect()
                            }
                            Err(e) => problems.push(e),
                        },
                        other => problems.push(format!("unknown setting [capture] {other}")),
                    }
                }
            }
            "paste" => {
                let Some(t) = item.as_table() else {
                    problems.push("[paste] must be a table".into());
                    continue;
                };
                for (k, v) in t.iter() {
                    match k {
                        "warn_multiline_in_terminals" => match v.as_bool() {
                            Some(b) => cfg.warn_multiline_in_terminals = b,
                            None => problems.push("warn_multiline_in_terminals must be true or false".into()),
                        },
                        "advanced_image_modifier" => match v.as_str().and_then(ClickModifier::parse) {
                            Some(m) => cfg.advanced_image_modifier = m,
                            None => problems.push("advanced_image_modifier must be \"ctrl\", \"shift\" or \"alt\"".into()),
                        },
                        "terminal_apps" => match strings(v, "terminal_apps") {
                            Ok(list) => {
                                cfg.terminal_apps = list.iter().map(|s| s.trim().to_ascii_lowercase()).collect()
                            }
                            Err(e) => problems.push(e),
                        },
                        "image_folder" => match v.as_str().map(clean_folder) {
                            Some(Ok(f)) => cfg.image_folder = f,
                            Some(Err(e)) => problems.push(e),
                            None => problems.push("image_folder must be text (a folder path, or \"\" for the Desktop)".into()),
                        },
                        other => problems.push(format!("unknown setting [paste] {other}")),
                    }
                }
            }
            "filters" => {
                let Some(arr) = item.as_array_of_tables() else {
                    problems.push("filters must be written as [[filters]] entries".into());
                    continue;
                };
                for (i, t) in arr.iter().enumerate() {
                    let name = t.get("name").and_then(Item::as_str).map(str::trim).unwrap_or("");
                    if name.is_empty() {
                        problems.push(format!("[[filters]] #{} needs a name", i + 1));
                        continue;
                    }
                    let exts = match t.get("extensions") {
                        Some(v) => match strings(v, "extensions") {
                            Ok(l) => l,
                            Err(e) => {
                                problems.push(format!("filter {name}: {e}"));
                                continue;
                            }
                        },
                        None => Vec::new(),
                    };
                    for (k, _) in t.iter() {
                        if k != "name" && k != "extensions" {
                            problems.push(format!("unknown setting in filter {name}: {k}"));
                        }
                    }
                    let extensions: Vec<String> = exts
                        .iter()
                        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect();
                    if extensions.is_empty() {
                        problems.push(format!("filter {name} needs at least one extension"));
                        continue;
                    }
                    cfg.custom_filters.push(CustomFilter { name: name.to_owned(), extensions });
                }
            }
            "appearance" => {
                let Some(t) = item.as_table() else {
                    problems.push("[appearance] must be a table".into());
                    continue;
                };
                for (k, v) in t.iter() {
                    match k {
                        "size" => {
                            // "auto" was written by an earlier version: it means the normal size
                            if v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("auto")) {
                                cfg.size_percent = SIZE_DEFAULT;
                            } else {
                                match int(v, "size") {
                                    Ok(n) if (u64::from(SIZE_MIN)..=u64::from(SIZE_MAX)).contains(&n) => {
                                        cfg.size_percent = n as u32
                                    }
                                    Ok(_) => problems.push(format!("size must be {SIZE_MIN} .. {SIZE_MAX}")),
                                    Err(e) => problems.push(e),
                                }
                            }
                        }
                        "mode" => match v.as_str().and_then(UiMode::parse) {
                            Some(m) => cfg.mode = m,
                            None => problems.push("mode must be \"complete\" or \"no-bloat\"".into()),
                        },
                        other => problems.push(format!("unknown setting [appearance] {other}")),
                    }
                }
            }
            "updates" => {
                let Some(t) = item.as_table() else {
                    problems.push("[updates] must be a table".into());
                    continue;
                };
                for (k, v) in t.iter() {
                    match k {
                        "check_for_updates" => match v.as_bool() {
                            Some(true) => cfg.updates = UpdateCheck::On,
                            Some(false) => cfg.updates = UpdateCheck::Off,
                            None => problems.push("check_for_updates must be true or false".into()),
                        },
                        other => problems.push(format!("unknown setting [updates] {other}")),
                    }
                }
            }
            other => problems.push(format!("unknown section [{other}]")),
        }
    }
    problems.extend(cfg.limits.problems());
    if problems.is_empty() {
        Ok(cfg)
    } else {
        Err(problems)
    }
}

/// Sets one setting in the file text, keeping comments and layout. `key` is `max_clips`,
/// `max_total_mb`, `max_age_days`, `max_clip_mb`, `max_pins`, `paused` (with or without its section
/// prefix). The result is validated before it is returned; on error nothing changes.
pub fn set_value(text: &str, key: &str, val: &str) -> Result<String, Vec<String>> {
    let key = key
        .strip_prefix("limits.")
        .or_else(|| key.strip_prefix("capture."))
        .or_else(|| key.strip_prefix("paste."))
        .or_else(|| key.strip_prefix("appearance."))
        .or_else(|| key.strip_prefix("updates."))
        .unwrap_or(key);
    let (section, item) = match key {
        "max_clips" | "max_total_mb" | "max_clip_mb" | "max_pins" => {
            let n: i64 = val
                .trim()
                .replace(',', "")
                .parse()
                .map_err(|_| vec![format!("{key} must be a whole number (got {val:?})")])?;
            ("limits", value(n))
        }
        "max_age_days" => {
            if val.trim().eq_ignore_ascii_case("never") {
                ("limits", value("never"))
            } else {
                let n: i64 = val
                    .trim()
                    .parse()
                    .map_err(|_| vec![format!("max_age_days must be a number of days or \"never\" (got {val:?})")])?;
                ("limits", value(n))
            }
        }
        "paused" => {
            let b = match val.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" => true,
                "false" | "no" | "off" => false,
                _ => return Err(vec![format!("paused must be true or false (got {val:?})")]),
            };
            ("capture", value(b))
        }
        "warn_multiline_in_terminals" => {
            let b = match val.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" => true,
                "false" | "no" | "off" => false,
                _ => return Err(vec![format!("warn_multiline_in_terminals must be true or false (got {val:?})")]),
            };
            ("paste", value(b))
        }
        "advanced_image_modifier" => match ClickModifier::parse(val) {
            Some(m) => ("paste", value(m.label().to_ascii_lowercase())),
            None => return Err(vec![format!("advanced_image_modifier must be ctrl, shift or alt (got {val:?})")]),
        },
        "image_folder" => match clean_folder(val) {
            Ok(f) => ("paste", value(f)),
            Err(e) => return Err(vec![e]),
        },
        "size" => {
            let v = val.trim().trim_end_matches('%');
            if v.eq_ignore_ascii_case("auto") {
                ("appearance", value(i64::from(SIZE_DEFAULT)))
            } else {
                let n: i64 = v
                    .parse()
                    .map_err(|_| vec![format!("size must be a percentage, {SIZE_MIN} .. {SIZE_MAX} (got {val:?})")])?;
                ("appearance", value(n))
            }
        }
        "mode" => match UiMode::parse(val) {
            Some(m) => ("appearance", value(m.key())),
            None => return Err(vec![format!("mode must be complete or no-bloat (got {val:?})")]),
        },
        "check_for_updates" => {
            let b = match val.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" => true,
                "false" | "no" | "off" => false,
                _ => return Err(vec![format!("check_for_updates must be true or false (got {val:?})")]),
            };
            ("updates", value(b))
        }
        other => return Err(vec![format!("unknown setting {other}")]),
    };
    let mut doc: DocumentMut = if text.trim().is_empty() {
        TEMPLATE.parse().expect("template is valid")
    } else {
        text.parse().map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?
    };
    if doc.get(section).is_none() {
        doc[section] = Item::Table(Table::new());
    }
    doc[section][key] = item;
    let out = doc.to_string();
    parse(&out)?; // never write a file we would refuse to load
    Ok(out)
}

/// Adds or replaces an app in `exclude_apps`.
pub fn add_excluded_app(text: &str, exe: &str) -> Result<String, Vec<String>> {
    let mut doc: DocumentMut = if text.trim().is_empty() {
        TEMPLATE.parse().expect("template is valid")
    } else {
        text.parse().map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?
    };
    if doc.get("capture").is_none() {
        doc["capture"] = Item::Table(Table::new());
    }
    let exe = exe.trim().to_ascii_lowercase();
    let arr = doc["capture"]["exclude_apps"].or_insert(value(Array::new()));
    let arr = arr.as_array_mut().ok_or_else(|| vec!["exclude_apps must be a list".to_owned()])?;
    if !arr.iter().any(|v| v.as_str() == Some(exe.as_str())) {
        arr.push(exe);
    }
    let out = doc.to_string();
    parse(&out)?;
    Ok(out)
}

/// Removes an app from `exclude_apps` (a no-op when it is not there).
pub fn remove_excluded_app(text: &str, exe: &str) -> Result<String, Vec<String>> {
    let mut doc: DocumentMut = text.parse().map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?;
    let exe = exe.trim().to_ascii_lowercase();
    if let Some(arr) = doc.get_mut("capture").and_then(|c| c.get_mut("exclude_apps")).and_then(Item::as_array_mut) {
        arr.retain(|v| v.as_str().map(str::to_ascii_lowercase).as_deref() != Some(exe.as_str()));
    }
    let out = doc.to_string();
    parse(&out)?;
    Ok(out)
}

/// Removes the custom filter called `name` (case-insensitive; a no-op when it is not there).
pub fn remove_custom_filter(text: &str, name: &str) -> Result<String, Vec<String>> {
    let mut doc: DocumentMut = text.parse().map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?;
    if let Some(a) = doc.get_mut("filters").and_then(Item::as_array_of_tables_mut) {
        let mut i = 0;
        while i < a.len() {
            let hit = a
                .get(i)
                .and_then(|t| t.get("name"))
                .and_then(Item::as_str)
                .is_some_and(|n| n.trim().eq_ignore_ascii_case(name.trim()));
            if hit {
                a.remove(i);
            } else {
                i += 1;
            }
        }
    }
    let out = doc.to_string();
    parse(&out)?;
    Ok(out)
}

/// The dropdown has room for this many custom filters.
pub const MAX_CUSTOM_FILTERS: usize = 12;

/// Appends a `[[filters]]` entry. Extensions may be written with or without the dot; the name must
/// not already be used by a built-in filter or an existing custom one.
pub fn add_custom_filter(text: &str, name: &str, extensions: &[String]) -> Result<String, Vec<String>> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 24 || name.chars().any(char::is_control) {
        return Err(vec!["a filter name is 1 to 24 characters".into()]);
    }
    const BUILTIN: [&str; 8] = ["everything", "text", "document", "image", "video", "audio", "compressed", "pinned"];
    if BUILTIN.contains(&name.to_lowercase().as_str()) {
        return Err(vec![format!("{name} is already a built-in filter")]);
    }
    let exts: Vec<String> = extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    if exts.is_empty() || exts.len() > 64 || exts.iter().any(|e| e.len() > 16 || !e.chars().all(|c| c.is_ascii_alphanumeric())) {
        return Err(vec!["give 1 to 64 extensions made of letters and digits".into()]);
    }
    let existing = if text.trim().is_empty() { Config::default() } else { parse(text)? };
    if existing.custom_filters.len() >= MAX_CUSTOM_FILTERS {
        return Err(vec![format!("at most {MAX_CUSTOM_FILTERS} custom filters; remove one from config.toml first")]);
    }
    if existing.custom_filters.iter().any(|f| f.name.eq_ignore_ascii_case(name)) {
        return Err(vec![format!("there is already a filter called {name}")]);
    }
    let mut doc: DocumentMut = if text.trim().is_empty() {
        TEMPLATE.parse().expect("template is valid")
    } else {
        text.parse().map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?
    };
    let mut t = Table::new();
    t["name"] = value(name);
    let mut arr = Array::new();
    for e in &exts {
        arr.push(e.as_str());
    }
    t["extensions"] = value(arr);
    match doc.get_mut("filters").and_then(Item::as_array_of_tables_mut) {
        Some(a) => a.push(t),
        None => {
            let mut a = toml_edit::ArrayOfTables::new();
            a.push(t);
            doc["filters"] = Item::ArrayOfTables(a);
        }
    }
    let out = doc.to_string();
    parse(&out)?;
    Ok(out)
}

/// Human-readable summary for `config show`.
pub fn describe(c: &Config) -> String {
    let l = &c.limits;
    let age = l.max_age_days.map_or("never".to_owned(), |d| format!("{d} days"));
    let size = if l.max_bytes % GB == 0 { format!("{} GB", l.max_bytes / GB) } else { format!("{} MB", l.max_bytes / MB) };
    let mut s = format!(
        "clips kept:     {}\ntotal size:     {}\nage:            {}\nper-clip size:  {} MB\npins allowed:   {}\ncapture:        {}\n",
        l.max_clips,
        size,
        age,
        l.max_clip_bytes / MB,
        l.max_pins,
        if c.paused { "paused" } else { "on" },
    );
    s.push_str(&format!(
        "carousel size:  {}\n",
        format!("{}%", c.size_percent)
    ));
    s.push_str(&format!("mode:           {}\n", c.mode.label()));
    s.push_str(&format!(
        "update check:   {}\n",
        match c.updates {
            UpdateCheck::Ask => "not decided yet (Windows asks once)",
            UpdateCheck::On => "once a day",
            UpdateCheck::Off => "never",
        }
    ));
    s.push_str(&format!("image editor:   {}+click where it should go\n", c.advanced_image_modifier.label()));
    s.push_str(&format!(
        "image folder:   {}\n",
        if c.image_folder.is_empty() { "the Desktop" } else { c.image_folder.as_str() }
    ));
    if !c.exclude_apps.is_empty() {
        s.push_str(&format!("excluded apps:  {}\n", c.exclude_apps.join(", ")));
    }
    if !c.custom_filters.is_empty() {
        let names: Vec<&str> = c.custom_filters.iter().map(|f| f.name.as_str()).collect();
        s.push_str(&format!("custom filters: {}\n", names.join(", ")));
    }
    for k in l.above_ui_maximum() {
        s.push_str(&format!("note: {k} is above the settings-window maximum (allowed in this file)\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::*;

    #[test]
    fn the_template_is_valid_and_gives_the_documented_defaults() {
        let c = parse(TEMPLATE).unwrap();
        assert_eq!(c.limits, Limits::default());
        assert!(!c.paused && c.exclude_apps.is_empty() && c.custom_filters.is_empty());
    }

    #[test]
    fn the_image_editor_key_defaults_to_ctrl_and_can_be_changed() {
        assert_eq!(parse("").unwrap().advanced_image_modifier, ClickModifier::Ctrl);
        assert_eq!(parse(TEMPLATE).unwrap().advanced_image_modifier, ClickModifier::Ctrl);
        let c = parse("[paste]\nadvanced_image_modifier = \"Shift\"\n").unwrap();
        assert_eq!(c.advanced_image_modifier, ClickModifier::Shift);
        assert!(parse("[paste]\nadvanced_image_modifier = \"win\"\n").is_err());
        assert!(parse("[paste]\nadvanced_image_modifier = 3\n").is_err());
        let out = set_value("", "advanced_image_modifier", "ALT").unwrap();
        assert_eq!(parse(&out).unwrap().advanced_image_modifier, ClickModifier::Alt);
        assert!(set_value("", "paste.advanced_image_modifier", "super").is_err());
        assert!(describe(&c).contains("Shift+click"));
    }

    #[test]
    fn the_image_folder_defaults_to_the_desktop_and_can_be_changed() {
        assert_eq!(parse("").unwrap().image_folder, "", "a file from before the setting existed");
        assert_eq!(parse(TEMPLATE).unwrap().image_folder, "");
        let c = parse("[paste]\nimage_folder = '  C:\\Users\\me\\Pictures\\Pasteroll  '\n").unwrap();
        assert_eq!(c.image_folder, "C:\\Users\\me\\Pictures\\Pasteroll", "spaces around it are not part of it");
        assert!(describe(&c).contains("Pictures"));
        assert!(describe(&Config::default()).contains("the Desktop"));
        assert!(parse("[paste]\nimage_folder = 3\n").is_err());
        assert!(parse("[paste]\nimage_folder = \"a\\u0007b\"\n").is_err(), "a control character");
        assert!(parse(&format!("[paste]\nimage_folder = \"{}\"\n", "x".repeat(2000))).is_err());
        let out = set_value("", "image_folder", "/home/me/Pictures").unwrap();
        assert_eq!(parse(&out).unwrap().image_folder, "/home/me/Pictures");
        let out = set_value(&out, "paste.image_folder", "").unwrap();
        assert_eq!(parse(&out).unwrap().image_folder, "", "an empty value goes back to the Desktop");
        assert!(set_value("", "image_folder", "a\u{7}b").is_err());
    }

    #[test]
    fn the_mode_defaults_to_complete_and_can_be_changed() {
        assert_eq!(parse("").unwrap().mode, UiMode::Complete, "a file from before modes existed");
        assert_eq!(parse(TEMPLATE).unwrap().mode, UiMode::Complete);
        for text in ["no-bloat", "NO_BLOAT", "No Bloat", "nobloat"] {
            let c = parse(&format!("[appearance]
mode = \"{text}\"
")).unwrap();
            assert_eq!(c.mode, UiMode::NoBloat, "{text}");
        }
        assert!(parse("[appearance]
mode = \"minimal\"
").is_err());
        assert!(parse("[appearance]
mode = 1
").is_err());
        let out = set_value("", "mode", "No Bloat").unwrap();
        assert_eq!(parse(&out).unwrap().mode, UiMode::NoBloat);
        assert!(out.contains("mode = \"no-bloat\""), "written in its canonical form");
        let back = set_value(&out, "appearance.mode", "complete").unwrap();
        assert_eq!(parse(&back).unwrap().mode, UiMode::Complete);
        assert!(set_value("", "mode", "tiny").is_err());
        for m in UiMode::ALL {
            assert_eq!(UiMode::parse(m.key()), Some(m));
            assert_eq!(UiMode::parse(m.label()), Some(m));
        }
        assert!(describe(&parse(&out).unwrap()).contains("No Bloat"));
    }

    #[test]
    fn an_empty_file_means_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn values_are_read_with_units() {
        let c = parse("[limits]\nmax_clips = 5000\nmax_total_mb = 2048\nmax_age_days = 30\nmax_clip_mb = 32\nmax_pins = 50\n")
            .unwrap();
        assert_eq!(c.limits.max_clips, 5_000);
        assert_eq!(c.limits.max_bytes, 2048 * MB);
        assert_eq!(c.limits.max_age_days, Some(30));
        assert_eq!(c.limits.max_clip_bytes, 32 * MB);
        assert_eq!(c.limits.max_pins, 50);
    }

    #[test]
    fn age_can_be_never_but_there_is_no_unlimited_clip_count() {
        assert_eq!(parse("[limits]\nmax_age_days = \"never\"\n").unwrap().limits.max_age_days, None);
        assert_eq!(parse("[limits]\nmax_age_days = \"Never\"\n").unwrap().limits.max_age_days, None);
        let e = parse("[limits]\nmax_clips = \"unlimited\"\n").unwrap_err();
        assert!(e[0].contains("whole number"), "{e:?}");
        assert!(parse("[limits]\nmax_clips = 0\n").is_err());
    }

    #[test]
    fn values_above_the_hard_maximum_are_rejected_with_a_message() {
        let e = parse("[limits]\nmax_clips = 5000000\n").unwrap_err();
        assert!(e.iter().any(|m| m.contains("1,000,000") && m.contains("5,000,000")), "{e:?}");
        let e = parse("[limits]\nmax_total_mb = 200000\n").unwrap_err();
        assert!(e.iter().any(|m| m.contains("max_total_mb")), "{e:?}");
        let e = parse("[limits]\nmax_clip_mb = 256\n").unwrap_err();
        assert!(e.iter().any(|m| m.contains("max_clip_mb")), "{e:?}");
    }

    #[test]
    fn values_between_ui_and_hard_maximum_load() {
        let c = parse("[limits]\nmax_clips = 500000\nmax_total_mb = 51200\n").unwrap();
        assert_eq!(c.limits.max_clips, 500_000);
        assert!(describe(&c).contains("above the settings-window maximum"));
    }

    #[test]
    fn typos_are_errors_not_silent() {
        let e = parse("[limits]\nmax_clipz = 10\n").unwrap_err();
        assert!(e[0].contains("unknown setting") && e[0].contains("max_clipz"), "{e:?}");
        let e = parse("[limit]\nmax_clips = 10\n").unwrap_err();
        assert!(e[0].contains("unknown section"), "{e:?}");
    }

    #[test]
    fn every_problem_is_reported_together() {
        let e = parse("[limits]\nmax_clips = 0\nmax_pins = -1\nbogus = 1\n").unwrap_err();
        assert!(e.len() >= 3, "{e:?}");
    }

    #[test]
    fn invalid_toml_is_reported() {
        assert!(parse("[limits\nmax_clips = ").unwrap_err()[0].contains("not valid TOML"));
    }

    #[test]
    fn capture_settings_and_exclusions_load_lowercased() {
        let c = parse("[capture]\npaused = true\nexclude_apps = [\"KeePass.exe\", \" 1Password.exe \"]\n").unwrap();
        assert!(c.paused);
        assert_eq!(c.exclude_apps, vec!["keepass.exe", "1password.exe"]);
    }

    #[test]
    fn paste_settings_load_and_default_to_warning() {
        assert!(parse("").unwrap().warn_multiline_in_terminals);
        let c = parse("[paste]\nwarn_multiline_in_terminals = false\nterminal_apps = [\"MyTerm.exe\"]\n").unwrap();
        assert!(!c.warn_multiline_in_terminals);
        assert_eq!(c.terminal_apps, vec!["myterm.exe"]);
        assert!(parse("[paste]\nnope = 1\n").is_err());
        assert!(parse("[paste]\nwarn_multiline_in_terminals = \"yes\"\n").is_err());
    }

    #[test]
    fn custom_filters_load_and_are_normalised() {
        let c = parse("[[filters]]\nname = \"Design\"\nextensions = [\".PSD\", \"ai\", \"\"]\n").unwrap();
        assert_eq!(c.custom_filters, vec![CustomFilter { name: "Design".into(), extensions: vec!["psd".into(), "ai".into()] }]);
        assert!(parse("[[filters]]\nname = \"\"\nextensions = [\"a\"]\n").is_err());
        assert!(parse("[[filters]]\nname = \"X\"\nextensions = []\n").is_err());
    }

    #[test]
    fn set_value_keeps_comments_and_other_settings() {
        let text = "# my notes\n[limits]\nmax_clips = 1000 # keep it small\nmax_age_days = 30\n";
        let out = set_value(text, "max_total_mb", "2048").unwrap();
        assert!(out.contains("# my notes") && out.contains("# keep it small"), "{out}");
        let c = parse(&out).unwrap();
        assert_eq!(c.limits.max_bytes, 2048 * MB);
        assert_eq!(c.limits.max_age_days, Some(30));
    }

    #[test]
    fn set_value_refuses_to_write_an_invalid_file() {
        let text = "[limits]\nmax_clips = 1000\n";
        assert!(set_value(text, "max_clips", "9999999").is_err());
        assert!(set_value(text, "max_clips", "lots").is_err());
        assert!(set_value(text, "nope", "1").is_err());
        assert!(set_value(text, "paused", "maybe").is_err());
    }

    #[test]
    fn set_value_accepts_never_and_separators() {
        let out = set_value("", "max_age_days", "never").unwrap();
        assert_eq!(parse(&out).unwrap().limits.max_age_days, None);
        let out = set_value("", "limits.max_clips", "50,000").unwrap();
        assert_eq!(parse(&out).unwrap().limits.max_clips, 50_000);
    }

    #[test]
    fn excluded_apps_can_be_added_once() {
        let a = add_excluded_app("", "KeePass.exe").unwrap();
        let b = add_excluded_app(&a, "keepass.exe").unwrap();
        assert_eq!(parse(&b).unwrap().exclude_apps, vec!["keepass.exe"]);
    }

    #[test]
    fn the_update_check_is_off_until_you_decide() {
        assert_eq!(parse("").unwrap().updates, UpdateCheck::Ask, "a file from before the setting existed");
        assert_eq!(parse(TEMPLATE).unwrap().updates, UpdateCheck::Ask, "the template does not decide for you");
        assert_eq!(Config::default().updates, UpdateCheck::Ask);
        let on = set_value("", "check_for_updates", "yes").unwrap();
        assert_eq!(parse(&on).unwrap().updates, UpdateCheck::On);
        assert!(on.contains("check_for_updates = true"));
        let off = set_value(&on, "updates.check_for_updates", "false").unwrap();
        assert_eq!(parse(&off).unwrap().updates, UpdateCheck::Off);
        assert!(set_value("", "check_for_updates", "maybe").is_err());
        assert!(parse("[updates]\ncheck_for_updates = \"yes\"\n").is_err(), "a word is not a boolean");
        assert!(parse("[updates]\ncheck = true\n").is_err(), "typos are errors");
        assert!(describe(&parse(&on).unwrap()).contains("once a day"));
        assert!(describe(&Config::default()).contains("not decided"));
    }

    #[test]
    fn describe_lists_the_effective_values() {
        let d = describe(&Config::default());
        assert!(d.contains("1000") && d.contains("500 MB") && d.contains("90 days"), "{d}");
        let d = describe(&Config { limits: Limits { max_age_days: None, max_bytes: 10 * GB, ..Limits::default() }, ..Config::default() });
        assert!(d.contains("never") && d.contains("10 GB"));
    }
    #[test]
    fn a_custom_filter_can_be_appended_and_is_read_back() {
        let out = add_custom_filter("", "Design", &[".PSD".into(), "ai".into()]).unwrap();
        let c = parse(&out).unwrap();
        assert_eq!(c.custom_filters, vec![CustomFilter { name: "Design".into(), extensions: vec!["psd".into(), "ai".into()] }]);
        let out2 = add_custom_filter(&out, "Sound", &["wav".into()]).unwrap();
        assert_eq!(parse(&out2).unwrap().custom_filters.len(), 2);
    }

    #[test]
    fn the_number_of_custom_filters_is_capped() {
        let mut text = String::new();
        for i in 0..MAX_CUSTOM_FILTERS {
            text = add_custom_filter(&text, &format!("F{i}"), &["abc".into()]).unwrap();
        }
        assert!(add_custom_filter(&text, "OneMore", &["abc".into()]).is_err());
    }

    #[test]
    fn a_custom_filter_is_validated_before_it_is_written() {
        assert!(add_custom_filter("", "Image", &["png".into()]).is_err(), "built-in name");
        assert!(add_custom_filter("", "", &["png".into()]).is_err());
        assert!(add_custom_filter("", "X", &[]).is_err());
        assert!(add_custom_filter("", "X", &["a b".into()]).is_err());
        let once = add_custom_filter("", "Design", &["psd".into()]).unwrap();
        assert!(add_custom_filter(&once, "design", &["ai".into()]).is_err(), "duplicate name");
    }

    #[test]
    fn the_terminal_warning_can_be_switched_from_a_settings_window() {
        let out = set_value("", "warn_multiline_in_terminals", "false").unwrap();
        assert!(!parse(&out).unwrap().warn_multiline_in_terminals);
        let back = set_value(&out, "paste.warn_multiline_in_terminals", "true").unwrap();
        assert!(parse(&back).unwrap().warn_multiline_in_terminals);
        assert!(set_value("", "warn_multiline_in_terminals", "maybe").is_err());
    }

    #[test]
    fn an_excluded_app_can_be_removed_and_nothing_else_changes() {
        let t = add_excluded_app("", "keepass.exe").unwrap();
        let t = add_excluded_app(&t, "bitwarden.exe").unwrap();
        let out = remove_excluded_app(&t, "KeePass.exe").unwrap();
        assert_eq!(parse(&out).unwrap().exclude_apps, vec!["bitwarden.exe".to_string()]);
        assert_eq!(remove_excluded_app(&out, "nothere.exe").unwrap(), out);
    }

    #[test]
    fn a_custom_filter_can_be_removed_by_name() {
        let t = add_custom_filter("", "Design", &["psd".into()]).unwrap();
        let t = add_custom_filter(&t, "Sound", &["wav".into()]).unwrap();
        let out = remove_custom_filter(&t, "design").unwrap();
        let names: Vec<String> = parse(&out).unwrap().custom_filters.into_iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["Sound".to_string()]);
        let none = remove_custom_filter(&out, "Sound").unwrap();
        assert!(parse(&none).unwrap().custom_filters.is_empty());
    }

}
