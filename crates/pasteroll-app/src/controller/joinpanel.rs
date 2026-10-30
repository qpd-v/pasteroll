//! The multi-paste options card. With several clips chosen, holding the editor key (Ctrl by default)
//! while clicking where they should go, or Ctrl+E while Multi-paste is on, opens a small card over
//! the carousel instead of pasting at once: what joins the clips (new line, space, comma, tab or
//! nothing), a list style (plain, numbered, bulleted) and the order, with a preview of the result.
//! Enter or Paste pastes it, Copy puts it on the clipboard, Esc goes back to the carousel with the
//! clips still chosen. The last choices are kept until Pasteroll is restarted.

use super::*;
use pasteroll_render::{PanelGroup, PanelHit, PanelView};

/// The most text that is joined and pasted (or copied) at once.
pub(super) const JOINED_MAX_BYTES: usize = 16 * 1024 * 1024;

/// What goes between the clips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Sep {
    #[default]
    Line,
    Space,
    Comma,
    Tab,
    Nothing,
}

impl Sep {
    const ALL: [Sep; 5] = [Sep::Line, Sep::Space, Sep::Comma, Sep::Tab, Sep::Nothing];

    fn label(self) -> &'static str {
        match self {
            Sep::Line => "New line",
            Sep::Space => "Space",
            Sep::Comma => "Comma",
            Sep::Tab => "Tab",
            Sep::Nothing => "None",
        }
    }

    fn text(self, line_break: &str) -> String {
        match self {
            Sep::Line => line_break.to_owned(),
            Sep::Space => " ".to_owned(),
            Sep::Comma => ", ".to_owned(),
            Sep::Tab => "\t".to_owned(),
            Sep::Nothing => String::new(),
        }
    }
}

/// How each clip starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Style {
    #[default]
    Plain,
    Numbered,
    Bulleted,
}

impl Style {
    const ALL: [Style; 3] = [Style::Plain, Style::Numbered, Style::Bulleted];

    fn label(self) -> &'static str {
        match self {
            Style::Plain => "Plain",
            Style::Numbered => "Numbered",
            Style::Bulleted => "Bulleted",
        }
    }
}

/// Which clip comes first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Order {
    /// The order they were chosen in.
    #[default]
    Chosen,
    Newest,
    Oldest,
}

impl Order {
    const ALL: [Order; 3] = [Order::Chosen, Order::Newest, Order::Oldest];

    fn label(self) -> &'static str {
        match self {
            Order::Chosen => "As chosen",
            Order::Newest => "Newest first",
            Order::Oldest => "Oldest first",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct JoinOptions {
    pub sep: Sep,
    pub style: Style,
    pub order: Order,
}

/// The text of the chosen clips (each without its trailing line breaks), and how many were left out.
pub(super) struct Pieces {
    pub items: Vec<(i64, String)>,
    /// Left out because they are pictures.
    pub skipped: usize,
    /// Left out because they could not be read (gone, or damaged).
    pub unreadable: usize,
}

impl Drop for Pieces {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for (_, t) in &mut self.items {
            t.zeroize();
        }
    }
}

/// The clips joined as `opts` say. `order_ids` is the history order, newest first (for the two
/// history orders); a clip not in it counts as the oldest.
pub(super) fn compose(items: &[(i64, String)], opts: &JoinOptions, order_ids: &[i64], line_break: &str) -> String {
    let rank = |id: i64| order_ids.iter().position(|k| *k == id).unwrap_or(usize::MAX);
    let mut order: Vec<usize> = (0..items.len()).collect();
    match opts.order {
        Order::Chosen => {}
        Order::Newest => order.sort_by_key(|i| rank(items[*i].0)),
        Order::Oldest => order.sort_by_key(|i| std::cmp::Reverse(rank(items[*i].0))),
    }
    let sep = opts.sep.text(line_break);
    let mut out = String::new();
    for (n, i) in order.into_iter().enumerate() {
        if n > 0 {
            out.push_str(&sep);
        }
        match opts.style {
            Style::Plain => {}
            Style::Numbered => out.push_str(&format!("{}. ", n + 1)),
            Style::Bulleted => out.push_str("- "),
        }
        out.push_str(&items[i].1);
    }
    out
}

/// The first lines of `text`, for the preview.
fn preview_of(text: &str) -> String {
    const LINES: usize = 4;
    const WIDTH: usize = 70;
    let flat = text.replace('\t', "    ");
    let mut out = String::new();
    let mut count = 0;
    for line in flat.split(['\r', '\n']) {
        if count == LINES {
            out.push('\u{2026}');
            break;
        }
        if count > 0 {
            out.push('\n');
        }
        let cut: String = line.chars().take(WIDTH).collect();
        out.push_str(&cut);
        if line.chars().count() > WIDTH {
            out.push('\u{2026}');
        }
        count += 1;
    }
    out
}

/// The card while it is open.
pub(super) struct JoinPanel {
    pub opts: JoinOptions,
    pieces: Pieces,
    order_ids: Vec<i64>,
    /// A group (0 to 2), then Paste, Copy, Cancel.
    focus: usize,
    hover: Option<PanelHit>,
    note: Option<String>,
}

const GROUPS: usize = 3;
const FOCUS_SLOTS: usize = GROUPS + 3;

impl JoinPanel {
    fn text(&self, line_break: &str) -> String {
        compose(&self.pieces.items, &self.opts, &self.order_ids, line_break)
    }

