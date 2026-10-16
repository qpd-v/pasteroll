//! The history interface the carousel reads through. The SQLite [`crate::Store`] implements it;
//! tests use an in-memory fake.

/// What the carousel needs to know about one clip to navigate and show a preview line.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: i64,
    /// Ordering key; larger = newer.
    pub order: f64,
}

/// A session reads a frozen view of history: clips copied after it opened are never shown, so the
/// list never shifts under the user's wheel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    pub max_order: f64,
    pub max_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Toward older clips: descending (order, id).
    Older,
    /// Toward newer clips: ascending (order, id).
    Newer,
}

pub trait HistorySource {
    type Error: std::error::Error;

    /// The newest clip's order and the highest id, or `None` when history is empty.
    fn snapshot(&mut self) -> Result<Option<Snapshot>, Self::Error>;

    /// Up to `limit` clips within `snap`, strictly beyond `after` (an `(order, id)` edge) in the
    /// direction `dir`. With `after == None`, start from the newest clip (`Older`) or the oldest
    /// (`Newer`). Results come back in the traversal direction: descending for `Older`, ascending
    /// for `Newer`.
    fn page(
        &mut self,
        snap: Snapshot,
        after: Option<(f64, i64)>,
        dir: Dir,
        limit: usize,
    ) -> Result<Vec<Item>, Self::Error>;

    /// Whether the clip still exists (it may have been deleted since the page was read).
    fn exists(&mut self, id: i64) -> Result<bool, Self::Error>;
}

/// A borrowed source reads the same history (tests and short-lived sessions).
impl<T: HistorySource + ?Sized> HistorySource for &mut T {
    type Error = T::Error;
    fn snapshot(&mut self) -> Result<Option<Snapshot>, Self::Error> {
        (**self).snapshot()
    }
    fn page(
        &mut self,
        snap: Snapshot,
        after: Option<(f64, i64)>,
        dir: Dir,
        limit: usize,
    ) -> Result<Vec<Item>, Self::Error> {
        (**self).page(snap, after, dir, limit)
    }
    fn exists(&mut self, id: i64) -> Result<bool, Self::Error> {
        (**self).exists(id)
    }
}
