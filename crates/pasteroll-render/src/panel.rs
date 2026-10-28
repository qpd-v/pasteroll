//! The multi-paste options card: a small panel over the carousel with a few choices (what joins the
//! clips, the list style, the order), a preview of the result, and Paste, Copy and Cancel. It is
//! modal while it is open: nothing behind it can be hit.

use super::*;

/// One row of choices: a label and options laid out side by side.
#[derive(Debug, Clone)]
pub struct PanelGroup {
    pub label: String,
    pub options: Vec<String>,
    pub selected: usize,
}

/// What a point on the card is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelHit {
    /// Option `.1` of group `.0`.
    Option(usize, usize),
    Paste,
    Copy,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct PanelView {
    pub title: String,
    pub groups: Vec<PanelGroup>,
    /// The first lines of what would be pasted.
    pub preview: String,
    /// A line for the person (a terminal warning, say); empty for none.
    pub note: String,
    /// Where the keyboard is: a group, then Paste, Copy, Cancel.
    pub focus: usize,
    pub hover: Option<PanelHit>,
}

/// Where everything on the card is (DIP).
#[derive(Debug, Clone)]
pub struct PanelGeometry {
    pub card: Rect,
    pub title: Rect,
    pub labels: Vec<Rect>,
    /// The option chips of each group.
    pub chips: Vec<Vec<Rect>>,
    pub preview_label: Rect,
    pub preview: Rect,
    pub note: Rect,
    pub paste: Rect,
    pub copy: Rect,
    pub cancel: Rect,
}

const CARD_MAX_W: f32 = 460.0;
const PAD: f32 = 16.0;
const CHIP_H: f32 = 30.0;
const GROUP_H: f32 = 56.0;
const PREVIEW_H: f32 = 66.0;
const BUTTON_H: f32 = 32.0;

pub fn panel_geometry(vm: &ViewModel, size_dip: (f32, f32)) -> Option<PanelGeometry> {
    let p = vm.panel.as_ref()?;
    let w = (size_dip.0 - 2.0 * OUTER - 24.0).min(CARD_MAX_W);
    let x = (size_dip.0 - w) / 2.0;
    let y = 26.0;
    let inner = w - 2.0 * PAD;
    let mut gy = y + 46.0;
    let (mut labels, mut chips) = (Vec::new(), Vec::new());
    for g in &p.groups {
        labels.push(Rect { x: x + PAD, y: gy, w: inner, h: 16.0 });
        let n = g.options.len().max(1) as f32;
        let gap = 6.0;
        let cw = (inner - gap * (n - 1.0)) / n;
        chips.push(
            (0..g.options.len())
                .map(|i| Rect { x: x + PAD + i as f32 * (cw + gap), y: gy + 18.0, w: cw, h: CHIP_H })
                .collect(),
        );
        gy += GROUP_H;
    }
    let preview_label = Rect { x: x + PAD, y: gy, w: inner, h: 16.0 };
    let preview = Rect { x: x + PAD, y: gy + 18.0, w: inner, h: PREVIEW_H };
    let note = Rect { x: x + PAD, y: preview.y + PREVIEW_H + 6.0, w: inner, h: 16.0 };
    let by = note.y + 22.0;
    let (cw, pw) = (84.0, 104.0);
    let cancel = Rect { x: x + w - PAD - cw, y: by, w: cw, h: BUTTON_H };
    let copy = Rect { x: cancel.x - 8.0 - cw, y: by, w: cw, h: BUTTON_H };
    let paste = Rect { x: copy.x - 8.0 - pw, y: by, w: pw, h: BUTTON_H };
    let card = Rect { x, y, w, h: by + BUTTON_H + PAD - y };
    Some(PanelGeometry { card, title: Rect { x: x + PAD, y: y + 14.0, w: inner, h: 22.0 }, labels, chips, preview_label, preview, note, paste, copy, cancel })
}

/// What is under `(px, py)` (pixels) on the card. Geometry only, so it can run on every pointer move.
pub fn panel_hit_at(vm: &ViewModel, size_dip: (f32, f32), scale: f32, px: f32, py: f32) -> Option<PanelHit> {
    let g = panel_geometry(vm, size_dip)?;
    let (x, y) = (px / scale, py / scale);
    for (gi, row) in g.chips.iter().enumerate() {
        for (oi, r) in row.iter().enumerate() {
            if r.contains(x, y) {
                return Some(PanelHit::Option(gi, oi));
            }
        }
    }
    [(g.paste, PanelHit::Paste), (g.copy, PanelHit::Copy), (g.cancel, PanelHit::Cancel)]
        .into_iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|(_, h)| h)
}

