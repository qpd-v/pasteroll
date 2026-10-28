//! Choosing several clips in the Complete carousel and pasting or copying them together.
//!
//! Ctrl+click chooses or un-chooses a clip, Ctrl+Shift+click chooses everything from the last clip
//! chosen to the one clicked. The Multi-paste button switches on the same thing for plain clicks
//! (Shift+click for a range). The chosen text clips are joined, one per line, in the order they were
//! chosen. Clicking where the text should go pastes it, Ctrl+C (or Copy in the right-click menu)
//! puts it on the clipboard instead. Pictures cannot be joined into text and are left out. No Bloat
//! has none of this: it shows one clip at a time.

use super::*;
use pasteroll_render::MenuView;

/// At most this many clips can be chosen at once.
const PICKS_MAX: usize = 500;
/// The most text that is joined and pasted (or copied) at once.
const JOINED_MAX_BYTES: usize = 16 * 1024 * 1024;

const MENU_COPY: usize = 0;
const MENU_CLEAR: usize = 1;

/// The right-click menu: where it was opened (DIP) and the entry under the pointer.
#[derive(Clone, Copy)]
pub(super) struct Menu {
    pub at: (f32, f32),
    pub hover: Option<usize>,
}

impl Menu {
    pub(super) fn view(self, picks: usize) -> MenuView {
        let copy = if picks > 1 { format!("Copy {picks} clips") } else { "Copy".to_owned() };
        MenuView { items: vec![copy, "Clear selection".to_owned()], at: self.at, hover: self.hover }
    }
}

/// The footer while clips are chosen.
pub(super) fn footer_text(n: usize, multi_on: bool) -> String {
    match (n, multi_on) {
        (0, _) => "Multi-paste: click the clips to choose, then click where to paste.".to_owned(),
        (1, true) => "1 chosen. Click more. Click where to paste, Ctrl+C copies.".to_owned(),
        (1, false) => "1 chosen. Ctrl+click more. Click where to paste, Ctrl+C copies.".to_owned(),
        _ => format!("{n} chosen. Click where to paste, Ctrl+C copies."),
    }
}

/// Chosen clips joined into one text.
struct Joined {
    text: String,
    /// How many clips went into `text`.
    used: usize,
    /// How many were left out because they are pictures.
    skipped: usize,
    /// How many were left out because they could not be read (gone, or damaged).
    unreadable: usize,
}

impl Drop for Joined {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.text.zeroize();
    }
}

impl Controller {
    fn multi_allowed(&self) -> bool {
        self.ui_mode == UiMode::Complete && self.viewer.is_none() && self.editor.is_none()
    }

    /// Ids of the rows drawn (the loaded stretch of history around the shown clip), and the index
    /// of the shown clip among them.
    pub(super) fn row_ids(&self) -> (Vec<i64>, usize) {
        let Some(s) = self.open.as_ref().and_then(|o| o.session.as_ref()) else { return (Vec::new(), 0) };
        let (items, cur) = s.window_items();
        // No Bloat shows the one clip, not the list around it
        let (before, after) = if self.ui_mode == UiMode::NoBloat { (0, 0) } else { (ROWS_BEFORE, ROWS_AFTER) };
        let lo = cur.saturating_sub(before);
        let hi = (cur + 1 + after).min(items.len());
        (items[lo..hi].iter().map(|i| i.id).collect(), cur - lo)
    }

    /// The content of clip `id`, as what would be put on the clipboard.
    pub(super) fn clip_payload(&self, id: i64) -> Option<Payload> {
        let st = self.store.borrow();
        match st.kind(id) {
            Ok(Some(Kind::Text)) => st.text(id).ok().flatten().map(Payload::Text),
            Ok(Some(Kind::Files)) => st.files(id).ok().flatten().map(Payload::Files),
            Ok(Some(Kind::Image)) => st.image_png(id).ok().flatten().map(Payload::Image),
            _ => None,
        }
    }

