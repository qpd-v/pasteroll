//! Rolls in the carousel: separate collections of clips that are rolled through left and right.
//!
//! The main roll is the ordinary history (new copies always land there). A custom roll holds the
//! clips moved into it with the right-click menu; "Most used" and "Images" are computed. The roll
//! that is showing is `Open::roll`; a session always starts on the main roll. Every roll is shown
//! through the same machinery as a search: a list of clips (`Src::Matches`) over the one store, so
//! search, filters, choosing, pasting and deleting all work in every roll.
//!
//! Rolling from one to the next slides the old picture out and the new one in (`slide`).

use super::*;
use pasteroll_core::config::RollSettings;
use pasteroll_core::store::rolls::{LEFT_ROLL, RIGHT_ROLL, ROLLS_MAX};
use pasteroll_core::store::{Roll, RollError, RollSide};
use pasteroll_render::{ease_out, slide_frames, Frame, RollView, SLIDE_MS};

/// A roll in the rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RollId {
    Main,
    Custom(i64),
    MostUsed,
    Images,
}

/// A place the "Move to roll" menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MoveTarget {
    /// A roll (`None`: the main one).
    Roll(Option<i64>),
    /// Make a new roll for the clips, on this side of Main.
    NewRoll(RollSide),
}

/// The old picture, while the new one slides in over it.
pub(super) struct Slide {
    from: Frame,
    /// `1` when the new roll comes from the right.
    dir: i8,
    since_ms: u64,
    /// Pixels from the top that do not move (the search bar and the favorite tiles).
    split_px: u32,
}

#[derive(Default)]
pub(super) struct Rolls {
    /// The custom rolls, as last read from the store.
    list: Vec<Roll>,
    settings: RollSettings,
    slide: Option<Slide>,
}

/// The clips of a roll as a list, or `None` for the main roll (it is the history itself).
pub(super) fn base_items(store: &Store, roll: RollId) -> Result<Option<Vec<Item>>, pasteroll_core::store::StoreError> {
    Ok(match roll {
        RollId::Main => None,
        RollId::Custom(id) => Some(store.roll_items(id)?),
        RollId::MostUsed => Some(store.most_used_items()?),
        RollId::Images => Some(store.image_items()?),
    })
}

/// Keeps the search results that belong to `roll`, in that roll's own order. Also returns how many
/// clips the roll has, for the "N of M" count.
pub(super) fn restrict(store: &Store, roll: RollId, mut items: Vec<Item>, history_len: usize) -> (usize, Vec<Item>) {
    match base_items(store, roll) {
        Ok(None) => {
            let rolled = store.rolled_ids().unwrap_or_default();
            items.retain(|i| !rolled.contains(&i.id));
            (history_len.saturating_sub(rolled.len()), items)
        }
        Ok(Some(base)) => {
            let order: HashMap<i64, f64> = base.iter().map(|i| (i.id, i.order)).collect();
            let mut kept: Vec<Item> = items.into_iter().filter_map(|i| order.get(&i.id).map(|o| Item { id: i.id, order: *o })).collect();
            kept.sort_by(|a, b| b.order.partial_cmp(&a.order).unwrap_or(std::cmp::Ordering::Equal).then(b.id.cmp(&a.id)));
            (base.len(), kept)
        }
        Err(_) => (0, Vec::new()),
    }
}

impl Controller {
    /// The rolls' settings (which computed rolls are on, and the keys), from the config.
    pub fn set_roll_settings(&mut self, settings: RollSettings) {
        self.rolls.settings = settings;
    }

    pub fn roll_settings(&self) -> &RollSettings {
        &self.rolls.settings
    }

    pub(super) fn reload_rolls(&mut self) {
        self.rolls.list = self.store.borrow().rolls().unwrap_or_default();
    }