impl Renderer {
    /// The card, over a dimmed carousel.
    pub(crate) fn draw_panel(&mut self, cv: &mut Canvas, vm: &ViewModel, g: &PanelGeometry, size_dip: (f32, f32)) {
        let Some(p) = &vm.panel else { return };
        let s = cv.s;
        // dim what is behind: it cannot be used now
        cv.fill(Rect { x: OUTER, y: OUTER, w: size_dip.0 - 2.0 * OUTER, h: size_dip.1 - 2.0 * OUTER }, RADIUS, (0, 0, 0), 0.58);
        cv.fill(g.card, 10.0, (38, 38, 46), 1.0);
        cv.stroke(g.card, 10.0, ACCENT, 0.7, 1.2);
        self.draw_text(cv, &p.title, (g.title.x * s, g.title.y * s), FONT_PX * s, 20.0 * s, None, Wrap::None, FOREGROUND, scaled(g.title, s));
        for (gi, group) in p.groups.iter().enumerate() {
            let focused = p.focus == gi;
            let l = g.labels[gi];
            self.draw_text(
                cv,
                &group.label,
                (l.x * s, l.y * s),
                SMALL_PX * s,
                16.0 * s,
                None,
                Wrap::None,
                if focused { ACCENT } else { MUTED },
                scaled(l, s),
            );
            for (oi, opt) in group.options.iter().enumerate() {
                let r = g.chips[gi][oi];
                let on = oi == group.selected;
                let hot = p.hover == Some(PanelHit::Option(gi, oi));
                if on {
                    cv.fill(r, 6.0, ACCENT, if hot { 1.0 } else { 0.92 });
                } else {
                    cv.fill(r, 6.0, if hot { (66, 66, 78) } else { FIELD }, 1.0);
                    cv.stroke(r, 6.0, MENU_BORDER, 1.0, 1.0);
                }
                if on && focused {
                    cv.stroke(Rect { x: r.x - 2.0, y: r.y - 2.0, w: r.w + 4.0, h: r.h + 4.0 }, 8.0, FOREGROUND, 0.9, 1.3);
                }
                let tw = self.measure(opt, FONT_PX * s, 18.0 * s) / s;
                self.draw_text(
                    cv,
                    opt,
                    ((r.x + (r.w - tw) / 2.0) * s, (r.y + 6.0) * s),
                    FONT_PX * s,
                    18.0 * s,
                    None,
                    Wrap::None,
                    if on { (16, 28, 25) } else { FOREGROUND },
                    scaled(r, s),
                );
            }
        }
        let pl = g.preview_label;
        self.draw_text(cv, "Preview", (pl.x * s, pl.y * s), SMALL_PX * s, 16.0 * s, None, Wrap::None, MUTED, scaled(pl, s));
        cv.fill(g.preview, 6.0, (28, 28, 34), 1.0);
        cv.stroke(g.preview, 6.0, MENU_BORDER, 1.0, 1.0);
        let inner = Rect { x: g.preview.x + 8.0, y: g.preview.y + 5.0, w: g.preview.w - 16.0, h: g.preview.h - 10.0 };
        self.draw_text(cv, &p.preview, (inner.x * s, inner.y * s), SMALL_PX * s, 16.0 * s, Some(inner.w * s), Wrap::WordOrGlyph, FOREGROUND, scaled(inner, s));
        if !p.note.is_empty() {
            let n = g.note;
            self.draw_text(cv, &p.note, (n.x * s, n.y * s), SMALL_PX * s, 16.0 * s, Some(n.w * s), Wrap::None, DANGER, scaled(n, s));
        }
        let buttons = [(g.paste, "Paste", PanelHit::Paste, 1usize), (g.copy, "Copy", PanelHit::Copy, 2), (g.cancel, "Cancel", PanelHit::Cancel, 3)];
        for (r, text, hit, slot) in buttons {
            let hot = p.hover == Some(hit);
            let primary = hit == PanelHit::Paste;
            if primary {
                cv.fill(r, 6.0, ACCENT, if hot { 1.0 } else { 0.92 });
            } else {
                cv.fill(r, 6.0, if hot { (66, 66, 78) } else { FIELD }, 1.0);
                cv.stroke(r, 6.0, MENU_BORDER, 1.0, 1.0);
            }
            if p.focus == p.groups.len() + slot - 1 {
                cv.stroke(Rect { x: r.x - 2.0, y: r.y - 2.0, w: r.w + 4.0, h: r.h + 4.0 }, 8.0, FOREGROUND, 0.9, 1.3);
            }
            let tw = self.measure(text, FONT_PX * s, 18.0 * s) / s;
            self.draw_text(
                cv,
                text,
                ((r.x + (r.w - tw) / 2.0) * s, (r.y + 7.0) * s),
                FONT_PX * s,
                18.0 * s,
                None,
                Wrap::None,
                if primary { (16, 28, 25) } else { FOREGROUND },
                scaled(r, s),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ViewModel {
        ViewModel {
            panel: Some(PanelView {
                title: "Paste 3 clips together".into(),
                groups: vec![
                    PanelGroup { label: "Join with".into(), options: ["New line", "Space", "Comma", "Tab", "None"].map(String::from).to_vec(), selected: 0 },
                    PanelGroup { label: "List style".into(), options: ["Plain", "Numbered", "Bulleted"].map(String::from).to_vec(), selected: 1 },
                    PanelGroup { label: "Order".into(), options: ["As chosen", "Newest first", "Oldest first"].map(String::from).to_vec(), selected: 0 },
                ],
                preview: "1. one\n2. two".into(),
                note: String::new(),
                focus: 0,
                hover: None,
            }),
            ..ViewModel::default()
        }
    }

    #[test]
    fn the_card_fits_the_window_and_its_parts_do_not_overlap() {
        let vm = view();
        let size = (WIDTH, HEIGHT);
        let g = panel_geometry(&vm, size).unwrap();
        assert!(g.card.x >= OUTER && g.card.x + g.card.w <= size.0 - OUTER && g.card.y + g.card.h <= size.1 - OUTER, "{:?}", g.card);
        let inside = |r: Rect| r.x >= g.card.x && r.y >= g.card.y && r.x + r.w <= g.card.x + g.card.w + 0.01 && r.y + r.h <= g.card.y + g.card.h + 0.01;
        for row in &g.chips {
            for r in row {
                assert!(inside(*r));
            }
        }
        for r in [g.preview, g.paste, g.copy, g.cancel, g.note] {
            assert!(inside(r), "{r:?} inside {:?}", g.card);
        }
        let all: Vec<Rect> = g.chips.iter().flatten().copied().chain([g.paste, g.copy, g.cancel, g.preview]).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn points_hit_options_and_buttons_and_nothing_else() {
        let mut vm = view();
        let size = (WIDTH, HEIGHT);
        let g = panel_geometry(&vm, size).unwrap();
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let at = |vm: &ViewModel, (x, y): (f32, f32)| panel_hit_at(vm, size, 1.0, x, y);
        assert_eq!(at(&vm, mid(g.chips[1][2])), Some(PanelHit::Option(1, 2)));
        assert_eq!(at(&vm, mid(g.paste)), Some(PanelHit::Paste));
        assert_eq!(at(&vm, mid(g.copy)), Some(PanelHit::Copy));
        assert_eq!(at(&vm, mid(g.cancel)), Some(PanelHit::Cancel));
        assert_eq!(at(&vm, (2.0, 2.0)), None);
        // and it is modal: the carousel behind it cannot be hit
        let mut r = Renderer::new();
        assert_eq!(r.hit_test(&vm, size, 1.0, 2.0, 2.0), Hit::Nothing);
        assert_eq!(r.hit_test(&vm, size, 1.0, mid(g.paste).0, mid(g.paste).1), Hit::Panel(PanelHit::Paste));
        vm.panel = None;
        assert_eq!(panel_hit_at(&vm, size, 1.0, 100.0, 100.0), None);
    }

    #[test]
    fn the_card_draws_over_a_dimmed_window() {
        let vm = view();
        let mut r = Renderer::new();
        let plain = r.render(&ViewModel::default(), (WIDTH, HEIGHT), 1.0);
        let with_panel = r.render(&vm, (WIDTH, HEIGHT), 1.0);
        let corner = (30, 14);
        let (a, b) = (plain.pixel(corner.0, corner.1), with_panel.pixel(corner.0, corner.1));
        assert!(b.0 < a.0 || b.1 < a.1 || b.2 < a.2, "dimmed: {a:?} -> {b:?}");
        let g = panel_geometry(&vm, (WIDTH, HEIGHT)).unwrap();
        let chip = g.chips[1][1];
        let px = with_panel.pixel((chip.x + 4.0) as u32, (chip.y + chip.h / 2.0) as u32);
        assert!(px.1 > px.0, "the chosen option is in the accent colour: {px:?}");
    }
}
