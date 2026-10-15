//! A tiny retained widget layer for the platform windows that are not the carousel itself (the
//! Linux settings window and tray menu). The platform lays widgets out in DIP, draws them with
//! [`Renderer::render_ui`], and maps pointer positions back with [`hit`] and the slider/list helpers.

use super::{scaled, Canvas, Frame, Rect, Renderer, ACCENT, BACKGROUND, FIELD, FOREGROUND, MUTED, SURFACE, SURFACE_BORDER};
use cosmic_text::Wrap;
use tiny_skia::Pixmap;

pub const LIST_ROW: f32 = 24.0;
const SLIDER_LABEL_W: f32 = 150.0;
const SLIDER_VALUE_W: f32 = 96.0;
const TEXT_PX: f32 = 14.0;
const NOTE_PX: f32 = 12.0;

#[derive(Debug, Clone, PartialEq)]
pub enum WKind {
    Heading,
    Label,
    /// Muted, wrapped text.
    Note,
    /// `index` of `count` steps; `value` is the text shown at the right.
    Slider { value: String, index: usize, count: usize },
    Check { checked: bool },
    /// `top` is the first visible row.
    List { items: Vec<String>, selected: Option<usize>, top: usize },
    Entry { placeholder: String, focused: bool },
    Button { primary: bool },
    MenuItem { checked: bool, enabled: bool },
    Separator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    pub id: u32,
    pub rect: Rect,
    pub text: String,
    pub kind: WKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chrome {
    /// A normal window: flat background.
    Window,
    /// A popup menu: bordered.
    Menu,
}

#[derive(Debug, Clone)]
pub struct UiView {
    pub size: (f32, f32),
    pub chrome: Chrome,
    pub widgets: Vec<Widget>,
    pub hover: Option<u32>,
}

impl Widget {
    pub fn new(id: u32, rect: Rect, text: impl Into<String>, kind: WKind) -> Widget {
        Widget { id, rect, text: text.into(), kind }
    }

    fn interactive(&self) -> bool {
        match &self.kind {
            WKind::Slider { .. } | WKind::Check { .. } | WKind::List { .. } | WKind::Entry { .. } | WKind::Button { .. } => {
                true
            }
            WKind::MenuItem { enabled, .. } => *enabled,
            _ => false,
        }
    }
}

/// The widget under a point, if it is one that reacts.
pub fn hit(view: &UiView, x: f32, y: f32) -> Option<&Widget> {
    view.widgets.iter().rev().find(|w| w.interactive() && w.rect.contains(x, y))
}

/// Horizontal extent of a slider's track.
pub fn slider_track(w: &Widget) -> (f32, f32) {
    (w.rect.x + SLIDER_LABEL_W, w.rect.x + w.rect.w - SLIDER_VALUE_W)
}

/// Which step a pointer at `x` means.
pub fn slider_index_at(w: &Widget, x: f32) -> usize {
    let WKind::Slider { count, .. } = &w.kind else { return 0 };
    if *count <= 1 {
        return 0;
    }
    let (x0, x1) = slider_track(w);
    let t = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
    (t * (*count as f32 - 1.0)).round() as usize
}

/// Which list row (index into `items`) a pointer at `y` means.
pub fn list_row_at(w: &Widget, y: f32) -> Option<usize> {
    let WKind::List { items, top, .. } = &w.kind else { return None };
    let row = ((y - (w.rect.y + 2.0)) / LIST_ROW).floor();
    if row < 0.0 {
        return None;
    }
    let i = top + row as usize;
    (i < items.len() && (row as f32 + 1.0) * LIST_ROW <= w.rect.h).then_some(i)
}

/// How many rows a list widget shows at once.
pub fn list_visible_rows(w: &Widget) -> usize {
    ((w.rect.h - 4.0) / LIST_ROW).floor().max(1.0) as usize
}

impl Renderer {
    pub fn render_ui(&mut self, view: &UiView, scale: f32) -> Frame {
        let s = scale;
        let (wpx, hpx) = ((view.size.0 * s).round().max(1.0) as u32, (view.size.1 * s).round().max(1.0) as u32);
        let mut cv = Canvas { pm: Pixmap::new(wpx, hpx).expect("pixmap"), s };
        cv.pm.fill(super::color(if view.chrome == Chrome::Menu { SURFACE } else { BACKGROUND }, 1.0));
        let full = Rect { x: 0.0, y: 0.0, w: view.size.0, h: view.size.1 };
        match view.chrome {
            Chrome::Window => cv.fill(full, 0.0, SURFACE, 1.0),
            Chrome::Menu => cv.stroke(
                Rect { x: 0.5, y: 0.5, w: view.size.0 - 1.0, h: view.size.1 - 1.0 },
                4.0,
                (74, 74, 88),
                1.0,
                1.0,
            ),
        }
        for w in &view.widgets {
            let hovered = view.hover == Some(w.id);
            self.draw_widget(&mut cv, w, hovered);
        }
        let mut bgra = cv.pm.take();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Frame { width: wpx, height: hpx, bgra }
    }

