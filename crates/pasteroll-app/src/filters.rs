//! The filter dropdown: Everything, Text, Document, Image, Video, Audio, Compressed, Pinned, and
//! the user's own. A filter is a predicate over a clip's kind, pinned flag and file extensions.

use pasteroll_core::config::CustomFilter;
use pasteroll_core::store::Kind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    Everything,
    Text,
    Document,
    Image,
    Video,
    Audio,
    Compressed,
    Pinned,
    /// The recycle bin: clips moved out of history, restorable for 30 days.
    Trash,
    /// Index into the user's custom filters.
    Custom(usize),
}

pub const DOCUMENT: &[&str] = &[
    "doc", "docx", "odt", "rtf", "txt", "md", "pdf", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp", "epub",
    "html", "htm", "pages", "numbers", "key",
];
pub const IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "svg", "heic", "heif", "ico", "avif",
];
pub const VIDEO: &[&str] = &["mp4", "mkv", "mov", "avi", "wmv", "webm", "m4v", "flv", "mpg", "mpeg"];
pub const AUDIO: &[&str] = &["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus", "aiff"];
pub const COMPRESSED: &[&str] = &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst", "cab"];

/// The built-in entries, in dropdown order.
pub const BUILTIN: &[(Filter, &str)] = &[
    (Filter::Everything, "Everything"),
    (Filter::Text, "Text"),
    (Filter::Document, "Document"),
    (Filter::Image, "Image"),
    (Filter::Video, "Video"),
    (Filter::Audio, "Audio"),
    (Filter::Compressed, "Compressed"),
    (Filter::Pinned, "Favorites"),
    (Filter::Trash, "Recycle bin"),
];

/// Everything the dropdown shows: built-ins, the user's filters, then the "add one" entry.
pub fn labels(custom: &[CustomFilter]) -> Vec<String> {
    let mut v: Vec<String> = BUILTIN.iter().map(|(_, n)| (*n).to_owned()).collect();
    v.extend(custom.iter().map(|c| c.name.clone()));
    v.push("Add custom filter...".to_owned());
    v
}

/// Which filter a dropdown row stands for; `None` for the trailing "add one" entry.
pub fn at(index: usize, custom: &[CustomFilter]) -> Option<Filter> {
    if index < BUILTIN.len() {
        Some(BUILTIN[index].0.clone())
    } else if index < BUILTIN.len() + custom.len() {
        Some(Filter::Custom(index - BUILTIN.len()))
    } else {
        None
    }
}

pub fn index_of(filter: &Filter, custom: &[CustomFilter]) -> usize {
    let _ = custom;
    match filter {
        Filter::Custom(i) => BUILTIN.len() + i,
        other => BUILTIN.iter().position(|(f, _)| f == other).unwrap_or(0),
    }
}

pub fn label(filter: &Filter, custom: &[CustomFilter]) -> String {
    match filter {
        Filter::Custom(i) => custom.get(*i).map_or("Custom".to_owned(), |c| c.name.clone()),
        other => BUILTIN.iter().find(|(f, _)| f == other).map_or("Everything", |(_, n)| n).to_owned(),
    }
}

/// Lower-case extension of a path, without the dot.
pub fn extension(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?;
    let dot = name.rfind('.')?;
    (dot > 0 && dot + 1 < name.len()).then(|| name[dot + 1..].to_ascii_lowercase())
}