    /// Is the right-click menu closed by this? (`true` if one was open.)
    pub(super) fn close_menu(&mut self) -> bool {
        let closed = self.open.as_mut().is_some_and(|o| o.ui.menu.take().is_some());
        if closed {
            self.dirty = true;
        }
        closed
    }

    /// Esc: forget the chosen clips. `false` when none were chosen.
    pub(super) fn clear_picks(&mut self) -> bool {
        match self.open.as_mut() {
            Some(o) if !o.ui.picks.is_empty() => {
                o.ui.picks.clear();
                o.ui.pick_anchor = None;
                self.dirty = true;
                self.withdraw_warning();
                true
            }
            _ => false,
        }
    }

    /// The image editor opens for the shown picture unless several clips are chosen. One chosen clip
    /// that is the picture itself still counts as "no choice".
    pub(super) fn picks_allow_editor(&self) -> bool {
        self.open.as_ref().is_some_and(|o| {
            o.ui.picks.is_empty() || (o.ui.picks.len() == 1 && Some(o.ui.picks[0]) == o.current_id())
        })
    }

    /// Mouse handling for choosing clips and for the right-click menu. `true` = the event is used.
    pub(super) fn multi_mouse(&mut self, p: &mut dyn Platform, m: MouseEvent, hit: Hit, scale: f32, now: u64) -> bool {
        if !self.multi_allowed() || self.open.is_none() {
            return false;
        }
        let row_id = |s: &Self| {
            let (ids, active) = s.row_ids();
            match hit {
                Hit::Row(i) => ids.get(i).copied(),
                Hit::Body { .. } => ids.get(active).copied(),
                _ => None,
            }
        };
        match m.kind {
            MouseKind::RightDown => {
                self.close_menu();
                let Some(id) = row_id(self) else { return true };
                if let Some(o) = self.open.as_mut() {
                    // like a file manager: a right click on a clip that is not chosen chooses it alone
                    if !o.ui.picks.contains(&id) {
                        o.ui.picks = vec![id];
                        o.ui.pick_anchor = Some(id);
                    }
                    o.ui.menu = Some(Menu { at: (m.x / scale, m.y / scale), hover: None });
                    self.dirty = true;
                }
                self.withdraw_warning();
                true
            }
            MouseKind::Down | MouseKind::DoubleClick => {
                if self.open.as_ref().is_some_and(|o| o.ui.menu.is_some()) {
                    self.close_menu();
                    if let Hit::MenuItem(i) = hit {
                        match i {
                            MENU_COPY => self.copy_picks(p, now),
                            MENU_CLEAR => {
                                self.clear_picks();
                            }
                            _ => {}
                        }
                    }
                    // a click that only closes the menu does nothing else
                    return true;
                }
                if hit == Hit::MultiButton {
                    self.toggle_multi_mode();
                    return true;
                }
                let mods = p.modifiers_now();
                let multi_on = self.open.as_ref().is_some_and(|o| o.ui.multi_on);
                // with Multi-paste on a plain click chooses; otherwise it takes Ctrl
                if mods.alt || !(mods.ctrl || multi_on) {
                    return false;
                }
                let Some(id) = row_id(self) else { return false };
                if mods.shift {
                    self.pick_range(id, now);
                } else {
                    self.pick_toggle(id, now);
                }
                true
            }
            _ => false,
        }
    }

    /// The Multi-paste button: plain clicks choose clips until it is pressed again. The clips
    /// already chosen stay chosen.
    fn toggle_multi_mode(&mut self) {
        if let Some(o) = self.open.as_mut() {
            o.ui.multi_on = !o.ui.multi_on;
            o.ui.dropdown_open = false;
            self.dirty = true;
        }
        self.withdraw_warning();
    }