    /// Option `option` of group `group` (what the chips show, in order).
    fn choose(&mut self, group: usize, option: usize) {
        match group {
            0 => self.opts.sep = Sep::ALL[option.min(Sep::ALL.len() - 1)],
            1 => self.opts.style = Style::ALL[option.min(Style::ALL.len() - 1)],
            _ => self.opts.order = Order::ALL[option.min(Order::ALL.len() - 1)],
        }
    }

    fn selected(&self, group: usize) -> (usize, usize) {
        match group {
            0 => (Sep::ALL.iter().position(|s| *s == self.opts.sep).unwrap_or(0), Sep::ALL.len()),
            1 => (Style::ALL.iter().position(|s| *s == self.opts.style).unwrap_or(0), Style::ALL.len()),
            _ => (Order::ALL.iter().position(|s| *s == self.opts.order).unwrap_or(0), Order::ALL.len()),
        }
    }

    /// The next (or previous) option of the focused group, or the next (or previous) button.
    fn step(&mut self, forward: bool) {
        if self.focus < GROUPS {
            let (at, n) = self.selected(self.focus);
            let next = if forward { (at + 1) % n } else { (at + n - 1) % n };
            self.choose(self.focus, next);
        } else {
            let buttons = FOCUS_SLOTS - GROUPS;
            let at = self.focus - GROUPS;
            self.focus = GROUPS + if forward { (at + 1) % buttons } else { (at + buttons - 1) % buttons };
        }
    }

    pub(super) fn view(&self) -> PanelView {
        let mut title = format!("Paste {} clips together", self.pieces.items.len());
        let left_out = self.pieces.skipped + self.pieces.unreadable;
        if left_out > 0 {
            title.push_str(&format!(" ({left_out} left out)"));
        }
        let labels = |group: usize, names: Vec<&'static str>, label: &str| PanelGroup {
            label: label.to_owned(),
            options: names.into_iter().map(str::to_owned).collect(),
            selected: self.selected(group).0,
        };
        PanelView {
            title,
            groups: vec![
                labels(0, Sep::ALL.iter().map(|s| s.label()).collect(), "Join with"),
                labels(1, Style::ALL.iter().map(|s| s.label()).collect(), "List style"),
                labels(2, Order::ALL.iter().map(|s| s.label()).collect(), "Order"),
            ],
            preview: preview_of(&self.text("\n")),
            note: self.note.clone().unwrap_or_default(),
            focus: self.focus,
            hover: self.hover,
        }
    }
}

