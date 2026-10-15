//! The tray icon for desktops that use StatusNotifierItem instead of the old XEmbed tray (KDE,
//! and GNOME with the AppIndicator extension).
//!
//! The icon, its menu (`com.canonical.dbusmenu`) and the registration with the watcher run on
//! the session bus in their own thread; what the person clicked comes back as [`SniEvent`]s that
//! the main loop polls. Nothing here touches the clipboard or the history.

use pasteroll_core::logo;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::blocking::Connection;
use zbus::zvariant::{Array, Dict, ObjectPath, Signature, StructureBuilder, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const ICON_SIZES: [u32; 5] = [16, 24, 32, 48, 64];

/// What happened to the icon.
#[derive(Debug, PartialEq, Eq)]
pub enum SniEvent {
    /// Left click.
    Activate,
    /// Right click on a host without a menu of its own: show ours at these coordinates.
    ContextMenu(i32, i32),
    Pause,
    Settings,
    Folder,
    Quit,
    /// The watcher left the bus (the extension was turned off): fall back to the XEmbed tray.
    Gone,
    /// The watcher is back and we registered again.
    Back,
}

pub type Pixmap = (i32, i32, Vec<u8>);

/// ARGB32, network byte order, as StatusNotifierItem wants it.
fn pixmaps(paused: bool) -> Vec<Pixmap> {
    ICON_SIZES
        .iter()
        .map(|&n| {
            let mut data = Vec::with_capacity((n * n * 4) as usize);
            for p in logo::rgba(n, paused).chunks_exact(4) {
                data.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
            }
            (n as i32, n as i32, data)
        })
        .collect()
}

struct Shared {
    paused: AtomicBool,
    tx: Mutex<Sender<SniEvent>>,
}

impl Shared {
    fn send(&self, e: SniEvent) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(e);
        }
    }
}

