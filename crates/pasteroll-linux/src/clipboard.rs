//! The X11 clipboard: serving what we paste (we own the CLIPBOARD selection and answer requests
//! from the program that receives the paste) and capturing what other programs copy.
//!
//! Large transfers use the INCR protocol in both directions.

use crate::x::{self, Atoms, XResult};
use pasteroll_app::{Captured, ClipboardSource, Payload, Skip};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;
use zeroize::Zeroize;

/// Chunk size for INCR transfers.
const INCR_CHUNK: usize = 128 * 1024;
/// Anything bigger than this is sent with INCR.
const INCR_THRESHOLD: usize = 256 * 1024;

// ------------------------------------------------------------------ text helpers (pure)

pub fn percent_encode_path(p: &str) -> String {
    let mut out = String::from("file://");
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = h {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Paths from a `text/uri-list` (or the gnome variant, whose first line is the action).
pub fn parse_uri_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        .map(|rest| {
            // file://host/path or file:///path
            let path = rest.find('/').map_or(rest, |i| &rest[i..]);
            String::from_utf8_lossy(&percent_decode(path)).into_owned()
        })
        .collect()
}

fn latin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).collect()
}

// ------------------------------------------------------------------ serving

struct Incr {
    requestor: Window,
    property: Atom,
    target: Atom,
    data: Vec<u8>,
    offset: usize,
    finished: bool,
}

/// What we currently offer on the CLIPBOARD selection.
pub struct Server {
    pub window: Window,
    served: Vec<(Atom, Vec<u8>)>,
    incrs: Vec<Incr>,
    owned: bool,
    /// Also owning PRIMARY (for programs that paste it on Shift+Insert).
    owned_primary: bool,
}

impl Server {
    pub fn new(window: Window) -> Server {
        Server { window, served: Vec::new(), incrs: Vec::new(), owned: false, owned_primary: false }
    }

    fn wipe(&mut self) {
        for (_, d) in &mut self.served {
            d.zeroize();
        }
        self.served.clear();
        for i in &mut self.incrs {
            i.data.zeroize();
        }
        self.incrs.clear();
    }

    /// Offers `payload` and takes ownership of the selection.
    pub fn set(&mut self, conn: &RustConnection, atoms: &Atoms, payload: &Payload) -> Result<(), String> {
        self.wipe();
        match payload {
            Payload::Text(t) => {
                let utf8 = t.clone().into_bytes();
                self.served.push((atoms.utf8_string, utf8.clone()));
                self.served.push((atoms.text_plain_utf8, utf8.clone()));
                self.served.push((atoms.text_plain, utf8));
                self.served.push((u32::from(AtomEnum::STRING), latin1(t)));
            }
            Payload::Files(files) => {
                let uris: Vec<String> = files.iter().map(|f| percent_encode_path(f)).collect();
                let list = uris.iter().map(|u| format!("{u}\r\n")).collect::<String>();
                let gnome = format!("copy\n{}", uris.join("\n"));
                self.served.push((atoms.uri_list, list.into_bytes()));
                self.served.push((atoms.gnome_files, gnome.into_bytes()));
                self.served.push((atoms.utf8_string, files.join("\n").into_bytes()));
            }
            Payload::Image(png) => self.served.push((atoms.image_png, png.clone())),
        }
        conn.set_selection_owner(self.window, atoms.clipboard, CURRENT_TIME).map_err(|e| e.to_string())?;
        let owner = conn
            .get_selection_owner(atoms.clipboard)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .owner;
        self.owned = owner == self.window;
        if self.owned {
            Ok(())
        } else {
            Err("could not take the clipboard".to_string())
        }
    }

