//! The parts of the app that are not the carousel: a notification-area icon (XEmbed system tray),
//! its popup menu, and the settings window. They are drawn with the renderer's small widget layer,
//! so there is no toolkit dependency. The loop gives this module the X events that belong to its
//! windows and carries out the [`ShellAction`]s it returns.

use crate::platform::LinuxPlatform;
use pasteroll_app::Key;
use pasteroll_core::config::{self, Config};
use pasteroll_core::limits::Limits;
use pasteroll_core::settings::{self, fmt_age, fmt_mb, thousands, Choice, Steps};
use pasteroll_render::{
    hit, list_row_at, list_visible_rows, slider_index_at, Chrome, Rect, Renderer, UiView, WKind, Widget,
};
use x11rb::connection::Connection;
use x11rb::properties::WmSizeHints;
use x11rb::protocol::shape;
use x11rb::protocol::xproto::*;
use crate::sni::{Sni, SniEvent};
use x11rb::protocol::Event as XEvent;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::CURRENT_TIME;

/// What the loop must do on the shell's behalf.
#[derive(Debug)]
pub enum ShellAction {
    TogglePause,
    Quit,
    OpenFolder,
    /// The settings window wants to save. `confirmed` is false until any deletion was agreed to.
    Save { text: String, limits: Limits, confirmed: bool },
}

// ------------------------------------------------------------------ small helpers

struct ShellAtoms {
    tray_selection: Atom,
    tray_opcode: Atom,
    xembed_info: Atom,
    manager: Atom,
    wm_protocols: Atom,
    wm_delete: Atom,
    net_wm_name: Atom,
    utf8: Atom,
    net_wm_pid: Atom,
    net_wm_icon: Atom,
}

impl ShellAtoms {
    fn new(conn: &impl Connection, screen: usize) -> Result<ShellAtoms, Box<dyn std::error::Error>> {
        let sel = format!("_NET_SYSTEM_TRAY_S{screen}");
        let names: [&[u8]; 8] = [
            sel.as_bytes(),
            b"_NET_SYSTEM_TRAY_OPCODE",
            b"_XEMBED_INFO",
            b"MANAGER",
            b"WM_PROTOCOLS",
            b"WM_DELETE_WINDOW",
            b"_NET_WM_NAME",
            b"UTF8_STRING",
        ];
        let cookies: Vec<_> = names.iter().map(|n| conn.intern_atom(false, n)).collect::<Result<_, _>>()?;
        let mut a = Vec::new();
        for c in cookies {
            a.push(c.reply()?.atom);
        }
        let pid = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let icon = conn.intern_atom(false, b"_NET_WM_ICON")?.reply()?.atom;
        Ok(ShellAtoms {
            tray_selection: a[0],
            tray_opcode: a[1],
            xembed_info: a[2],
            manager: a[3],
            wm_protocols: a[4],
            wm_delete: a[5],
            net_wm_name: a[6],
            utf8: a[7],
            net_wm_pid: pid,
            net_wm_icon: icon,
        })
    }
}

fn put_bgra(conn: &impl Connection, win: Window, gc: Gcontext, depth: u8, w: u32, h: u32, data: &[u8]) {
    let max = conn.maximum_request_bytes().saturating_sub(64);
    let row = (w as usize) * 4;
    let rows_per = (max / row.max(1)).clamp(1, h as usize);
    let mut y = 0usize;
    while y < h as usize {
        let n = rows_per.min(h as usize - y);
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            win,
            gc,
            w as u16,
            n as u16,
            0,
            y as i16,
            0,
            depth,
            &data[y * row..(y + n) * row],
        );
        y += n;
    }
    let _ = conn.flush();
}

/// The logo at `n` x `n` for a tray icon: opaque BGRA where the logo is (colour kept, transparent
/// pixels get the dark panel colour) plus a 1-bit mask of where the logo is, which the server uses
/// to cut the window to shape so the tray's own background shows through the rest.
pub fn icon_pixels(n: u32, paused: bool) -> (Vec<u8>, Vec<bool>) {
    let rgba = pasteroll_core::logo::rgba(n, paused);
    let mut bgra = Vec::with_capacity(rgba.len());
    let mut solid = Vec::with_capacity(rgba.len() / 4);
    let bg = (31u32, 36u32, 48u32);
    for p in rgba.chunks_exact(4) {
        let a = u32::from(p[3]);
        solid.push(a >= 100);
        let mix = |c: u8, b: u32| ((u32::from(c) * a + b * (255 - a)) / 255) as u8;
        bgra.extend_from_slice(&[mix(p[2], bg.2), mix(p[1], bg.1), mix(p[0], bg.0), 255]);
    }
    (bgra, solid)
}

/// Packs a mask for an X bitmap: rows padded to `pad_bits`, bits in `lsb_first` order.
pub fn mask_bits(solid: &[bool], n: usize, pad_bits: usize, lsb_first: bool) -> Vec<u8> {
    let row_bytes = n.div_ceil(pad_bits) * pad_bits / 8;
    let mut out = vec![0u8; row_bytes * n];
    for y in 0..n {
        for x in 0..n {
            if solid[y * n + x] {
                let bit = if lsb_first { x % 8 } else { 7 - x % 8 };
                out[y * row_bytes + x / 8] |= 1 << bit;
            }
        }
    }
    out
}

/// `_NET_WM_ICON` data: width, height, then ARGB pixels (straight alpha), for each size.
pub fn wm_icon_data(sizes: &[u32]) -> Vec<u32> {
    let mut v = Vec::new();
    for &n in sizes {
        v.push(n);
        v.push(n);
        for p in pasteroll_core::logo::rgba(n, false).chunks_exact(4) {
            v.push((u32::from(p[3]) << 24) | (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]));
        }
    }
    v
}

// ------------------------------------------------------------------ the shell

struct TrayIcon {
    win: Window,
    gc: Gcontext,
    size: u32,
    docked: bool,
}

struct Menu {
    win: Window,
    gc: Gcontext,
    size: (u32, u32),
    view: UiView,
}

const ID_PAUSE: u32 = 1;
const ID_SETTINGS: u32 = 2;
const ID_FOLDER: u32 = 3;
const ID_QUIT: u32 = 4;

