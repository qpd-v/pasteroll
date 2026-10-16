//! The X11 implementation of [`pasteroll_app::Platform`] and the translation of X events into
//! controller events.
//!
//! Input model: the activation gesture (Ctrl+Shift+middle-click) is a passive grab on the root
//! window. While a session is open there is an active keyboard grab (typing goes to the search box)
//! and passive grabs on every pointer button in *sync* mode: each press is held until we decide to
//! consume it (wheel, clicks on the overlay) or replay it to the program underneath (a click that
//! should place the caret before the paste). A watchdog in `run.rs` exits the process if the loop
//! hangs while input is grabbed, because a stuck grab would freeze the desktop.
//!
//! The expanded viewer is a second override-redirect window. While it is open the wheel is passed
//! on raw (with Ctrl and Shift), clicks inside it go to it, and a click elsewhere closes it
//! instead of pasting there.

use crate::clipboard::Server;
use crate::keys;
use crate::sys;
use crate::x::{self, Atoms, Keymap, XResult};
use pasteroll_app::{
    ClickPoll, Event, MouseEvent, MouseKind, OverlayInfo, PasteKey, Payload, Platform, SaveChoice, Target, Wheel,
};
use pasteroll_render::{Frame, HEIGHT, WIDTH};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape;
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::Event as XEvent;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;
use zeroize::Zeroize;

const DOUBLE_CLICK_MS: u128 = 400;
const DOUBLE_CLICK_PX: i32 = 5;
/// Corner radius of the overlay, in device-independent pixels (matches the Windows rounding).
const CORNER_DIP: f32 = 8.0;

/// The monitor containing the point; the nearest one when it is in a gap.
fn pick_monitor(rects: &[(i32, i32, i32, i32)], at: (i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let dist = |r: &(i32, i32, i32, i32)| {
        let dx = (r.0 - at.0).max(at.0 - (r.0 + r.2 - 1)).max(0);
        let dy = (r.1 - at.1).max(at.1 - (r.1 + r.3 - 1)).max(0);
        i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy)
    };
    rects.iter().copied().min_by_key(dist)
}

/// A w*h rectangle with rounded corners, as a list of rectangles (one band per corner row).
fn rounded_rects(w: u32, h: u32, r: u32) -> Vec<Rectangle> {
    let r = r.min(w / 2).min(h / 2);
    let mut out = Vec::new();
    for i in 0..r {
        // distance of the row centre from the arc's centre line
        let dy = f64::from(r) - f64::from(i) - 0.5;
        let inset = (f64::from(r) - (f64::from(r * r) - dy * dy).max(0.0).sqrt()).round() as u32;
        let width = (w - 2 * inset) as u16;
        out.push(Rectangle { x: inset as i16, y: i as i16, width, height: 1 });
        out.push(Rectangle { x: inset as i16, y: (h - 1 - i) as i16, width, height: 1 });
    }
    out.push(Rectangle { x: 0, y: r as i16, width: w as u16, height: (h - 2 * r) as u16 });
    out
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn pointer_picks_its_monitor_or_the_nearest() {
        let m = [(0, 0, 1920, 1080), (1920, 0, 1280, 1024)];
        assert_eq!(pick_monitor(&m, (100, 100)), Some(m[0]));
        assert_eq!(pick_monitor(&m, (2000, 500)), Some(m[1]));
        assert_eq!(pick_monitor(&m, (1919, 1079)), Some(m[0]));
        assert_eq!(pick_monitor(&m, (3500, 2000)), Some(m[1]));
        assert_eq!(pick_monitor(&[], (1, 1)), None);
    }

    #[test]
    fn rounded_rects_cut_only_the_corners() {
        let rs = rounded_rects(100, 60, 8);
        // the outermost row is inset, the middle band is full width
        assert!(rs.iter().any(|r| r.y == 0 && r.x > 0));
        assert!(rs.iter().any(|r| r.x == 0 && r.width == 100 && r.y == 8 && r.height == 44));
        // symmetric top/bottom, nothing leaves the box
        for r in &rs {
            assert!(i32::from(r.x) + i32::from(r.width) <= 100);
            assert!(i32::from(r.y) + i32::from(r.height) <= 60);
        }
        assert_eq!(rounded_rects(100, 60, 0).len(), 1);
    }
}

/// What a translated X event asks the loop to do.
pub enum Action {
    Event(Event),
    ClipboardChanged,
}

struct ClickState {
    window: Window,
    since: Instant,
}