    /// Also offers the same content as the PRIMARY selection.
    pub fn own_primary(&mut self, conn: &RustConnection) -> bool {
        if !self.owned {
            return false;
        }
        let primary = u32::from(AtomEnum::PRIMARY);
        let _ = conn.set_selection_owner(self.window, primary, CURRENT_TIME);
        self.owned_primary = conn
            .get_selection_owner(primary)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.owner == self.window);
        self.owned_primary
    }

    pub fn cleared(&mut self, selection: Atom, clipboard: Atom) {
        if selection == clipboard {
            self.owned = false;
        } else if selection == u32::from(AtomEnum::PRIMARY) {
            self.owned_primary = false;
        }
        if !self.owned && !self.owned_primary {
            self.wipe();
        }
    }

    fn notify(&self, conn: &RustConnection, req: &SelectionRequestEvent, property: Atom) {
        let ev = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property,
        };
        let _ = conn.send_event(false, req.requestor, EventMask::NO_EVENT, ev);
        let _ = conn.flush();
    }

    pub fn on_request(&mut self, conn: &RustConnection, atoms: &Atoms, req: &SelectionRequestEvent) {
        let property = if req.property == 0 { req.target } else { req.property };
        let primary = u32::from(AtomEnum::PRIMARY);
        let allowed = (req.selection == atoms.clipboard && self.owned)
            || (req.selection == primary && self.owned_primary);
        if !allowed {
            return self.notify(conn, req, 0);
        }
        if req.target == atoms.targets {
            let mut list: Vec<u32> = vec![atoms.targets, atoms.timestamp];
            list.extend(self.served.iter().map(|(a, _)| *a));
            let _ = conn.change_property32(PropMode::REPLACE, req.requestor, property, AtomEnum::ATOM, &list);
            return self.notify(conn, req, property);
        }
        if req.target == atoms.timestamp {
            let _ = conn.change_property32(PropMode::REPLACE, req.requestor, property, AtomEnum::INTEGER, &[req.time]);
            return self.notify(conn, req, property);
        }
        let Some((_, data)) = self.served.iter().find(|(a, _)| *a == req.target) else {
            return self.notify(conn, req, 0);
        };
        if data.len() > INCR_THRESHOLD {
            // announce an incremental transfer; chunks follow as the requestor deletes the property
            let _ = conn.change_window_attributes(
                req.requestor,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            );
            let _ = conn.change_property32(
                PropMode::REPLACE,
                req.requestor,
                property,
                atoms.incr,
                &[data.len().min(u32::MAX as usize) as u32],
            );
            self.incrs.push(Incr {
                requestor: req.requestor,
                property,
                target: req.target,
                data: data.clone(),
                offset: 0,
                finished: false,
            });
            return self.notify(conn, req, property);
        }
        let _ = conn.change_property8(PropMode::REPLACE, req.requestor, property, req.target, data);
        self.notify(conn, req, property);
    }

    /// The requestor deleted the property: send the next chunk of an INCR transfer. The transfer
    /// ends with a zero-length chunk, and is forgotten when that one has been consumed too.
    pub fn on_property_deleted(&mut self, conn: &RustConnection, win: Window, prop: Atom) {
        let Some(i) = self.incrs.iter().position(|t| t.requestor == win && t.property == prop) else { return };
        if self.incrs[i].finished {
            let mut done = self.incrs.remove(i);
            done.data.zeroize();
            return;
        }
        let t = &mut self.incrs[i];
        if t.offset >= t.data.len() {
            let _ = conn.change_property8(PropMode::REPLACE, t.requestor, t.property, t.target, &[]);
            t.finished = true;
        } else {
            let end = (t.offset + INCR_CHUNK).min(t.data.len());
            let _ = conn.change_property8(PropMode::REPLACE, t.requestor, t.property, t.target, &t.data[t.offset..end]);
            t.offset = end;
        }
        let _ = conn.flush();
    }
}

// ------------------------------------------------------------------ capture

pub struct Capture {
    conn: RustConnection,
    atoms: Atoms,
    win: Window,
    root: Window,
    /// Bumped by the main loop whenever the selection owner changes.
    pub counter: Arc<AtomicU64>,
    /// The helper window that serves our own pastes (so we do not record them).
    own_window: Arc<AtomicU32>,
    max_bytes: usize,
}

