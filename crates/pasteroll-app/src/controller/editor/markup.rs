//! Drawing and writing on the picture in the image editor: pen, brush, circle, arrow and text.
//!
//! The marks are plain data in [`EditParams::marks`] (pixels of the original picture), so undo, redo
//! and the saved result all see them. While the Draw and write section is open the editor shows
//! the original with the marks painted on it; a text being typed is a [`TextEdit`] (the caret and
//! the selection, as character positions) and is drawn with them.

use super::*;
use pasteroll_core::imgedit::{Mark, MARKS_MAX, MARK_POINTS_MAX, MARK_TEXT_MAX};
use pasteroll_render::MarkEdit;

pub(super) const MARK_TOOLS: [MarkKind; 5] = [MarkKind::Pen, MarkKind::Brush, MarkKind::Ellipse, MarkKind::Arrow, MarkKind::Text];

/// The fonts of the picker, when installed.
pub(super) const FONT_CANDIDATES: [&str; 17] = [
    "Segoe UI",
    "Arial",
    "Calibri",
    "Cambria",
    "Comic Sans MS",
    "Consolas",
    "Courier New",
    "Georgia",
    "Impact",
    "Lucida Console",
    "Palatino Linotype",
    "Segoe Print",
    "Segoe Script",
    "Tahoma",
    "Times New Roman",
    "Trebuchet MS",
    "Verdana",
];

pub(super) const DEFAULT_MARK_COLOR: Col = [230, 57, 70, 255];

/// A shape smaller than this (pixels of the picture) was a click, not a drawing.
const MIN_SHAPE: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TextEdit {
    pub index: usize,
    pub caret: usize,
    pub anchor: usize,
}

impl TextEdit {
    /// The selected stretch (empty when nothing is selected).
    pub(super) fn range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
}

impl Editor {
    pub(super) fn markup_mode(&self) -> bool {
        self.open_sec == Some(Sec::Markup)
    }

    fn longer_side(&self) -> f32 {
        self.src_dims.0.max(self.src_dims.1).max(1) as f32
    }

    /// The line width for the size setting, in pixels of the picture.
    fn stroke_px(&self) -> f32 {
        (self.mark_size * self.longer_side() / 800.0).max(1.0)
    }

    /// The text height for the size setting, in pixels of the picture.
    pub(super) fn text_px(&self) -> f32 {
        (self.mark_size * 5.0 * self.longer_side() / 800.0).max(8.0)
    }
}

/// Replaces the characters `a..b` of the mark's text with `ins`, each in `color`.
fn replace_range(m: &mut Mark, a: usize, b: usize, ins: &[char], color: [u8; 4]) {
    let mut chars: Vec<char> = m.text.chars().collect();
    m.colors.resize(chars.len(), m.color);
    let b = b.min(chars.len());
    let a = a.min(b);
    chars.splice(a..b, ins.iter().copied());
    m.colors.splice(a..b, std::iter::repeat_n(color, ins.len()));
    m.text = chars.into_iter().collect();
}

impl Controller {
    /// Repaints the original with the marks on it, for the preview of the Draw and write section.
    pub(super) fn refresh_markup(&mut self) {
        let Some(e) = self.editor.as_ref().filter(|e| e.markup_mode()) else { return };
        let Some((proxy, k)) = e.proxy.clone() else { return };
        let marks = e.params.scaled(k).marks;
        let edit = e.text_edit.map(|t| MarkEdit { index: t.index, caret: t.caret, anchor: t.anchor });
        let mut img = (*proxy).clone();
        self.renderer.burn_marks(&mut img, &marks, edit);
        if let Some(e) = self.editor.as_mut() {
            e.markup_img = Some(Arc::new(img));
        }
    }