pub struct LinuxPlatform {
    pub conn: RustConnection,
    pub root: Window,
    pub atoms: Atoms,
    keymap: Keymap,
    scale: f32,
    /// The carousel size setting (percent of normal).
    size_percent: u32,
    screen_size: (u16, u16),
    depth: u8,
    overlay: Window,
    gc: Gcontext,
    pub helper: Window,
    overlay_pos: (i32, i32),
    overlay_size: (u32, u32),
    mapped: bool,
    last_frame: Option<(Vec<u8>, u32, u32)>,
    generation: u64,
    active: bool,
    keyboard_grabbed: bool,
    /// Left button went down on the overlay and has not been released.
    dragging: bool,
    last_down: Option<(Instant, i32, i32)>,
    click: Option<ClickState>,
    /// The modifier keys (X11 state bits) held at the last click on the program underneath.
    click_mods: u16,
    /// The image editor is open: a click outside its window is swallowed.
    modal: bool,
    own_pid: u32,
    pub server: Server,
    pub clip_counter: Arc<AtomicU64>,
    pub own_window: Arc<AtomicU32>,
    /// True while any grab is held (read by the watchdog).
    pub grabbed: Arc<AtomicBool>,
    start: Instant,
    pub db: PathBuf,
    has_ewmh: bool,
    /// The expanded viewer's window and state.
    viewer: Window,
    viewer_pos: (i32, i32),
    viewer_size: (u32, u32),
    viewer_mapped: bool,
    viewer_round: Option<bool>,
    viewer_frame: Option<(Vec<u8>, u32, u32)>,
    viewer_mode: bool,
    /// Left button went down in the viewer and has not been released.
    viewer_dragging: bool,
    /// An animation is running: the loop ticks faster.
    pub fast_ticks: bool,
}

fn mask(bits: &[EventMask]) -> EventMask {
    bits.iter().fold(EventMask::NO_EVENT, |a, b| a | *b)
}

impl LinuxPlatform {
    pub fn new(db: PathBuf, grabbed: Arc<AtomicBool>) -> XResult<LinuxPlatform> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        let screen = conn.setup().roots[screen_num].clone();
        let root = screen.root;
        let atoms = Atoms::new(&conn)?;
        let keymap = Keymap::load(&conn)?;
        let scale = x::scale_from_xft(&conn, &atoms, root);

