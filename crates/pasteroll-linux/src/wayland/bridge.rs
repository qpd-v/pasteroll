//! Bounded, owner-pinned communication with the companion inside GNOME Shell.

use super::protocol::{self, Capabilities, Error, WireEvent, MAX_BYTES, MAX_EVENTS};
use pasteroll_app::{Captured, ClipboardSource, Payload, Skip};
use pasteroll_render::Frame;
use std::time::Duration;
use zbus::blocking::{connection::Builder, Connection, Proxy};
use zeroize::{Zeroize, Zeroizing};

const NAME: &str = "com.pasteroll.Gnome";
const PATH: &str = "/com/pasteroll/Gnome";
const INTERFACE: &str = "com.pasteroll.Gnome.Bridge2";

type Rect = (i32, i32, u32, u32);

#[derive(Clone)]
pub struct Client {
    connection: Connection,
    owner: String,
    pub capabilities: Capabilities,
    pub epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub sequence: u64,
    pub types: Vec<String>,
    pub source_app: Option<String>,
    pub excluded: bool,
}

/// The foreground window and pointer state in GNOME logical coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub id: u64,
    pub pid: u32,
    pub exe: String,
    pub class: String,
    pub desktop: bool,
    pub pointer: (i32, i32),
    pub modifiers: u32,
    pub monitor: Rect,
    pub work: Rect,
    pub scale: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiState {
    pub paused: bool,
    pub size: u32,
    pub clips: u32,
    pub total_bytes: u64,
    pub age_days: u32,
    pub clip_bytes: u64,
    pub pins: u32,
    pub exclude_apps: Vec<String>,
    pub warn_multiline: bool,
    pub terminal_apps: Vec<String>,
    pub autostart: bool,
    pub first_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsResult {
    pub changed: bool,
    pub confirmed: bool,
    pub paused: bool,
    pub size: u32,
    pub clips: u32,
    pub total_bytes: u64,
    pub age_days: u32,
    pub clip_bytes: u64,
    pub pins: u32,
    pub exclude_apps: Vec<String>,
    pub warn_multiline: bool,
    pub terminal_apps: Vec<String>,
    pub autostart: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickPollResult {
    Pending,
    Rejected,
    Desktop,
    Target {
        id: u64,
        pid: u32,
        exe: String,
        class: String,
        desktop: bool,
    },
}

fn transport(error: zbus::Error) -> Error {
    if let zbus::Error::MethodError(name, _, _) = &error {
        let name = name.as_str();
        if name.ends_with(".Stale") {
            return Error::Stale;
        }
        if name.ends_with(".Bounds") {
            return Error::Bounds;
        }
        if name.ends_with(".AccessDenied") {
            return Error::UntrustedPeer;
        }
        if name.ends_with(".Version") {
            return Error::Version;
        }
        if name.ends_with(".Protocol") {
            return Error::Protocol;
        }
    }
    // A peer may put clipboard bytes in an error message. Never expose it.
    Error::Disconnected
}

fn short_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(|c| c == '\0' || c.is_control())
}

fn optional_text(value: &str, max: usize) -> bool {
    value.is_empty() || short_text(value, max)
}

fn valid_rect((_, _, width, height): Rect) -> bool {
    width > 0 && height > 0 && width <= i32::MAX as u32 && height <= i32::MAX as u32
}

fn valid_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0 && scale <= 64.0
}

fn string_list_ok(values: &[String]) -> bool {
    values.len() <= 128 && values.iter().all(|s| short_text(s, 256))
}

fn fit_frame(frame: &Frame, max_pixels: usize) -> Option<Frame> {
    let pixels = (frame.width as usize).checked_mul(frame.height as usize)?;
    if pixels == 0 || frame.bgra.len() != pixels.saturating_mul(4) {
        return None;
    }
    if pixels <= max_pixels && frame.width <= 4096 && frame.height <= 4096 {
        return None;
    }
    let ratio = (max_pixels as f64 / pixels as f64).sqrt().min(1.0);
    let width = ((frame.width as f64 * ratio).floor() as u32).clamp(1, 4096);
    let height = ((frame.height as f64 * ratio).floor() as u32).clamp(1, 4096);
    let mut bgra = Vec::with_capacity(
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?,
    );
    for y in 0..height {
        let sy = (((y as u64 * 2 + 1) * frame.height as u64) / (height as u64 * 2)) as u32;
        for x in 0..width {
            let sx = (((x as u64 * 2 + 1) * frame.width as u64) / (width as u64 * 2)) as u32;
            let at = ((sy as usize * frame.width as usize) + sx as usize) * 4;
            bgra.extend_from_slice(&frame.bgra[at..at + 4]);
        }
    }
    Some(Frame {
        width,
        height,
        bgra,
    })
}