const S_SLIDER: [u32; 4] = [101, 102, 103, 104];
const S_PAUSE: u32 = 120;
const S_WARN: u32 = 121;
const S_AUTOSTART: u32 = 122;
const S_MOD: [u32; 3] = [123, 124, 125];
const S_NOTICE_OK: u32 = 170;
const S_EX_LIST: u32 = 130;
const S_EX_ENTRY: u32 = 131;
const S_EX_ADD: u32 = 132;
const S_EX_REMOVE: u32 = 133;
const S_FL_LIST: u32 = 140;
const S_FL_REMOVE: u32 = 141;
const S_SAVE: u32 = 150;
const S_CANCEL: u32 = 151;
const S_CONFIRM: u32 = 160;
const S_BACK: u32 = 161;

const SET_W: f32 = 520.0;
const SET_H: f32 = 728.0;
const S_SIZE: u32 = 105;

struct SettingsWin {
    win: Window,
    gc: Gcontext,
    scale: f32,
    orig: Config,
    steps: Steps,
    pos: [usize; 4],
    size_choices: Vec<u32>,
    size_pos: usize,
    paused: bool,
    warn: bool,
    autostart: bool,
    autostart_was: bool,
    modifier: config::ClickModifier,
    exclude: Vec<String>,
    filters: Vec<String>,
    ex_sel: Option<usize>,
    fl_sel: Option<usize>,
    ex_top: usize,
    fl_top: usize,
    entry: String,
    entry_focus: bool,
    hover: Option<u32>,
    drag: Option<u32>,
    status: String,
    /// Waiting for the person to agree to deletions: the message, and what would be saved.
    confirm: Option<(String, String, Limits)>,
}

struct NoticeWin {
    win: Window,
    gc: Gcontext,
    scale: f32,
    hover: bool,
}

pub struct Shell {
    notice: Option<NoticeWin>,
    atoms: ShellAtoms,
    renderer: Option<Renderer>,
    tray: Option<TrayIcon>,
    menu: Option<Menu>,
    settings: Option<SettingsWin>,
    paused: bool,
    /// The StatusNotifierItem icon, when the desktop has a watcher for one (else the XEmbed tray).
    sni: Option<Sni>,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

impl Shell {
    pub fn new(p: &LinuxPlatform, screen: usize, paused: bool) -> Result<Shell, Box<dyn std::error::Error>> {
        let atoms = ShellAtoms::new(&p.conn, screen)?;
        // MANAGER messages (a tray appearing later) arrive on the root window
        p.conn.change_window_attributes(
            p.root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )?;
        let sni = if std::env::var_os("PASTEROLL_NO_SNI").is_some() { None } else { Sni::start(paused) };
        let mut s = Shell { notice: None, atoms, renderer: None, tray: None, menu: None, settings: None, paused, sni };
        if s.sni.is_none() {
            s.dock(p);
        }
        Ok(s)
    }

    fn renderer(&mut self) -> &mut Renderer {
        self.renderer.get_or_insert_with(Renderer::new)
    }

    pub fn set_paused(&mut self, p: &LinuxPlatform, paused: bool) {
        self.paused = paused;
        if let Some(s) = &self.sni {
            s.set_paused(paused);
        }
        self.paint_tray(p);
    }

    // ------------------------------------------------------------------ tray icon

    /// What the StatusNotifier icon was asked to do since the last call.
    pub fn poll_sni(&mut self, p: &LinuxPlatform) -> Vec<ShellAction> {
        let mut out = Vec::new();
        let mut events = Vec::new();
        if let Some(s) = &self.sni {
            while let Ok(e) = s.events.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            match e {
                SniEvent::Activate | SniEvent::Settings => self.open_settings(p),
                SniEvent::ContextMenu(x, y) => self.open_menu(p, (x, y)),
                SniEvent::Pause => out.push(ShellAction::TogglePause),
                SniEvent::Folder => out.push(ShellAction::OpenFolder),
                SniEvent::Quit => out.push(ShellAction::Quit),
                // the watcher is gone: use the old tray, if there is one
                SniEvent::Gone => self.dock(p),
                SniEvent::Back => {
                    if let Some(t) = self.tray.take() {
                        let _ = p.conn.destroy_window(t.win);
                        let _ = p.conn.free_gc(t.gc);
                        let _ = p.conn.flush();
                    }
                }
            }
        }
        out
    }

    /// Asks the system tray (if there is one) to embed our icon window.
    fn dock(&mut self, p: &LinuxPlatform) {
        let Ok(owner) = p.conn.get_selection_owner(self.atoms.tray_selection).map(|c| c.reply()) else { return };
        let Ok(owner) = owner else { return };
        if owner.owner == 0 {
            return;
        }
        if self.tray.is_none() {
            let (Ok(win), Ok(gc)) = (p.conn.generate_id(), p.conn.generate_id()) else { return };
            let ok = p
                .conn
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    win,
                    p.root,
                    0,
                    0,
                    24,
                    24,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new()
                        .background_pixel(0x001f2430)
                        .event_mask(EventMask::BUTTON_PRESS | EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY),
                )
                .is_ok()
                && p.conn.create_gc(gc, win, &CreateGCAux::new()).is_ok();
            if !ok {
                return;
            }
            // XEmbed: version 0, mapped
            let _ = p.conn.change_property32(PropMode::REPLACE, win, self.atoms.xembed_info, self.atoms.xembed_info, &[0, 1]);
            self.tray = Some(TrayIcon { win, gc, size: 24, docked: false });
        }
        let Some(t) = self.tray.as_mut() else { return };
        let ev = ClientMessageEvent::new(32, owner.owner, self.atoms.tray_opcode, [CURRENT_TIME, 0, t.win, 0, 0]);
        let _ = p.conn.send_event(false, owner.owner, EventMask::NO_EVENT, ev);
        let _ = p.conn.flush();
        t.docked = true;
    }