        let overlay = conn.generate_id()?;
        conn.create_window(
            screen.root_depth,
            overlay,
            root,
            0,
            0,
            WIDTH as u16,
            HEIGHT as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(screen.black_pixel)
                .event_mask(EventMask::EXPOSURE),
        )?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, overlay, &CreateGCAux::new())?;
        // pointer moves light up the card's buttons; leaving the window turns them off
        conn.change_window_attributes(
            overlay,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::EXPOSURE | EventMask::POINTER_MOTION | EventMask::LEAVE_WINDOW),
        )?;
        let viewer = conn.generate_id()?;
        conn.create_window(
            screen.root_depth,
            viewer,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(screen.black_pixel)
                .event_mask(EventMask::EXPOSURE | EventMask::POINTER_MOTION | EventMask::LEAVE_WINDOW),
        )?;

        // a never-mapped window that owns the clipboard while we serve a paste
        let helper = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            helper,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;

        conn.xfixes_query_version(5, 0)?.reply()?;
        conn.xfixes_select_selection_input(
            helper,
            atoms.clipboard,
            xfixes::SelectionEventMask::SET_SELECTION_OWNER
                | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
                | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?;
        // key mapping changes, root property changes (EWMH)
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
        conn.flush()?;

        let has_ewmh = conn
            .get_property(false, root, atoms.net_active_window, AtomEnum::WINDOW, 0, 1)?
            .reply()
            .map(|r| r.value_len > 0)
            .unwrap_or(false);
        let own_window = Arc::new(AtomicU32::new(helper));
        Ok(LinuxPlatform {
            server: Server::new(helper),
            conn,
            root,
            atoms,
            keymap,
            scale,
            size_percent: 100,
            screen_size: (screen.width_in_pixels, screen.height_in_pixels),
            depth: screen.root_depth,
            overlay,
            gc,
            helper,
            overlay_pos: (0, 0),
            overlay_size: (WIDTH as u32, HEIGHT as u32),
            mapped: false,
            last_frame: None,
            generation: 0,
            active: false,
            keyboard_grabbed: false,
            dragging: false,
            last_down: None,
            click: None,
            click_mods: 0,
            modal: false,
            own_pid: std::process::id(),
            clip_counter: Arc::new(AtomicU64::new(0)),
            own_window,
            grabbed,
            start: Instant::now(),
            db,
            has_ewmh,
            viewer,
            viewer_pos: (0, 0),
            viewer_size: (1, 1),
            viewer_mapped: false,
            viewer_round: None,
            viewer_frame: None,
            viewer_mode: false,
            viewer_dragging: false,
            fast_ticks: false,
        })
    }

    // ------------------------------------------------------------------ activation grab

    /// Passive grab for Ctrl+Shift+middle-click, with every combination of Caps/Num Lock.
    pub fn grab_activation(&self) -> Result<(), String> {
        let base = ModMask::CONTROL | ModMask::SHIFT;
        for extra in [ModMask::default(), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2] {
            self.conn
                .grab_button(
                    false,
                    self.root,
                    mask(&[EventMask::BUTTON_PRESS, EventMask::BUTTON_RELEASE]),
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    0u32,
                    0u32,
                    ButtonIndex::M2,
                    base | extra,
                )
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|_| "Ctrl+Shift+middle-click is already used by another program".to_string())?;
        }
        self.conn.flush().map_err(|e| e.to_string())
    }

    fn grab_session(&mut self) {
        let ok = self
            .conn
            .grab_keyboard(false, self.root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.status == GrabStatus::SUCCESS);
        self.keyboard_grabbed = ok;
        if !ok {
            self.log("could not grab the keyboard; typing into the search box will not work this time");
        }
        let events = mask(&[EventMask::BUTTON_PRESS, EventMask::BUTTON_RELEASE, EventMask::BUTTON_MOTION]);
        for b in [ButtonIndex::M1, ButtonIndex::M2, ButtonIndex::M3, ButtonIndex::M4, ButtonIndex::M5] {
            let _ = self.conn.grab_button(
                false,
                self.root,
                events,
                GrabMode::SYNC,
                GrabMode::ASYNC,
                0u32,
                0u32,
                b,
                ModMask::ANY,
            );
        }
        let _ = self.conn.flush();
    }

    fn release_session(&mut self) {
        let _ = self.conn.allow_events(Allow::ASYNC_POINTER, CURRENT_TIME);
        let _ = self.conn.ungrab_keyboard(CURRENT_TIME);
        for b in [ButtonIndex::M1, ButtonIndex::M2, ButtonIndex::M3, ButtonIndex::M4, ButtonIndex::M5] {
            let _ = self.conn.ungrab_button(b, self.root, ModMask::ANY);
        }
        self.keyboard_grabbed = false;
        // the session grabs replaced any overlapping activation grab: put it back
        let _ = self.grab_activation();
        let _ = self.conn.flush();
    }

    // ------------------------------------------------------------------ geometry

    fn inside_viewer(&self, x: i32, y: i32) -> bool {
        self.viewer_mapped
            && x >= self.viewer_pos.0
            && y >= self.viewer_pos.1
            && x < self.viewer_pos.0 + self.viewer_size.0 as i32
            && y < self.viewer_pos.1 + self.viewer_size.1 as i32
    }

    fn viewer_local(&self, rx: i32, ry: i32) -> (f32, f32) {
        ((rx - self.viewer_pos.0) as f32, (ry - self.viewer_pos.1) as f32)
    }

    fn inside_overlay(&self, x: i32, y: i32) -> bool {
        self.mapped
            && x >= self.overlay_pos.0
            && y >= self.overlay_pos.1
            && x < self.overlay_pos.0 + self.overlay_size.0 as i32
            && y < self.overlay_pos.1 + self.overlay_size.1 as i32
    }

    /// The deepest window under a root coordinate.
    fn window_at(&self, x: i32, y: i32) -> Window {
        let mut win = self.root;
        for _ in 0..32 {
            let Some(r) = self
                .conn
                .translate_coordinates(self.root, win, x as i16, y as i16)
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                break;
            };
            if r.child == 0 {
                break;
            }
            win = r.child;
        }
        win
    }

    fn pointer(&self) -> (i32, i32, u16) {
        self.conn
            .query_pointer(self.root)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| (i32::from(r.root_x), i32::from(r.root_y), u16::from(r.mask)))
            .unwrap_or((0, 0, 0))
    }

    fn target_for(&self, win: Window) -> Target {
        let client = x::client_of(&self.conn, self.root, win);
        let pid = x::pid_of(&self.conn, &self.atoms, client).unwrap_or(0);
        Target {
            id: u64::from(client),
            pid,
            exe: if pid != 0 { x::exe_of_pid(pid).unwrap_or_default() } else { String::new() },
            class: x::class_of(&self.conn, client).unwrap_or_default(),
            desktop: client == self.root || x::is_desktop(&self.conn, &self.atoms, client),
        }
    }

    fn active_window(&self) -> Option<Window> {
        if self.has_ewmh {
            let r = self
                .conn
                .get_property(false, self.root, self.atoms.net_active_window, AtomEnum::WINDOW, 0, 1)
                .ok()?
                .reply()
                .ok()?;
            if let Some(w) = r.value32().and_then(|mut it| it.next()).filter(|w| *w != 0) {
                return Some(w);
            }
        }
        let f = self.conn.get_input_focus().ok()?.reply().ok()?.focus;
        (f != 0 && f != self.root && f != u32::from(InputFocus::POINTER_ROOT)).then_some(f)
    }

    // ------------------------------------------------------------------ event translation

    pub fn reload_keymap(&mut self) {
        if let Ok(k) = Keymap::load(&self.conn) {
            self.keymap = k;
        }
    }

    pub fn translate(&mut self, ev: XEvent) -> Vec<Action> {
        let mut out = Vec::new();
        match ev {
            XEvent::ButtonPress(e) => self.button_press(&e, &mut out),
            XEvent::ButtonRelease(e) => {
                if self.active && self.viewer_dragging && e.detail == 1 {
                    self.viewer_dragging = false;
                    let (x, y) = self.viewer_local(i32::from(e.root_x), i32::from(e.root_y));
                    out.push(Action::Event(Event::ViewerMouse(MouseEvent { kind: MouseKind::Up, x, y })));
                } else if self.active && self.dragging && e.detail == 1 {
                    self.dragging = false;
                    let (x, y) = self.local(i32::from(e.root_x), i32::from(e.root_y));
                    out.push(Action::Event(Event::Mouse(MouseEvent { kind: MouseKind::Up, x, y })));
                }
            }
            XEvent::MotionNotify(e) => {
                let (rx, ry) = (i32::from(e.root_x), i32::from(e.root_y));
                if self.active && self.viewer_dragging {
                    let (x, y) = self.viewer_local(rx, ry);
                    out.push(Action::Event(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x, y })));
                } else if self.active && self.dragging {
                    let (x, y) = self.local(rx, ry);
                    out.push(Action::Event(Event::Mouse(MouseEvent { kind: MouseKind::Move, x, y })));
                } else if self.active && e.event == self.viewer {
                    let (x, y) = (f32::from(e.event_x), f32::from(e.event_y));
                    out.push(Action::Event(Event::ViewerMouse(MouseEvent { kind: MouseKind::Move, x, y })));
                } else if self.active && e.event == self.overlay {
                    let (x, y) = (f32::from(e.event_x), f32::from(e.event_y));
                    out.push(Action::Event(Event::Mouse(MouseEvent { kind: MouseKind::Move, x, y })));
                }
            }
            XEvent::LeaveNotify(e) => {
                let m = MouseEvent { kind: MouseKind::Leave, x: 0.0, y: 0.0 };
                if self.active && e.event == self.viewer {
                    out.push(Action::Event(Event::ViewerMouse(m)));
                } else if self.active && e.event == self.overlay {
                    out.push(Action::Event(Event::Mouse(m)));
                }
            }
            XEvent::KeyPress(e) => {
                if self.active {
                    if let Some(k) = keys::classify(&self.keymap, e.detail, u16::from(e.state)) {
                        out.push(Action::Event(Event::Key(k, self.generation)));
                    }
                }
            }
            XEvent::Expose(e) => {
                if e.window == self.overlay && e.count == 0 {
                    self.repaint();
                }
                if e.window == self.viewer && e.count == 0 {
                    if let Some((data, w, h)) = self.viewer_frame.take() {
                        self.put_to(self.viewer, &data, w, h);
                        self.viewer_frame = Some((data, w, h));
                    }
                }
            }
            XEvent::SelectionRequest(r) => {
                let atoms = &self.atoms;
                self.server.on_request(&self.conn, atoms, &r);
            }
            XEvent::SelectionClear(c) => self.server.cleared(c.selection, self.atoms.clipboard),
            XEvent::PropertyNotify(p) => {
                if p.state == Property::DELETE {
                    self.server.on_property_deleted(&self.conn, p.window, p.atom);
                }
            }
            XEvent::XfixesSelectionNotify(n) => {
                if n.selection == self.atoms.clipboard {
                    self.clip_counter.fetch_add(1, Ordering::SeqCst);
                    out.push(Action::ClipboardChanged);
                }
            }
            XEvent::MappingNotify(_) => self.reload_keymap(),
            _ => {}
        }
        out
    }

    fn local(&self, rx: i32, ry: i32) -> (f32, f32) {
        ((rx - self.overlay_pos.0) as f32, (ry - self.overlay_pos.1) as f32)
    }

    fn button_press(&mut self, e: &ButtonPressEvent, out: &mut Vec<Action>) {
        let (rx, ry) = (i32::from(e.root_x), i32::from(e.root_y));
        if !self.active {
            // the activation gesture (async grab: nothing to release)
            if e.detail == 2 {
                out.push(Action::Event(Event::Activate));
            }
            return;
        }
        let gen = self.generation;
        if self.viewer_mode {
            self.viewer_button_press(e, out);
            return;
        }
        let inside = self.inside_overlay(rx, ry);
        let mut replay = false;
        match e.detail {
            4 => out.push(Action::Event(Event::Step(pasteroll_core::Step::Newer, gen))),
            5 => out.push(Action::Event(Event::Step(pasteroll_core::Step::Older, gen))),
            1 if inside => {
                let now = Instant::now();
                let double = self.last_down.is_some_and(|(t, x, y)| {
                    now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                        && (x - rx).abs() <= DOUBLE_CLICK_PX
                        && (y - ry).abs() <= DOUBLE_CLICK_PX
                });
                self.last_down = if double { None } else { Some((now, rx, ry)) };
                self.dragging = true;
                let (x, y) = self.local(rx, ry);
                let kind = if double { MouseKind::DoubleClick } else { MouseKind::Down };
                out.push(Action::Event(Event::Mouse(MouseEvent { kind, x, y })));
            }
            1 => {
                // a click on the program underneath: let it through (it places the caret), then
                // paste there once the click has settled
                let deep = self.window_at(rx, ry);
                self.click_mods = u16::from(e.state);
                self.click = Some(ClickState { window: deep, since: Instant::now() });
                replay = true;
                out.push(Action::Event(Event::ClickPaste(gen)));
            }
            2 | 3 if !inside => {
                replay = true;
                out.push(Action::Event(Event::ClickCancel(gen)));
            }
            6 | 7 => {}
            _ if !inside => replay = true,
            _ => {}
        }
        let mode = if replay { Allow::REPLAY_POINTER } else { Allow::ASYNC_POINTER };
        let _ = self.conn.allow_events(mode, CURRENT_TIME);
        let _ = self.conn.flush();
    }

    /// A button press while the viewer is open: the wheel turns raw, clicks in the viewer go to
    /// it, a click anywhere else closes it (and reaches what was clicked).
    fn viewer_button_press(&mut self, e: &ButtonPressEvent, out: &mut Vec<Action>) {
        let (rx, ry) = (i32::from(e.root_x), i32::from(e.root_y));
        let gen = self.generation;
        let state = u16::from(e.state);
        let (ctrl, shift) = (state & keys::CONTROL != 0, state & keys::SHIFT != 0);
        let inside = self.inside_viewer(rx, ry);
        let mut replay = false;
        match e.detail {
            4..=7 => {
                let delta = if e.detail == 4 || e.detail == 7 { 120 } else { -120 };
                let wheel = Wheel { delta, horizontal: e.detail >= 6, ctrl, shift };
                out.push(Action::Event(Event::Wheel(wheel, gen)));
            }
            1 if inside => {
                let now = Instant::now();
                let double = self.last_down.is_some_and(|(t, x, y)| {
                    now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                        && (x - rx).abs() <= DOUBLE_CLICK_PX
                        && (y - ry).abs() <= DOUBLE_CLICK_PX
                });
                self.last_down = if double { None } else { Some((now, rx, ry)) };
                self.viewer_dragging = true;
                let (x, y) = self.viewer_local(rx, ry);
                let kind = if double { MouseKind::DoubleClick } else { MouseKind::Down };
                out.push(Action::Event(Event::ViewerMouse(MouseEvent { kind, x, y })));
            }
            _ if inside => {}
            // the image editor is modal: the click goes nowhere
            _ if self.modal => {}
            _ => {
                replay = true;
                out.push(Action::Event(Event::ClickOutside(gen)));
            }
        }
        let mode = if replay { Allow::REPLAY_POINTER } else { Allow::ASYNC_POINTER };
        let _ = self.conn.allow_events(mode, CURRENT_TIME);
        let _ = self.conn.flush();
    }

    /// Asks the window manager to make `win` the active window again (after a dialog).
    fn activate(&self, win: Window) {
        if self.has_ewmh {
            let ev = ClientMessageEvent::new(32, win, self.atoms.net_active_window, [2u32, CURRENT_TIME, 0, 0, 0]);
            let _ = self.conn.send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ev,
            );
        } else {
            let _ = self.conn.set_input_focus(InputFocus::PARENT, win, CURRENT_TIME);
        }
        let _ = self.conn.flush();
    }

    fn repaint(&mut self) {
        if let Some((data, w, h)) = self.last_frame.take() {
            self.put(&data, w, h);
            self.last_frame = Some((data, w, h));
        }
    }

    fn put(&self, data: &[u8], w: u32, h: u32) {
        self.put_to(self.overlay, data, w, h);
    }

    fn put_to(&self, win: Window, data: &[u8], w: u32, h: u32) {
        // keep each request well under the server's maximum
        let max = self.conn.maximum_request_bytes().saturating_sub(64);
        let row_bytes = (w as usize) * 4;
        let rows_per = (max / row_bytes.max(1)).clamp(1, h as usize);
        let mut y = 0usize;
        while y < h as usize {
            let n = rows_per.min(h as usize - y);
            let chunk = &data[y * row_bytes..(y + n) * row_bytes];
            let _ = self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                win,
                self.gc,
                w as u16,
                n as u16,
                0,
                y as i16,
                0,
                self.depth,
                chunk,
            );
            y += n;
        }
        let _ = self.conn.flush();
    }

    pub fn depth(&self) -> u8 {
        self.depth
    }

    pub fn set_size(&mut self, size_percent: u32) {
        self.size_percent = size_percent;
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// `(x, y, w, h)` of the monitor containing the point, or of the whole screen.
    fn monitor_at(&self, at: (i32, i32)) -> (i32, i32, i32, i32) {
        let whole = (0, 0, i32::from(self.screen_size.0), i32::from(self.screen_size.1));
        let Some(reply) = self.conn.randr_get_monitors(self.root, true).ok().and_then(|c| c.reply().ok()) else {
            return whole;
        };
        let rects: Vec<(i32, i32, i32, i32)> = reply
            .monitors
            .iter()
            .map(|m| (i32::from(m.x), i32::from(m.y), i32::from(m.width), i32::from(m.height)))
            .filter(|r| r.2 > 0 && r.3 > 0)
            .collect();
        pick_monitor(&rects, at).unwrap_or(whole)
    }

    pub fn screen_size(&self) -> (i32, i32) {
        (i32::from(self.screen_size.0), i32::from(self.screen_size.1))
    }

    /// Is there a window manager that tells us about the active window (so it also gives focus)?
    pub fn has_wm(&self) -> bool {
        self.has_ewmh
    }

    pub fn classify_key(&self, code: u8, state: u16) -> Option<pasteroll_app::Key> {
        keys::classify(&self.keymap, code, state)
    }

    fn keycode(&self, sym: u32) -> Option<u8> {
        self.keymap.keycode_any(sym)
    }

    fn fake_key(&self, code: u8, press: bool) -> bool {
        let ty = if press { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT };
        self.conn.xtest_fake_input(ty, code, 0, self.root, 0, 0, 0).is_ok()
    }

    /// Is the key with this keycode physically/logically down right now?
    fn key_down(&self, code: u8) -> bool {
        self.conn
            .query_keymap()
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.keys.get(usize::from(code / 8)).is_some_and(|b| b & (1 << (code % 8)) != 0))
    }
}

