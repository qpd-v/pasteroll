//! The local "brain" a coding agent may read through the MCP server.
//!
//! The person picks rolls in `config.toml` (`[brain] rolls = [...]`). Everything here is read-only
//! and resolves those names against the history: Main is the history less the clips moved into a
//! roll; LEFT and RIGHT are the built-in rolls; a custom roll is selected by its name; Most used and
//! Images are computed. The server never sees a clip outside the selected rolls.

use crate::config::BrainSettings;
use crate::store::rolls::{LEFT_ROLL, RIGHT_ROLL};
use crate::{Item, Kind, Store};

/// A place clips can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Main,
    Left,
    Right,
    MostUsed,
    Images,
    Custom(i64),
}

/// One roll that can be shared, with how many clips it holds.
#[derive(Debug, Clone)]
pub struct Choice {
    pub name: String,
    pub source: Source,
    pub count: usize,
}

/// One clip as the MCP server shows it. The text, file paths and picture are only filled for the
/// fields that belong to the clip's kind.
#[derive(Debug, Clone)]
pub struct Clip {
    pub id: i64,
    pub kind: Kind,
    pub text: Option<String>,
    pub files: Vec<String>,
    /// The PNG, when the clip is a picture, pictures are switched on and it fits the limit.
    pub image_png: Option<Vec<u8>>,
    /// The picture's stored size in bytes (0 when it is not a picture).
    pub image_bytes: usize,
    pub width: u32,
    pub height: u32,
    /// Unix seconds; 0 when an older version did not record it.
    pub copied_at: i64,
    /// The program it was copied from, empty when unknown.
    pub source_app: String,
    pub favorite: bool,
    /// The shared roll it was found in.
    pub roll: String,
    /// Text, paths or the picture were cut because they are too large.
    pub truncated: bool,
}

impl Clip {
    /// Whether the text was cut by `max_chars`, or a file list was cut.
    pub fn text_is_cut(&self) -> bool {
        self.kind == Kind::Text && self.text.as_ref().is_some_and(|t| t.chars().count() > 0) && self.truncated
    }
}

/// Every roll a person could share, in the carousel's order: LEFT, the rolls on Main's left, Main,
/// the rolls on its right, RIGHT, Most used, Images.
pub fn choices(store: &Store) -> Result<Vec<Choice>, String> {
    let rolls = store.rolls().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    out.push(Choice {
        name: "LEFT".to_owned(),
        source: Source::Left,
        count: store.roll_len(LEFT_ROLL).map_err(|e| e.to_string())?,
    });
    for r in rolls.iter().filter(|r| r.side == crate::store::RollSide::Left) {
        out.push(Choice {
            name: r.name.clone(),
            source: Source::Custom(r.id),
            count: store.roll_len(r.id).map_err(|e| e.to_string())?,
        });
    }
    out.push(Choice {
        name: "Main".to_owned(),
        source: Source::Main,
        count: store.main_len().map_err(|e| e.to_string())?,
    });
    for r in rolls.iter().filter(|r| r.side == crate::store::RollSide::Right) {
        out.push(Choice {
            name: r.name.clone(),
            source: Source::Custom(r.id),
            count: store.roll_len(r.id).map_err(|e| e.to_string())?,
        });
    }
    out.push(Choice {
        name: "RIGHT".to_owned(),
        source: Source::Right,
        count: store.roll_len(RIGHT_ROLL).map_err(|e| e.to_string())?,
    });
    out.push(Choice {
        name: "Most used".to_owned(),
        source: Source::MostUsed,
        count: store.most_used_items().map_err(|e| e.to_string())?.len(),
    });
    out.push(Choice {
        name: "Images".to_owned(),
        source: Source::Images,
        count: store.image_items().map_err(|e| e.to_string())?.len(),
    });
    Ok(out)
}

fn normalized(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The source a name means (`None` when no such roll exists).
fn find_source<'a>(choices: &'a [Choice], name: &str) -> Option<&'a Choice> {
    let name = normalized(name);
    choices.iter().find(|c| {
        let builtin = c.name.to_lowercase();
        if builtin == name {
            return true;
        }
        match c.source {
            Source::MostUsed => matches!(name.as_str(), "most-used" | "most_used" | "mostused"),
            Source::Images => name == "image",
            _ => false,
        }
    })
}

