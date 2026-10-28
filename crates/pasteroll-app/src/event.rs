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
    // The keys below matter mostly in the expanded viewer. Left, Right, Home, End, F3 and F11 are
    // only taken from the application underneath while the viewer is open.
    Left,
    Right,
    Home,
    End,
    F3,
    ShiftF3,
    F11,
    /// Delete for good (after a confirmation), not to the recycle bin.
    ShiftDelete,
    CtrlC,
    CtrlE,
    CtrlF,
    CtrlI,
    CtrlS,
    CtrlZ,
    /// Redo (Ctrl+Y, Ctrl+Shift+Z) in the image editor.
    CtrlY,
    /// Multi-paste on or off.
    CtrlM,
    /// With Multi-paste on: move to the next clip up and add it to the choice (Ctrl), or grow or
    /// shrink the chosen range (Shift).
    CtrlUp,
    CtrlDown,
    ShiftUp,
    ShiftDown,
}

impl Key {
    /// Shift or Ctrl with Up or Down as the plain arrow (where choosing several clips has no meaning).
    pub fn plain_arrow(self) -> Key {
        match self {
            Key::CtrlUp | Key::ShiftUp => Key::Up,
            Key::CtrlDown | Key::ShiftDown => Key::Down,
            k => k,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Up,
    Move,
    DoubleClick,
    /// The right button went down over the carousel (it opens the Copy menu).
    RightDown,
    /// The pointer left the window.
    Leave,
}

/// A wheel turn while the expanded viewer is open (the carousel turns the wheel into steps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wheel {
    /// In 120ths of a notch; positive is away from the user (up), or right when `horizontal`.
    pub delta: i32,
    pub horizontal: bool,
    pub ctrl: bool,
    pub shift: bool,
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
    /// Open history at a UI anchor without assuming the shell is a paste target.
    BrowseAt(i32, i32),
    Step(Step, u64),
    Key(Key, u64),
    /// A left click completed outside the overlay: paste there.
    ClickPaste(u64),
    /// Some other click outside the overlay: close without pasting.
    ClickCancel(u64),
    Mouse(MouseEvent),
    /// Mouse input over the expanded viewer's window, in its device pixels.
    ViewerMouse(MouseEvent),
    Wheel(Wheel, u64),
    /// A click outside Pasteroll's windows while the viewer is open (it closes the viewer).
    ClickOutside(u64),
    /// ~25-40 ms timer (faster while something is animating).
    Tick,
    /// A worker finished something; check for results.
    Wake,
}