/// Nearest-neighbour stretch of a BGRA picture (animation frames only: they are on screen for a
/// few milliseconds each).
fn stretch(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw.max(1) as usize, sh.max(1) as usize, dw.max(1) as usize, dh.max(1) as usize);
    let xs: Vec<usize> = (0..dw).map(|x| (x * sw / dw).min(sw - 1) * 4).collect();
    let mut out = vec![0u8; dw * dh * 4];
    for (y, row) in out.chunks_exact_mut(dw * 4).enumerate() {
        let sy = (y * sh / dh).min(sh - 1);
        let line = &src[sy * sw * 4..(sy + 1) * sw * 4];
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&line[xs[x]..xs[x] + 4]);
        }
    }
    out
}

#[cfg(test)]
mod stretch_tests {
    #[test]
    fn stretching_doubles_pixels() {
        let src = vec![1, 1, 1, 255, 2, 2, 2, 255];
        let out = super::stretch(&src, 2, 1, 4, 2);
        assert_eq!(out.len(), 4 * 2 * 4);
        assert_eq!(&out[0..4], &[1, 1, 1, 255]);
        assert_eq!(&out[8..12], &[2, 2, 2, 255]);
        assert_eq!(&out[16..20], &[1, 1, 1, 255]);
    }
}

impl Drop for LinuxPlatform {
    fn drop(&mut self) {
        self.release_session();
        if let Some((d, _, _)) = self.last_frame.as_mut() {
            d.zeroize();
        }
        self.grabbed.store(false, Ordering::SeqCst);
    }
}