struct Item {
    shared: Arc<Shared>,
    normal: Vec<Pixmap>,
    paused: Vec<Pixmap>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".into()
    }
    #[zbus(property)]
    fn id(&self) -> String {
        "pasteroll".into()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        "Pasteroll".into()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        "Active".into()
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        String::new()
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        if self.shared.paused.load(Ordering::SeqCst) {
            self.paused.clone()
        } else {
            self.normal.clone()
        }
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> String {
        String::new()
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> String {
        String::new()
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
        (String::new(), Vec::new(), "Pasteroll".into(), "Clipboard history".into())
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    fn activate(&self, _x: i32, _y: i32) {
        self.shared.send(SniEvent::Activate);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, x: i32, y: i32) {
        self.shared.send(SniEvent::ContextMenu(x, y));
    }
    fn scroll(&self, _delta: i32, _orientation: String) {}
}

const ID_ROOT: i32 = 0;
const ID_PAUSE: i32 = 1;
const ID_SETTINGS: i32 = 2;
const ID_FOLDER: i32 = 3;
const ID_SEP: i32 = 4;
const ID_QUIT: i32 = 5;

struct Menu {
    shared: Arc<Shared>,
    revision: Mutex<u32>,
}

fn props(items: &[(&str, Value<'static>)]) -> Value<'static> {
    let mut d = Dict::new(&Signature::Str, &Signature::Variant);
    for (k, v) in items {
        let _ = d.append(Value::from(k.to_string()), Value::Value(Box::new(v.clone())));
    }
    Value::Dict(d)
}

fn node(id: i32, items: &[(&str, Value<'static>)], children: Vec<Value<'static>>) -> Value<'static> {
    let mut a = Array::new(&Signature::Variant);
    for c in children {
        let _ = a.append(Value::Value(Box::new(c)));
    }
    Value::Structure(
        StructureBuilder::new()
            .add_field(id)
            .append_field(props(items))
            .append_field(Value::Array(a))
            .build()
            .expect("a structure with fields"),
    )
}

fn label(text: &str) -> Vec<(&'static str, Value<'static>)> {
    vec![("label", Value::from(text.to_string())), ("enabled", Value::from(true)), ("visible", Value::from(true))]
}

fn item_props(id: i32, paused: bool) -> Vec<(&'static str, Value<'static>)> {
    match id {
        ID_PAUSE => label(if paused { "Resume recording" } else { "Pause recording" }),
        ID_SETTINGS => label("Settings..."),
        ID_FOLDER => label("Open data folder"),
        ID_SEP => vec![("type", Value::from("separator".to_string())), ("visible", Value::from(true))],
        ID_QUIT => label("Quit Pasteroll"),
        _ => vec![("children-display", Value::from("submenu".to_string()))],
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".into()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        "normal".into()
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_layout(&self, _parent: i32, _depth: i32, _names: Vec<String>) -> (u32, Value<'static>) {
        let paused = self.shared.paused.load(Ordering::SeqCst);
        let kids = [ID_PAUSE, ID_SETTINGS, ID_FOLDER, ID_SEP, ID_QUIT]
            .iter()
            .map(|&id| node(id, &item_props(id, paused), Vec::new()))
            .collect();
        let rev = self.revision.lock().map(|r| *r).unwrap_or(1);
        (rev, node(ID_ROOT, &item_props(ID_ROOT, paused), kids))
    }

    fn get_group_properties(&self, ids: Vec<i32>, _names: Vec<String>) -> Vec<(i32, HashMap<String, Value<'static>>)> {
        let paused = self.shared.paused.load(Ordering::SeqCst);
        ids.into_iter()
            .map(|id| (id, item_props(id, paused).into_iter().map(|(k, v)| (k.to_string(), v)).collect()))
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> Value<'static> {
        let paused = self.shared.paused.load(Ordering::SeqCst);
        item_props(id, paused)
            .into_iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v)
            .unwrap_or_else(|| Value::from(String::new()))
    }

    fn event(&self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        match id {
            ID_PAUSE => self.shared.send(SniEvent::Pause),
            ID_SETTINGS => self.shared.send(SniEvent::Settings),
            ID_FOLDER => self.shared.send(SniEvent::Folder),
            ID_QUIT => self.shared.send(SniEvent::Quit),
            _ => {}
        }
    }

    fn event_group(&self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        for (id, ev, data, ts) in events {
            self.event(id, ev, data, ts);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }
}

enum Ctl {
    Paused(bool),
}

/// A running StatusNotifierItem. Dropping it closes the connection and removes the icon.
pub struct Sni {
    pub events: Receiver<SniEvent>,
    ctl: Sender<Ctl>,
}

impl Sni {
    /// Registers the icon with the desktop's watcher. `None` when there is no session bus or no
    /// watcher (the caller then uses the XEmbed tray).
    pub fn start(paused: bool) -> Option<Sni> {
        let (tx, events) = mpsc::channel();
        let (ctl, ctl_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let shared = Arc::new(Shared { paused: AtomicBool::new(paused), tx: Mutex::new(tx) });
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("sni".into())
            .spawn(move || {
                let conn = match setup(&sh) {
                    Some(c) => c,
                    None => {
                        let _ = ready_tx.send(false);
                        return;
                    }
                };
                let _ = ready_tx.send(true);
                watch(&conn, &sh, &ctl_rx);
            })
            .ok()?;
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(true) => Some(Sni { events, ctl }),
            _ => None,
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.ctl.send(Ctl::Paused(paused));
    }
}

fn watcher_present(conn: &Connection) -> bool {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &WATCHER,
    )
    .ok()
    .and_then(|m| m.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

fn register(conn: &Connection, name: &str) -> bool {
    conn.call_method(Some(WATCHER), WATCHER_PATH, Some(WATCHER), "RegisterStatusNotifierItem", &name).is_ok()
}

fn bus_name() -> String {
    format!("org.kde.StatusNotifierItem-{}-1", std::process::id())
}

/// Connects, exports the item and the menu, claims the name and registers; `None` on any failure.
fn setup(shared: &Arc<Shared>) -> Option<Connection> {
    let name = bus_name();
    let item = Item {
        shared: shared.clone(),
        normal: pixmaps(false),
        paused: pixmaps(true),
    };
    let menu = Menu { shared: shared.clone(), revision: Mutex::new(1) };
    let conn = zbus::blocking::connection::Builder::session()
        .ok()?
        .name(name.as_str())
        .ok()?
        .serve_at(ITEM_PATH, item)
        .ok()?
        .serve_at(MENU_PATH, menu)
        .ok()?
        .build()
        .ok()?;
    if !watcher_present(&conn) || !register(&conn, &name) {
        return None;
    }
    Some(conn)
}

/// Keeps the icon current and follows the watcher coming and going, until the owner is dropped.
fn watch(conn: &Connection, shared: &Arc<Shared>, ctl: &Receiver<Ctl>) {
    let name = bus_name();
    let mut present = true;
    loop {
        match ctl.recv_timeout(Duration::from_secs(1)) {
            Ok(Ctl::Paused(p)) => {
                shared.paused.store(p, Ordering::SeqCst);
                let _ = conn.emit_signal(None::<&str>, ITEM_PATH, "org.kde.StatusNotifierItem", "NewIcon", &());
                let _ = conn.emit_signal(
                    None::<&str>,
                    MENU_PATH,
                    "com.canonical.dbusmenu",
                    "LayoutUpdated",
                    &(2u32, 0i32),
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = watcher_present(conn);
                if now && !present {
                    if register(conn, &name) {
                        present = true;
                        shared.send(SniEvent::Back);
                    }
                } else if !now && present {
                    present = false;
                    shared.send(SniEvent::Gone);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixmaps_are_argb_in_the_requested_sizes() {
        let p = pixmaps(false);
        assert_eq!(p.len(), ICON_SIZES.len());
        for (w, h, d) in &p {
            assert_eq!(w, h);
            assert_eq!(d.len(), (*w as usize) * (*h as usize) * 4);
        }
        // somewhere the logo is opaque, somewhere (a corner) it is transparent
        let (_, _, d) = &p[4];
        assert!(d.chunks_exact(4).any(|px| px[0] > 128));
        assert!(d.chunks_exact(4).any(|px| px[0] == 0));
        assert_ne!(pixmaps(true)[4].2, p[4].2);
    }

    #[test]
    fn the_menu_has_five_entries_under_the_root() {
        let (tx, _rx) = mpsc::channel();
        let m = Menu { shared: Arc::new(Shared { paused: AtomicBool::new(true), tx: Mutex::new(tx) }), revision: Mutex::new(1) };
        let (rev, root) = m.get_layout(0, -1, Vec::new());
        assert_eq!(rev, 1);
        let Value::Structure(s) = root else { panic!("root is a structure") };
        let f = s.fields();
        assert_eq!(f.len(), 3);
        let Value::Array(kids) = &f[2] else { panic!("children are an array") };
        assert_eq!(kids.len(), 5);
        let g = m.get_group_properties(vec![ID_PAUSE], Vec::new());
        assert_eq!(g[0].1.get("label"), Some(&Value::from("Resume recording".to_string())));
    }

    #[test]
    fn menu_clicks_become_events() {
        let (tx, rx) = mpsc::channel();
        let m = Menu { shared: Arc::new(Shared { paused: AtomicBool::new(false), tx: Mutex::new(tx) }), revision: Mutex::new(1) };
        m.event(ID_QUIT, "clicked".into(), Value::from(0i32), 0);
        m.event(ID_PAUSE, "hovered".into(), Value::from(0i32), 0);
        m.event(ID_SETTINGS, "clicked".into(), Value::from(0i32), 0);
        assert_eq!(rx.try_recv().unwrap(), SniEvent::Quit);
        assert_eq!(rx.try_recv().unwrap(), SniEvent::Settings);
        assert!(rx.try_recv().is_err());
    }
}