impl Client {
    pub fn connect() -> Result<Self, Error> {
        let connection = Builder::session()
            .map_err(|_| Error::Missing)?
            .method_timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| Error::Missing)?;
        let bus = Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(|_| Error::Missing)?;
        let owner: String = bus
            .call("GetNameOwner", &(NAME,))
            .map_err(|_| Error::Missing)?;
        let shell: String = bus
            .call("GetNameOwner", &("org.gnome.Shell",))
            .map_err(|_| Error::UntrustedPeer)?;
        let uid: u32 = bus
            .call("GetConnectionUnixUser", &(owner.as_str(),))
            .map_err(|_| Error::UntrustedPeer)?;
        if owner != shell || uid != unsafe { libc::geteuid() } {
            return Err(Error::UntrustedPeer);
        }
        let proxy = Proxy::new(&connection, owner.as_str(), PATH, INTERFACE)
            .map_err(|_| Error::Protocol)?;
        let (version, bits, epoch, max_clip, max_frame): (u32, u32, u64, u32, u32) = proxy
            .call("Handshake", &(protocol::VERSION,))
            .map_err(transport)?;
        let capabilities = Capabilities::negotiate(version, bits)?;
        if epoch == 0
            || max_clip as usize != MAX_BYTES
            || max_frame as usize != protocol::MAX_FRAME_BYTES
        {
            return Err(Error::Protocol);
        }
        drop(proxy);
        drop(bus);
        Ok(Self {
            connection,
            owner,
            capabilities,
            epoch,
        })
    }

    fn proxy(&self) -> Result<Proxy<'_>, Error> {
        Proxy::new(&self.connection, self.owner.as_str(), PATH, INTERFACE)
            .map_err(|_| Error::Disconnected)
    }

    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        let (sequence, types, source, excluded): (u64, Vec<String>, String, bool) =
            self.proxy()?.call("Snapshot", &()).map_err(transport)?;
        if types.len() > 128
            || types.iter().any(|s| !short_text(s, 256))
            || !optional_text(&source, 256)
        {
            return Err(Error::Protocol);
        }
        Ok(Snapshot {
            sequence,
            types,
            source_app: (!source.is_empty()).then_some(source),
            excluded,
        })
    }

    pub fn read(&self, sequence: u64, mime: &str, limit: usize) -> Result<Vec<u8>, Error> {
        if limit == 0 || limit > MAX_BYTES || mime.is_empty() || mime.len() > 256 {
            return Err(Error::Bounds);
        }
        let data: Vec<u8> = self
            .proxy()?
            .call("Read", &(sequence, mime, limit as u32))
            .map_err(transport)?;
        let mut bytes = Zeroizing::new(data);
        if bytes.len() > limit {
            bytes.zeroize();
            return Err(Error::Bounds);
        }
        if self.snapshot()?.sequence != sequence {
            bytes.zeroize();
            return Err(Error::Stale);
        }
        Ok(std::mem::take(&mut *bytes))
    }

    pub fn capture(&self, limit: usize) -> Result<Captured, Skip> {
        let snapshot = self.snapshot().map_err(|_| Skip::Busy)?;
        if snapshot.excluded {
            return Err(Skip::Excluded);
        }
        let mime = protocol::mime_for(&snapshot.types)?;
        let bytes = self
            .read(snapshot.sequence, mime, limit.min(MAX_BYTES))
            .map_err(|e| match e {
                Error::Bounds => Skip::TooLarge,
                _ => Skip::Busy,
            })?;
        protocol::decode(mime, bytes, limit)
    }

    pub fn write(&self, payload: &Payload) -> Result<u64, Error> {
        let (mime, data) = protocol::encode(payload)?;
        let mut bytes = Zeroizing::new(data);
        let result = self
            .proxy()?
            .call("Write", &(mime, bytes.as_slice()))
            .map_err(transport);
        bytes.zeroize();
        result
    }

    pub fn context(&self) -> Result<Context, Error> {
        let (id, pid, exe, class, desktop, x, y, modifiers, monitor, work, scale): (
            u64,
            u32,
            String,
            String,
            bool,
            i32,
            i32,
            u32,
            Rect,
            Rect,
            f64,
        ) = self.proxy()?.call("Context", &()).map_err(transport)?;
        if !optional_text(&exe, 256)
            || !optional_text(&class, 256)
            || !valid_rect(monitor)
            || !valid_rect(work)
            || !valid_scale(scale)
        {
            return Err(Error::Protocol);
        }
        Ok(Context {
            id,
            pid,
            exe,
            class,
            desktop,
            pointer: (x, y),
            modifiers,
            monitor,
            work,
            scale,
        })
    }

    pub fn current(&self, id: u64, pid: u32) -> Result<bool, Error> {
        self.proxy()
            .and_then(|p| p.call("Current", &(id, pid)).map_err(transport))
    }

    pub fn privileged(&self, id: u64, pid: u32) -> Result<bool, Error> {
        self.proxy()
            .and_then(|p| p.call("Privileged", &(id, pid)).map_err(transport))
    }

    /// Only the explicit keyboard opt-in may start an interactive generation.
    pub fn input_active(&self, on: bool) -> Result<u64, Error> {
        let generation: u64 = self
            .proxy()?
            .call("InputActive", &(on,))
            .map_err(transport)?;
        if generation == 0 {
            return Err(Error::Protocol);
        }
        Ok(generation)
    }

    pub fn modes(&self, generation: u64, viewer: bool, modal: bool) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        self.proxy()?
            .call("Modes", &(generation, viewer, modal))
            .map_err(transport)
    }

    pub fn viewer_show(
        &self,
        generation: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        rounded: bool,
    ) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(Error::Bounds);
        }
        self.proxy()?
            .call("ViewerShow", &(generation, x, y, width, height, rounded))
            .map_err(transport)
    }

    /// Large device-pixel frames are sampled down to the negotiated transport bound. The
    /// companion stretches the result to the viewer actor.
    pub fn viewer_present(&self, generation: u64, frame: &Frame) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        let fitted = fit_frame(frame, protocol::MAX_FRAME_BYTES / 4);
        let sent = fitted.as_ref().unwrap_or(frame);
        protocol::frame_len(sent.width, sent.height)?;
        if sent.bgra.len() != protocol::frame_len(sent.width, sent.height)? {
            return Err(Error::Bounds);
        }
        let result = self
            .proxy()?
            .call(
                "ViewerPresent",
                &(generation, sent.width, sent.height, sent.bgra.as_slice()),
            )
            .map_err(transport);
        if let Some(mut fitted) = fitted {
            fitted.bgra.zeroize();
        }
        result
    }

    pub fn viewer_hide(&self, generation: u64) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        self.proxy()?
            .call("ViewerHide", &(generation,))
            .map_err(transport)
    }

    pub fn poll_click(&self, generation: u64, first: bool) -> Result<ClickPollResult, Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        let (status, id, pid, exe, class, desktop): (u32, u64, u32, String, String, bool) = self
            .proxy()?
            .call("PollClick", &(generation, first))
            .map_err(transport)?;
        if !optional_text(&exe, 256) || !optional_text(&class, 256) {
            return Err(Error::Protocol);
        }
        Ok(match status {
            0 => ClickPollResult::Pending,
            1 => ClickPollResult::Rejected,
            2 => ClickPollResult::Desktop,
            3 if id != 0 && pid != 0 => ClickPollResult::Target {
                id,
                pid,
                exe,
                class,
                desktop,
            },
            _ => ClickPollResult::Rejected,
        })
    }

    pub fn send_paste(
        &self,
        generation: u64,
        id: u64,
        pid: u32,
        chord: u32,
    ) -> Result<bool, Error> {
        if generation == 0 || id == 0 || pid == 0 || chord > 2 {
            return Err(Error::Protocol);
        }
        self.proxy()?
            .call("SendPaste", &(generation, id, pid, chord))
            .map_err(transport)
    }

    pub fn suspend(&self, generation: u64, on: bool) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        self.proxy()?
            .call("Suspend", &(generation, on))
            .map_err(transport)
    }

    pub fn ui_state(&self, state: &UiState) -> Result<(), Error> {
        if !string_list_ok(&state.exclude_apps) || !string_list_ok(&state.terminal_apps) {
            return Err(Error::Protocol);
        }
        self.proxy()?
            .call(
                "UiState",
                &(
                    state.paused,
                    state.size,
                    state.clips,
                    state.total_bytes,
                    state.age_days,
                    state.clip_bytes,
                    state.pins,
                    state.exclude_apps.clone(),
                    state.warn_multiline,
                    state.terminal_apps.clone(),
                    state.autostart,
                    state.first_run,
                ),
            )
            .map_err(transport)
    }

    pub fn open_settings(&self) -> Result<(), Error> {
        self.proxy()?.call("OpenSettings", &()).map_err(transport)
    }

    pub fn settings_result(&self) -> Result<SettingsResult, Error> {
        let (
            changed,
            confirmed,
            paused,
            size,
            clips,
            total_bytes,
            age_days,
            clip_bytes,
            pins,
            exclude_apps,
            warn_multiline,
            terminal_apps,
            autostart,
        ): (
            bool,
            bool,
            bool,
            u32,
            u32,
            u64,
            u32,
            u64,
            u32,
            Vec<String>,
            bool,
            Vec<String>,
            bool,
        ) = self
            .proxy()?
            .call("SettingsResult", &())
            .map_err(transport)?;
        if !string_list_ok(&exclude_apps) || !string_list_ok(&terminal_apps) {
            return Err(Error::Protocol);
        }
        Ok(SettingsResult {
            changed,
            confirmed,
            paused,
            size,
            clips,
            total_bytes,
            age_days,
            clip_bytes,
            pins,
            exclude_apps,
            warn_multiline,
            terminal_apps,
            autostart,
        })
    }

    pub fn confirm_settings(&self, clips: u32, bytes: u64, by_age: u32) -> Result<(), Error> {
        self.proxy()?
            .call("ConfirmSettings", &(clips, bytes, by_age))
            .map_err(transport)
    }

    pub fn poll(&self) -> Result<Vec<WireEvent>, Error> {
        let events: Vec<WireEvent> = self.proxy()?.call("Poll", &()).map_err(transport)?;
        if events.len() > MAX_EVENTS {
            return Err(Error::Protocol);
        }
        Ok(events)
    }

    fn mouse_proxy(&self) -> Result<Proxy<'_>, Error> {
        Proxy::new(
            &self.connection,
            self.owner.as_str(),
            "/com/pasteroll/Gnome/Mouse",
            "com.pasteroll.Gnome.Mouse1",
        )
        .map_err(|_| Error::Protocol)
    }

    pub fn arm_mouse(&mut self, enabled: bool) -> Result<(), Error> {
        let bits: u32 = self
            .mouse_proxy()?
            .call("Arm", &(enabled,))
            .map_err(transport)?;
        self.capabilities = Capabilities::negotiate(protocol::VERSION, bits)?;
        if enabled && !self.capabilities.mouse_activation() {
            return Err(Error::Missing);
        }
        Ok(())
    }

    pub fn begin_mouse(&self, ticket: u64) -> Result<u64, Error> {
        if ticket == 0 {
            return Err(Error::Stale);
        }
        let generation: u64 = self
            .mouse_proxy()?
            .call("Begin", &(ticket,))
            .map_err(transport)?;
        if generation == 0 {
            return Err(Error::Protocol);
        }
        Ok(generation)
    }

    pub fn show_mouse(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        generation: u64,
    ) -> Result<(u64, i32, i32, f64), Error> {
        protocol::frame_len(width, height)?;
        let result: (u64, i32, i32, f64) = self
            .mouse_proxy()?
            .call("Show", &(x, y, width, height, generation))
            .map_err(transport)?;
        if result.0 != generation || generation == 0 || !valid_scale(result.3) {
            return Err(Error::Protocol);
        }
        Ok(result)
    }

    pub fn show(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        keyboard_opt_in: bool,
    ) -> Result<(u64, i32, i32, f64), Error> {
        protocol::frame_len(width, height)?;
        let result: (u64, i32, i32, f64) = self
            .proxy()?
            .call("Show", &(x, y, width, height, keyboard_opt_in))
            .map_err(transport)?;
        if result.0 == 0 || !valid_scale(result.3) {
            return Err(Error::Protocol);
        }
        Ok(result)
    }

    pub fn present(&self, generation: u64, frame: &Frame) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        if protocol::frame_len(frame.width, frame.height)? != frame.bgra.len() {
            return Err(Error::Bounds);
        }
        self.proxy()?
            .call(
                "Present",
                &(generation, frame.width, frame.height, frame.bgra.as_slice()),
            )
            .map_err(transport)
    }

    pub fn hide(&self, generation: u64) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        self.proxy()?
            .call("Hide", &(generation,))
            .map_err(transport)
    }

    pub fn arm_keyboard(&self, enabled: bool, explicit_opt_in: bool) -> Result<(), Error> {
        if enabled {
            self.capabilities.allow_interaction(explicit_opt_in)?;
        }
        self.proxy()?
            .call("ArmKeyboard", &(enabled,))
            .map_err(transport)
    }
}