impl Platform for LinuxPlatform {
    fn foreground(&mut self) -> Option<Target> {
        let win = self.active_window()?;
        if win == self.overlay || win == self.helper || win == self.viewer {
            return None;
        }
        let t = self.target_for(win);
        if t.pid == self.own_pid {
            return None;
        }
        Some(t)
    }

    fn target_is_current(&mut self, target: &Target) -> bool {
        match self.active_window() {
            Some(w) => u64::from(x::client_of(&self.conn, self.root, w)) == target.id || (target.desktop && {
                let t = self.target_for(w);
                t.desktop
            }),
            None => target.desktop,
        }
    }

    fn target_privileged(&mut self, target: &Target) -> bool {
        target.pid != 0 && x::pid_is_other_user(target.pid)
    }

    fn cursor(&self) -> (i32, i32) {
        let (x, y, _) = self.pointer();
        (x, y)
    }

    fn overlay_show(&mut self, at: (i32, i32)) -> OverlayInfo {
        // the monitor under the pointer (the whole screen when RandR cannot tell)
        let (mx, my, sw, sh) = self.monitor_at(at);
        let factor = pasteroll_core::settings::size_factor(self.size_percent);
        let scale = self.scale * factor;
        let w = ((WIDTH * scale).round() as i32).clamp(200, sw.max(200));
        let h = ((HEIGHT * scale).round() as i32).clamp(150, sh.max(150));
        // next to the pointer, moved back onto that monitor if it would overflow
        let mut x = at.0 + 12;
        let mut y = at.1 + 12;
        if x + w > mx + sw {
            x = (at.0 - 12 - w).max(mx);
        }
        if y + h > my + sh {
            y = (my + sh - h).max(my);
        }
        self.overlay_pos = (x, y);
        self.overlay_size = (w as u32, h as u32);
        let _ = self.conn.configure_window(
            self.overlay,
            &ConfigureWindowAux::new()
                .x(x)
                .y(y)
                .width(w as u32)
                .height(h as u32)
                .stack_mode(StackMode::ABOVE),
        );
        let rects = rounded_rects(w as u32, h as u32, (CORNER_DIP * scale).round() as u32);
        let _ = shape::rectangles(
            &self.conn,
            shape::SO::SET,
            shape::SK::BOUNDING,
            ClipOrdering::UNSORTED,
            self.overlay,
            0,
            0,
            &rects,
        );
        let _ = self.conn.map_window(self.overlay);
        let _ = self.conn.flush();
        self.mapped = true;
        let monitor = (mx, my, sw.max(1) as u32, sh.max(1) as u32);
        OverlayInfo { scale, size_px: (w as u32, h as u32), origin_px: (x, y), monitor_px: monitor, work_px: monitor }
    }



