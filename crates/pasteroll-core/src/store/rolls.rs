//! Rolls: named collections of clips, next to the main history.
//!
//! A clip is in the main roll unless it has been moved into another roll (`roll_clips`, at most one
//! roll per clip). Besides the custom rolls there are two built-in ones, [`LEFT_ROLL`] and
//! [`RIGHT_ROLL`]: they are always there (a person can only hide them in Settings, never delete them,
//! and the clips in a hidden one stay in it), and they are not rows of the `rolls` table. New copies always land in the main roll. A clip in a custom roll is exempt from
//! the count, size and age limits, like a favorite: a collection must not empty itself. Deleting a
//! roll gives its clips back to the main roll (it deletes no clip). Roll names are sealed with the
//! history key like the rest of a clip's text, and are re-sealed when the key rotates.
//!
//! Two more collections are computed rather than stored: the most used clips (`usage` counts the
//! whole-clip pastes) and the picture clips.
//!
//! These tables are additive: a build that does not know them opens the same history unchanged.

use super::*;

/// How many custom rolls there can be (the right-click menu lists them all, and must fit the window).
pub const ROLLS_MAX: usize = 8;
/// The ids of the two built-in rolls (custom rolls have positive ids).
pub const LEFT_ROLL: i64 = -1;
pub const RIGHT_ROLL: i64 = -2;

/// Is `id` one of the two built-in rolls?
pub fn is_builtin_roll(id: i64) -> bool {
    id == LEFT_ROLL || id == RIGHT_ROLL
}
/// A roll's name is 1 to this many characters.
pub const ROLL_NAME_MAX: usize = 24;
/// The most clips the "most used" collection lists.
pub const MOST_USED_MAX: usize = 100;
/// Names the built-in rolls go by (a custom roll cannot take one).
pub const RESERVED_NAMES: [&str; 5] = ["main", "left", "right", "most used", "images"];

const AAD_ROLL_NAME: &[u8] = b"carousel/roll-name";

/// Which side of the main roll a custom roll sits on in the rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RollSide {
    /// Rolls before Main.
    Left,
    /// Rolls after Main.
    #[default]
    Right,
}

impl RollSide {
    pub fn label(self) -> &'static str {
        match self {
            RollSide::Left => "left",
            RollSide::Right => "right",
        }
    }

    /// The value stored in the `rolls` table (left first, so `ORDER BY side` is the rotation order).
    pub fn code(self) -> i64 {
        match self {
            RollSide::Left => 0,
            RollSide::Right => 1,
        }
    }

    pub fn from_code(code: i64) -> RollSide {
        if code == 0 {
            RollSide::Left
        } else {
            RollSide::Right
        }
    }
}

/// A custom roll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roll {
    pub id: i64,
    pub name: String,
    /// The side of Main it rolls on.
    pub side: RollSide,
    /// Place within that side (1 = first in the rotation on that side).
    pub pos: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum RollError {
    /// Not allowed, in words for a person.
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// A name that may be given to a roll: trimmed, 1 to [`ROLL_NAME_MAX`] characters, no control
/// characters, and not one a built-in roll uses.
pub fn clean_roll_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > ROLL_NAME_MAX || name.chars().any(char::is_control) {
        return Err(format!("A roll's name is 1 to {ROLL_NAME_MAX} characters."));
    }
    if RESERVED_NAMES.contains(&name.to_lowercase().as_str()) {
        return Err(format!("{name} is the name of a built-in roll."));
    }
    Ok(name.to_owned())
}