    /// Ctrl+click: chooses the clip, or takes it out again.
    fn pick_toggle(&mut self, id: i64, now: u64) {
        let Some(o) = self.open.as_mut() else { return };
        if let Some(at) = o.ui.picks.iter().position(|k| *k == id) {
            o.ui.picks.remove(at);
        } else if o.ui.picks.len() >= PICKS_MAX {
            self.flash(&format!("At most {PICKS_MAX} clips can be chosen at once"), now);
            return;
        } else {
            o.ui.picks.push(id);
        }
        o.ui.pick_anchor = Some(id);
        self.dirty = true;
        self.withdraw_warning();
    }

    /// Ctrl+Shift+click: chooses every clip from the anchor to `id`, in the order walked from the
    /// anchor. Only the stretch of history that is loaded can be walked (around 300 clips about the
    /// one shown); with no anchor, or one outside it, this is a plain Ctrl+click.
    fn pick_range(&mut self, id: i64, now: u64) {
        let Some(o) = self.open.as_ref() else { return };
        let ids: Vec<i64> = o
            .session
            .as_ref()
            .map(|s| s.window_items().0.iter().map(|i| i.id).collect())
            .unwrap_or_default();
        let ends = o
            .ui
            .pick_anchor
            .and_then(|a| Some((ids.iter().position(|k| *k == a)?, ids.iter().position(|k| *k == id)?)));
        let Some((from, to)) = ends else {
            self.pick_toggle(id, now);
            return;
        };
        let span: Vec<i64> = if from <= to {
            ids[from..=to].to_vec()
        } else {
            ids[to..=from].iter().rev().copied().collect()
        };
        let Some(o) = self.open.as_mut() else { return };
        let mut full = false;
        for k in span {
            if o.ui.picks.contains(&k) {
                continue;
            }
            if o.ui.picks.len() >= PICKS_MAX {
                full = true;
                break;
            }
            o.ui.picks.push(k);
        }
        // the anchor stays: a second Ctrl+Shift+click grows or shrinks the walk from the same clip
        self.dirty = true;
        self.withdraw_warning();
        if full {
            self.flash(&format!("At most {PICKS_MAX} clips can be chosen at once"), now);
        }
    }