    fn viewer_show(&mut self, rect: (i32, i32, u32, u32), rounded: bool) {
        let (w, h) = (rect.2.max(1), rect.3.max(1));
        let _ = self.conn.configure_window(
            self.viewer,
            &ConfigureWindowAux::new().x(rect.0).y(rect.1).width(w).height(h).stack_mode(StackMode::ABOVE),
        );
        if self.viewer_size != (w, h) || self.viewer_round != Some(rounded) {
            let r = if rounded { (CORNER_DIP * self.scale).round() as u32 } else { 0 };
            let rects = rounded_rects(w, h, r);
            let _ = shape::rectangles(&self.conn, shape::SO::SET, shape::SK::BOUNDING, ClipOrdering::UNSORTED, self.viewer, 0, 0, &rects);
            self.viewer_round = Some(rounded);
        }
        self.viewer_pos = (rect.0, rect.1);
        self.viewer_size = (w, h);
        if !self.viewer_mapped {
            let _ = self.conn.map_window(self.viewer);
            self.viewer_mapped = true;
        }
        let _ = self.conn.flush();
    }

    fn viewer_present(&mut self, frame: &Frame) {
        if !self.viewer_mapped {
            return;
        }
        let (w, h) = self.viewer_size;
        // animation frames are drawn small and stretched here (the server cannot scale)
        let data = if (frame.width, frame.height) == (w, h) {
            frame.bgra.clone()
        } else {
            stretch(&frame.bgra, frame.width, frame.height, w, h)
        };
        self.put_to(self.viewer, &data, w, h);
        if let Some((d, _, _)) = self.viewer_frame.as_mut() {
            d.zeroize();
        }
        self.viewer_frame = Some((data, w, h));
    }