/// Does a clip pass `filter`? `paths` are the file paths of a files clip (empty otherwise).
pub fn matches(filter: &Filter, kind: Kind, pinned: bool, paths: &[&str], custom: &[CustomFilter]) -> bool {
    let any_ext = |set: &[&str]| paths.iter().filter_map(|p| extension(p)).any(|e| set.contains(&e.as_str()));
    match filter {
        Filter::Everything => true,
        Filter::Pinned => pinned,
        // the bin's own index holds only binned clips
        Filter::Trash => true,
        Filter::Text => kind == Kind::Text,
        Filter::Image => kind == Kind::Image || (kind == Kind::Files && any_ext(IMAGE)),
        Filter::Document => kind == Kind::Files && any_ext(DOCUMENT),
        Filter::Video => kind == Kind::Files && any_ext(VIDEO),
        Filter::Audio => kind == Kind::Files && any_ext(AUDIO),
        Filter::Compressed => kind == Kind::Files && any_ext(COMPRESSED),
        Filter::Custom(i) => match custom.get(*i) {
            Some(c) => {
                kind == Kind::Files
                    && paths
                        .iter()
                        .filter_map(|p| extension(p))
                        .any(|e| c.extensions.iter().any(|x| *x == e))
            }
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design() -> Vec<CustomFilter> {
        vec![CustomFilter { name: "Design".into(), extensions: vec!["psd".into(), "ai".into()] }]
    }

    #[test]
    fn extensions_are_read_from_the_file_name_only() {
        assert_eq!(extension("C:\\a.b\\report.PDF").as_deref(), Some("pdf"));
        assert_eq!(extension("/home/me/archive.tar.gz").as_deref(), Some("gz"));
        assert_eq!(extension("C:\\a.b\\README"), None);
        assert_eq!(extension("C:\\x\\.gitignore"), None, "a leading dot is not an extension");
        assert_eq!(extension("C:\\x\\trailing."), None);
    }

    #[test]
    fn each_category_picks_the_right_files() {
        let c = [];
        let pdf = ["C:\\d\\a.pdf"];
        let png = ["C:\\d\\a.png"];
        let mp4 = ["C:\\d\\a.mp4"];
        let mp3 = ["C:\\d\\a.mp3"];
        let zip = ["C:\\d\\a.zip"];
        assert!(matches(&Filter::Document, Kind::Files, false, &pdf, &c));
        assert!(matches(&Filter::Image, Kind::Files, false, &png, &c));
        assert!(matches(&Filter::Video, Kind::Files, false, &mp4, &c));
        assert!(matches(&Filter::Audio, Kind::Files, false, &mp3, &c));
        assert!(matches(&Filter::Compressed, Kind::Files, false, &zip, &c));
        assert!(!matches(&Filter::Document, Kind::Files, false, &png, &c));
        assert!(!matches(&Filter::Video, Kind::Files, false, &pdf, &c));
    }

    #[test]
    fn a_clipboard_bitmap_is_an_image_but_not_a_document() {
        assert!(matches(&Filter::Image, Kind::Image, false, &[], &[]));
        assert!(!matches(&Filter::Document, Kind::Image, false, &[], &[]));
    }

    #[test]
    fn text_and_pinned_and_everything() {
        assert!(matches(&Filter::Text, Kind::Text, false, &[], &[]));
        assert!(!matches(&Filter::Text, Kind::Files, false, &["a.txt"], &[]));
        assert!(matches(&Filter::Pinned, Kind::Text, true, &[], &[]));
        assert!(!matches(&Filter::Pinned, Kind::Text, false, &[], &[]));
        assert!(matches(&Filter::Everything, Kind::Image, false, &[], &[]));
    }

    #[test]
    fn a_files_clip_with_several_files_matches_if_any_does() {
        let mixed = ["C:\\d\\notes.txt", "C:\\d\\photo.jpg"];
        assert!(matches(&Filter::Document, Kind::Files, false, &mixed, &[]));
        assert!(matches(&Filter::Image, Kind::Files, false, &mixed, &[]));
        assert!(!matches(&Filter::Audio, Kind::Files, false, &mixed, &[]));
    }

    #[test]
    fn custom_filters_match_their_own_extensions() {
        let c = design();
        assert!(matches(&Filter::Custom(0), Kind::Files, false, &["C:\\x\\logo.PSD"], &c));
        assert!(!matches(&Filter::Custom(0), Kind::Files, false, &["C:\\x\\logo.png"], &c));
        assert!(!matches(&Filter::Custom(0), Kind::Text, false, &[], &c));
        assert!(!matches(&Filter::Custom(5), Kind::Files, false, &["a.psd"], &c), "dangling index matches nothing");
    }

    #[test]
    fn the_dropdown_lists_builtins_custom_filters_and_the_add_entry() {
        let l = labels(&design());
        assert_eq!(l.first().map(String::as_str), Some("Everything"));
        assert_eq!(l[BUILTIN.len()], "Design");
        assert_eq!(l.last().map(String::as_str), Some("Add custom filter..."));
        assert_eq!(l.len(), BUILTIN.len() + 2);
    }

    #[test]
    fn rows_map_back_to_filters() {
        let c = design();
        assert_eq!(at(0, &c), Some(Filter::Everything));
        assert_eq!(at(BUILTIN.len(), &c), Some(Filter::Custom(0)));
        assert_eq!(at(BUILTIN.len() + 1, &c), None, "the add entry is not a filter");
        assert_eq!(index_of(&Filter::Custom(0), &c), BUILTIN.len());
        assert_eq!(index_of(&Filter::Pinned, &c), 7);
        assert_eq!(label(&Filter::Custom(0), &c), "Design");
        assert_eq!(label(&Filter::Video, &c), "Video");
    }
}