pub struct Capture {
    client: Client,
    max_bytes: usize,
    source: Option<String>,
}

impl Capture {
    pub fn new(client: Client, max_bytes: usize) -> Self {
        Self {
            client,
            max_bytes,
            source: None,
        }
    }
}

impl ClipboardSource for Capture {
    fn sequence(&mut self) -> u64 {
        self.client
            .snapshot()
            .map(|s| {
                self.source = s.source_app;
                s.sequence
            })
            .unwrap_or(0)
    }

    fn source_app(&mut self) -> Option<String> {
        self.source.clone()
    }

    fn capture(&mut self) -> Result<Captured, Skip> {
        self.client.capture(self.max_bytes)
    }
}

pub fn check() -> Result<(), Error> {
    let client = Client::connect()?;
    println!(
        "GNOME bridge protocol {}; clipboard, overlay, input, viewer and settings reported",
        protocol::VERSION
    );
    println!(
        "Capabilities: paste={} viewer={} settings={} keyboard={} global-mouse={}",
        client.capabilities.paste(),
        client.capabilities.viewer(),
        client.capabilities.settings(),
        client.capabilities.keyboard_activation(),
        client.capabilities.mouse_activation()
    );
    if !client.capabilities.paste()
        || !client.capabilities.viewer()
        || !client.capabilities.settings()
        || !client.capabilities.keyboard_activation()
    {
        return Err(Error::Protocol);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_frames_are_sampled_to_a_transport_safe_size() {
        let frame = Frame {
            width: 8,
            height: 8,
            bgra: vec![7; 8 * 8 * 4],
        };
        assert!(fit_frame(&frame, 64).is_none());
        let fitted = fit_frame(&frame, 16).expect("frame must shrink");
        assert_eq!((fitted.width, fitted.height), (4, 4));
        assert_eq!(fitted.bgra.len(), 4 * 4 * 4);
        assert!(fitted.bgra.iter().all(|b| *b == 7));
    }

    #[test]
    fn malformed_frames_are_refused() {
        assert!(fit_frame(
            &Frame {
                width: 0,
                height: 1,
                bgra: Vec::new()
            },
            16
        )
        .is_none());
        assert!(fit_frame(
            &Frame {
                width: 2,
                height: 2,
                bgra: vec![0; 15]
            },
            16
        )
        .is_none());
    }

    #[test]
    fn peer_text_and_geometry_are_bounded() {
        assert!(!short_text("", 10));
        assert!(!short_text("a\nb", 10));
        assert!(short_text("kitty", 10));
        assert!(valid_rect((0, 0, 1920, 1080)));
        assert!(!valid_rect((0, 0, 0, 1080)));
        assert!(valid_scale(1.0) && !valid_scale(0.0) && !valid_scale(f64::NAN));
    }
}