/// The configured sources, in the order they are written. An unknown or empty selection is an
/// error a person can act on.
pub fn selected(store: &Store, settings: &BrainSettings) -> Result<Vec<Choice>, String> {
    let all = choices(store)?;
    if settings.rolls.is_empty() {
        return Err("No rolls are shared with agents. Add one with `pasteroll brain add <roll>` or in config.toml under [brain] rolls.".into());
    }
    let mut out: Vec<Choice> = Vec::new();
    for name in &settings.rolls {
        let Some(c) = find_source(&all, name) else {
            return Err(format!(
                "config.toml names a roll that does not exist: {name:?}. Shared rolls: {}. Fix it with `pasteroll brain remove {name}` or `pasteroll brain add ROLL`.",
                settings.rolls.join(", ")
            ));
        };
        if !out.iter().any(|o| o.source == c.source) {
            out.push(c.clone());
        }
    }
    Ok(out)
}

/// How many clips one call may return.
fn cap(settings: &BrainSettings, limit: Option<usize>) -> usize {
    limit.unwrap_or(settings.max_clips).clamp(1, settings.max_clips)
}

fn source_items(store: &Store, source: Source, fetch: usize) -> Result<Vec<Item>, String> {
    let mut items = match source {
        Source::Main => store.main_items(fetch).map_err(|e| e.to_string())?,
        Source::Left => store.roll_items(LEFT_ROLL).map_err(|e| e.to_string())?,
        Source::Right => store.roll_items(RIGHT_ROLL).map_err(|e| e.to_string())?,
        Source::Custom(id) => store.roll_items(id).map_err(|e| e.to_string())?,
        Source::MostUsed => store.most_used_items().map_err(|e| e.to_string())?,
        Source::Images => store.image_items().map_err(|e| e.to_string())?,
    };
    items.truncate(fetch);
    Ok(items)
}

/// Builds the clip the server may show. `None` when the clip is gone, in the recycle bin or not of a
/// kind this build understands.
fn read_clip(store: &Store, settings: &BrainSettings, id: i64, roll: &str) -> Result<Option<Clip>, String> {
    let Some(info) = store.info(id).map_err(|e| e.to_string())? else { return Ok(None) };
    if info.trashed.is_some() {
        return Ok(None);
    }
    let mut truncated = false;
    let mut text = None;
    let mut files = Vec::new();
    let mut image_png = None;
    let mut image_bytes = 0;
    match info.kind {
        Kind::Text => {
            let full = store.text(id).map_err(|e| e.to_string())?.unwrap_or_default();
            if full.chars().count() > settings.max_clip_chars {
                text = Some(full.chars().take(settings.max_clip_chars).collect());
                truncated = true;
            } else {
                text = Some(full);
            }
        }
        Kind::Files => {
            let all = store.files(id).map_err(|e| e.to_string())?.unwrap_or_default();
            if all.len() > 200 {
                files = all.into_iter().take(200).collect();
                truncated = true;
            } else {
                files = all;
            }
        }
        Kind::Image => {
            if let Some(png) = store.image_png(id).map_err(|e| e.to_string())? {
                image_bytes = png.len();
                if settings.include_images && png.len() <= settings.max_image_kb * 1024 {
                    image_png = Some(png);
                } else if settings.include_images {
                    truncated = true;
                }
            }
        }
    }
    Ok(Some(Clip {
        id,
        kind: info.kind,
        text,
        files,
        image_png,
        image_bytes,
        width: info.width,
        height: info.height,
        copied_at: info.copied_at,
        source_app: info.source,
        favorite: info.pinned,
        roll: roll.to_owned(),
        truncated,
    }))
}

fn matches_query(clip: &Clip, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    match clip.kind {
        Kind::Text => clip.text.as_deref().is_some_and(|t| t.to_lowercase().contains(&q)),
        Kind::Files => {
            clip.files.iter().any(|f| f.to_lowercase().contains(&q)) || clip.source_app.to_lowercase().contains(&q)
        }
        // A picture has no searchable text here (OCR clips are text clips).
        Kind::Image => false,
    }
}

/// What one read returns: the sources that were searched, and the clips.
pub struct Read {
    pub sources: Vec<Choice>,
    pub clips: Vec<Clip>,
}

