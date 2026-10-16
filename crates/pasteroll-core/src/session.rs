//! The carousel session: one open-to-close cycle through history.
//!
//! Behaviour:
//! - the view is a frozen snapshot taken at open;
//! - history is read in batches and only a sliding window is kept in memory;
//! - stepping past either end wraps to the opposite end;
//! - clips deleted since they were read are skipped.

use crate::source::{Dir, HistorySource, Item, Snapshot};

pub const BATCH_SIZE: usize = 100;
pub const CACHE_SIZE: usize = 300;

/// What a step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// Moved one clip in the requested direction.
    Stepped,
    /// Ran off an end and landed on the opposite end.
    Wrapped,
    /// Nothing left to show (every clip was deleted); the session should close.
    Empty,
}

pub struct Session<S: HistorySource> {
    src: S,
    snap: Snapshot,
    /// Newest first. `items[cur]` is the candidate.
    items: Vec<Item>,
    cur: usize,
    batch: usize,
    cache: usize,
}

impl<S: HistorySource> Session<S> {
    /// Opens on the newest clip. `Ok(None)` when history is empty.
    pub fn open(src: S) -> Result<Option<Self>, S::Error> {
        Self::open_with(src, BATCH_SIZE, CACHE_SIZE)
    }

    pub fn open_with(mut src: S, batch: usize, cache: usize) -> Result<Option<Self>, S::Error> {
        assert!(batch >= 1 && cache >= batch * 2, "cache must hold two batches");
        let Some(snap) = src.snapshot()? else {
            return Ok(None);
        };
        let items = src.page(snap, None, Dir::Older, batch)?;
        if items.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self { src, snap, items, cur: 0, batch, cache }))
    }

    /// The clip currently shown.
    pub fn current(&self) -> &Item {
        &self.items[self.cur]
    }

    /// Index of the candidate within the in-memory window, and the window's size. For tests and
    /// diagnostics only: it is not a position in the whole history.
    pub fn window(&self) -> (usize, usize) {
        (self.cur, self.items.len())
    }

    /// The in-memory window (newest first) and the candidate's index in it. For drawing the
    /// neighbours of the active clip; it is not the whole history.
    pub fn window_items(&self) -> (&[Item], usize) {
        (&self.items, self.cur)
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.src
    }

    /// Opens on `at` (which must still exist within the source), with its neighbours either side
    /// loaded, as if the user had stepped to it. Falls back to the newest clip when `at` is gone.
    pub fn open_at(mut src: S, at: &Item) -> Result<Option<Self>, S::Error> {
        let Some(snap) = src.snapshot()? else {
            return Ok(None);
        };
        let inside = at.order <= snap.max_order && at.id <= snap.max_id;
        if !inside || !src.exists(at.id)? {
            return Self::open(src);
        }
        let mut items = src.page(snap, Some((at.order, at.id)), Dir::Newer, BATCH_SIZE)?;
        items.reverse();
        let cur = items.len();
        items.push(at.clone());
        items.extend(src.page(snap, Some((at.order, at.id)), Dir::Older, BATCH_SIZE)?);
        Ok(Some(Self { src, snap, items, cur, batch: BATCH_SIZE, cache: CACHE_SIZE }))
    }

    /// Moves to the clip with this id if it is in the in-memory window. `false` otherwise.
    pub fn select(&mut self, id: i64) -> bool {
        match self.items.iter().position(|i| i.id == id) {
            Some(i) => {
                self.cur = i;
                true
            }
            None => false,
        }
    }

    pub fn older(&mut self) -> Result<Moved, S::Error> {
        let mut wrapped = false;
        if self.cur + 1 >= self.items.len() && !self.load(Dir::Older)? {
            if !self.restart(Dir::Older)? {
                return Ok(Moved::Empty);
            }
            wrapped = true;
        }
        if !wrapped {
            self.cur += 1;
        }
        self.settle(Dir::Older, wrapped)
    }

    pub fn newer(&mut self) -> Result<Moved, S::Error> {
        let mut wrapped = false;
        if self.cur == 0 && !self.load(Dir::Newer)? {
            if !self.restart(Dir::Newer)? {
                return Ok(Moved::Empty);
            }
            wrapped = true;
        }
        if !wrapped {
            self.cur -= 1;
        }
        self.settle(Dir::Newer, wrapped)
    }

    /// Reads the next batch beyond the window edge in `dir`. `false` when there is nothing more.
    fn load(&mut self, dir: Dir) -> Result<bool, S::Error> {
        match dir {
            Dir::Older => {
                let edge = self.items.last().expect("window is never empty");
                let page =
                    self.src.page(self.snap, Some((edge.order, edge.id)), Dir::Older, self.batch)?;
                if page.is_empty() {
                    return Ok(false);
                }
                self.items.extend(page);
                while self.items.len() > self.cache {
                    self.items.remove(0);
                    self.cur = self.cur.saturating_sub(1);
                }
                Ok(true)
            }
            Dir::Newer => {
                let edge = self.items.first().expect("window is never empty");
                let page =
                    self.src.page(self.snap, Some((edge.order, edge.id)), Dir::Newer, self.batch)?;
                if page.is_empty() {
                    return Ok(false);
                }
                // `page` ascends away from the edge; inserting at the front restores newest-first.
                for item in page.iter().cloned() {
                    self.items.insert(0, item);
                }
                self.cur += page.len();
                while self.items.len() > self.cache {
                    self.items.pop();
                }
                Ok(true)
            }
        }
    }

    /// Wrap: reload from the far end. Lands on the newest clip (`Older` wrap) or the oldest
    /// (`Newer` wrap).
    fn restart(&mut self, dir: Dir) -> Result<bool, S::Error> {
        match dir {
            Dir::Older => {
                let page = self.src.page(self.snap, None, Dir::Older, self.batch)?;
                if page.is_empty() {
                    return Ok(false);
                }
                self.items = page;
                self.cur = 0;
            }
            Dir::Newer => {
                let mut page = self.src.page(self.snap, None, Dir::Newer, self.batch)?;
                if page.is_empty() {
                    return Ok(false);
                }
                page.reverse();
                self.cur = page.len() - 1;
                self.items = page;
            }
        }
        Ok(true)
    }

    /// Makes `items[cur]` a clip that still exists, continuing in `dir` past deleted ones and
    /// wrapping at the ends exactly as a normal step would.
    fn settle(&mut self, dir: Dir, mut wrapped: bool) -> Result<Moved, S::Error> {
        loop {
            if self.cur >= self.items.len() {
                // Stepped older past the window's end after removals.
                if !self.load(Dir::Older)? {
                    if !self.restart(Dir::Older)? {
                        return Ok(Moved::Empty);
                    }
                    wrapped = true;
                }
                continue;
            }
            if self.src.exists(self.items[self.cur].id)? {
                return Ok(if wrapped { Moved::Wrapped } else { Moved::Stepped });
            }
            self.items.remove(self.cur);
            if self.items.is_empty() {
                return Ok(Moved::Empty);
            }
            // Older: `cur` now indexes the next older clip already. Newer: step up one slot.
            if dir == Dir::Newer {
                if self.cur == 0 {
                    if !self.load(Dir::Newer)? {
                        if !self.restart(Dir::Newer)? {
                            return Ok(Moved::Empty);
                        }
                        wrapped = true;
                        continue;
                    }
                }
                self.cur -= 1;
            }
        }
    }
}