    fn viewer_hide(&mut self) {
        let _ = self.conn.unmap_window(self.viewer);
        let _ = self.conn.flush();
        self.viewer_mapped = false;
        self.viewer_dragging = false;
        if let Some((d, _, _)) = self.viewer_frame.as_mut() {
            d.zeroize();
        }
        self.viewer_frame = None;
    }

    fn viewer_mode(&mut self, on: bool) {
        self.viewer_mode = on;
        self.viewer_dragging = false;
    }

    fn animating(&mut self, on: bool) {
        self.fast_ticks = on;
    }

    fn modal_clicks(&mut self, on: bool) {
        self.modal = on;
    }

    fn click_modifiers(&self) -> pasteroll_app::Modifiers {
        let m = self.click_mods;
        pasteroll_app::Modifiers { ctrl: m & keys::CONTROL != 0, shift: m & keys::SHIFT != 0, alt: m & keys::ALT != 0 }
    }

    fn load_presets(&self) -> String {
        std::fs::read_to_string(self.db.with_file_name("image-presets.txt")).unwrap_or_default()
    }

    fn save_presets(&mut self, text: &str) -> Result<(), String> {
        let path = self.db.with_file_name("image-presets.txt");
        let tmp = self.db.with_file_name("image-presets.txt.tmp");
        std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
    }

    fn folder_dialog(&mut self, start: Option<&Path>, target: &Target) -> SaveChoice {
        // another program again: see `save_dialog`
        self.input_active(false);
        let _ = self.conn.unmap_window(self.viewer);
        let _ = self.conn.unmap_window(self.overlay);
        let _ = self.conn.flush();
        let choice = crate::dialogs::pick_folder(start);
        let _ = self.conn.map_window(self.overlay);
        let _ = self.conn.map_window(self.viewer);
        let _ = self.conn.flush();
        self.repaint();
        if let Some((data, w, h)) = self.viewer_frame.take() {
            self.put_to(self.viewer, &data, w, h);
            self.viewer_frame = Some((data, w, h));
        }
        self.activate(target.id as Window);
        self.input_active(true);
        choice
    }

    fn shift_held(&self) -> bool {
        let (_, _, m) = self.pointer();
        m & keys::SHIFT != 0
    }

    fn save_dialog(&mut self, name: &str, _ext: &str, target: &Target) -> SaveChoice {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let folder = self.desktop_folder().unwrap_or(home);
        let suggested = folder.join(name);
        // The dialog is another program: it needs the keyboard, and our windows (above everything)
        // would hide it. Both are given back afterwards.
        self.input_active(false);
        let _ = self.conn.unmap_window(self.viewer);
        let _ = self.conn.unmap_window(self.overlay);
        let _ = self.conn.flush();
        let choice = crate::dialogs::save_file(&suggested);
        let _ = self.conn.map_window(self.overlay);
        let _ = self.conn.map_window(self.viewer);
        let _ = self.conn.flush();
        self.repaint();
        if let Some((data, w, h)) = self.viewer_frame.take() {
            self.put_to(self.viewer, &data, w, h);
            self.viewer_frame = Some((data, w, h));
        }
        self.activate(target.id as Window);
        self.input_active(true);
        choice
    }