    /// The rolls in the order they are rolled through: the built-in LEFT, the rolls placed on the
    /// left of Main, Main, the rolls placed on its right, the built-in RIGHT, then the computed
    /// ones that are switched on. The built-in ends can be hidden.
    pub(super) fn rotation(&self) -> Vec<RollId> {
        let side = |s: RollSide| self.rolls.list.iter().filter(move |r| r.side == s).map(|r| RollId::Custom(r.id));
        let mut v = Vec::new();
        if self.rolls.settings.left {
            v.push(RollId::Custom(LEFT_ROLL));
        }
        v.extend(side(RollSide::Left));
        v.push(RollId::Main);
        v.extend(side(RollSide::Right));
        if self.rolls.settings.right {
            v.push(RollId::Custom(RIGHT_ROLL));
        }
        if self.rolls.settings.most_used {
            v.push(RollId::MostUsed);
        }
        if self.rolls.settings.images {
            v.push(RollId::Images);
        }
        v
    }

    pub(super) fn roll_name(&self, roll: RollId) -> String {
        match roll {
            RollId::Main => "Main".to_owned(),
            RollId::MostUsed => "Most used".to_owned(),
            RollId::Images => "Images".to_owned(),
            RollId::Custom(LEFT_ROLL) => "LEFT".to_owned(),
            RollId::Custom(RIGHT_ROLL) => "RIGHT".to_owned(),
            RollId::Custom(id) => self.rolls.list.iter().find(|r| r.id == id).map_or_else(|| "Roll".to_owned(), |r| r.name.clone()),
        }
    }

    /// A roll that has been deleted or switched off falls back to the main roll.
    pub(super) fn fix_roll(&mut self) {
        let rotation = self.rotation();
        if let Some(o) = self.open.as_mut() {
            if !rotation.contains(&o.roll) {
                o.roll = RollId::Main;
            }
        }
    }

    /// What the footer shows: the roll's name and place, once there is more than one roll. During
    /// the slide between two rolls it also carries the direction, for the arrow that flashes.
    pub(super) fn roll_view(&self) -> Option<RollView> {
        let o = self.open.as_ref()?;
        let rotation = self.rotation();
        if rotation.len() < 2 || self.ui_mode != UiMode::Complete || o.in_trash() {
            return None;
        }
        let position = rotation.iter().position(|r| *r == o.roll).unwrap_or(0) + 1;
        let flash = self.rolls.slide.as_ref().map_or(0, |s| s.dir);
        Some(RollView { name: self.roll_name(o.roll), position, count: rotation.len(), flash })
    }

    /// Rolls one step: `-1` to the previous roll, `1` to the next (the ends join up).
    pub(super) fn roll_step(&mut self, p: &mut dyn Platform, dir: i8, now: u64) {
        if self.open.is_none() || self.viewer.is_some() || self.editor.is_some() {
            return;
        }
        self.reload_rolls();
        self.fix_roll();
        let Some(o) = self.open.as_ref() else { return };
        if o.in_trash() {
            self.flash("Leave the recycle bin to roll between rolls", now);
            return;
        }
        let rotation = self.rotation();
        if rotation.len() < 2 {
            self.flash("There is only one roll. Turn on LEFT or RIGHT in Settings, or use Move to roll > New roll.", now);
            return;
        }
        let at = rotation.iter().position(|r| *r == o.roll).unwrap_or(0) as i64;
        let next = rotation[(at + i64::from(dir.signum())).rem_euclid(rotation.len() as i64) as usize];
        // the picture that is leaving
        let mut vm = self.view_model();
        let size = self.size_dip();
        let scale = self.overlay.scale;
        let from = self.renderer.render(&vm, size, scale);
        let split_px = (geometry(&vm, size, scale).list.y * scale).round().max(0.0) as u32;
        vm.wipe();
        if let Some(o) = self.open.as_mut() {
            o.roll = next;
            // a choice, a menu or a card belongs to the roll it was made on
            o.ui.picks.clear();
            o.ui.pick_anchor = None;
            o.ui.menu = None;
            o.ui.join = None;
            o.ui.dropdown_open = false;
            o.ui.scroll = 0.0;
        }
        self.withdraw_warning();
        self.refresh(p);
        if self.open.as_ref().is_some_and(|o| o.session.is_none()) {
            let name = self.roll_name(next);
            self.status = Some(Status { text: format!("{name} is empty. Move a clip into it with the right-click menu."), until_ms: now + 4_000 });
        } else if self.ui_mode == UiMode::NoBloat {
            let (name, position) = (self.roll_name(next), rotation.iter().position(|r| *r == next).unwrap_or(0) + 1);
            self.flash(&format!("{name} ({position} of {})", rotation.len()), now);
        }
        if self.ui_mode == UiMode::Complete {
            self.rolls.slide = Some(Slide { from, dir: if dir < 0 { -1 } else { 1 }, since_ms: now, split_px });
            p.animating(true);
        }
        self.dirty = true;
    }