impl<S: HistorySource + Clone> Session<S> {
    /// An independent copy at the same place: stepping one does not move the other (the expanded
    /// viewer browses with a fork while the carousel underneath keeps its place).
    pub fn fork(&self) -> Self {
        Self {
            src: self.src.clone(),
            snap: self.snap,
            items: self.items.clone(),
            cur: self.cur,
            batch: self.batch,
            cache: self.cache,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::convert::Infallible;
    use std::fmt;

    #[derive(Debug)]
    pub struct Never(Infallible);
    impl fmt::Display for Never {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self.0 {}
        }
    }
    impl std::error::Error for Never {}

    /// In-memory history. Ids are assigned in insertion order; `order` may be set explicitly.
    #[derive(Default)]
    pub struct Fake {
        pub clips: Vec<Item>,
        pub deleted: HashSet<i64>,
        pub next_id: i64,
    }

    impl Fake {
        pub fn with_n(n: usize) -> Self {
            let mut f = Fake::default();
            for _ in 0..n {
                f.add();
            }
            f
        }
        pub fn add(&mut self) -> i64 {
            self.next_id += 1;
            let id = self.next_id;
            self.clips.push(Item { id, order: id as f64 });
            id
        }
        fn key(i: &Item) -> (f64, i64) {
            (i.order, i.id)
        }
    }

    impl HistorySource for Fake {
        type Error = Never;
        fn snapshot(&mut self) -> Result<Option<Snapshot>, Never> {
            let live: Vec<_> = self.clips.iter().filter(|c| !self.deleted.contains(&c.id)).collect();
            Ok(live.iter().map(|c| c.order).fold(None, |m: Option<f64>, o| {
                Some(m.map_or(o, |m| m.max(o)))
            })
            .map(|max_order| Snapshot {
                max_order,
                max_id: live.iter().map(|c| c.id).max().unwrap(),
            }))
        }
        fn page(
            &mut self,
            snap: Snapshot,
            after: Option<(f64, i64)>,
            dir: Dir,
            limit: usize,
        ) -> Result<Vec<Item>, Never> {
            // A deleted clip is gone from queries, like a DELETE in SQL.
            let mut v: Vec<Item> = self
                .clips
                .iter()
                .filter(|c| !self.deleted.contains(&c.id))
                .filter(|c| c.order <= snap.max_order && c.id <= snap.max_id)
                .cloned()
                .collect();
            v.sort_by(|a, b| Self::key(a).partial_cmp(&Self::key(b)).unwrap());
            if dir == Dir::Older {
                v.reverse();
            }
            let v = v.into_iter().filter(|c| match (after, dir) {
                (None, _) => true,
                (Some(e), Dir::Older) => Self::key(c) < e,
                (Some(e), Dir::Newer) => Self::key(c) > e,
            });
            Ok(v.take(limit).collect())
        }
        fn exists(&mut self, id: i64) -> Result<bool, Never> {
            Ok(self.clips.iter().any(|c| c.id == id) && !self.deleted.contains(&id))
        }
    }

    fn ids(s: &Session<Fake>) -> i64 {
        s.current().id
    }

    #[test]
    fn empty_history_does_not_open() {
        assert!(Session::open(Fake::default()).unwrap().is_none());
    }

    #[test]
    fn open_at_lands_on_the_clip_with_both_neighbours() {
        let mut f = Fake::with_n(250);
        let mut s = Session::open_at(&mut f, &Item { id: 120, order: 120.0 }).unwrap().unwrap();
        assert_eq!(s.current().id, 120);
        s.older().unwrap();
        assert_eq!(s.current().id, 119);
        s.newer().unwrap();
        s.newer().unwrap();
        assert_eq!(s.current().id, 121);
        // the newest end still wraps as usual
        let mut s = Session::open_at(&mut f, &Item { id: 250, order: 250.0 }).unwrap().unwrap();
        assert_eq!(s.newer().unwrap(), Moved::Wrapped);
        assert_eq!(s.current().id, 1);
    }

    #[test]
    fn open_at_a_missing_clip_opens_on_the_newest() {
        let mut f = Fake::with_n(5);
        f.deleted.insert(3);
        let s = Session::open_at(&mut f, &Item { id: 3, order: 3.0 }).unwrap().unwrap();
        assert_eq!(s.current().id, 5);
    }

    #[test]
    fn a_fork_steps_on_its_own() {
        let f = Fake::with_n(5);
        let snap = Snapshot { max_order: 5.0, max_id: 5 };
        let items = (1..=5).rev().map(|id| Item { id, order: id as f64 }).collect();
        let mut a = Session { src: Shared(&f), snap, items, cur: 0, batch: 2, cache: 4 };
        a.older().unwrap();
        let mut b = a.fork();
        b.older().unwrap();
        b.older().unwrap();
        assert_eq!((a.current().id, b.current().id), (4, 2));
        assert!(a.select(2) && a.current().id == 2);
        assert!(!a.select(99));
    }

    /// A shareable read-only view of a [`Fake`], so a session over it can be forked.
    #[derive(Clone, Copy)]
    struct Shared<'a>(&'a Fake);

