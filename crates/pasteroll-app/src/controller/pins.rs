//! Pinned clips as small tiles between the search bar and the list.
//!
//! A tile shows a picture clip's picture (cropped to a square) or a symbol for text, files, video
//! and audio. A click pastes the clip. Resting the pointer on a tile for a second opens a popup
//! with the whole clip (text, or the picture bigger) that follows the pointer while it stays on the
//! tile. The right-click menu pastes, unpins, and sets the colour of the tile's edge and symbol;
//! the colours are kept by the platform in a small file (`pin-colors.txt`: clip number, colour).

use super::*;
use pasteroll_render::{PinKind, PinTip, PinView};

/// At most this many pinned clips are loaded for the tiles (as many as fit are drawn).
const PIN_MAX: usize = 24;
/// How long the pointer rests on a tile before the popup opens.
const TIP_DELAY_MS: u64 = 1_000;
/// The popup shows at most this much of a text.
const TIP_TEXT_CHARS: usize = 1_500;
const COLORS_MAX: usize = 500;

/// The right-click menu's colours, by name.
pub(super) const PIN_COLORS: [(&str, (u8, u8, u8)); 8] = [
    ("Red", (232, 96, 96)),
    ("Orange", (240, 150, 70)),
    ("Yellow", (232, 200, 80)),
    ("Green", (110, 200, 110)),
    ("Teal", (47, 174, 151)),
    ("Blue", (90, 150, 235)),
    ("Purple", (160, 120, 230)),
    ("Pink", (235, 120, 180)),
];

pub(super) const PIN_MENU_PASTE: usize = 0;
pub(super) const PIN_MENU_UNPIN: usize = 1;
pub(super) const PIN_MENU_PLAIN: usize = 2;

/// The entries and colour dots of a pinned clip's right-click menu.
pub(super) fn menu_entries() -> (Vec<String>, Vec<Option<(u8, u8, u8)>>) {
    let mut items = vec!["Paste".to_owned(), "Remove favorite".to_owned(), "Plain colour".to_owned()];
    let mut dots = vec![None, None, None];
    for (name, rgb) in PIN_COLORS {
        items.push(name.to_owned());
        dots.push(Some(rgb));
    }
    (items, dots)
}

#[derive(Default)]
pub(super) struct Pins {
    /// The pinned clips shown, newest first.
    ids: Vec<i64>,
    kinds: HashMap<i64, PinKind>,
    /// `ids` is out of date (a clip was pinned, unpinned, deleted or restored).
    stale: bool,
    colors: HashMap<i64, (u8, u8, u8)>,
    /// The tile the pointer is on, since when, and where the pointer is (DIP).
    hover: Option<(usize, u64)>,
    at: (f32, f32),
    /// The popup is open (the pointer rested for [`TIP_DELAY_MS`]).
    tip_on: bool,
    /// The popup's text, kept while the pointer stays on the same clip.
    tip_text: Option<(i64, String)>,
    /// A pinned clip whose multi-line paste into a terminal was announced; clicking it again goes ahead.
    warned: Option<i64>,
}

impl Pins {
    pub(super) fn forget_tip(&mut self) {
        use zeroize::Zeroize;
        if let Some((_, t)) = self.tip_text.as_mut() {
            t.zeroize();
        }
        self.tip_text = None;
        self.tip_on = false;
    }

    /// The pointer left the tiles (or the overlay closed).
    pub(super) fn reset(&mut self) {
        self.hover = None;
        self.warned = None;
        self.forget_tip();
    }

    pub(super) fn invalidate(&mut self) {
        self.stale = true;
    }

    fn colors_text(&self) -> String {
        let mut entries: Vec<_> = self.colors.iter().collect();
        entries.sort_by_key(|(id, _)| **id);
        entries.iter().map(|(id, c)| format!("{id} {:02x}{:02x}{:02x}\n", c.0, c.1, c.2)).collect()
    }

    fn parse_colors(&mut self, text: &str) {
        self.colors.clear();
        for line in text.lines().take(COLORS_MAX) {
            let mut it = line.split_whitespace();
            let (Some(id), Some(hex)) = (it.next(), it.next()) else { continue };
            let (Ok(id), true) = (id.parse::<i64>(), hex.len() == 6) else { continue };
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            if let (Some(r), Some(g), Some(b)) = (byte(0), byte(2), byte(4)) {
                self.colors.insert(id, (r, g, b));
            }
        }
    }
}

fn kind_of_files(first: &str) -> PinKind {
    let ext = first.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "mpg" | "mpeg" | "flv" | "3gp" => PinKind::Video,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" | "mid" | "midi" => PinKind::Audio,
        _ => PinKind::File,
    }
}

impl Controller {
    /// Called when the carousel opens: reads the colours file and reloads the pinned list.
    pub(super) fn pins_open(&mut self, p: &mut dyn Platform) {
        let text = p.load_pin_colors();
        self.pins.parse_colors(&text);
        self.pins.reset();
        self.pins.stale = true;
    }

    fn pins_reload(&mut self) {
        if !self.pins.stale {
            return;
        }
        self.pins.stale = false;
        let ids = self.store.borrow().pinned_ids(PIN_MAX).unwrap_or_default();
        self.pins.kinds.clear();
        for &id in &ids {
            let kind = match self.store.borrow().info(id) {
                Ok(Some(i)) => match i.kind {
                    Kind::Text => PinKind::Text,
                    Kind::Image => PinKind::Image,
                    Kind::Files => kind_of_files(i.search.lines().next().unwrap_or("")),
                },
                _ => PinKind::Text,
            };
            self.pins.kinds.insert(id, kind);
        }
        self.pins.ids = ids;
    }