    /// Called from the timer: keeps the slide going, and ends it.
    pub(super) fn slide_tick(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(s) = self.rolls.slide.as_ref() else { return };
        if now.saturating_sub(s.since_ms) >= SLIDE_MS {
            self.rolls.slide = None;
            self.animation_over(p);
        }
        self.dirty = true;
    }

    /// The frame to show: while a slide runs, the old picture and the new one in between.
    pub(super) fn slide_frame(&mut self, now: u64, frame: Frame) -> Frame {
        let Some(s) = self.rolls.slide.as_ref() else { return frame };
        let t = ease_out(now.saturating_sub(s.since_ms) as f32 / SLIDE_MS as f32);
        slide_frames(&s.from, &frame, t, s.dir, s.split_px)
    }

    /// Drops a slide that is running. `true` when there was one.
    pub(super) fn end_slide(&mut self) -> bool {
        self.rolls.slide.take().is_some()
    }

    /// The places the "Move to roll" menu offers, in the order it lists them: the same order the
    /// carousel rolls in, then the two ways to make a new roll (one on each side of Main).
    pub(super) fn move_targets(&self) -> Vec<MoveTarget> {
        let mut v: Vec<MoveTarget> = self
            .rotation()
            .into_iter()
            .filter_map(|r| match r {
                RollId::Main => Some(MoveTarget::Roll(None)),
                RollId::Custom(id) => Some(MoveTarget::Roll(Some(id))),
                // computed rolls hold clips, they do not take them
                RollId::MostUsed | RollId::Images => None,
            })
            .collect();
        // making a roll means typing its name, and a pinned-open carousel has no keyboard
        let kept = self.open.as_ref().is_some_and(|o| o.kept);
        if !kept && self.rolls.list.len() < ROLLS_MAX {
            v.push(MoveTarget::NewRoll(RollSide::Left));
            v.push(MoveTarget::NewRoll(RollSide::Right));
        }
        v
    }

    /// The lines of the "Move to roll" menu: Back, then one per target.
    pub(super) fn move_menu_items(&self) -> Vec<String> {
        let mut items = vec!["< Back".to_owned()];
        for t in self.move_targets() {
            items.push(match t {
                MoveTarget::Roll(None) => "Main".to_owned(),
                MoveTarget::Roll(Some(id)) => self.roll_name(RollId::Custom(id)),
                MoveTarget::NewRoll(RollSide::Left) => "New left roll...".to_owned(),
                MoveTarget::NewRoll(RollSide::Right) => "New right roll...".to_owned(),
            });
        }
        items
    }

    /// The clips a right-click menu acts on: the clip it was opened on, or else the chosen ones.
    pub(super) fn menu_ids(&self, target: Option<i64>) -> Vec<i64> {
        match target {
            Some(id) => vec![id],
            None => self.open.as_ref().map(|o| o.ui.picks.clone()).unwrap_or_default(),
        }
    }