    fn text_at(&mut self, cv: &mut Canvas, text: &str, x: f32, y: f32, px: f32, rgb: super::Rgb, clip: Rect) -> f32 {
        let s = cv.s;
        self.draw_text(cv, text, (x * s, y * s), px * s, (px + 5.0) * s, None, Wrap::None, rgb, scaled(clip, s))
            / s
    }

    fn draw_widget(&mut self, cv: &mut Canvas, w: &Widget, hovered: bool) {
        let r = w.rect;
        let ty = r.y + (r.h - TEXT_PX - 5.0).max(0.0) / 2.0;
        match &w.kind {
            WKind::Heading => {
                self.text_at(cv, &w.text, r.x, ty, 15.0, FOREGROUND, r);
            }
            WKind::Label => {
                self.text_at(cv, &w.text, r.x, ty, TEXT_PX, FOREGROUND, r);
            }
            WKind::Note => {
                let s = cv.s;
                self.draw_text(
                    cv,
                    &w.text,
                    (r.x * s, r.y * s),
                    NOTE_PX * s,
                    (NOTE_PX + 4.0) * s,
                    Some(r.w * s),
                    Wrap::WordOrGlyph,
                    MUTED,
                    scaled(r, s),
                );
            }
            WKind::Slider { value, index, count } => {
                self.text_at(cv, &w.text, r.x, ty, TEXT_PX, FOREGROUND, r);
                let (x0, x1) = slider_track(w);
                let cy = r.y + r.h / 2.0;
                cv.fill(Rect { x: x0, y: cy - 2.0, w: x1 - x0, h: 4.0 }, 2.0, FIELD, 1.0);
                let t = if *count > 1 { *index as f32 / (*count as f32 - 1.0) } else { 0.0 };
                let kx = x0 + t * (x1 - x0);
                cv.fill(Rect { x: x0, y: cy - 2.0, w: kx - x0, h: 4.0 }, 2.0, ACCENT, 1.0);
                cv.fill(Rect { x: kx - 7.0, y: cy - 7.0, w: 14.0, h: 14.0 }, 7.0, if hovered { (255, 255, 255) } else { FOREGROUND }, 1.0);
                let vw = self.measure(value, TEXT_PX * cv.s, (TEXT_PX + 5.0) * cv.s) / cv.s;
                self.text_at(cv, value, r.x + r.w - vw, ty, TEXT_PX, MUTED, r);
            }
            WKind::Check { checked } => {
                let b = Rect { x: r.x, y: r.y + (r.h - 16.0) / 2.0, w: 16.0, h: 16.0 };
                if *checked {
                    cv.fill(b, 3.0, ACCENT, 1.0);
                    cv.line(&[(b.x + 3.5, b.y + 8.5), (b.x + 6.5, b.y + 11.5), (b.x + 12.5, b.y + 4.5)], BACKGROUND, 2.0);
                } else {
                    cv.fill(b, 3.0, FIELD, 1.0);
                    cv.stroke(b, 3.0, if hovered { ACCENT } else { (110, 110, 124) }, 1.0, 1.2);
                }
                self.text_at(cv, &w.text, r.x + 26.0, ty, TEXT_PX, FOREGROUND, r);
            }
            WKind::List { items, selected, top } => {
                cv.fill(r, 4.0, FIELD, 1.0);
                cv.stroke(r, 4.0, SURFACE_BORDER, 1.0, 1.0);
                let rows = list_visible_rows(w);
                for (k, item) in items.iter().enumerate().skip(*top).take(rows) {
                    let row = Rect { x: r.x + 2.0, y: r.y + 2.0 + (k - top) as f32 * LIST_ROW, w: r.w - 4.0, h: LIST_ROW };
                    if *selected == Some(k) {
                        cv.fill(row, 3.0, ACCENT, 0.28);
                    }
                    self.text_at(cv, item, row.x + 8.0, row.y + 3.0, TEXT_PX, FOREGROUND, row);
                }
                if items.is_empty() {
                    self.text_at(cv, &w.text, r.x + 8.0, r.y + 5.0, NOTE_PX, MUTED, r);
                }
            }
            WKind::Entry { placeholder, focused } => {
                cv.fill(r, 4.0, FIELD, 1.0);
                cv.stroke(r, 4.0, if *focused { ACCENT } else { SURFACE_BORDER }, 1.0, 1.0);
                if w.text.is_empty() {
                    self.text_at(cv, placeholder, r.x + 8.0, ty, TEXT_PX, MUTED, r);
                } else {
                    self.text_at(cv, &w.text, r.x + 8.0, ty, TEXT_PX, FOREGROUND, r);
                }
                if *focused {
                    let tw = if w.text.is_empty() { 0.0 } else { self.measure(&w.text, TEXT_PX * cv.s, (TEXT_PX + 5.0) * cv.s) / cv.s };
                    let cx = (r.x + 9.0 + tw).min(r.x + r.w - 4.0);
                    cv.line(&[(cx, r.y + 6.0), (cx, r.y + r.h - 6.0)], ACCENT, 1.4);
                }
            }
            WKind::Button { primary } => {
                let (fill, text) = if *primary { (ACCENT, BACKGROUND) } else { (FIELD, FOREGROUND) };
                cv.fill(r, 5.0, fill, if hovered { 0.85 } else { 1.0 });
                if !primary {
                    cv.stroke(r, 5.0, if hovered { ACCENT } else { SURFACE_BORDER }, 1.0, 1.0);
                }
                let tw = self.measure(&w.text, TEXT_PX * cv.s, (TEXT_PX + 5.0) * cv.s) / cv.s;
                self.text_at(cv, &w.text, r.x + (r.w - tw) / 2.0, ty, TEXT_PX, text, r);
            }
            WKind::MenuItem { checked, enabled } => {
                if hovered && *enabled {
                    cv.fill(Rect { x: r.x + 3.0, y: r.y + 1.0, w: r.w - 6.0, h: r.h - 2.0 }, 4.0, ACCENT, 0.28);
                }
                if *checked {
                    cv.line(&[(r.x + 10.0, r.y + r.h / 2.0), (r.x + 13.0, r.y + r.h / 2.0 + 3.0), (r.x + 19.0, r.y + r.h / 2.0 - 4.0)], ACCENT, 1.8);
                }
                let col = if *enabled { FOREGROUND } else { MUTED };
                self.text_at(cv, &w.text, r.x + 28.0, ty, TEXT_PX, col, r);
            }
            WKind::Separator => {
                cv.line(&[(r.x + 6.0, r.y + r.h / 2.0), (r.x + r.w - 6.0, r.y + r.h / 2.0)], SURFACE_BORDER, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slider() -> Widget {
        Widget::new(
            1,
            Rect { x: 10.0, y: 10.0, w: 500.0, h: 30.0 },
            "Clips kept",
            WKind::Slider { value: "1,000 clips".into(), index: 3, count: 5 },
        )
    }

    #[test]
    fn slider_positions_map_to_steps_and_back() {
        let w = slider();
        let (x0, x1) = slider_track(&w);
        assert_eq!(slider_index_at(&w, x0 - 50.0), 0);
        assert_eq!(slider_index_at(&w, x1 + 50.0), 4);
        assert_eq!(slider_index_at(&w, (x0 + x1) / 2.0), 2);
        for i in 0..5 {
            let x = x0 + (x1 - x0) * i as f32 / 4.0;
            assert_eq!(slider_index_at(&w, x), i);
        }
    }

    #[test]
    fn hit_finds_the_topmost_interactive_widget() {
        let view = UiView {
            size: (300.0, 100.0),
            chrome: Chrome::Window,
            widgets: vec![
                Widget::new(1, Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 }, "note", WKind::Note),
                Widget::new(2, Rect { x: 10.0, y: 10.0, w: 80.0, h: 30.0 }, "Save", WKind::Button { primary: true }),
                Widget::new(3, Rect { x: 10.0, y: 60.0, w: 80.0, h: 20.0 }, "x", WKind::Check { checked: false }),
            ],
            hover: None,
        };
        assert_eq!(hit(&view, 20.0, 20.0).map(|w| w.id), Some(2));
        assert_eq!(hit(&view, 20.0, 70.0).map(|w| w.id), Some(3));
        assert_eq!(hit(&view, 200.0, 20.0).map(|w| w.id), None, "a note does not react");
    }

    #[test]
    fn list_rows_follow_the_scroll_offset() {
        let w = Widget::new(
            1,
            Rect { x: 0.0, y: 0.0, w: 200.0, h: 4.0 + 3.0 * LIST_ROW },
            "",
            WKind::List { items: (0..10).map(|i| i.to_string()).collect(), selected: None, top: 4 },
        );
        assert_eq!(list_visible_rows(&w), 3);
        assert_eq!(list_row_at(&w, 3.0), Some(4));
        assert_eq!(list_row_at(&w, 2.0 + LIST_ROW + 1.0), Some(5));
        assert_eq!(list_row_at(&w, 2.0 + 3.0 * LIST_ROW + 5.0), None, "below the last visible row");
    }

    #[test]
    fn a_view_renders_to_a_frame_of_the_right_size() {
        let view = UiView {
            size: (240.0, 120.0),
            chrome: Chrome::Window,
            widgets: vec![
                Widget::new(1, Rect { x: 10.0, y: 10.0, w: 220.0, h: 24.0 }, "Pause recording", WKind::Check { checked: true }),
                slider(),
                Widget::new(3, Rect { x: 10.0, y: 80.0, w: 90.0, h: 28.0 }, "Save", WKind::Button { primary: true }),
            ],
            hover: Some(3),
        };
        let mut r = Renderer::new();
        let f = r.render_ui(&view, 1.5);
        assert_eq!((f.width, f.height), (360, 180));
        assert_eq!(f.bgra.len(), 360 * 180 * 4);
    }
}