impl Capture {
    pub fn new(counter: Arc<AtomicU64>, own_window: Arc<AtomicU32>, max_bytes: usize) -> XResult<Capture> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        let root = conn.setup().roots[screen_num].root;
        let atoms = Atoms::new(&conn)?;
        let win = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
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
        conn.flush()?;
        Ok(Capture { conn, atoms, win, root, counter, own_window, max_bytes })
    }

    fn wait_for<F: FnMut(&Event) -> bool>(&self, timeout: Duration, mut want: F) -> Option<Event> {
        let end = Instant::now() + timeout;
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(ev)) => {
                    if want(&ev) {
                        return Some(ev);
                    }
                }
                Ok(None) => {
                    if Instant::now() >= end {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return None,
            }
        }
    }

    /// Asks the owner to convert the selection to `target` and returns the bytes.
    fn fetch(&self, target: Atom) -> Result<(Atom, Vec<u8>), Skip> {
        let c = &self.conn;
        c.delete_property(self.win, self.atoms.transfer).map_err(|_| Skip::Busy)?;
        c.convert_selection(self.win, self.atoms.clipboard, target, self.atoms.transfer, CURRENT_TIME)
            .map_err(|_| Skip::Busy)?;
        c.flush().map_err(|_| Skip::Busy)?;
        let notify = self
            .wait_for(Duration::from_millis(1500), |e| matches!(e, Event::SelectionNotify(_)))
            .ok_or(Skip::Busy)?;
        let Event::SelectionNotify(n) = notify else { return Err(Skip::Busy) };
        if n.property == 0 {
            return Err(Skip::Unsupported);
        }
        let first = c
            .get_property(false, self.win, self.atoms.transfer, AtomEnum::ANY, 0, 0)
            .map_err(|_| Skip::Busy)?
            .reply()
            .map_err(|_| Skip::Busy)?;
        if first.type_ == self.atoms.incr {
            c.delete_property(self.win, self.atoms.transfer).map_err(|_| Skip::Busy)?;
            c.flush().map_err(|_| Skip::Busy)?;
            let mut data: Vec<u8> = Vec::new();
            let mut kind = target;
            loop {
                let ev = self.wait_for(Duration::from_millis(3000), |e| {
                    matches!(e, Event::PropertyNotify(p) if p.window == self.win
                        && p.atom == self.atoms.transfer && p.state == Property::NEW_VALUE)
                });
                if ev.is_none() {
                    data.zeroize();
                    return Err(Skip::Busy);
                }
                let r = c
                    .get_property(true, self.win, self.atoms.transfer, AtomEnum::ANY, 0, u32::MAX / 4)
                    .map_err(|_| Skip::Busy)?
                    .reply()
                    .map_err(|_| Skip::Busy)?;
                c.flush().map_err(|_| Skip::Busy)?;
                if r.value.is_empty() {
                    break;
                }
                kind = r.type_;
                data.extend_from_slice(&r.value);
                if data.len() > self.max_bytes {
                    data.zeroize();
                    return Err(Skip::TooLarge);
                }
            }
            return Ok((kind, data));
        }
        // single transfer: read it all (length in 32-bit units)
        let r = c
            .get_property(true, self.win, self.atoms.transfer, AtomEnum::ANY, 0, u32::MAX / 4)
            .map_err(|_| Skip::Busy)?
            .reply()
            .map_err(|_| Skip::Busy)?;
        if r.value.len() > self.max_bytes {
            return Err(Skip::TooLarge);
        }
        Ok((r.type_, r.value))
    }

    fn targets(&self) -> Result<Vec<Atom>, Skip> {
        let (_, bytes) = self.fetch(self.atoms.targets)?;
        Ok(bytes.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
}

impl ClipboardSource for Capture {
    fn sequence(&mut self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    fn capture(&mut self) -> Result<Captured, Skip> {
        let before = self.counter.load(Ordering::SeqCst);
        let owner = self
            .conn
            .get_selection_owner(self.atoms.clipboard)
            .map_err(|_| Skip::Busy)?
            .reply()
            .map_err(|_| Skip::Busy)?
            .owner;
        if owner == 0 {
            return Err(Skip::Unsupported);
        }
        if owner == self.own_window.load(Ordering::SeqCst) {
            return Err(Skip::Excluded); // our own paste
        }
        let targets = self.targets()?;
        let a = &self.atoms;
        if targets.contains(&a.password_hint) {
            return Err(Skip::Excluded);
        }
        let result = if targets.contains(&a.uri_list) || targets.contains(&a.gnome_files) {
            let which = if targets.contains(&a.uri_list) { a.uri_list } else { a.gnome_files };
            let (_, bytes) = self.fetch(which)?;
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            let files = parse_uri_list(&text);
            text.zeroize();
            if files.is_empty() {
                return Err(Skip::Unsupported);
            }
            Captured::Files(files)
        } else if targets.contains(&a.image_png) {
            let (_, bytes) = self.fetch(a.image_png)?;
            if bytes.is_empty() {
                return Err(Skip::Unsupported);
            }
            Captured::Image(bytes)
        } else if targets.contains(&a.utf8_string) || targets.contains(&a.text_plain_utf8) {
            let which = if targets.contains(&a.utf8_string) { a.utf8_string } else { a.text_plain_utf8 };
            let (_, bytes) = self.fetch(which)?;
            let t = String::from_utf8_lossy(&bytes).into_owned();
            if t.is_empty() {
                return Err(Skip::Unsupported);
            }
            Captured::Text(t)
        } else if targets.contains(&u32::from(AtomEnum::STRING)) {
            let (_, bytes) = self.fetch(u32::from(AtomEnum::STRING))?;
            let t: String = bytes.iter().map(|b| *b as char).collect();
            if t.is_empty() {
                return Err(Skip::Unsupported);
            }
            Captured::Text(t)
        } else {
            return Err(Skip::Unsupported);
        };
        // the owner changed while we were reading: what we have may be a mix, so try again later
        if self.counter.load(Ordering::SeqCst) != before {
            return Err(Skip::Busy);
        }
        Ok(result)
    }

    fn source_app(&mut self) -> Option<String> {
        let owner = self.conn.get_selection_owner(self.atoms.clipboard).ok()?.reply().ok()?.owner;
        if owner == 0 {
            return None;
        }
        let client = x::client_of(&self.conn, self.root, owner);
        let pid = x::pid_of(&self.conn, &self.atoms, owner).or_else(|| x::pid_of(&self.conn, &self.atoms, client))?;
        x::exe_of_pid(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip_through_uris() {
        for p in ["/home/me/a b.txt", "/tmp/日本語/ü.png", "/x/100%/y#z?.md"] {
            let uri = percent_encode_path(p);
            assert!(uri.starts_with("file:///"));
            assert!(!uri.contains(' '));
            assert_eq!(parse_uri_list(&format!("{uri}\r\n")), vec![p.to_string()]);
        }
    }

    #[test]
    fn a_uri_list_skips_comments_blank_lines_and_other_schemes() {
        let text = "# comment\r\nfile:///a/b\r\n\r\nhttps://example.com/x\r\nfile://localhost/c%20d\r\n";
        assert_eq!(parse_uri_list(text), vec!["/a/b".to_string(), "/c d".to_string()]);
    }

    #[test]
    fn the_gnome_format_has_an_action_line_first() {
        let text = "copy\nfile:///a\nfile:///b";
        assert_eq!(parse_uri_list(text), vec!["/a".to_string(), "/b".to_string()]);
    }

    #[test]
    fn percent_decoding_leaves_bad_escapes_alone() {
        assert_eq!(percent_decode("a%20b"), b"a b");
        assert_eq!(percent_decode("100%"), b"100%");
        assert_eq!(percent_decode("%zz"), b"%zz");
        assert_eq!(percent_decode("%4"), b"%4");
    }

    #[test]
    fn latin1_replaces_what_it_cannot_hold() {
        assert_eq!(latin1("aé€"), vec![b'a', 0xe9, b'?']);
    }
}
