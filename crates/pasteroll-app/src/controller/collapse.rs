//! The tall card turning into a one-line row while clips are being chosen for a multiple paste, and
//! back again. The controller only keeps the amount (0 to 1); the renderer lays the list out from it.

use super::*;

/// How long the card takes to collapse or open again.
const COLLAPSE_MS: u64 = 200;

/// Rows shown either side of the active one once the card is a one-liner (the list then has room
/// for about a dozen).
const COLLAPSED_BEFORE: usize = 11;
const COLLAPSED_AFTER: usize = 11;

/// Where the card is between tall (0) and one line (1), and where it is heading.
#[derive(Default)]
pub(super) struct Collapse {
    value: f32,
    from: f32,
    to: f32,
    start_ms: u64,
    running: bool,
}

impl Collapse {
    pub(super) fn value(&self) -> f32 {
        self.value
    }

    pub(super) fn running(&self) -> bool {
        self.running
    }

    /// Is the card a one-liner or heading to be one? (More rows are wanted then.)
    pub(super) fn collapsing(&self) -> bool {
        if self.running {
            self.to > 0.5
        } else {
            self.value > 0.5
        }
    }

    /// Heads for collapsed (`true`) or tall (`false`). Returns whether that started a move.
    pub(super) fn aim(&mut self, collapsed: bool, now: u64) -> bool {
        let to = if collapsed { 1.0 } else { 0.0 };
        let already = if self.running { self.to == to } else { self.value == to };
        if already {
            return false;
        }
        self.from = self.value;
        self.to = to;
        self.start_ms = now;
        self.running = true;
        true
    }

    /// Moves to where the clock says. Returns whether it is still moving.
    pub(super) fn step(&mut self, now: u64) -> bool {
        if !self.running {
            return false;
        }
        let t = (now.saturating_sub(self.start_ms) as f32 / COLLAPSE_MS as f32).clamp(0.0, 1.0);
        let e = t * t * (3.0 - 2.0 * t);
        self.value = if t >= 1.0 { self.to } else { self.from + (self.to - self.from) * e };
        self.running = t < 1.0;
        self.running
    }
}

impl Controller {
    /// How many rows are loaded either side of the shown clip.
    pub(super) fn rows_span(&self) -> (usize, usize) {
        if self.open.as_ref().is_some_and(|o| o.ui.collapse.collapsing()) {
            (COLLAPSED_BEFORE, COLLAPSED_AFTER)
        } else {
            (ROWS_BEFORE, ROWS_AFTER)
        }
    }

    /// Multi-paste is in use: its button is on, or clips are chosen.
    fn wants_collapse(&self) -> bool {
        self.ui_mode == UiMode::Complete && self.open.as_ref().is_some_and(|o| o.ui.multi_on || !o.ui.picks.is_empty())
    }

    /// Called after every event: starts the card moving when multi-paste has just begun or ended.
    pub(super) fn sync_collapse(&mut self, p: &mut dyn Platform, now: u64) {
        let want = self.wants_collapse();
        let Some(o) = self.open.as_mut() else { return };
        if o.ui.collapse.aim(want, now) {
            p.animating(true);
            self.dirty = true;
        }
    }

    /// Called on every tick: moves the card along.
    pub(super) fn collapse_tick(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(o) = self.open.as_mut() else { return };
        if !o.ui.collapse.running() {
            return;
        }
        let more = o.ui.collapse.step(now);
        self.dirty = true;
        if !more {
            self.animation_over(p);
        }
    }

    /// The card has stopped moving: slow the timer again unless something else is still moving.
    pub(super) fn animation_over(&mut self, p: &mut dyn Platform) {
        if !self.viewer_animating() && !self.editor_animating() {
            p.animating(false);
        }
    }
}