/// Reads the selected rolls. `only` limits the read to one shared roll (by name); `query` matches
/// text and file paths (pictures are only returned when there is no query).
pub fn read(
    store: &Store,
    settings: &BrainSettings,
    only: Option<&str>,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Read, String> {
    let mut sources = selected(store, settings)?;
    if let Some(name) = only {
        let all = choices(store)?;
        let Some(wanted) = find_source(&all, name) else {
            return Err("That roll is not shared with agents.".into());
        };
        if !sources.iter().any(|s| s.source == wanted.source) {
            return Err("That roll is not shared with agents.".into());
        }
        sources.retain(|s| s.source == wanted.source);
    }
    let max = cap(settings, limit);
    let query = query.unwrap_or("");
    let fetch = if query.trim().is_empty() { max.saturating_mul(2).max(50) } else { (max * 4).clamp(200, 2_000) };
    let mut seen = std::collections::HashSet::new();
    let mut clips = Vec::new();
    for source in &sources {
        if clips.len() >= max {
            break;
        }
        for item in source_items(store, source.source, fetch)? {
            if clips.len() >= max {
                break;
            }
            if !seen.insert(item.id) {
                continue;
            }
            let Some(clip) = read_clip(store, settings, item.id, &source.name)? else { continue };
            if matches_query(&clip, query) {
                clips.push(clip);
            }
        }
    }
    Ok(Read { sources, clips })
}

/// One clip by id, only if it is in a shared roll. `None` when it is not (the caller must not say
/// whether the clip exists).
pub fn get(store: &Store, settings: &BrainSettings, id: i64) -> Result<Option<Clip>, String> {
    let sources = selected(store, settings)?;
    let info = match store.info(id).map_err(|e| e.to_string())? {
        Some(i) if i.trashed.is_none() => i,
        _ => return Ok(None),
    };
    let roll_of = store.roll_of(id).map_err(|e| e.to_string())?;
    for source in &sources {
        let belongs = match source.source {
            Source::Main => roll_of.is_none(),
            Source::Left => roll_of == Some(LEFT_ROLL),
            Source::Right => roll_of == Some(RIGHT_ROLL),
            Source::Custom(roll) => roll_of == Some(roll),
            Source::Images => info.kind == Kind::Image,
            Source::MostUsed => store
                .most_used_items()
                .map_err(|e| e.to_string())?
                .iter()
                .any(|i| i.id == id),
        };
        if belongs {
            return read_clip(store, settings, id, &source.name);
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RollSide;
    use crate::vault::{Vault, MASTER_LEN};

    const DAY: i64 = 86_400;

    fn mem() -> Store {
        Store::open_in_memory(Vault::new(&[7; MASTER_LEN])).unwrap()
    }

    fn names(read: &Read) -> Vec<(&str, i64)> {
        read.clips.iter().map(|c| (c.text.as_deref().unwrap_or(""), c.id)).collect()
    }

    #[test]
    fn choices_follow_the_carousel_and_count_each_roll() {
        let mut s = mem();
        let a = s.add_text("a", DAY).unwrap();
        let b = s.add_text("b", DAY).unwrap();
        let work = s.add_roll_on("Work", RollSide::Left).unwrap();
        let bills = s.add_roll("Bills").unwrap();
        s.set_clip_roll(b, Some(work)).unwrap();
        s.set_clip_roll(a, Some(bills)).unwrap();
        let c = choices(&s).unwrap();
        assert_eq!(
            c.iter().map(|c| (c.name.as_str(), c.count)).collect::<Vec<_>>(),
            vec![("LEFT", 0), ("Work", 1), ("Main", 0), ("Bills", 1), ("RIGHT", 0), ("Most used", 0), ("Images", 0)]
        );
    }

    #[test]
    fn only_the_configured_rolls_are_read_and_unknown_names_are_refused() {
        let mut s = mem();
        s.add_text("main one", DAY).unwrap();
        let work = s.add_roll("Work").unwrap();
        let w = s.add_text("work one", DAY).unwrap();
        s.set_clip_roll(w, Some(work)).unwrap();
        let settings = BrainSettings { rolls: vec!["main".into()], ..BrainSettings::default() };
        let out = read(&s, &settings, None, None, None).unwrap();
        assert_eq!(names(&out), vec![("main one", out.clips[0].id)], "a rolled clip is not in Main");
        assert_eq!(out.sources.len(), 1);
        let settings = BrainSettings { rolls: vec!["Main".into(), "work".into()], ..BrainSettings::default() };
        let out = read(&s, &settings, None, None, None).unwrap();
        assert_eq!(out.clips.len(), 2);
        assert_eq!(out.sources.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["Main", "Work"]);
        let bad = BrainSettings { rolls: vec!["Nope".into()], ..BrainSettings::default() };
        assert!(selected(&s, &bad).unwrap_err().contains("does not exist"));
        let empty = BrainSettings { rolls: Vec::new(), ..BrainSettings::default() };
        assert!(selected(&s, &empty).unwrap_err().contains("No rolls are shared"));
    }

    #[test]
    fn a_clip_is_read_once_even_when_it_appears_in_two_rolls() {
        let mut s = mem();
        let png = crate::image::encode_png(&crate::image::Rgba { width: 2, height: 2, pixels: vec![9; 16] }).unwrap();
        let image = s.add_image_png(&png, DAY).unwrap();
        let settings = BrainSettings { rolls: vec!["images".into(), "images".into()], max_clips: 10, ..BrainSettings::default() };
        let out = read(&s, &settings, None, None, None).unwrap();
        assert_eq!(out.clips.len(), 1);
        assert_eq!(out.clips[0].id, image);
        assert_eq!(out.sources.len(), 1, "the repeated source is kept once");
    }

    #[test]
    fn pictures_are_opt_in_and_query_skips_them() {
        let mut s = mem();
        let png = crate::image::encode_png(&crate::image::Rgba { width: 2, height: 2, pixels: vec![9; 16] }).unwrap();
        let image = s.add_image_png(&png, DAY).unwrap();
        let off = BrainSettings { rolls: vec!["images".into()], max_clips: 10, include_images: false, ..BrainSettings::default() };
        let out = read(&s, &off, None, None, None).unwrap();
        assert_eq!(out.clips[0].id, image);
        assert!(out.clips[0].image_png.is_none(), "no picture by default");
        assert!(!out.clips[0].truncated);
        let on = BrainSettings { include_images: true, ..off.clone() };
        let out = read(&s, &on, None, None, None).unwrap();
        assert_eq!(out.clips[0].image_png.as_deref(), Some(png.as_slice()));
        let out = read(&s, &on, None, Some("holiday"), None).unwrap();
        assert!(out.clips.is_empty(), "a picture has no text to search");
    }

    #[test]
    fn a_query_matches_text_and_file_paths_and_the_limit_is_respected() {
        let mut s = mem();
        s.add_text("apple pie", DAY).unwrap();
        s.add_text("banana", DAY).unwrap();
        s.add_text("apple tart", DAY).unwrap();
        let files = s.add_files(&["C:\\Users\\me\\Documents\\apple.pdf".to_owned()], DAY).unwrap();
        let settings = BrainSettings { rolls: vec!["main".into()], max_clips: 2, ..BrainSettings::default() };
        let out = read(&s, &settings, None, Some("APPLE"), None).unwrap();
        assert_eq!(out.clips.len(), 2, "the configured cap wins over the query");
        assert!(out.clips.iter().any(|c| c.id == files));
        let out = read(&s, &settings, None, None, Some(1)).unwrap();
        assert_eq!(out.clips.len(), 1);
        let out = read(&s, &settings, None, None, Some(999)).unwrap();
        assert_eq!(out.clips.len(), 2, "a call cannot go over the configured cap");
    }

    #[test]
    fn a_clip_can_only_be_fetched_from_a_shared_roll() {
        let mut s = mem();
        let work = s.add_roll("Work").unwrap();
        let w = s.add_text("work one", DAY).unwrap();
        let m = s.add_text("main one", DAY).unwrap();
        s.set_clip_roll(w, Some(work)).unwrap();
        let settings = BrainSettings { rolls: vec!["main".into()], ..BrainSettings::default() };
        assert!(get(&s, &settings, m).unwrap().is_some());
        assert!(get(&s, &settings, w).unwrap().is_none(), "its roll is not shared");
        let settings = BrainSettings { rolls: vec!["work".into()], ..BrainSettings::default() };
        assert!(get(&s, &settings, w).unwrap().is_some());
        assert!(get(&s, &settings, 99_999).unwrap().is_none());
    }

    #[test]
    fn long_text_is_cut_to_the_setting() {
        let mut s = mem();
        let id = s.add_text(&"x".repeat(1_500), DAY).unwrap();
        let settings = BrainSettings { rolls: vec!["main".into()], max_clip_chars: 1_000, ..BrainSettings::default() };
        let clip = get(&s, &settings, id).unwrap().unwrap();
        assert_eq!(clip.text.as_deref().map(str::len), Some(1_000));
        assert!(clip.truncated && clip.text_is_cut());
    }
}