impl Store {
    /// The custom rolls, in the rotation order: the ones on the left of Main first, then the ones
    /// on its right, each side in its own order.
    pub fn rolls(&self) -> Result<Vec<Roll>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT id, name, side, pos FROM rolls ORDER BY side, pos, id")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, sealed, side, pos) = row?;
            let name = self
                .vault
                .open(AAD_ROLL_NAME, &sealed)
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| "Roll".to_owned());
            out.push(Roll { id, name, side: RollSide::from_code(side), pos });
        }
        Ok(out)
    }

    /// Makes a roll at the end of its side. Returns its id.
    pub fn add_roll(&mut self, name: &str) -> Result<i64, RollError> {
        self.add_roll_on(name, RollSide::Right)
    }

    /// Makes a roll at the end of `side` (the last place on that side of Main). Returns its id.
    pub fn add_roll_on(&mut self, name: &str, side: RollSide) -> Result<i64, RollError> {
        let name = clean_roll_name(name).map_err(RollError::Refused)?;
        let existing = self.rolls()?;
        if existing.len() >= ROLLS_MAX {
            return Err(RollError::Refused(format!("There can be at most {ROLLS_MAX} rolls.")));
        }
        if existing.iter().any(|r| r.name.to_lowercase() == name.to_lowercase()) {
            return Err(RollError::Refused(format!("There is a roll called {name} already.")));
        }
        let pos: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(pos), 0) + 1 FROM rolls WHERE side = ?1",
            [side.code()],
            |r| r.get(0),
        )?;
        let sealed = self.vault.seal(AAD_ROLL_NAME, name.as_bytes());
        self.conn.execute("INSERT INTO rolls(side, pos, name) VALUES(?1, ?2, ?3)", params![side.code(), pos, sealed])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Puts a roll at `pos` on `side` of Main, moving any other roll there along. Used when the
    /// Settings window saves a new order.
    pub fn place_roll(&mut self, id: i64, side: RollSide, pos: i64) -> Result<(), StoreError> {
        if is_builtin_roll(id) {
            return Ok(());
        }
        self.conn.execute("UPDATE rolls SET side = ?1, pos = ?2 WHERE id = ?3", params![side.code(), pos, id])?;
        Ok(())
    }

    pub fn rename_roll(&mut self, id: i64, name: &str) -> Result<(), RollError> {
        let name = clean_roll_name(name).map_err(RollError::Refused)?;
        let others = self.rolls()?;
        if others.iter().any(|r| r.id != id && r.name.to_lowercase() == name.to_lowercase()) {
            return Err(RollError::Refused(format!("There is a roll called {name} already.")));
        }
        let sealed = self.vault.seal(AAD_ROLL_NAME, name.as_bytes());
        self.conn.execute("UPDATE rolls SET name = ?1 WHERE id = ?2", params![sealed, id])?;
        Ok(())
    }

    /// Removes a custom roll. Its clips go back to the main roll; none is deleted. The built-in
    /// rolls cannot be removed (this does nothing for them).
    pub fn delete_roll(&mut self, id: i64) -> Result<(), StoreError> {
        if is_builtin_roll(id) {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM roll_clips WHERE roll_id = ?1", [id])?;
        tx.execute("DELETE FROM rolls WHERE id = ?1", [id])?;
        tx.commit()?;
        // the clips are subject to the limits again
        self.recount()?;
        self.trim_if_over()?;
        Ok(())
    }

    /// Moves a clip into custom roll `roll`, or back to the main roll (`None`). `false` when the
    /// clip is not in history or the roll does not exist.
    pub fn set_clip_roll(&mut self, clip: i64, roll: Option<i64>) -> Result<bool, StoreError> {
        let there: bool = self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM clips WHERE id = ?1 AND trashed IS NULL)", [clip], |r| r.get(0))?;
        if !there {
            return Ok(false);
        }
        match roll {
            Some(r) => {
                let known: bool = is_builtin_roll(r)
                    || self.conn.query_row("SELECT EXISTS(SELECT 1 FROM rolls WHERE id = ?1)", [r], |x| x.get(0))?;
                if !known {
                    return Ok(false);
                }
                self.conn.execute(
                    "INSERT INTO roll_clips(clip_id, roll_id) VALUES(?1, ?2) \
                     ON CONFLICT(clip_id) DO UPDATE SET roll_id = excluded.roll_id",
                    params![clip, r],
                )?;
            }
            None => {
                self.conn.execute("DELETE FROM roll_clips WHERE clip_id = ?1", [clip])?;
            }
        }
        self.recount()?;
        self.trim_if_over()?;
        Ok(true)
    }

    /// The custom roll a clip is in (`None`: the main roll).
    pub fn roll_of(&self, clip: i64) -> Result<Option<i64>, StoreError> {
        Ok(self.conn.query_row("SELECT roll_id FROM roll_clips WHERE clip_id = ?1", [clip], |r| r.get(0)).optional()?)
    }

    /// Every clip that is in a custom roll (so not in the main one).
    pub fn rolled_ids(&self) -> Result<std::collections::HashSet<i64>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT clip_id FROM roll_clips")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The clips of custom roll `roll`, newest first.
    pub fn roll_items(&self, roll: i64) -> Result<Vec<Item>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.ord FROM clips c JOIN roll_clips r ON r.clip_id = c.id \
             WHERE r.roll_id = ?1 AND c.trashed IS NULL ORDER BY c.ord DESC, c.id DESC",
        )?;
        let rows = stmt.query_map([roll], |r| Ok(Item { id: r.get(0)?, order: r.get(1)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// How many clips are in custom roll `roll`.
    pub fn roll_len(&self, roll: i64) -> Result<usize, StoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM roll_clips r JOIN clips c ON c.id = r.clip_id WHERE r.roll_id = ?1 AND c.trashed IS NULL",
            [roll],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// The most used clips: those pasted whole at least once, most pasted first (ties: newest
    /// first), at most [`MOST_USED_MAX`]. `Item::order` stands for the rank (larger = earlier).
    pub fn most_used_items(&self) -> Result<Vec<Item>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.ord, u.n FROM usage u JOIN clips c ON c.id = u.clip_id \
             WHERE c.trashed IS NULL ORDER BY u.n DESC, c.ord DESC, c.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([MOST_USED_MAX as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, ord, n) = row?;
            // pastes first, then recency; recency only ever breaks a tie
            out.push(Item { id, order: n as f64 * 1.0e9 + ord.clamp(0.0, 9.0e8) });
        }
        Ok(out)
    }

    /// The picture clips of every roll, newest first.
    pub fn image_items(&self) -> Result<Vec<Item>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, ord FROM clips WHERE kind = ?1 AND trashed IS NULL ORDER BY ord DESC, id DESC")?;
        let rows = stmt.query_map([Kind::Image as i64], |r| Ok(Item { id: r.get(0)?, order: r.get(1)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Counts one whole-clip paste of `id`.
    pub(super) fn bump_usage(&self, id: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO usage(clip_id, n) VALUES(?1, 1) ON CONFLICT(clip_id) DO UPDATE SET n = n + 1",
            [id],
        )?;
        Ok(())
    }

    /// Re-seals the roll names under `new_vault` (inside the key rotation's transaction).
    pub(super) fn reseal_roll_names(tx: &rusqlite::Transaction<'_>, old: &Vault, new: &Vault) -> Result<(), StoreError> {
        let rows: Vec<(i64, Vec<u8>)> = {
            let mut stmt = tx.prepare("SELECT id, name FROM rolls")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (id, sealed) in rows {
            let plain = old.open(AAD_ROLL_NAME, &sealed).unwrap_or_else(|| b"Roll".to_vec());
            tx.execute("UPDATE rolls SET name = ?1 WHERE id = ?2", params![new.seal(AAD_ROLL_NAME, &plain), id])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::MB;
    use crate::vault::MASTER_LEN;

    const DAY: i64 = 86_400;

    fn mem() -> Store {
        Store::open_in_memory(Vault::new(&[7; MASTER_LEN])).unwrap()
    }

    fn ids(items: &[Item]) -> Vec<i64> {
        items.iter().map(|i| i.id).collect()
    }

    fn main_ids(s: &mut Store) -> Vec<i64> {
        match s.snapshot().unwrap() {
            Some(snap) => ids(&s.page(snap, None, Dir::Older, 100).unwrap()),
            None => Vec::new(),
        }
    }

    #[test]
    fn names_are_checked_and_stored_sealed() {
        let mut s = mem();
        assert!(s.add_roll("   ").is_err());
        assert!(s.add_roll(&"x".repeat(ROLL_NAME_MAX + 1)).is_err());
        assert!(s.add_roll("Main").is_err(), "a built-in roll's name");
        assert!(s.add_roll("most USED").is_err());
        assert!(s.add_roll("left").is_err() && s.add_roll("RIGHT").is_err());
        let a = s.add_roll("  Work  ").unwrap();
        let b = s.add_roll("Receipts").unwrap();
        assert!(s.add_roll("work").is_err(), "names are unique, ignoring case");
        let rolls = s.rolls().unwrap();
        assert_eq!(
            rolls,
            vec![
                Roll { id: a, name: "Work".into(), side: RollSide::Right, pos: 1 },
                Roll { id: b, name: "Receipts".into(), side: RollSide::Right, pos: 2 }
            ]
        );
        s.rename_roll(b, "Bills").unwrap();
        assert!(s.rename_roll(b, "WORK").is_err());
        assert_eq!(s.rolls().unwrap()[1].name, "Bills");
        // the name is not in the file in the clear
        let raw: Vec<u8> = s.conn.query_row("SELECT name FROM rolls WHERE id = ?1", [a], |r| r.get(0)).unwrap();
        assert!(!raw.windows(4).any(|w| w == b"Work"));
    }

    #[test]
    fn rolls_keep_a_side_and_an_order_on_each_side_of_main() {
        let mut s = mem();
        let a = s.add_roll_on("Before", RollSide::Left).unwrap();
        let b = s.add_roll("After").unwrap();
        let c = s.add_roll_on("Nearest", RollSide::Left).unwrap();
        assert_eq!(
            s.rolls().unwrap().iter().map(|r| (r.name.as_str(), r.side, r.pos)).collect::<Vec<_>>(),
            vec![("Before", RollSide::Left, 1), ("Nearest", RollSide::Left, 2), ("After", RollSide::Right, 1)]
        );
        // putting a roll elsewhere is what the settings window does when it saves a new order
        s.place_roll(b, RollSide::Left, 1).unwrap();
        s.place_roll(a, RollSide::Left, 2).unwrap();
        s.place_roll(c, RollSide::Right, 1).unwrap();
        assert_eq!(
            s.rolls().unwrap().iter().map(|r| (r.name.as_str(), r.side, r.pos)).collect::<Vec<_>>(),
            vec![("After", RollSide::Left, 1), ("Before", RollSide::Left, 2), ("Nearest", RollSide::Right, 1)]
        );
        // a built-in roll has no row to move
        s.place_roll(LEFT_ROLL, RollSide::Left, 9).unwrap();
        assert_eq!(s.rolls().unwrap().len(), 3);
    }

    #[test]
    fn a_moved_clip_leaves_the_main_roll_and_stays_in_its_own() {
        let mut s = mem();
        let a = s.add_text("a", DAY).unwrap();
        let b = s.add_text("b", DAY).unwrap();
        let c = s.add_text("c", DAY).unwrap();
        let work = s.add_roll("Work").unwrap();
        assert!(s.set_clip_roll(b, Some(work)).unwrap());
        assert_eq!(main_ids(&mut s), vec![c, a]);
        assert_eq!(ids(&s.roll_items(work).unwrap()), vec![b]);
        assert_eq!(s.roll_of(b).unwrap(), Some(work));
        assert_eq!(s.roll_len(work).unwrap(), 1);
        // and back
        assert!(s.set_clip_roll(b, None).unwrap());
        assert_eq!(main_ids(&mut s), vec![c, b, a]);
        assert!(!s.set_clip_roll(999, Some(work)).unwrap(), "no such clip");
        assert!(!s.set_clip_roll(a, Some(999)).unwrap(), "no such roll");
    }

    #[test]
    fn a_new_copy_lands_in_the_main_roll() {
        let mut s = mem();
        let work = s.add_roll("Work").unwrap();
        let a = s.add_text("a", DAY).unwrap();
        s.set_clip_roll(a, Some(work)).unwrap();
        let b = s.add_text("b", DAY).unwrap();
        assert_eq!(main_ids(&mut s), vec![b]);
    }

    #[test]
    fn clips_in_a_custom_roll_survive_every_limit() {
        let mut s = mem();
        s.set_limits(Limits { max_clips: 5, max_bytes: 500 * MB, max_age_days: Some(30), ..Limits::default() }).unwrap();
        let work = s.add_roll("Work").unwrap();
        let kept = s.add_text("keep me", 10 * DAY).unwrap();
        s.set_clip_roll(kept, Some(work)).unwrap();
        for i in 0..40 {
            s.add_text(&format!("filler {i}"), 10 * DAY).unwrap();
        }
        s.maintain(200 * DAY).unwrap(); // everything else is ancient and over the count
        assert_eq!(ids(&s.roll_items(work).unwrap()), vec![kept], "a collection must not empty itself");
        // taking it out of the roll makes it an ordinary clip again
        s.set_clip_roll(kept, None).unwrap();
        s.maintain(400 * DAY).unwrap();
        assert!(s.text(kept).unwrap().is_none());
    }

    #[test]
    fn deleting_a_roll_gives_its_clips_back_and_deleting_a_clip_leaves_no_trace() {
        let mut s = mem();
        let work = s.add_roll("Work").unwrap();
        let a = s.add_text("a", DAY).unwrap();
        let b = s.add_text("b", DAY).unwrap();
        s.set_clip_roll(a, Some(work)).unwrap();
        s.set_clip_roll(b, Some(work)).unwrap();
        s.delete(b).unwrap();
        assert_eq!(s.roll_len(work).unwrap(), 1);
        assert!(s.rolled_ids().unwrap().contains(&a) && !s.rolled_ids().unwrap().contains(&b));
        s.delete_roll(work).unwrap();
        assert!(s.rolls().unwrap().is_empty());
        assert_eq!(main_ids(&mut s), vec![a], "the clip is back in the main roll, not deleted");
    }

    #[test]
    fn the_recycle_bin_takes_a_clip_out_of_its_roll_view_and_back() {
        let mut s = mem();
        let work = s.add_roll("Work").unwrap();
        let a = s.add_text("a", DAY).unwrap();
        s.set_clip_roll(a, Some(work)).unwrap();
        s.trash(a, DAY).unwrap();
        assert!(s.roll_items(work).unwrap().is_empty());
        s.restore(a).unwrap();
        assert_eq!(ids(&s.roll_items(work).unwrap()), vec![a], "it was in the roll and still is");
    }

    #[test]
    fn most_used_ranks_by_whole_pastes_then_recency() {
        let mut s = mem();
        let a = s.add_text("a", DAY).unwrap();
        let b = s.add_text("b", DAY).unwrap();
        let c = s.add_text("c", DAY).unwrap();
        assert!(s.most_used_items().unwrap().is_empty(), "nothing has been pasted");
        s.mark_pasted(a, DAY, true).unwrap();
        s.mark_pasted(a, DAY, true).unwrap();
        s.mark_pasted(c, DAY, true).unwrap();
        s.mark_pasted(b, DAY, false).unwrap(); // a part of it, or a touch: not counted
        let items = s.most_used_items().unwrap();
        assert_eq!(ids(&items), vec![a, c]);
        assert!(items[0].order > items[1].order);
        s.delete(a).unwrap();
        assert_eq!(ids(&s.most_used_items().unwrap()), vec![c]);
    }

    #[test]
    fn the_picture_collection_lists_pictures_only() {
        let mut s = mem();
        s.add_text("words", DAY).unwrap();
        let png = crate::image::encode_png(&crate::image::Rgba { width: 2, height: 2, pixels: vec![9; 16] }).unwrap();
        let p1 = s.add_image_png(&png, DAY).unwrap();
        assert_eq!(ids(&s.image_items().unwrap()), vec![p1]);
    }

    #[test]
    fn the_built_in_rolls_hold_clips_like_any_other_and_cannot_be_deleted() {
        let mut s = mem();
        s.set_limits(Limits { max_clips: 3, max_bytes: 500 * MB, max_age_days: Some(30), ..Limits::default() }).unwrap();
        assert!(s.rolls().unwrap().is_empty(), "they are not custom rolls: no rows, so not in the list");
        let a = s.add_text("a", 10 * DAY).unwrap();
        assert!(s.set_clip_roll(a, Some(LEFT_ROLL)).unwrap());
        assert_eq!(ids(&s.roll_items(LEFT_ROLL).unwrap()), vec![a]);
        assert_eq!(s.roll_of(a).unwrap(), Some(LEFT_ROLL));
        assert!(main_ids(&mut s).is_empty());
        let b = s.add_text("b", 10 * DAY).unwrap();
        s.set_clip_roll(b, Some(RIGHT_ROLL)).unwrap();
        for i in 0..20 {
            s.add_text(&format!("filler {i}"), 10 * DAY).unwrap();
        }
        s.maintain(300 * DAY).unwrap();
        assert_eq!(s.roll_len(LEFT_ROLL).unwrap() + s.roll_len(RIGHT_ROLL).unwrap(), 2, "exempt from the limits");
        s.delete_roll(LEFT_ROLL).unwrap();
        assert_eq!(s.roll_len(LEFT_ROLL).unwrap(), 1, "deleting a built-in roll does nothing");
        assert!(is_builtin_roll(RIGHT_ROLL) && !is_builtin_roll(1));
    }

    #[test]
    fn key_rotation_keeps_the_roll_names() {
        let mut s = mem();
        s.add_roll("Work").unwrap();
        s.add_text("a", DAY).unwrap();
        s.rotate(Vault::new(&[9; MASTER_LEN]), DAY).unwrap();
        assert_eq!(s.rolls().unwrap()[0].name, "Work");
    }
}