impl Controller {
    /// The text of `picks`: each clip without its trailing line breaks (pictures and unreadable clips
    /// are counted, not included). `line_break` joins the paths of a files clip.
    pub(super) fn load_pieces(&self, picks: &[i64], line_break: &str) -> Result<Pieces, &'static str> {
        use zeroize::Zeroize;
        let st = self.store.borrow();
        let mut out = Pieces { items: Vec::new(), skipped: 0, unreadable: 0 };
        let mut bytes = 0usize;
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
                    // each clip is one piece: its own trailing line breaks would leave blank lines
                    let trimmed = t.trim_end_matches(['\r', '\n']).to_owned();
                    t.zeroize();
                    bytes += trimmed.len() + 8;
                    if bytes > JOINED_MAX_BYTES {
                        let mut trimmed = trimmed;
                        trimmed.zeroize();
                        return Err("Too much text to join at once");
                    }
                    out.items.push((id, trimmed));
                }
                None => out.unreadable += 1,
            }
        }
        Ok(out)
    }

    /// The ids of the loaded stretch of history, newest first.
    pub(super) fn history_ids(&self) -> Vec<i64> {
        self.open
            .as_ref()
            .and_then(|o| o.session.as_ref())
            .map(|s| s.window_items().0.iter().map(|i| i.id).collect())
            .unwrap_or_default()
    }

    /// Opens the card for the clips chosen. Nothing happens, with a word about why, when fewer than two
    /// text clips are chosen.
    pub(super) fn open_join_panel(&mut self, p: &mut dyn Platform, now: u64) {
        let Some(o) = self.open.as_ref() else { return };
        let picks = o.ui.picks.clone();
        if picks.len() < 2 {
            self.flash("Choose two or more clips first (Ctrl+click, or Shift+Down)", now);
            return;
        }
        let pieces = match self.load_pieces(&picks, p.line_break()) {
            Ok(pieces) if pieces.items.len() >= 2 => pieces,
            Ok(_) => {
                self.flash("Choose two or more text clips (pictures cannot be joined)", now);
                return;
            }
            Err(why) => {
                self.flash(why, now);
                return;
            }
        };
        let order_ids = self.history_ids();
        let opts = self.join_last;
        if let Some(o) = self.open.as_mut() {
            o.ui.menu = None;
            o.ui.dropdown_open = false;
            o.ui.join = Some(JoinPanel { opts, pieces, order_ids, focus: 0, hover: None, note: None });
        }
        self.withdraw_warning();
        self.dirty = true;
    }

    fn close_join_panel(&mut self) {
        if let Some(o) = self.open.as_mut() {
            o.ui.join = None;
            self.dirty = true;
        }
    }

    /// While the card is open it takes the input meant for the carousel. `true` = the event is used.
    pub(super) fn join_event(&mut self, p: &mut dyn Platform, ev: Event, now: u64) -> bool {
        match ev {
            Event::Key(key, gen) => {
                if self.live(p, gen) {
                    self.touch(p, now);
                    self.join_key(p, key, now);
                }
                true
            }
            Event::Step(step, gen) => {
                if self.live(p, gen) {
                    self.touch(p, now);
                    self.join_step(step == Step::Older);
                }
                true
            }
            Event::Mouse(m) => {
                self.touch(p, now);
                self.join_mouse(p, m, now);
                true
            }
            // a click elsewhere does not paste while the card is up; the wheel and the viewer's
            // click are not for it either
            Event::ClickPaste(_) | Event::ClickOutside(_) | Event::Wheel(..) | Event::OverlayWheel(_) | Event::ViewerMouse(_) => true,
            _ => false,
        }
    }

    fn join_step(&mut self, forward: bool) {
        if let Some(j) = self.open.as_mut().and_then(|o| o.ui.join.as_mut()) {
            j.step(forward);
            j.note = None;
            self.dirty = true;
        }
    }

    fn join_key(&mut self, p: &mut dyn Platform, key: Key, now: u64) {
        let focus = self.open.as_ref().and_then(|o| o.ui.join.as_ref()).map_or(0, |j| j.focus);
        match key {
            Key::Escape => self.close_join_panel(),
            Key::Enter => match focus {
                f if f == GROUPS + 1 => self.join_copy(p, now),
                f if f == GROUPS + 2 => self.close_join_panel(),
                _ => self.join_paste(p),
            },
            Key::CtrlC => self.join_copy(p, now),
            Key::Tab => {
                if let Some(j) = self.open.as_mut().and_then(|o| o.ui.join.as_mut()) {
                    j.focus = (j.focus + 1) % FOCUS_SLOTS;
                    self.dirty = true;
                }
            }
            Key::Up | Key::Left => self.join_step(false),
            Key::Down | Key::Right => self.join_step(true),
            _ => {}
        }
    }

    fn join_mouse(&mut self, p: &mut dyn Platform, m: MouseEvent, now: u64) {
        let mut vm = self.view_model();
        let hit = self.renderer.hit_test(&vm, self.size_dip(), self.overlay.scale, m.x, m.y);
        vm.wipe();
        let over = match hit {
            Hit::Panel(h) => Some(h),
            _ => None,
        };
        match m.kind {
            MouseKind::Move => {
                if let Some(j) = self.open.as_mut().and_then(|o| o.ui.join.as_mut()) {
                    if j.hover != over {
                        j.hover = over;
                        self.dirty = true;
                    }
                }
            }
            MouseKind::Down | MouseKind::DoubleClick => match over {
                Some(PanelHit::Option(group, option)) => {
                    if let Some(j) = self.open.as_mut().and_then(|o| o.ui.join.as_mut()) {
                        j.choose(group, option);
                        j.focus = group;
                        j.note = None;
                        self.dirty = true;
                    }
                }
                Some(PanelHit::Paste) => self.join_paste(p),
                Some(PanelHit::Copy) => self.join_copy(p, now),
                Some(PanelHit::Cancel) => self.close_join_panel(),
                None => {}
            },
            _ => {}
        }
    }

    /// Pastes the joined text into the session's window. A multi-line paste into a terminal asks first,
    /// on the card.
    fn join_paste(&mut self, p: &mut dyn Platform) {
        let Some((id, text, used, opts, warned)) = self.open.as_ref().and_then(|o| {
            let j = o.ui.join.as_ref()?;
            let first = j.pieces.items.first()?.0;
            Some((first, j.text(p.line_break()), j.pieces.items.len(), j.opts, o.ui.warned))
        }) else {
            return;
        };
        self.join_last = opts;
        self.joined_paste = Some(used);
        let checked = self.paste_item(p, id, Some(text), warned);
        self.joined_paste = None;
        if let PasteCheck::Warn(lines) = checked {
            if let Some(o) = self.open.as_mut() {
                o.ui.warned = true;
                if let Some(j) = o.ui.join.as_mut() {
                    j.note = Some(if o.kept {
                        format!("{lines} lines into a terminal may run commands. Press Paste again to go ahead.")
                    } else {
                        format!("{lines} lines into a terminal may run commands. Enter pastes, Esc cancels.")
                    });
                }
            }
            self.dirty = true;
        }
    }

    /// Puts the joined text on the clipboard, pastes nothing and closes.
    fn join_copy(&mut self, p: &mut dyn Platform, now: u64) {
        let Some((text, used, skipped, opts)) = self.open.as_ref().and_then(|o| {
            let j = o.ui.join.as_ref()?;
            Some((j.text(p.line_break()), j.pieces.items.len(), j.pieces.skipped + j.pieces.unreadable, j.opts))
        }) else {
            return;
        };
        self.join_last = opts;
        self.finish_copy(p, Payload::Text(text), used, skipped, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<(i64, String)> {
        vec![(5, "five".into()), (9, "nine".into()), (2, "two".into())]
    }

    #[test]
    fn the_defaults_are_one_clip_per_line_in_the_order_chosen() {
        let out = compose(&items(), &JoinOptions::default(), &[], "\n");
        assert_eq!(out, "five\nnine\ntwo");
    }

    #[test]
    fn each_separator_joins_as_it_says() {
        let o = |sep| JoinOptions { sep, ..JoinOptions::default() };
        assert_eq!(compose(&items(), &o(Sep::Space), &[], "\r\n"), "five nine two");
        assert_eq!(compose(&items(), &o(Sep::Comma), &[], "\r\n"), "five, nine, two");
        assert_eq!(compose(&items(), &o(Sep::Tab), &[], "\r\n"), "five\tnine\ttwo");
        assert_eq!(compose(&items(), &o(Sep::Nothing), &[], "\r\n"), "fiveninetwo");
        assert_eq!(compose(&items(), &o(Sep::Line), &[], "\r\n"), "five\r\nnine\r\ntwo", "the platform's own line break");
    }

    #[test]
    fn lists_are_numbered_or_bulleted_from_the_first_clip() {
        let numbered = JoinOptions { style: Style::Numbered, ..JoinOptions::default() };
        assert_eq!(compose(&items(), &numbered, &[], "\n"), "1. five\n2. nine\n3. two");
        let bulleted = JoinOptions { style: Style::Bulleted, sep: Sep::Space, ..JoinOptions::default() };
        assert_eq!(compose(&items(), &bulleted, &[], "\n"), "- five - nine - two");
    }

    #[test]
    fn the_history_orders_follow_the_list_not_the_choosing() {
        // newest first in the list: 2, then 5, then 9
        let history = [2, 7, 5, 8, 9];
        let newest = JoinOptions { order: Order::Newest, ..JoinOptions::default() };
        assert_eq!(compose(&items(), &newest, &history, "\n"), "two\nfive\nnine");
        let oldest = JoinOptions { order: Order::Oldest, ..JoinOptions::default() };
        assert_eq!(compose(&items(), &oldest, &history, "\n"), "nine\nfive\ntwo");
        // a clip that is not in the list counts as the oldest
        let unknown = vec![(1, "gone".to_owned()), (2, "two".to_owned())];
        assert_eq!(compose(&unknown, &newest, &history, "\n"), "two\ngone");
    }

    #[test]
    fn the_preview_is_a_few_short_lines() {
        let long = (0..9).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let p = preview_of(&long);
        assert_eq!(p.lines().count(), 4, "four lines, the last one ending with the ellipsis");
        assert!(p.ends_with('\u{2026}'));
        let wide = preview_of(&"x".repeat(200));
        assert!(wide.chars().count() <= 71);
        assert_eq!(preview_of("a\tb"), "a    b");
    }
}