    fn paint_tray(&self, p: &LinuxPlatform) {
        let Some(t) = &self.tray else { return };
        let n = t.size.clamp(16, 128);
        let (bgra, solid) = icon_pixels(n, self.paused);
        put_bgra(&p.conn, t.win, t.gc, p.depth(), n, n, &bgra);
        // cut the window to the logo's outline (best effort: without the extension the dark
        // panel colour shows around it)
        let setup = p.conn.setup();
        let pad = usize::from(setup.bitmap_format_scanline_pad).max(8);
        let lsb = setup.bitmap_format_bit_order == ImageOrder::LSB_FIRST;
        let bits = mask_bits(&solid, n as usize, pad, lsb);
        if let (Ok(pm), Ok(gc)) = (p.conn.generate_id(), p.conn.generate_id()) {
            if p.conn.create_pixmap(1, pm, t.win, n as u16, n as u16).is_ok() && p.conn.create_gc(gc, pm, &CreateGCAux::new().foreground(1).background(0)).is_ok() {
                let _ = p.conn.put_image(ImageFormat::XY_BITMAP, pm, gc, n as u16, n as u16, 0, 0, 0, 1, &bits);
                let _ = shape::mask(&p.conn, shape::SO::SET, shape::SK::BOUNDING, t.win, 0, 0, pm);
                let _ = p.conn.free_gc(gc);
                let _ = p.conn.free_pixmap(pm);
                let _ = p.conn.flush();
            }
        }
    }

    // ------------------------------------------------------------------ first-run notice