    /// The text of `picks`, one clip per line.
    fn join_picks(&self, picks: &[i64], line_break: &str) -> Result<Joined, &'static str> {
        use zeroize::Zeroize;
        let st = self.store.borrow();
        let mut out = Joined { text: String::new(), used: 0, skipped: 0, unreadable: 0 };
        for &id in picks {
            let piece = match st.kind(id) {
                Ok(Some(Kind::Text)) => st.text(id).ok().flatten(),
                Ok(Some(Kind::Files)) => st.files(id).ok().flatten().map(|f| f.join(line_break)),
                Ok(Some(Kind::Image)) => {
                    out.skipped += 1;
                    continue;
                }
                _ => None,
            };
            match piece {
                Some(mut t) => {
                    // each clip is one line: its own trailing line breaks would leave blank lines
                    let trimmed = t.trim_end_matches(['\r', '\n']);
                    let extra = trimmed.len() + if out.used > 0 { line_break.len() } else { 0 };
                    if out.text.len() + extra > JOINED_MAX_BYTES {
                        t.zeroize();
                        return Err("Too much text to join at once");
                    }
                    if out.used > 0 {
                        out.text.push_str(line_break);
                    }
                    out.text.push_str(trimmed);
                    out.used += 1;
                    t.zeroize();
                }
                None => out.unreadable += 1,
            }
        }
        Ok(out)
    }

    /// Keeps a multi-paste in history as one text clip (shown with the multi-paste badge), unless
    /// recording is paused. Failing to keep it never stops the paste.
    pub(super) fn record_joined(&mut self, p: &mut dyn Platform, text: &str, clips: usize) {
        if self.is_paused() {
            return;
        }
        let now = p.now_secs();
        let kept = self.store.borrow_mut().add_joined_text(text, now, clips);
        match kept {
            Ok(_) => p.log(&format!("kept a multi-paste of {clips} clips in history")),
            Err(e) => p.log(&format!("could not keep the multi-paste in history: {e}")),
        }
    }

    /// Enter, or a click where the text should go, with clips chosen.
    pub(super) fn confirm_picks(&mut self, p: &mut dyn Platform) {
        let Some(open) = self.open.as_ref() else { return };
        let (picks, warned) = (open.ui.picks.clone(), open.ui.warned);
        let now = p.now_ms();
        let mut joined = None;
        let (id, selection) = match picks.as_slice() {
            [] => return,
            // one clip: pasted as itself, so a picture stays a picture
            [only] => (*only, None),
            many => match self.join_picks(many, p.line_break()) {
                Ok(j) if j.used > 0 => {
                    if j.skipped + j.unreadable > 0 {
                        p.log(&format!("{} clip(s) left out of a multiple paste", j.skipped + j.unreadable));
                    }
                    joined = (j.used > 1).then_some(j.used);
                    (many[0], Some(j.text.clone()))
                }
                Ok(_) => {
                    self.flash("Pictures cannot be joined into text. Choose text clips.", now);
                    return;
                }
                Err(why) => {
                    self.flash(why, now);
                    return;
                }
            },
        };
        self.joined_paste = joined;
        let checked = self.paste_item(p, id, selection, warned);
        self.joined_paste = None;
        if let PasteCheck::Warn(lines) = checked {
            if let Some(o) = self.open.as_mut() {
                o.ui.warned = true;
            }
            // Stays up until the user decides: Enter to paste, anything else to back out.
            self.status = Some(Status {
                text: format!("{lines} line(s) into a terminal may run commands. Enter pastes, Esc cancels."),
                until_ms: u64::MAX,
            });
            self.dirty = true;
        }
    }

    /// Ctrl+C or Copy in the menu: the chosen clips (or the one shown, when none are chosen) go on
    /// the clipboard, nothing is pasted, and the carousel closes. Pasteroll's own writes are not
    /// recorded again.
    pub(super) fn copy_picks(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(o) = self.open.as_ref() else { return };
        let ids: Vec<i64> = if o.ui.picks.is_empty() { o.current_id().into_iter().collect() } else { o.ui.picks.clone() };
        let highlighted = if o.ui.picks.is_empty() { self.selected_text() } else { None };
        let (payload, used, skipped) = match ids.as_slice() {
            _ if highlighted.is_some() => (Payload::Text(highlighted.unwrap_or_default()), 1, 0),
            [] => return, // nothing matches the search
            [one] => match self.clip_payload(*one) {
                Some(pl) => (pl, 1, 0),
                None => {
                    self.flash("That clip could not be read", now);
                    return;
                }
            },
            many => match self.join_picks(many, p.line_break()) {
                Ok(j) if j.used > 0 => (Payload::Text(j.text.clone()), j.used, j.skipped + j.unreadable),
                Ok(_) => {
                    self.flash("Pictures cannot be joined into text. Choose text clips.", now);
                    return;
                }
                Err(why) => {
                    self.flash(why, now);
                    return;
                }
            },
        };
        let payload = WipeOnDrop(payload);
        if let Err(e) = p.write_clipboard(&payload.0) {
            p.log(&format!("could not write clipboard: {e}"));
            self.flash("Could not copy to the clipboard", now);
            return;
        }
        p.log(&format!("copied {used} clip(s) to the clipboard without pasting"));
        if used > 1 {
            if let Payload::Text(t) = &payload.0 {
                self.record_joined(p, t, used);
            }
        }
        let message = match (used, skipped) {
            (1, _) => "Copied".to_owned(),
            (n, 0) => format!("Copied {n} clips"),
            (n, s) => format!("Copied {n} clips ({s} left out: pictures or unreadable)"),
        };
        self.end_session(p);
        self.notify(p, &message, now);
    }
}