    /// The picture for a pinned picture clip's tile or popup (bigger when at hand), asked for once.
    fn pin_picture(&mut self, id: i64) -> Option<Arc<core_image::Rgba>> {
        if let Some((_, img)) = self.thumbs.iter().find(|(k, _)| *k == id) {
            return Some(img.clone());
        }
        self.row_thumb(id)
    }

    /// Fills the view model's tiles and popup.
    pub(super) fn view_pins(&mut self, vm: &mut ViewModel) {
        let hidden = vm.slim || self.open.as_ref().is_none_or(|o| o.in_trash());
        if hidden {
            return;
        }
        self.pins_reload();
        let ids = self.pins.ids.clone();
        for id in &ids {
            let kind = self.pins.kinds.get(id).copied().unwrap_or(PinKind::Text);
            let thumb = if kind == PinKind::Image { self.pin_picture(*id) } else { None };
            vm.pins.push(PinView { kind, color: self.pins.colors.get(id).copied(), thumb });
        }
        let Some((idx, _)) = self.pins.hover.filter(|_| self.pins.tip_on && vm.menu.is_none()) else { return };
        let Some(&id) = ids.get(idx) else { return };
        let at = self.pins.at;
        if vm.pins.get(idx).is_some_and(|p| p.kind == PinKind::Image) {
            vm.pin_tip = Some(PinTip { at, text: String::new(), image: self.pin_picture(id) });
            return;
        }
        if self.pins.tip_text.as_ref().is_none_or(|(k, _)| *k != id) {
            let text = {
                let st = self.store.borrow();
                match st.kind(id) {
                    Ok(Some(Kind::Text)) => {
                        let full = st.text(id).ok().flatten().unwrap_or_default();
                        let mut cut: String = full.chars().take(TIP_TEXT_CHARS).collect();
                        if full.chars().count() > TIP_TEXT_CHARS {
                            cut.push('\u{2026}');
                        }
                        cut
                    }
                    Ok(Some(Kind::Files)) => st.files(id).ok().flatten().map(|f| f.join("\n")).unwrap_or_default(),
                    _ => String::new(),
                }
            };
            self.pins.tip_text = Some((id, text));
        }
        let text = self.pins.tip_text.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
        vm.pin_tip = Some(PinTip { at, text, image: None });
    }

    /// The pointer moved over (or off) the tiles.
    pub(super) fn pin_hover_move(&mut self, tile: Option<usize>, at: (f32, f32), now: u64) {
        let was = self.pins.hover.map(|h| h.0);
        if tile != was {
            self.pins.forget_tip();
            self.pins.hover = tile.map(|i| (i, now));
            self.pins.at = at;
            self.dirty = true;
        } else if tile.is_some() {
            self.pins.at = at;
            // the popup follows the pointer
            if self.pins.tip_on {
                self.dirty = true;
            }
        }
    }

    /// Called from the timer: opens the popup once the pointer has rested long enough.
    pub(super) fn pins_tick(&mut self, now: u64) {
        if let Some((_, since)) = self.pins.hover {
            if !self.pins.tip_on && now.saturating_sub(since) >= TIP_DELAY_MS && self.open.is_some() {
                self.pins.tip_on = true;
                self.dirty = true;
            }
        }
    }

    fn pin_id(&self, tile: usize) -> Option<i64> {
        self.pins.ids.get(tile).copied()
    }

    /// A click on a tile: paste the clip.
    pub(super) fn pin_click(&mut self, p: &mut dyn Platform, tile: usize) {
        let Some(id) = self.pin_id(tile) else { return };
        self.paste_pin(p, id);
    }

    fn paste_pin(&mut self, p: &mut dyn Platform, id: i64) {
        let warned = self.pins.warned == Some(id);
        self.pins.forget_tip();
        if let PasteCheck::Warn(lines) = self.paste_item(p, id, None, warned) {
            self.pins.warned = Some(id);
            self.status = Some(Status {
                text: format!("{lines} line(s) into a terminal may run commands. Click the favorite again to paste, Esc cancels."),
                until_ms: u64::MAX,
            });
            self.dirty = true;
        }
    }

    /// Right click on a tile: the menu.
    pub(super) fn pin_menu_target(&self, tile: usize) -> Option<i64> {
        self.pin_id(tile)
    }

    /// A choice made in a pinned clip's menu.
    pub(super) fn pin_menu_pick(&mut self, p: &mut dyn Platform, id: i64, entry: usize, now: u64) {
        match entry {
            PIN_MENU_PASTE => self.paste_pin(p, id),
            PIN_MENU_UNPIN => {
                if let Some(msg) = self.set_pin(p, id, now) {
                    self.flash(&msg, now);
                }
                self.pins.invalidate();
                self.dirty = true;
            }
            PIN_MENU_PLAIN => self.set_pin_color(p, id, None),
            i => self.set_pin_color(p, id, PIN_COLORS.get(i - PIN_MENU_PLAIN - 1).map(|c| c.1)),
        }
    }

    fn set_pin_color(&mut self, p: &mut dyn Platform, id: i64, color: Option<(u8, u8, u8)>) {
        match color {
            Some(c) => {
                if self.pins.colors.len() < COLORS_MAX || self.pins.colors.contains_key(&id) {
                    self.pins.colors.insert(id, c);
                }
            }
            None => {
                self.pins.colors.remove(&id);
            }
        }
        if let Err(e) = p.save_pin_colors(&self.pins.colors_text()) {
            p.log(&format!("could not save the favorite colours: {e}"));
        }
        self.dirty = true;
    }
}