    /// "New roll 1", "New roll 2"...: the first of those no roll is called yet.
    fn next_default_roll_name(&self) -> String {
        let taken: Vec<String> = self.rolls.list.iter().map(|r| r.name.to_lowercase()).collect();
        (1..).map(|n| format!("New roll {n}")).find(|name| !taken.contains(&name.to_lowercase())).unwrap_or_default()
    }

    /// "Move to roll > New left/right roll...": the search box asks for the name (Enter makes the
    /// roll on that side and moves the clips into it, nothing typed gives it a number, Esc cancels).
    /// A pinned-open carousel has no keyboard, so it makes the roll at once with the numbered name.
    pub(super) fn begin_new_roll(&mut self, p: &mut dyn Platform, ids: Vec<i64>, side: RollSide, now: u64) {
        if ids.is_empty() {
            return;
        }
        if self.open.as_ref().is_some_and(|o| o.kept) {
            let name = self.next_default_roll_name();
            return self.create_roll_and_move(p, ids, &name, side, now);
        }
        if let Some(o) = self.open.as_mut() {
            o.ui.dropdown_open = false;
            o.ui.menu = None;
            o.adding_filter = true;
            o.adding_roll = Some(ids);
            o.adding_roll_side = side;
            o.query.clear();
        }
        self.refresh(p);
    }

    /// Enter in the search box while it is naming a new roll.
    pub(super) fn finish_add_roll(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(o) = self.open.as_ref() else { return };
        let ids = o.adding_roll.clone().unwrap_or_default();
        let side = o.adding_roll_side;
        let typed = o.query.trim().to_owned();
        let name = if typed.is_empty() { self.next_default_roll_name() } else { typed };
        self.create_roll_and_move(p, ids, &name, side, now);
    }

    fn create_roll_and_move(&mut self, p: &mut dyn Platform, ids: Vec<i64>, name: &str, side: RollSide, now: u64) {
        let made = self.store.borrow_mut().add_roll_on(name, side);
        match made {
            Ok(id) => {
                self.reload_rolls();
                if let Some(o) = self.open.as_mut() {
                    o.adding_filter = false;
                    o.adding_roll = None;
                    o.query.clear();
                }
                self.move_clips_to(p, ids, Some(id), now);
            }
            Err(RollError::Refused(why)) => self.flash(&why, now),
            Err(e) => {
                p.log(&format!("could not make a roll: {e}"));
                self.flash("Could not make the roll", now);
            }
        }
    }

    /// Esc while naming a roll.
    pub(super) fn cancel_new_roll(&mut self) {
        if let Some(o) = self.open.as_mut() {
            o.adding_roll = None;
        }
    }

    /// Moves these clips into `roll` (`None`: the main roll).
    pub(super) fn move_clips_to(&mut self, p: &mut dyn Platform, ids: Vec<i64>, roll: Option<i64>, now: u64) {
        if ids.is_empty() {
            return;
        }
        let mut moved = 0usize;
        for id in &ids {
            match self.store.borrow_mut().set_clip_roll(*id, roll) {
                Ok(true) => moved += 1,
                Ok(false) => {}
                Err(e) => p.log(&format!("could not move a clip to a roll: {e}")),
            }
            self.row_cache.remove(id);
        }
        if let Some(o) = self.open.as_mut() {
            o.ui.picks.clear();
            o.ui.pick_anchor = None;
            o.ui.menu = None;
            o.index = None;
        }
        let name = match roll {
            None => "Main".to_owned(),
            Some(id) => self.rolls.list.iter().find(|r| r.id == id).map_or_else(|| "the roll".to_owned(), |r| r.name.clone()),
        };
        let text = match moved {
            0 => "Nothing was moved".to_owned(),
            1 => format!("Moved to {name}"),
            n => format!("Moved {n} clips to {name}"),
        };
        self.refresh(p);
        self.flash(&text, now);
    }
}
