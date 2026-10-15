//! The search index and the list of matches.
//!
//! The index holds, for every clip, the small searchable text the store keeps (the first 2,000
//! characters of a text clip, the file paths of a files clip, a short description of an image). It
//! lives in memory for the app's lifetime and is rebuilt when the history changes, so typing is
//! instant. A query matches when every word of it appears (case-insensitively) in that text.

use crate::filters::{self, Filter};
use pasteroll_core::config::CustomFilter;
use pasteroll_core::source::{Dir, HistorySource, Item, Snapshot};
use pasteroll_core::store::{Kind, SearchRow, StoreError};
use zeroize::Zeroize;

/// Entry in the index. `lower` is `text` lower-cased once, so matching is a plain substring scan.
struct Entry {
    id: i64,
    order: f64,
    kind: Kind,
    pinned: bool,
    lower: String,
    /// File paths of a files clip (split once), for the extension filters.
    paths: Vec<String>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.lower.zeroize();
        self.paths.iter_mut().for_each(|p| p.zeroize());
    }
}

#[derive(Default)]
pub struct SearchIndex {
    entries: Vec<Entry>,
}

/// The words of a query: split on whitespace, lower-cased.
pub fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

impl SearchIndex {
    pub fn from_rows(rows: Vec<SearchRow>) -> SearchIndex {
        let entries = rows
            .into_iter()
            .map(|mut r| {
                let paths = if r.kind == Kind::Files {
                    r.text.split('\n').filter(|s| !s.is_empty()).map(str::to_owned).collect()
                } else {
                    Vec::new()
                };
                let lower = r.text.to_lowercase();
                r.text.zeroize();
                Entry { id: r.id, order: r.order, kind: r.kind, pinned: r.pinned, lower, paths }
            })
            .collect();
        SearchIndex { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clips passing `filter` whose text contains every word of `query`, newest first.
    pub fn search(&self, query: &str, filter: &Filter, custom: &[CustomFilter]) -> Vec<Item> {
        let words = terms(query);
        let mut out: Vec<Item> = self
            .entries
            .iter()
            .filter(|e| {
                let paths: Vec<&str> = e.paths.iter().map(String::as_str).collect();
                filters::matches(filter, e.kind, e.pinned, &paths, custom)
                    && words.iter().all(|w| e.lower.contains(w.as_str()))
            })
            .map(|e| Item { id: e.id, order: e.order })
            .collect();
        // newest first, ties by id, exactly the order the store pages in
        out.sort_by(|a, b| {
            b.order.partial_cmp(&a.order).unwrap_or(std::cmp::Ordering::Equal).then(b.id.cmp(&a.id))
        });
        out
    }

    /// Updates one clip's flags in place (a pin toggled in the overlay).
    pub fn set_pinned(&mut self, id: i64, pinned: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.pinned = pinned;
        }
    }

    pub fn remove(&mut self, id: i64) {
        self.entries.retain(|e| e.id != id);
    }
}

/// Character ranges (not bytes) of `title` where any word of the query occurs, merged and sorted.
/// `title` is what is drawn: lower-casing must not change its length, which holds for the scripts
/// that matter; where it would, that entry simply gets no highlight.
pub fn highlight_ranges(title: &str, words: &[String]) -> Vec<(usize, usize)> {
    highlight_ranges_limited(title, words, HIGHLIGHT_VISIBLE_CHARS, HIGHLIGHT_MAX_RANGES)
}

/// Like [`highlight_ranges`] with explicit limits on where a range may start and how many are kept.
pub fn highlight_ranges_limited(
    title: &str,
    words: &[String],
    max_start: usize,
    max_ranges: usize,
) -> Vec<(usize, usize)> {
    let chars: Vec<char> = title.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| {
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(one), None) => one,
                _ => *c,
            }
        })
        .collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let needle: Vec<char> = w.chars().collect();
        if needle.is_empty() || needle.len() > lower.len() {
            continue;
        }
        let mut i = 0;
        while i + needle.len() <= lower.len() {
            if lower[i..i + needle.len()] == needle[..] {
                ranges.push((i, i + needle.len()));
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in ranges {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    // Only what can be seen in a one-line row matters, and each range costs a text shaping pass.
    merged.retain(|r| r.0 < max_start);
    merged.truncate(max_ranges);
    merged
}

const HIGHLIGHT_VISIBLE_CHARS: usize = 120;
const HIGHLIGHT_MAX_RANGES: usize = 8;

/// A [`HistorySource`] over a fixed list of matches. Existence is still checked against the real
/// history, so a clip deleted since the search ran is skipped.
pub struct MatchSource<S> {
    items: Vec<Item>,
    inner: S,
}

impl<S: HistorySource> MatchSource<S> {
    pub fn new(items: Vec<Item>, inner: S) -> Self {
        MatchSource { items, inner }
    }
}

impl<S: HistorySource<Error = StoreError>> HistorySource for MatchSource<S> {
    type Error = StoreError;

    fn snapshot(&mut self) -> Result<Option<Snapshot>, StoreError> {
        Ok(self.items.iter().map(|i| i.order).fold(None, |m: Option<f64>, o| Some(m.map_or(o, |m| m.max(o)))).map(
            |max_order| Snapshot { max_order, max_id: self.items.iter().map(|i| i.id).max().unwrap_or(0) },
        ))
    }

    fn page(
        &mut self,
        snap: Snapshot,
        after: Option<(f64, i64)>,
        dir: Dir,
        limit: usize,
    ) -> Result<Vec<Item>, StoreError> {
        let key = |i: &Item| (i.order, i.id);
        let mut v: Vec<Item> = self
            .items
            .iter()
            .filter(|i| i.order <= snap.max_order && i.id <= snap.max_id)
            .cloned()
            .collect();
        v.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
        if dir == Dir::Older {
            v.reverse();
        }
        let v = v.into_iter().filter(|c| match (after, dir) {
            (None, _) => true,
            (Some(e), Dir::Older) => key(c) < e,
            (Some(e), Dir::Newer) => key(c) > e,
        });
        Ok(v.take(limit).collect())
    }

    fn exists(&mut self, id: i64) -> Result<bool, StoreError> {
        self.inner.exists(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, order: f64, kind: Kind, pinned: bool, text: &str) -> SearchRow {
        SearchRow { id, order, kind, pinned, text: text.to_owned() }
    }

    fn index() -> SearchIndex {
        SearchIndex::from_rows(vec![
            row(5, 5.0, Kind::Text, false, "Meeting notes: ship the spike"),
            row(4, 4.0, Kind::Files, false, "C:\\Users\\me\\Documents\\Invoice-2026.pdf"),
            row(3, 3.0, Kind::Image, false, "image 200x120"),
            row(2, 2.0, Kind::Text, true, "https://example.com/Invoice/pay"),
            row(1, 1.0, Kind::Files, false, "C:\\Users\\me\\Music\\song.mp3\nC:\\Users\\me\\Music\\cover.jpg"),
        ])
    }

    fn ids(v: &[Item]) -> Vec<i64> {
        v.iter().map(|i| i.id).collect()
    }

    #[test]
    fn an_empty_query_with_everything_returns_all_newest_first() {
        assert_eq!(ids(&index().search("", &Filter::Everything, &[])), vec![5, 4, 3, 2, 1]);
    }

    #[test]
    fn matching_is_case_insensitive_and_includes_file_names() {
        let r = index().search("invoice", &Filter::Everything, &[]);
        assert_eq!(ids(&r), vec![4, 2], "the pdf by file name and the url by text, newest first");
        assert_eq!(ids(&index().search("INVOICE-2026", &Filter::Everything, &[])), vec![4]);
    }

    #[test]
    fn every_word_must_match_in_any_order() {
        assert_eq!(ids(&index().search("spike meeting", &Filter::Everything, &[])), vec![5]);
        assert!(index().search("spike invoice", &Filter::Everything, &[]).is_empty());
    }

    #[test]
    fn filters_and_queries_combine() {
        let i = index();
        assert_eq!(ids(&i.search("invoice", &Filter::Document, &[])), vec![4]);
        assert_eq!(ids(&i.search("invoice", &Filter::Text, &[])), vec![2]);
        assert_eq!(ids(&i.search("", &Filter::Pinned, &[])), vec![2]);
        assert_eq!(ids(&i.search("", &Filter::Audio, &[])), vec![1]);
        assert_eq!(ids(&i.search("", &Filter::Image, &[])), vec![3, 1], "bitmap clip and the files clip with a jpg");
    }

    #[test]
    fn a_file_extension_is_searchable() {
        assert_eq!(ids(&index().search("mp3", &Filter::Everything, &[])), vec![1]);
        assert_eq!(ids(&index().search(".pdf", &Filter::Everything, &[])), vec![4]);
    }

    #[test]
    fn no_match_is_an_empty_list_not_an_error() {
        assert!(index().search("zzzzqqq", &Filter::Everything, &[]).is_empty());
    }

    #[test]
    fn pin_and_removal_update_the_index_in_place() {
        let mut i = index();
        i.set_pinned(5, true);
        assert_eq!(ids(&i.search("", &Filter::Pinned, &[])), vec![5, 2]);
        i.remove(2);
        assert_eq!(ids(&i.search("", &Filter::Pinned, &[])), vec![5]);
        assert_eq!(i.len(), 4);
    }

    #[test]
    fn highlights_cover_each_occurrence_and_merge_overlaps() {
        let w = terms("note meet");
        assert_eq!(highlight_ranges("Meeting notes: ship", &w), vec![(0, 4), (8, 12)]);
        assert_eq!(highlight_ranges("aaaa", &terms("aa a")), vec![(0, 4)]);
        assert!(highlight_ranges("nothing here", &terms("zzz")).is_empty());
        assert!(highlight_ranges("anything", &[]).is_empty());
    }

    #[test]
    fn highlights_are_capped_so_a_pathological_clip_cannot_stall_drawing() {
        let title = "e ".repeat(1000);
        let r = highlight_ranges(&title, &terms("e"));
        assert!(r.len() <= 8, "{}", r.len());
        assert!(r.iter().all(|(a, _)| *a < 120));
    }

    #[test]
    fn highlights_are_in_characters_not_bytes() {
        let w = terms("cafe");
        assert_eq!(highlight_ranges("é café 😀 cafe", &w), vec![(9, 13)]);
    }

    #[test]
    fn a_search_over_twenty_thousand_clips_is_fast_enough_to_type_into() {
        let rows: Vec<SearchRow> = (0..20_000)
            .map(|i| row(i, i as f64, Kind::Text, false, &format!("clip number {i} with some ordinary text in it, about a line long")))
            .collect();
        let idx = SearchIndex::from_rows(rows);
        let start = std::time::Instant::now();
        let r = idx.search("number 19999", &Filter::Everything, &[]);
        let took = start.elapsed();
        assert_eq!(r.len(), 1);
        assert!(took < std::time::Duration::from_millis(250), "one search took {took:?}");
    }
}
