//! What a platform backend feeds the controller.

use pasteroll_core::Step;

/// Keys the controller understands while the overlay is open. The platform decides which physical
/// keys to swallow and translates them to these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
    Up,
    Down,
    PageUp,
    PageDown,
    Backspace,
    CtrlBackspace,
    Delete,
    /// Delete the clip even while the search box has text.
    CtrlDelete,
    Tab,
    CtrlP,
    CtrlA,
    /// A typed character (layout- and shift-aware), for the search box.
    Char(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Up,
    Move,
    DoubleClick,
}

/// A mouse event over the overlay, in device pixels relative to its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub x: f32,
    pub y: f32,
}

/// Events carry the input generation they were produced under, so commands that were queued by a
/// session that has already ended are dropped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// The activation gesture was made.
    Activate,
    Step(Step, u64),
    Key(Key, u64),
    /// A left click completed outside the overlay: paste there.
    ClickPaste(u64),
    /// Some other click outside the overlay: close without pasting.
    ClickCancel(u64),
    Mouse(MouseEvent),
    /// ~25-40 ms timer.
    Tick,
    /// A worker finished something; check for results.
    Wake,
}