    /// A window position (px) as a point of the original picture, kept inside it.
    fn mark_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (px, py) = self.crop_point(x, y)?;
        let e = self.editor.as_ref()?;
        Some((px.clamp(0.0, e.src_dims.0 as f32), py.clamp(0.0, e.src_dims.1 as f32)))
    }

    /// The pointer went down on the picture while the Draw and write section is open.
    pub(super) fn markup_press(&mut self, m: MouseEvent) {
        let Some(pt) = self.mark_point(m.x, m.y) else { return };
        let Some(e) = self.editor.as_ref() else { return };
        if e.mark_tool == MarkKind::Text {
            self.text_press(pt, m.kind == MouseKind::DoubleClick);
            return;
        }
        self.finish_text_edit();
        let Some(e) = self.editor.as_mut() else { return };
        if e.params.marks.len() >= MARKS_MAX {
            e.toast("That is the most marks one picture takes. Remove some first.", 0);
            self.dirty = true;
            return;
        }
        let mut mark = Mark::new(e.mark_tool, e.mark_color, e.stroke_px());
        mark.pts.push(pt);
        if matches!(mark.kind, MarkKind::Ellipse | MarkKind::Arrow) {
            mark.pts.push(pt);
        }
        e.params.marks.push(mark);
        e.drag = Some(Drag::Mark { idx: e.params.marks.len() - 1 });
        self.refresh_markup();
        self.dirty = true;
    }

    pub(super) fn markup_drag(&mut self, idx: usize, x: f32, y: f32) {
        let Some(pt) = self.mark_point(x, y) else { return };
        let Some(e) = self.editor.as_mut() else { return };
        let Some(m) = e.params.marks.get_mut(idx) else { return };
        match m.kind {
            MarkKind::Pen | MarkKind::Brush => {
                let far = m.pts.last().is_none_or(|l| (l.0 - pt.0).hypot(l.1 - pt.1) >= 1.0);
                if far && m.pts.len() < MARK_POINTS_MAX {
                    m.pts.push(pt);
                }
            }
            MarkKind::Ellipse | MarkKind::Arrow => {
                if let Some(last) = m.pts.get_mut(1) {
                    *last = pt;
                }
            }
            MarkKind::Text => {}
        }
        self.refresh_markup();
        self.dirty = true;
    }

    /// The pointer came up after drawing mark `idx`: a shape that is only a click is dropped.
    pub(super) fn markup_release(&mut self, idx: usize) {
        let Some(e) = self.editor.as_mut() else { return };
        let tiny = e.params.marks.get(idx).is_some_and(|m| {
            matches!(m.kind, MarkKind::Ellipse | MarkKind::Arrow)
                && m.pts.len() == 2
                && (m.pts[0].0 - m.pts[1].0).abs() < MIN_SHAPE
                && (m.pts[0].1 - m.pts[1].1).abs() < MIN_SHAPE
        });
        if tiny {
            e.params.marks.remove(idx);
        }
        self.refresh_markup();
    }

    /// The text mark under a point of the picture (the topmost one).
    fn text_mark_at(&mut self, pt: (f32, f32)) -> Option<usize> {
        let marks: Vec<(usize, Mark)> = self
            .editor
            .as_ref()?
            .params
            .marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.kind == MarkKind::Text && !m.pts.is_empty())
            .map(|(i, m)| (i, m.clone()))
            .collect();
        for (i, m) in marks.into_iter().rev() {
            let (w, h) = self.renderer.mark_text_size(&m);
            let (ox, oy) = m.pts[0];
            let pad = (m.size * 0.2).max(3.0);
            if pt.0 >= ox - pad && pt.0 <= ox + w.max(m.size * 0.6) + pad && pt.1 >= oy - pad && pt.1 <= oy + h + pad {
                return Some(i);
            }
        }
        None
    }

    /// The character of mark `idx` nearest to a point of the picture.
    fn text_char_at(&mut self, idx: usize, pt: (f32, f32)) -> usize {
        let Some(m) = self.editor.as_ref().and_then(|e| e.params.marks.get(idx)).cloned() else { return 0 };
        let Some(&(ox, oy)) = m.pts.first() else { return 0 };
        self.renderer.mark_text_hit(&m, pt.0 - ox, pt.1 - oy)
    }

    fn text_press(&mut self, pt: (f32, f32), double: bool) {
        let current = self.editor.as_ref().and_then(|e| e.text_edit).map(|t| t.index);
        let mut hit = self.text_mark_at(pt);
        if hit.is_none() || hit != current {
            // another text, or empty ground: the one being written is done first (an empty one goes)
            self.finish_text_edit();
            hit = self.text_mark_at(pt);
        }
        match hit {
            Some(idx) => {
                let at = self.text_char_at(idx, pt);
                let Some(e) = self.editor.as_mut() else { return };
                let (mut caret, mut anchor) = (at, at);
                if double {
                    if let Some(m) = e.params.marks.get(idx) {
                        let chars: Vec<char> = m.text.chars().collect();
                        let word = |c: char| c.is_alphanumeric() || c == '_';
                        let mut a = at.min(chars.len());
                        let mut b = a;
                        while a > 0 && word(chars[a - 1]) {
                            a -= 1;
                        }
                        while b < chars.len() && word(chars[b]) {
                            b += 1;
                        }
                        (anchor, caret) = (a, b);
                    }
                }
                e.text_edit = Some(TextEdit { index: idx, caret, anchor });
                e.drag = Some(Drag::TextSel { idx });
            }
            None => {
                let Some(e) = self.editor.as_mut() else { return };
                if e.params.marks.len() >= MARKS_MAX {
                    e.toast("That is the most marks one picture takes. Remove some first.", 0);
                    self.dirty = true;
                    return;
                }
                let size = e.text_px();
                let mut mark = Mark::new(MarkKind::Text, e.mark_color, size);
                mark.font = e.font.clone();
                // the click is where the first line is centred
                mark.pts.push((pt.0, pt.1 - size * 0.625));
                e.params.marks.push(mark);
                e.text_edit = Some(TextEdit { index: e.params.marks.len() - 1, caret: 0, anchor: 0 });
            }
        }
        self.refresh_markup();
        self.dirty = true;
    }

    /// Dragging over the text being written selects part of it.
    pub(super) fn markup_select_drag(&mut self, idx: usize, x: f32, y: f32) {
        let Some(pt) = self.mark_point(x, y) else { return };
        let at = self.text_char_at(idx, pt);
        let Some(e) = self.editor.as_mut() else { return };
        if let Some(te) = e.text_edit.as_mut().filter(|t| t.index == idx) {
            te.caret = at;
        }
        self.refresh_markup();
        self.dirty = true;
    }

    /// Ends the writing of a text: an empty one is dropped, a written one becomes a step to undo.
    pub(super) fn finish_text_edit(&mut self) {
        let Some(e) = self.editor.as_mut() else { return };
        let Some(te) = e.text_edit.take() else { return };
        if e.params.marks.get(te.index).is_some_and(|m| m.text.trim().is_empty()) {
            e.params.marks.remove(te.index);
        }
        e.checkpoint();
        self.edited();
    }

    /// A key while a text is being written. `true` = the key was used.
    pub(super) fn text_key(&mut self, key: Key) -> bool {
        let Some(e) = self.editor.as_mut() else { return false };
        let Some(mut te) = e.text_edit else { return false };
        let color = e.mark_color;
        let Some(m) = e.params.marks.get_mut(te.index) else {
            e.text_edit = None;
            return false;
        };
        let chars: Vec<char> = m.text.chars().collect();
        let n = chars.len();
        let (a, b) = te.range();
        let line_start = |pos: usize| chars[..pos.min(n)].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
        let line_end = |pos: usize| chars[pos.min(n)..].iter().position(|c| *c == '\n').map_or(n, |i| pos.min(n) + i);
        match key {
            Key::Escape => {
                self.finish_text_edit();
                return true;
            }
            Key::Char(c) if !c.is_control() => {
                if n - (b - a) < MARK_TEXT_MAX {
                    replace_range(m, a, b, &[c], color);
                    te.caret = a + 1;
                    te.anchor = te.caret;
                }
            }
            Key::Enter => {
                if n - (b - a) < MARK_TEXT_MAX {
                    replace_range(m, a, b, &['\n'], color);
                    te.caret = a + 1;
                    te.anchor = te.caret;
                }
            }
            Key::Backspace | Key::CtrlBackspace => {
                if a != b {
                    replace_range(m, a, b, &[], color);
                    te.caret = a;
                } else if a > 0 {
                    replace_range(m, a - 1, a, &[], color);
                    te.caret = a - 1;
                }
                te.anchor = te.caret;
            }
            Key::Delete | Key::CtrlDelete => {
                if a != b {
                    replace_range(m, a, b, &[], color);
                    te.caret = a;
                } else if a < n {
                    replace_range(m, a, a + 1, &[], color);
                }
                te.anchor = te.caret;
            }
            Key::Left => {
                te.caret = if a != b { a } else { te.caret.saturating_sub(1) };
                te.anchor = te.caret;
            }
            Key::Right => {
                te.caret = if a != b { b } else { (te.caret + 1).min(n) };
                te.anchor = te.caret;
            }
            Key::Home => {
                te.caret = line_start(te.caret);
                te.anchor = te.caret;
            }
            Key::End => {
                te.caret = line_end(te.caret);
                te.anchor = te.caret;
            }
            Key::CtrlA => {
                te.anchor = 0;
                te.caret = n;
            }
            _ => return false,
        }
        if let Some(e) = self.editor.as_mut() {
            e.text_edit = Some(te);
        }
        self.refresh_markup();
        self.dirty = true;
        true
    }

    /// A click on one of the Draw and write section's own items (the tool, colour, font, buttons).
    pub(super) fn markup_item_press(&mut self, id: u32, n: u32, part: u16) {
        let part = usize::from(part);
        match n {
            1 => {
                let Some(tool) = markup::MARK_TOOLS.get(part).copied() else { return };
                self.finish_text_edit();
                if let Some(e) = self.editor.as_mut() {
                    e.mark_tool = tool;
                }
                self.dirty = true;
            }
            2 => self.open_color(ColorTarget::Mark, id),
            4 => {
                let Some(e) = self.editor.as_mut() else { return };
                let font = if part == 0 { String::new() } else { e.fonts.get(part - 1).cloned().unwrap_or_default() };
                e.font = font.clone();
                // the text being written changes font too
                if let Some(te) = e.text_edit {
                    if let Some(m) = e.params.marks.get_mut(te.index) {
                        m.font = font;
                    }
                    e.checkpoint();
                }
                self.edited();
            }
            5 => {
                self.finish_text_edit();
                let Some(e) = self.editor.as_mut() else { return };
                if part == 0 {
                    e.params.marks.pop();
                } else {
                    e.params.marks.clear();
                }
                e.checkpoint();
                self.edited();
            }
            _ => {}
        }
    }
}