    impl HistorySource for Shared<'_> {
        type Error = Never;
        fn snapshot(&mut self) -> Result<Option<Snapshot>, Never> {
            unreachable!()
        }
        fn page(&mut self, snap: Snapshot, after: Option<(f64, i64)>, dir: Dir, limit: usize) -> Result<Vec<Item>, Never> {
            let mut v: Vec<Item> = self
                .0
                .clips
                .iter()
                .filter(|c| !self.0.deleted.contains(&c.id) && c.order <= snap.max_order && c.id <= snap.max_id)
                .cloned()
                .collect();
            v.sort_by(|a, b| Fake::key(a).partial_cmp(&Fake::key(b)).unwrap());
            if dir == Dir::Older {
                v.reverse();
            }
            let v = v.into_iter().filter(|c| match (after, dir) {
                (None, _) => true,
                (Some(e), Dir::Older) => Fake::key(c) < e,
                (Some(e), Dir::Newer) => Fake::key(c) > e,
            });
            Ok(v.take(limit).collect())
        }
        fn exists(&mut self, id: i64) -> Result<bool, Never> {
            Ok(self.0.clips.iter().any(|c| c.id == id) && !self.0.deleted.contains(&id))
        }
    }

    #[test]
    fn opens_on_the_newest_clip() {
        let s = Session::open(Fake::with_n(5)).unwrap().unwrap();
        assert_eq!(ids(&s), 5);
    }

    #[test]
    fn older_walks_down_then_wraps_to_newest() {
        let mut s = Session::open(Fake::with_n(3)).unwrap().unwrap();
        assert_eq!(s.older().unwrap(), Moved::Stepped);
        assert_eq!(ids(&s), 2);
        assert_eq!(s.older().unwrap(), Moved::Stepped);
        assert_eq!(ids(&s), 1);
        assert_eq!(s.older().unwrap(), Moved::Wrapped);
        assert_eq!(ids(&s), 3);
    }

    #[test]
    fn newer_from_newest_wraps_to_oldest() {
        let mut s = Session::open(Fake::with_n(4)).unwrap().unwrap();
        assert_eq!(s.newer().unwrap(), Moved::Wrapped);
        assert_eq!(ids(&s), 1);
        assert_eq!(s.newer().unwrap(), Moved::Stepped);
        assert_eq!(ids(&s), 2);
    }

    #[test]
    fn single_clip_wraps_onto_itself() {
        let mut s = Session::open(Fake::with_n(1)).unwrap().unwrap();
        assert_eq!(s.older().unwrap(), Moved::Wrapped);
        assert_eq!(ids(&s), 1);
        assert_eq!(s.newer().unwrap(), Moved::Wrapped);
        assert_eq!(ids(&s), 1);
    }

    #[test]
    fn sliding_window_visits_every_clip_in_order_and_stays_bounded() {
        let mut s = Session::open_with(Fake::with_n(40), 3, 6).unwrap().unwrap();
        let mut seen = vec![ids(&s)];
        for _ in 0..39 {
            assert_eq!(s.older().unwrap(), Moved::Stepped);
            seen.push(ids(&s));
            assert!(s.window().1 <= 6, "window grew to {}", s.window().1);
        }
        assert_eq!(seen, (1..=40).rev().collect::<Vec<_>>());
    }

    #[test]
    fn walking_back_up_after_the_window_slid_retraces_the_same_clips() {
        let mut s = Session::open_with(Fake::with_n(40), 3, 6).unwrap().unwrap();
        for _ in 0..30 {
            s.older().unwrap();
        }
        assert_eq!(ids(&s), 10);
        for want in 11..=40 {
            assert_eq!(s.newer().unwrap(), Moved::Stepped);
            assert_eq!(ids(&s), want);
            assert!(s.window().1 <= 6);
        }
    }

    #[test]
    fn deleted_clips_are_skipped_going_older() {
        let mut s = Session::open(Fake::with_n(5)).unwrap().unwrap();
        s.source_mut().deleted.extend([4, 3]);
        assert_eq!(s.older().unwrap(), Moved::Stepped);
        assert_eq!(ids(&s), 2);
    }

    #[test]
    fn deleted_clips_are_skipped_going_newer() {
        let mut s = Session::open(Fake::with_n(5)).unwrap().unwrap();
        s.older().unwrap();
        s.older().unwrap();
        s.older().unwrap(); // on 2
        assert_eq!(ids(&s), 2);
        s.source_mut().deleted.extend([3, 4]);
        assert_eq!(s.newer().unwrap(), Moved::Stepped);
        assert_eq!(ids(&s), 5);
    }

    #[test]
    fn deleting_the_whole_tail_wraps_instead_of_stalling() {
        let mut s = Session::open(Fake::with_n(3)).unwrap().unwrap();
        s.source_mut().deleted.extend([1, 2]);
        // 3 is the only survivor; older runs off the end and wraps onto it.
        assert_eq!(s.older().unwrap(), Moved::Wrapped);
        assert_eq!(ids(&s), 3);
    }

    #[test]
    fn deleting_everything_newer_wraps_to_the_oldest() {
        let mut s = Session::open(Fake::with_n(5)).unwrap().unwrap();
        for _ in 0..3 {
            s.older().unwrap();
        }
        assert_eq!(ids(&s), 2);
        s.source_mut().deleted.extend([3, 4, 5]);
        assert_eq!(s.newer().unwrap(), Moved::Wrapped);
        assert_eq!(ids(&s), 1);
    }

    #[test]
    fn all_clips_deleted_reports_empty() {
        let mut s = Session::open(Fake::with_n(3)).unwrap().unwrap();
        s.source_mut().deleted.extend([1, 2, 3]);
        assert_eq!(s.older().unwrap(), Moved::Empty);
    }

    #[test]
    fn clips_copied_during_the_session_do_not_appear() {
        let mut s = Session::open_with(Fake::with_n(4), 2, 4).unwrap().unwrap();
        let fresh = s.source_mut().add(); // id 5, newer than the snapshot
        let mut seen = vec![ids(&s)];
        for _ in 0..4 {
            s.older().unwrap();
            seen.push(ids(&s));
        }
        assert!(!seen.contains(&fresh), "snapshot leaked {fresh}: {seen:?}");
        assert_eq!(seen, vec![4, 3, 2, 1, 4]);
    }

    #[test]
    fn equal_order_is_broken_by_id() {
        let mut f = Fake::default();
        for id in 1..=3 {
            f.clips.push(Item { id, order: 10.0 });
        }
        f.next_id = 3;
        let mut s = Session::open(f).unwrap().unwrap();
        let mut seen = vec![ids(&s)];
        s.older().unwrap();
        seen.push(ids(&s));
        s.older().unwrap();
        seen.push(ids(&s));
        assert_eq!(seen, vec![3, 2, 1]);
    }
}