    pub fn show_notice(&mut self, p: &LinuxPlatform) {
        if self.notice.is_some() {
            return;
        }
        let scale = p.scale();
        let (w, h) = (460.0f32, 300.0f32);
        let (wpx, hpx) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let (sw, sh) = p.screen_size();
        let (x, y) = (((sw - wpx as i32) / 2).max(0), ((sh - hpx as i32) / 3).max(0));
        let (Ok(win), Ok(gc)) = (p.conn.generate_id(), p.conn.generate_id()) else { return };
        let ok = p
            .conn
            .create_window(
                p.depth(),
                win,
                p.root,
                x as i16,
                y as i16,
                wpx as u16,
                hpx as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().background_pixel(0x0025252b).event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::POINTER_MOTION
                        | EventMask::KEY_PRESS
                        | EventMask::STRUCTURE_NOTIFY,
                ),
            )
            .is_ok()
            && p.conn.create_gc(gc, win, &CreateGCAux::new()).is_ok();
        if !ok {
            return;
        }
        let a = &self.atoms;
        let title = crate::install::NOTICE_TITLE.as_bytes();
        let _ = p.conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, title);
        let _ = p.conn.change_property8(PropMode::REPLACE, win, a.net_wm_name, a.utf8, title);
        let _ = p.conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_CLASS, AtomEnum::STRING, b"pasteroll\0Pasteroll\0");
        let _ = p.conn.change_property32(PropMode::REPLACE, win, a.wm_protocols, AtomEnum::ATOM, &[a.wm_delete]);
        let _ = p.conn.change_property32(PropMode::REPLACE, win, a.net_wm_icon, AtomEnum::CARDINAL, &wm_icon_data(&[32, 64]));
        let mut hints = WmSizeHints::new();
        hints.min_size = Some((wpx as i32, hpx as i32));
        hints.max_size = Some((wpx as i32, hpx as i32));
        let _ = hints.set(&p.conn, win, AtomEnum::WM_NORMAL_HINTS);
        self.notice = Some(NoticeWin { win, gc, scale, hover: false });
        let _ = p.conn.map_window(win);
        if !p.has_wm() {
            let _ = p.conn.set_input_focus(InputFocus::PARENT, win, CURRENT_TIME);
        }
        let _ = p.conn.flush();
        self.paint_notice(p);
    }

    fn notice_view(&self) -> Option<UiView> {
        let n = self.notice.as_ref()?;
        let widgets = vec![
            Widget::new(0, rect(20.0, 16.0, 420.0, 26.0), crate::install::NOTICE_TITLE, WKind::Heading),
            Widget::new(0, rect(20.0, 52.0, 420.0, 190.0), crate::install::NOTICE, WKind::Note),
            Widget::new(S_NOTICE_OK, rect(350.0, 254.0, 90.0, 32.0), "OK", WKind::Button { primary: true }),
        ];
        Some(UiView { size: (460.0, 300.0), chrome: Chrome::Window, widgets, hover: n.hover.then_some(S_NOTICE_OK) })
    }

    fn paint_notice(&mut self, p: &LinuxPlatform) {
        let Some(view) = self.notice_view() else { return };
        let (win, gc, scale) = {
            let n = self.notice.as_ref().expect("checked");
            (n.win, n.gc, n.scale)
        };
        let f = self.renderer().render_ui(&view, scale);
        put_bgra(&p.conn, win, gc, p.depth(), f.width, f.height, &f.bgra);
    }

    fn close_notice(&mut self, p: &LinuxPlatform) {
        if let Some(n) = self.notice.take() {
            let _ = p.conn.destroy_window(n.win);
            let _ = p.conn.free_gc(n.gc);
            let _ = p.conn.flush();
        }
    }

    // ------------------------------------------------------------------ popup menu

    fn open_menu(&mut self, p: &LinuxPlatform, at: (i32, i32)) {
        if self.menu.is_some() {
            self.close_menu(p);
        }
        let item_h = 28.0f32;
        let w = 208.0f32;
        let pause_text = if self.paused { "Resume recording" } else { "Pause recording" };
        let rows: [(u32, &str, bool); 4] = [
            (ID_PAUSE, pause_text, false),
            (ID_SETTINGS, "Settings...", false),
            (ID_FOLDER, "Open data folder", false),
            (ID_QUIT, "Quit Pasteroll", false),
        ];
        let mut widgets = Vec::new();
        let mut y = 4.0f32;
        for (i, (id, text, _)) in rows.iter().enumerate() {
            if i == 3 {
                widgets.push(Widget::new(0, rect(0.0, y, w, 8.0), "", WKind::Separator));
                y += 8.0;
            }
            widgets.push(Widget::new(*id, rect(0.0, y, w, item_h), *text, WKind::MenuItem { checked: false, enabled: true }));
            y += item_h;
        }
        let h = y + 4.0;
        let scale = p.scale();
        let (wpx, hpx) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let (sw, sh) = p.screen_size();
        let mut x = at.0 - (wpx as i32) / 2;
        let mut yy = at.1 + 8;
        x = x.clamp(0, (sw - wpx as i32).max(0));
        if yy + hpx as i32 > sh {
            yy = (at.1 - hpx as i32 - 8).max(0);
        }
        let (Ok(win), Ok(gc)) = (p.conn.generate_id(), p.conn.generate_id()) else { return };
        let created = p
            .conn
            .create_window(
                p.depth(),
                win,
                p.root,
                x as i16,
                yy as i16,
                wpx as u16,
                hpx as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .background_pixel(0x0025252b)
                    .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION),
            )
            .is_ok()
            && p.conn.create_gc(gc, win, &CreateGCAux::new()).is_ok();
        if !created {
            return;
        }
        let _ = p.conn.map_window(win);
        // a click anywhere else closes it: take the pointer and keyboard while it is up
        let _ = p.conn.grab_pointer(
            true,
            win,
            EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            0u32,
            0u32,
            CURRENT_TIME,
        );
        let _ = p.conn.grab_keyboard(true, win, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC);
        let _ = p.conn.flush();
        let view = UiView { size: (w, h), chrome: Chrome::Menu, widgets, hover: None };
        self.menu = Some(Menu { win, gc, size: (wpx, hpx), view });
        self.paint_menu(p);
    }

    fn paint_menu(&mut self, p: &LinuxPlatform) {
        let scale = p.scale();
        let Some(m) = self.menu.as_ref() else { return };
        let (win, gc, view) = (m.win, m.gc, m.view.clone());
        let f = self.renderer().render_ui(&view, scale);
        put_bgra(&p.conn, win, gc, p.depth(), f.width, f.height, &f.bgra);
    }

    fn close_menu(&mut self, p: &LinuxPlatform) {
        if let Some(m) = self.menu.take() {
            let _ = p.conn.ungrab_pointer(CURRENT_TIME);
            let _ = p.conn.ungrab_keyboard(CURRENT_TIME);
            let _ = p.conn.destroy_window(m.win);
            let _ = p.conn.free_gc(m.gc);
            let _ = p.conn.flush();
        }
    }

    // ------------------------------------------------------------------ settings window

    pub fn open_settings(&mut self, p: &LinuxPlatform) {
        if let Some(s) = &self.settings {
            // already open: raise it
            let _ = p.conn.configure_window(s.win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
            let _ = p.conn.map_window(s.win);
            let _ = p.conn.flush();
            return;
        }
        let cfg = match pasteroll_core::keystore::load_config(&p.db) {
            Ok(c) => c,
            Err(problems) => {
                eprintln!("[pasteroll] settings: config.toml is not valid: {}", problems.join("; "));
                return;
            }
        };
        let scale = p.scale();
        let (wpx, hpx) = ((SET_W * scale).round() as u32, (SET_H * scale).round() as u32);
        let (sw, sh) = p.screen_size();
        let (x, y) = (((sw - wpx as i32) / 2).max(0), ((sh - hpx as i32) / 2).max(0));
        let (Ok(win), Ok(gc)) = (p.conn.generate_id(), p.conn.generate_id()) else { return };
        let ok = p
            .conn
            .create_window(
                p.depth(),
                win,
                p.root,
                x as i16,
                y as i16,
                wpx as u16,
                hpx as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().background_pixel(0x0025252b).event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::BUTTON_MOTION
                        | EventMask::POINTER_MOTION
                        | EventMask::KEY_PRESS
                        | EventMask::STRUCTURE_NOTIFY,
                ),
            )
            .is_ok()
            && p.conn.create_gc(gc, win, &CreateGCAux::new()).is_ok();
        if !ok {
            return;
        }
        let a = &self.atoms;
        let _ = p.conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, b"Pasteroll settings");
        let _ = p.conn.change_property8(PropMode::REPLACE, win, a.net_wm_name, a.utf8, b"Pasteroll settings");
        let _ = p.conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_CLASS, AtomEnum::STRING, b"pasteroll\0Pasteroll\0");
        let _ = p.conn.change_property32(PropMode::REPLACE, win, a.wm_protocols, AtomEnum::ATOM, &[a.wm_delete]);
        let _ = p.conn.change_property32(PropMode::REPLACE, win, a.net_wm_pid, AtomEnum::CARDINAL, &[std::process::id()]);
        let _ = p.conn.change_property32(PropMode::REPLACE, win, a.net_wm_icon, AtomEnum::CARDINAL, &wm_icon_data(&[32, 64]));
        let mut hints = WmSizeHints::new();
        hints.min_size = Some((wpx as i32, hpx as i32));
        hints.max_size = Some((wpx as i32, hpx as i32));
        let _ = hints.set(&p.conn, win, AtomEnum::WM_NORMAL_HINTS);

        let steps = Steps::for_limits(&cfg.limits);
        let pos = steps.positions(&cfg.limits);
        let size_choices = settings::size_choices(cfg.size_percent);
        let size_pos = size_choices.iter().position(|v| *v == cfg.size_percent).unwrap_or(size_choices.len() / 2);
        let state = SettingsWin {
            win,
            gc,
            scale,
            steps,
            pos,
            size_choices,
            size_pos,
            paused: cfg.paused,
            warn: cfg.warn_multiline_in_terminals,
            autostart: crate::install::autostart_enabled(),
            autostart_was: crate::install::autostart_enabled(),
            modifier: cfg.advanced_image_modifier,
            exclude: cfg.exclude_apps.clone(),
            filters: cfg.custom_filters.iter().map(|f| f.name.clone()).collect(),
            orig: cfg,
            ex_sel: None,
            fl_sel: None,
            ex_top: 0,
            fl_top: 0,
            entry: String::new(),
            entry_focus: false,
            hover: None,
            drag: None,
            status: String::new(),
            confirm: None,
        };
        self.settings = Some(state);
        let _ = p.conn.map_window(win);
        if !p.has_wm() {
            let _ = p.conn.set_input_focus(InputFocus::PARENT, win, CURRENT_TIME);
        }
        let _ = p.conn.flush();
        self.paint_settings(p);
    }

    fn close_settings(&mut self, p: &LinuxPlatform) {
        if let Some(s) = self.settings.take() {
            let _ = p.conn.destroy_window(s.win);
            let _ = p.conn.free_gc(s.gc);
            let _ = p.conn.flush();
        }
    }

    fn settings_view(&self) -> Option<UiView> {
        let s = self.settings.as_ref()?;
        let mut w: Vec<Widget> = Vec::new();
        if let Some((msg, _, _)) = &s.confirm {
            w.push(Widget::new(0, rect(16.0, 16.0, 488.0, 24.0), "Delete clips?", WKind::Heading));
            w.push(Widget::new(0, rect(16.0, 52.0, 488.0, 120.0), msg.clone(), WKind::Note));
            w.push(Widget::new(S_CONFIRM, rect(250.0, 584.0, 164.0, 32.0), "Delete and save", WKind::Button { primary: true }));
            w.push(Widget::new(S_BACK, rect(420.0, 584.0, 84.0, 32.0), "Back", WKind::Button { primary: false }));
            return Some(UiView { size: (SET_W, SET_H), chrome: Chrome::Window, widgets: w, hover: s.hover });
        }
        let (clips, size, age, clip) = s.steps.at(s.pos);
        w.push(Widget::new(0, rect(16.0, 12.0, 488.0, 24.0), "How much history to keep", WKind::Heading));
        let rows: [(&str, String, usize); 4] = [
            ("Clips kept", format!("{} clips", thousands(clips)), 0),
            ("Total size", fmt_mb(size), 1),
            ("Keep for", fmt_age(age), 2),
            ("Largest single clip", fmt_mb(clip), 3),
        ];
        for (i, (label, value, k)) in rows.into_iter().enumerate() {
            w.push(Widget::new(
                S_SLIDER[i],
                rect(16.0, 44.0 + 36.0 * i as f32, 488.0, 30.0),
                label,
                WKind::Slider { value, index: s.pos[k], count: s.steps.counts()[k] },
            ));
        }
        w.push(Widget::new(
            0,
            rect(16.0, 192.0, 488.0, 36.0),
            "When a limit is reached the oldest clips are removed first. Pinned clips are never removed.",
            WKind::Note,
        ));
        w.push(Widget::new(
            S_SIZE,
            rect(16.0, 232.0, 488.0, 30.0),
            "Carousel size",
            WKind::Slider {
                value: settings::fmt_size(s.size_choices[s.size_pos.min(s.size_choices.len() - 1)]),
                index: s.size_pos,
                count: s.size_choices.len(),
            },
        ));
        w.push(Widget::new(S_PAUSE, rect(16.0, 272.0, 400.0, 24.0), "Pause recording", WKind::Check { checked: s.paused }));
        w.push(Widget::new(
            S_WARN,
            rect(16.0, 300.0, 488.0, 24.0),
            "Ask before pasting several lines into a terminal",
            WKind::Check { checked: s.warn },
        ));
        w.push(Widget::new(
            S_AUTOSTART,
            rect(16.0, 328.0, 488.0, 24.0),
            if pasteroll_core::paths::is_portable() { pasteroll_core::paths::PORTABLE_AUTOSTART } else { "Start Pasteroll when I sign in" },
            if pasteroll_core::paths::is_portable() { WKind::Note } else { WKind::Check { checked: s.autostart } },
        ));
        w.push(Widget::new(
            0,
            rect(16.0, 364.0, 488.0, 40.0),
            "To edit an image before it is saved to the desktop, hold this key while you click the desktop:",
            WKind::Note,
        ));
        for (i, m) in config::ClickModifier::ALL.iter().enumerate() {
            w.push(Widget::new(
                S_MOD[i],
                rect(16.0 + 110.0 * i as f32, 408.0, 100.0, 24.0),
                m.label(),
                WKind::Check { checked: s.modifier == *m },
            ));
        }
        w.push(Widget::new(0, rect(16.0, 440.0, 488.0, 22.0), "Never record copies made in these programs", WKind::Label));
        w.push(Widget::new(
            S_EX_LIST,
            rect(16.0, 466.0, 300.0, 76.0),
            "No programs listed",
            WKind::List { items: s.exclude.clone(), selected: s.ex_sel, top: s.ex_top },
        ));
        w.push(Widget::new(
            S_EX_ENTRY,
            rect(330.0, 466.0, 174.0, 28.0),
            s.entry.clone(),
            WKind::Entry { placeholder: "program name".into(), focused: s.entry_focus },
        ));
        w.push(Widget::new(S_EX_ADD, rect(330.0, 500.0, 84.0, 28.0), "Add", WKind::Button { primary: false }));
        w.push(Widget::new(S_EX_REMOVE, rect(420.0, 500.0, 84.0, 28.0), "Remove", WKind::Button { primary: false }));
        w.push(Widget::new(0, rect(330.0, 534.0, 174.0, 30.0), "For example keepassxc", WKind::Note));
        w.push(Widget::new(
            0,
            rect(16.0, 572.0, 488.0, 22.0),
            "Custom filters (add them from the filter menu in the overlay)",
            WKind::Label,
        ));
        w.push(Widget::new(
            S_FL_LIST,
            rect(16.0, 598.0, 300.0, 76.0),
            "No custom filters",
            WKind::List { items: s.filters.clone(), selected: s.fl_sel, top: s.fl_top },
        ));
        w.push(Widget::new(S_FL_REMOVE, rect(330.0, 598.0, 84.0, 28.0), "Remove", WKind::Button { primary: false }));
        if !s.status.is_empty() {
            w.push(Widget::new(0, rect(16.0, 672.0, 300.0, 44.0), s.status.clone(), WKind::Note));
        }
        w.push(Widget::new(S_SAVE, rect(330.0, 680.0, 84.0, 32.0), "Save", WKind::Button { primary: true }));
        w.push(Widget::new(S_CANCEL, rect(420.0, 680.0, 84.0, 32.0), "Cancel", WKind::Button { primary: false }));
        Some(UiView { size: (SET_W, SET_H), chrome: Chrome::Window, widgets: w, hover: s.hover })
    }

    fn paint_settings(&mut self, p: &LinuxPlatform) {
        let Some(view) = self.settings_view() else { return };
        let (win, gc, scale) = {
            let s = self.settings.as_ref().expect("checked");
            (s.win, s.gc, s.scale)
        };
        let f = self.renderer().render_ui(&view, scale);
        put_bgra(&p.conn, win, gc, p.depth(), f.width, f.height, &f.bgra);
    }

    fn settings_choice(&self) -> Option<Choice> {
        let s = self.settings.as_ref()?;
        let (clips, size_mb, age_days, clip_mb) = s.steps.at(s.pos);
        Some(Choice {
            clips,
            size_mb,
            age_days,
            clip_mb,
            paused: s.paused,
            warn_multiline: s.warn,
            size_percent: s.size_choices[s.size_pos.min(s.size_choices.len() - 1)],
            advanced_modifier: s.modifier,
            // the mode has no control in this window yet: Save keeps whatever the file says
            mode: s.orig.mode,
            exclude_apps: s.exclude.clone(),
            filter_names: s.filters.clone(),
        })
    }

    /// The window pressed Save: work out the new file, or say why not.
    fn request_save(&mut self, p: &LinuxPlatform, out: &mut Vec<ShellAction>) {
        // "start at sign-in" is a desktop-entry file, not part of config.toml: apply it at once
        let autostart = self.settings.as_ref().map(|s| (s.autostart, s.autostart_was));
        if let Some((want, was)) = autostart {
            if want != was {
                if let Err(e) = crate::install::set_autostart(want) {
                    self.set_status(e);
                    return;
                }
                if let Some(s) = self.settings.as_mut() {
                    s.autostart_was = want;
                }
            }
        }
        let Some(choice) = self.settings_choice() else { return };
        let text = match pasteroll_core::keystore::read_config_text(&p.db) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(e);
                return;
            }
        };
        let orig = self.settings.as_ref().expect("open").orig.clone();
        let new_text = match settings::build_text(&orig, &choice, &text) {
            Ok(t) => t,
            Err(problems) => {
                self.set_status(problems.join(" "));
                return;
            }
        };
        if new_text == text {
            self.close_settings(p);
            return;
        }
        match config::parse(&new_text) {
            Ok(cfg) => out.push(ShellAction::Save { text: new_text, limits: cfg.limits, confirmed: false }),
            Err(problems) => self.set_status(problems.join(" ")),
        }
    }

    fn set_status(&mut self, msg: String) {
        if let Some(s) = self.settings.as_mut() {
            s.status = msg;
        }
    }

    /// The loop found that saving would delete clips: ask first.
    pub fn ask_confirm(&mut self, p: &LinuxPlatform, message: String, text: String, limits: Limits) {
        if let Some(s) = self.settings.as_mut() {
            s.confirm = Some((message, text, limits));
        }
        self.paint_settings(p);
    }

    pub fn settings_failed(&mut self, p: &LinuxPlatform, msg: String) {
        self.set_status(msg);
        self.paint_settings(p);
    }

    pub fn settings_saved(&mut self, p: &LinuxPlatform) {
        self.close_settings(p);
    }

    // ------------------------------------------------------------------ events

    /// Does this event belong to one of our windows?
    pub fn owns(&self, ev: &XEvent) -> bool {
        let tray = self.tray.as_ref().map(|t| t.win);
        let menu = self.menu.as_ref().map(|m| m.win);
        let set = self.settings.as_ref().map(|s| s.win);
        let note = self.notice.as_ref().map(|n| n.win);
        let is = |w: Window| Some(w) == tray || Some(w) == menu || Some(w) == set || Some(w) == note;
        match ev {
            XEvent::ButtonPress(e) => is(e.event),
            XEvent::ButtonRelease(e) => is(e.event),
            XEvent::MotionNotify(e) => is(e.event),
            XEvent::KeyPress(e) => is(e.event),
            XEvent::Expose(e) => is(e.window),
            XEvent::ConfigureNotify(e) => is(e.window),
            XEvent::DestroyNotify(e) => is(e.window),
            XEvent::ClientMessage(m) => is(m.window) || m.type_ == self.atoms.manager,
            _ => false,
        }
    }

    pub fn handle(&mut self, p: &LinuxPlatform, ev: XEvent) -> Vec<ShellAction> {
        let mut out = Vec::new();
        match ev {
            XEvent::ClientMessage(m) => self.client_message(p, &m, &mut out),
            XEvent::Expose(e) => {
                if e.count == 0 {
                    if Some(e.window) == self.tray.as_ref().map(|t| t.win) {
                        self.paint_tray(p);
                    } else if Some(e.window) == self.menu.as_ref().map(|m| m.win) {
                        self.paint_menu(p);
                    } else if Some(e.window) == self.notice.as_ref().map(|n| n.win) {
                        self.paint_notice(p);
                    } else {
                        self.paint_settings(p);
                    }
                }
            }
            XEvent::ConfigureNotify(e) => {
                if let Some(t) = self.tray.as_mut().filter(|t| t.win == e.window) {
                    t.size = u32::from(e.width.min(e.height)).max(16);
                    self.paint_tray(p);
                }
            }
            XEvent::DestroyNotify(e) => {
                if self.tray.as_ref().is_some_and(|t| t.win == e.window) {
                    self.tray = None; // the tray went away; MANAGER will tell us when one is back
                }
            }
            XEvent::ButtonPress(e) => self.button(p, &e, &mut out),
            XEvent::ButtonRelease(e) => {
                if let Some(s) = self.settings.as_mut().filter(|s| s.win == e.event) {
                    s.drag = None;
                }
            }
            XEvent::MotionNotify(e) => self.motion(p, &e),
            XEvent::KeyPress(e) => self.key(p, &e, &mut out),
            _ => {}
        }
        out
    }

    fn client_message(&mut self, p: &LinuxPlatform, m: &ClientMessageEvent, _out: &mut Vec<ShellAction>) {
        if m.type_ == self.atoms.manager {
            // a tray manager appeared (data[1] is the selection): dock again
            let d = m.data.as_data32();
            if d[1] == self.atoms.tray_selection {
                if let Some(t) = self.tray.as_mut() {
                    t.docked = false;
                }
                self.dock(p);
            }
            return;
        }
        if m.type_ == self.atoms.wm_protocols && m.data.as_data32()[0] == self.atoms.wm_delete {
            if self.settings.as_ref().is_some_and(|s| s.win == m.window) {
                self.close_settings(p);
            }
            if self.notice.as_ref().is_some_and(|n| n.win == m.window) {
                self.close_notice(p);
            }
        }
    }

    fn button(&mut self, p: &LinuxPlatform, e: &ButtonPressEvent, out: &mut Vec<ShellAction>) {
        let tray = self.tray.as_ref().map(|t| t.win);
        if Some(e.event) == tray {
            match e.detail {
                1 => self.open_settings(p),
                3 => self.open_menu(p, (i32::from(e.root_x), i32::from(e.root_y))),
                _ => {}
            }
            return;
        }
        if self.menu.as_ref().is_some_and(|m| m.win == e.event) {
            self.menu_click(p, e, out);
            return;
        }
        if self.settings.as_ref().is_some_and(|s| s.win == e.event) {
            self.settings_click(p, e, out);
        }
        if self.notice.as_ref().is_some_and(|n| n.win == e.event) && e.detail == 1 {
            let s = p.scale();
            let (x, y) = (f32::from(e.event_x) / s, f32::from(e.event_y) / s);
            if self.notice_view().and_then(|v| hit(&v, x, y).map(|w| w.id)) == Some(S_NOTICE_OK) {
                self.close_notice(p);
            }
        }
    }

    fn menu_click(&mut self, p: &LinuxPlatform, e: &ButtonPressEvent, out: &mut Vec<ShellAction>) {
        let Some(m) = self.menu.as_ref() else { return };
        let scale = p.scale();
        let (x, y) = (f32::from(e.event_x) / scale, f32::from(e.event_y) / scale);
        let inside = e.event_x >= 0
            && e.event_y >= 0
            && (e.event_x as u32) < m.size.0
            && (e.event_y as u32) < m.size.1;
        let id = if inside && e.detail == 1 { hit(&m.view, x, y).map(|w| w.id) } else { None };
        self.close_menu(p);
        match id {
            Some(ID_PAUSE) => out.push(ShellAction::TogglePause),
            Some(ID_SETTINGS) => self.open_settings(p),
            Some(ID_FOLDER) => out.push(ShellAction::OpenFolder),
            Some(ID_QUIT) => out.push(ShellAction::Quit),
            _ => {}
        }
    }

    fn motion(&mut self, p: &LinuxPlatform, e: &MotionNotifyEvent) {
        let scale = p.scale();
        let (x, y) = (f32::from(e.event_x) / scale, f32::from(e.event_y) / scale);
        if self.notice.as_ref().is_some_and(|n| n.win == e.event) {
            let over = self.notice_view().and_then(|v| hit(&v, x, y).map(|w| w.id)) == Some(S_NOTICE_OK);
            if let Some(n) = self.notice.as_mut() {
                if n.hover != over {
                    n.hover = over;
                    self.paint_notice(p);
                }
            }
            return;
        }
        if let Some(m) = self.menu.as_mut().filter(|m| m.win == e.event) {
            let h = hit(&m.view, x, y).map(|w| w.id);
            if h != m.view.hover {
                m.view.hover = h;
                self.paint_menu(p);
            }
            return;
        }
        let Some(view) = self.settings_view() else { return };
        let Some(s) = self.settings.as_mut().filter(|s| s.win == e.event) else { return };
        if let Some(id) = s.drag {
            if let Some(w) = view.widgets.iter().find(|w| w.id == id) {
                if let Some(i) = S_SLIDER.iter().position(|v| *v == id) {
                    let idx = slider_index_at(w, x);
                    if s.pos[i] != idx {
                        s.pos[i] = idx;
                        self.paint_settings(p);
                    }
                } else if id == S_SIZE {
                    let idx = slider_index_at(w, x);
                    if s.size_pos != idx {
                        s.size_pos = idx;
                        self.paint_settings(p);
                    }
                }
            }
            return;
        }
        let h = hit(&view, x, y).map(|w| w.id);
        if h != s.hover {
            s.hover = h;
            self.paint_settings(p);
        }
    }

    fn settings_click(&mut self, p: &LinuxPlatform, e: &ButtonPressEvent, out: &mut Vec<ShellAction>) {
        let scale = p.scale();
        let (x, y) = (f32::from(e.event_x) / scale, f32::from(e.event_y) / scale);
        let Some(view) = self.settings_view() else { return };
        let target = hit(&view, x, y).cloned();
        let mut action: Option<u32> = None;
        {
            let Some(s) = self.settings.as_mut() else { return };
            s.status.clear();
            // the wheel scrolls the list under the pointer
            if e.detail == 4 || e.detail == 5 {
                if let Some(w) = target.as_ref() {
                    let down = e.detail == 5;
                    if let WKind::List { items, .. } = &w.kind {
                        let max_top = items.len().saturating_sub(list_visible_rows(w));
                        let top = if w.id == S_EX_LIST { &mut s.ex_top } else { &mut s.fl_top };
                        *top = if down { (*top + 1).min(max_top) } else { top.saturating_sub(1) };
                    }
                }
                self.paint_settings(p);
                return;
            }
            if e.detail != 1 {
                return;
            }
            s.entry_focus = false;
            match target {
                Some(w) => match &w.kind {
                    WKind::Slider { .. } => {
                        if let Some(i) = S_SLIDER.iter().position(|v| *v == w.id) {
                            s.pos[i] = slider_index_at(&w, x);
                            s.drag = Some(w.id);
                        } else if w.id == S_SIZE {
                            s.size_pos = slider_index_at(&w, x);
                            s.drag = Some(w.id);
                        }
                    }
                    WKind::Check { .. } => {
                        if w.id == S_PAUSE {
                            s.paused = !s.paused;
                        } else if w.id == S_WARN {
                            s.warn = !s.warn;
                        } else if w.id == S_AUTOSTART {
                            s.autostart = !s.autostart;
                        } else if let Some(i) = S_MOD.iter().position(|v| *v == w.id) {
                            // one of the three keys, like radio buttons
                            s.modifier = config::ClickModifier::ALL[i];
                        }
                    }
                    WKind::List { .. } => {
                        let row = list_row_at(&w, y);
                        if w.id == S_EX_LIST {
                            s.ex_sel = row;
                        } else {
                            s.fl_sel = row;
                        }
                    }
                    WKind::Entry { .. } => s.entry_focus = true,
                    WKind::Button { .. } => action = Some(w.id),
                    _ => {}
                },
                None => {}
            }
        }
        match action {
            Some(S_SAVE) => self.request_save(p, out),
            Some(S_CANCEL) => {
                self.close_settings(p);
                return;
            }
            Some(S_EX_ADD) => self.add_exclusion(),
            Some(S_EX_REMOVE) => {
                if let Some(s) = self.settings.as_mut() {
                    if let Some(i) = s.ex_sel.take().filter(|i| *i < s.exclude.len()) {
                        s.exclude.remove(i);
                        s.ex_top = s.ex_top.min(s.exclude.len().saturating_sub(1));
                    }
                }
            }
            Some(S_FL_REMOVE) => {
                if let Some(s) = self.settings.as_mut() {
                    if let Some(i) = s.fl_sel.take().filter(|i| *i < s.filters.len()) {
                        s.filters.remove(i);
                        s.fl_top = s.fl_top.min(s.filters.len().saturating_sub(1));
                    }
                }
            }
            Some(S_CONFIRM) => {
                if let Some((_, text, limits)) = self.settings.as_mut().and_then(|s| s.confirm.take()) {
                    out.push(ShellAction::Save { text, limits, confirmed: true });
                }
            }
            Some(S_BACK) => {
                if let Some(s) = self.settings.as_mut() {
                    s.confirm = None;
                }
            }
            _ => {}
        }
        self.paint_settings(p);
    }

    fn add_exclusion(&mut self) {
        let Some(s) = self.settings.as_mut() else { return };
        let name = s.entry.trim().to_ascii_lowercase();
        if name.is_empty() || name.contains(['/', '\\']) || name.chars().any(char::is_control) {
            s.status = "Type a program's name, such as keepassxc".to_owned();
            return;
        }
        if !s.exclude.contains(&name) {
            s.exclude.push(name);
        }
        s.entry.clear();
    }

    fn key(&mut self, p: &LinuxPlatform, e: &KeyPressEvent, out: &mut Vec<ShellAction>) {
        // the popup menu holds the keyboard grab while it is open: Escape closes it
        if self.menu.as_ref().is_some_and(|m| m.win == e.event) {
            if matches!(p.classify_key(e.detail, u16::from(e.state)), Some(Key::Escape)) {
                self.close_menu(p);
            }
            return;
        }
        let Some(k) = p.classify_key(e.detail, u16::from(e.state)) else { return };
        if self.notice.as_ref().is_some_and(|n| n.win == e.event) {
            if matches!(k, Key::Escape | Key::Enter) {
                self.close_notice(p);
            }
            return;
        }
        let (confirming, focus) = match self.settings.as_ref() {
            Some(s) => (s.confirm.is_some(), s.entry_focus),
            None => return,
        };
        match k {
            Key::Escape => {
                if confirming {
                    if let Some(s) = self.settings.as_mut() {
                        s.confirm = None;
                    }
                    self.paint_settings(p);
                } else {
                    self.close_settings(p);
                }
            }
            Key::Enter if focus && !confirming => {
                self.add_exclusion();
                self.paint_settings(p);
            }
            Key::Enter if !focus && !confirming => self.request_save(p, out),
            Key::Tab if !confirming => {
                if let Some(s) = self.settings.as_mut() {
                    s.entry_focus = !s.entry_focus;
                }
                self.paint_settings(p);
            }
            Key::Char(c) if focus => {
                if let Some(s) = self.settings.as_mut() {
                    if s.entry.chars().count() < 48 {
                        s.entry.push(c);
                    }
                }
                self.paint_settings(p);
            }
            Key::Backspace if focus => {
                if let Some(s) = self.settings.as_mut() {
                    s.entry.pop();
                }
                self.paint_settings(p);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transparent_icon_shape_matches_the_approved_artwork() {
        for n in [16u32, 24, 48] {
            let (bgra, solid) = icon_pixels(n, false);
            assert_eq!(bgra.len(), (n * n * 4) as usize);
            assert_eq!(solid.len(), (n * n) as usize);
            assert!(bgra.chunks_exact(4).all(|p| p[3] == 255));
            let rgba = pasteroll_core::logo::rgba(n, false);
            assert_eq!(solid, rgba.chunks_exact(4).map(|p| p[3] >= 100).collect::<Vec<_>>());
            assert!(solid.iter().any(|s| *s), "the roll must remain visible");
            assert!(solid.iter().any(|s| !*s), "the transparent background must not become a square");
        }
    }

    #[test]
    fn a_paused_icon_looks_different() {
        assert_ne!(icon_pixels(24, false).0, icon_pixels(24, true).0);
    }

    #[test]
    fn mask_rows_are_padded_and_bit_ordered() {
        // a 10 px wide mask with only the first and last pixel set, padded to 32-bit rows
        let mut solid = vec![false; 100];
        solid[0] = true;
        solid[9] = true;
        let lsb = mask_bits(&solid, 10, 32, true);
        assert_eq!(lsb[0], 0b0000_0001);
        assert_eq!(lsb[1], 0b0000_0010);
        let msb = mask_bits(&solid, 10, 32, false);
        assert_eq!(msb[0], 0b1000_0000);
        assert_eq!(msb[1], 0b0100_0000);
    }

    #[test]
    fn the_window_icon_data_has_a_header_per_size() {
        let d = wm_icon_data(&[32, 64]);
        assert_eq!(d[0], 32);
        assert_eq!(d[1], 32);
        assert_eq!(d.len(), 2 + 32 * 32 + 2 + 64 * 64);
        assert_eq!(d[2 + 32 * 32], 64);
    }
}