    fn format_time(&self, secs: i64) -> String {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe {
            libc::localtime_r(&t, &mut tm);
        }
        format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min)
    }

    fn overlay_present(&mut self, frame: &Frame) {
        if !self.mapped {
            return;
        }
        let (w, h) = (frame.width, frame.height);
        self.put(&frame.bgra, w, h);
        match self.last_frame.as_mut() {
            Some((d, ow, oh)) => {
                d.zeroize();
                *d = frame.bgra.clone();
                *ow = w;
                *oh = h;
            }
            None => self.last_frame = Some((frame.bgra.clone(), w, h)),
        }
    }

    fn overlay_hide(&mut self) {
        let _ = self.conn.unmap_window(self.overlay);
        let _ = self.conn.flush();
        self.mapped = false;
        if let Some((d, _, _)) = self.last_frame.as_mut() {
            d.zeroize();
        }
        self.last_frame = None;
    }

    fn input_active(&mut self, active: bool) {
        self.generation += 1;
        if active == self.active {
            return;
        }
        self.active = active;
        self.dragging = false;
        self.last_down = None;
        if active {
            self.grab_session();
        } else {
            self.release_session();
        }
        self.grabbed.store(active, Ordering::SeqCst);
    }

    fn input_generation(&self) -> u64 {
        self.generation
    }

    fn heartbeat(&mut self) {}

    fn modifiers_released(&mut self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        loop {
            let (_, _, m) = self.pointer();
            // Alt (Mod1) and Super (Mod4) turn the paste chord into something else
            if m & (keys::ALT | keys::SUPER) == 0 {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn poll_click(&mut self, _first: bool) -> ClickPoll {
        let Some(click) = self.click.as_ref() else {
            return ClickPoll::Rejected("no click to paste at");
        };
        let (_, _, m) = self.pointer();
        if m & (1 << 8) != 0 {
            return ClickPoll::Pending; // Button1 still down
        }
        let win = click.window;
        let since = click.since;
        let client = x::client_of(&self.conn, self.root, win);
        if client == self.overlay || client == self.helper {
            return ClickPoll::Rejected("click was on the overlay");
        }
        let target = self.target_for(win);
        if target.pid == self.own_pid {
            return ClickPoll::Rejected("click was on our own window");
        }
        if target.desktop && win == self.root {
            return ClickPoll::Desktop;
        }
        if target.desktop {
            return ClickPoll::Desktop;
        }
        // let the window manager hand over focus; without one, focus the window ourselves
        match self.active_window() {
            Some(w) if u64::from(x::client_of(&self.conn, self.root, w)) == target.id => {}
            _ => {
                if since.elapsed() < Duration::from_millis(120) {
                    return ClickPoll::Pending;
                }
                if !self.has_ewmh {
                    let _ = self.conn.set_input_focus(InputFocus::PARENT, client, CURRENT_TIME);
                    let _ = self.conn.flush();
                } else if since.elapsed() < Duration::from_millis(500) {
                    return ClickPoll::Pending;
                }
            }
        }
        ClickPoll::Target(target)
    }

    fn write_clipboard(&mut self, payload: &Payload) -> Result<(), String> {
        let r = self.server.set(&self.conn, &self.atoms, payload);
        let _ = self.conn.flush();
        r
    }

    fn send_paste(&mut self, key: PasteKey) -> bool {
        let (Some(ctrl), Some(shift)) = (self.keycode(x::XK_CONTROL_L), self.keycode(x::XK_SHIFT_L)) else {
            return false;
        };
        let v = self.keycode(0x76);
        let insert = self.keycode(x::XK_INSERT);
        let (mods, main): (Vec<u8>, Option<u8>) = match key {
            PasteKey::CtrlV => (vec![ctrl], v),
            PasteKey::ShiftInsert => (vec![shift], insert),
            PasteKey::CtrlShiftV => (vec![ctrl, shift], v),
        };
        let Some(main) = main else { return false };
        if key == PasteKey::ShiftInsert {
            // Shift+Insert pastes PRIMARY in the xterm family: offer the same content there too
            self.server.own_primary(&self.conn);
        }
        // Ctrl/Shift still held from the activation gesture must not leak into the chord: lift them
        // first and press them again afterwards, so the user's state is as they left it.
        let held: Vec<u8> = [ctrl, shift].into_iter().filter(|c| self.key_down(*c)).collect();
        let mut ok = true;
        for c in &held {
            ok &= self.fake_key(*c, false);
        }
        for c in &mods {
            ok &= self.fake_key(*c, true);
        }
        ok &= self.fake_key(main, true);
        ok &= self.fake_key(main, false);
        for c in mods.iter().rev() {
            ok &= self.fake_key(*c, false);
        }
        for c in &held {
            ok &= self.fake_key(*c, true);
        }
        // a round trip proves the server processed all of it
        ok && self.conn.flush().is_ok() && self.conn.get_input_focus().ok().and_then(|c| c.reply().ok()).is_some()
    }

    fn desktop_folder(&self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let dirs = std::fs::read_to_string(home.join(".config").join("user-dirs.dirs")).unwrap_or_default();
        let from_file = dirs.lines().find_map(|l| {
            let v = l.trim().strip_prefix("XDG_DESKTOP_DIR=")?;
            let v = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
            Some(PathBuf::from(v))
        });
        let p = from_file.unwrap_or_else(|| home.join("Desktop"));
        p.is_dir().then_some(p)
    }

    fn local_timestamp(&self) -> String {
        let t = unsafe { libc::time(std::ptr::null_mut()) };
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe {
            libc::localtime_r(&t, &mut tm);
        }
        format!(
            "{:04}-{:02}-{:02} {:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }

    fn file_created(&mut self, _path: &Path) {}

    fn now_secs(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn log(&self, msg: &str) {
        eprintln!("[pasteroll] {msg}");
    }

    fn add_custom_filter(
        &mut self,
        name: &str,
        extensions: &[String],
    ) -> Result<Vec<pasteroll_core::config::CustomFilter>, String> {
        let text = pasteroll_core::keystore::read_config_text(&self.db)?;
        let new_text = pasteroll_core::config::add_custom_filter(&text, name, extensions)
            .map_err(|problems| problems.join("; "))?;
        pasteroll_core::keystore::write_config_text::<sys::LinuxOs>(&self.db, &new_text)?;
        pasteroll_core::config::parse(&new_text)
            .map(|c| c.custom_filters)
            .map_err(|problems| problems.join("; "))
    }
}
