//! Pinning the carousel open.
//!
//! A carousel that is pinned open ("kept") outlives the click that normally closes it: it stays on
//! top of the other windows while they are used, and the input hooks let go of the keyboard and the
//! wheel everywhere else (only the wheel over the carousel itself still scrolls it). Because the
//! keyboard belongs to the window being used, there is no typing into the search box and no
//! Enter/Esc; the bar's Paste button and the favorite tiles paste into whichever window is in front
//! at that moment, and the pin button closes it again. The expanded viewer and the image editor
//! need the hooks, so they are not offered while it is pinned. Multi-paste works with the mouse
//! alone: the Multi-paste button (or Ctrl+click) chooses clips, the Paste button pastes them.

use super::*;

/// How often a pinned-open carousel is put back above windows that made themselves topmost.
pub(super) const RAISE_EVERY_MS: u64 = 1_000;
/// How long the hint stays in the footer after pinning.
const HINT_MS: u64 = 6_000;

impl Controller {
    /// The Paste button in the bar: pastes the chosen clips (or the one shown) into the window in
    /// front. With two or more chosen, Ctrl+click (the advanced-paste key) opens the options card.
    pub(super) fn paste_button(&mut self, p: &mut dyn Platform, now: u64) {
        let picks = self.open.as_ref().map_or(0, |o| o.ui.picks.len());
        if picks >= 2 && p.modifiers_now().has(self.advanced_modifier) {
            self.open_join_panel(p, now);
        } else {
            self.confirm(p);
        }
    }

    /// A paste or copy went through while pinned open: the choice is spent, but the carousel stays.
    pub(super) fn finish_kept_picks(&mut self) {
        if let Some(o) = self.open.as_mut() {
            o.ui.picks.clear();
            o.ui.pick_anchor = None;
            o.ui.join = None;
            o.ui.menu = None;
            o.ui.warned = false;
        }
        self.status = None;
        self.dirty = true;
    }

    /// The pin button (or Ctrl+K): pins the carousel open, or closes it when it already is.
    pub(super) fn toggle_keep(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(open) = self.open.as_ref() else { return };
        if open.kept {
            p.log("session closed: pin button");
            self.end_session(p);
            return;
        }
        // the viewer and the editor own the keyboard and the wheel; they come first
        if self.viewer.is_some() || self.editor.is_some() {
            return;
        }
        self.close_menu();
        self.clear_picks();
        self.withdraw_warning();
        if let Some(o) = self.open.as_mut() {
            o.kept = true;
            o.browse_only = false;
            o.ui.click = None;
            o.ui.multi_on = false;
            o.ui.dropdown_open = false;
            o.ui.warned = false;
        }
        self.kept_wheel.reset();
        self.last_activity_ms = now;
        self.last_raise_ms = now;
        p.input_active(false);
        self.status = Some(Status {
            text: "Pinned open. The Paste button pastes into the window you were using; click the pin to close.".into(),
            until_ms: now + HINT_MS,
        });
        self.dirty = true;
        p.log("session pinned open");
    }
}
