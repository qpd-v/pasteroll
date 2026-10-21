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

/// Foreground window and geometry in Shell stage units, with independent UI/device scales.
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
    pub stage_scale: f64,
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

// Split only on complete rows: every source pixel is delivered exactly once,
// without lowering UI/text resolution. Allocation is bounded before any IPC.
fn viewer_strips(frame: &Frame) -> Result<Vec<(u32, u32, std::ops::Range<usize>)>, Error> {
    let len = protocol::viewer_frame_len(frame.width, frame.height)?;
    if frame.bgra.len() != len {
        return Err(Error::Bounds);
    }
    let stride = frame.width as usize * 4;
    let rows = (protocol::MAX_FRAME_BYTES / stride) as u32;
    let mut strips = Vec::new();
    let mut y = 0;
    while y < frame.height {
        let height = rows.min(frame.height - y);
        let start = y as usize * stride;
        strips.push((y, height, start..start + height as usize * stride));
        y += height;
    }
    Ok(strips)
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
        let (id, pid, exe, class, desktop, x, y, modifiers, monitor, work, scale, stage_scale): (
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
            f64,
        ) = self.proxy()?.call("Context", &()).map_err(transport)?;
        if !optional_text(&exe, 256)
            || !optional_text(&class, 256)
            || !valid_rect(monitor)
            || !valid_rect(work)
            || !valid_scale(scale)
            || !valid_scale(stage_scale)
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
            stage_scale,
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

    /// Small frames use one message. Larger viewers use bounded, lossless strips,
    /// made visible atomically only after the whole frame has been accepted.
    pub fn viewer_present(&self, generation: u64, frame: &Frame) -> Result<(), Error> {
        if generation == 0 {
            return Err(Error::Stale);
        }
        let strips = viewer_strips(frame)?;
        let proxy = self.proxy()?;
        if strips.len() == 1 {
            return proxy
                .call(
                    "ViewerPresent",
                    &(generation, frame.width, frame.height, frame.bgra.as_slice()),
                )
                .map_err(transport);
        }
        let token: u64 = proxy
            .call("ViewerBegin", &(generation, frame.width, frame.height))
            .map_err(transport)?;
        if token == 0 {
            return Err(Error::Protocol);
        }
        for (y, height, bytes) in strips {
            proxy
                .call::<_, _, ()>(
                    "ViewerStrip",
                    &(generation, token, y, height, &frame.bgra[bytes]),
                )
                .map_err(transport)?;
        }
        proxy
            .call("ViewerCommit", &(generation, token))
            .map_err(transport)
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

    #[derive(Default)]
    struct TransferEvidence {
        width: u32,
        height: u32,
        next_y: u32,
        bytes: usize,
        strips: usize,
        valid: bool,
        committed: bool,
    }

    struct SyntheticViewer(std::sync::Arc<std::sync::Mutex<TransferEvidence>>);

    #[zbus::interface(name = "com.pasteroll.Gnome.Bridge2")]
    impl SyntheticViewer {
        fn viewer_begin(&self, generation: u64, width: u32, height: u32) -> u64 {
            let mut state = self.0.lock().unwrap();
            *state = TransferEvidence {
                width,
                height,
                valid: generation == 7,
                ..Default::default()
            };
            19
        }

        fn viewer_strip(&self, generation: u64, frame: u64, y: u32, height: u32, bytes: Vec<u8>) {
            let mut state = self.0.lock().unwrap();
            let correct = generation == 7
                && frame == 19
                && y == state.next_y
                && bytes.len() == (state.width * height * 4) as usize
                && bytes.len() <= protocol::MAX_FRAME_BYTES
                && bytes
                    .iter()
                    .enumerate()
                    .all(|(i, b)| *b == ((state.bytes + i) % 251) as u8);
            state.valid &= correct;
            state.next_y += height;
            state.bytes += bytes.len();
            state.strips += 1;
        }

        fn viewer_commit(&self, generation: u64, frame: u64) {
            let mut state = self.0.lock().unwrap();
            state.committed = generation == 7 && frame == 19 && state.next_y == state.height;
        }
    }

    #[test]
    fn four_k_viewer_crosses_a_private_bus_without_resampling() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        struct PrivateBus(std::process::Child);
        impl Drop for PrivateBus {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        // Explicit address on both connections; never use the desktop/session bus.
        let mut bus = PrivateBus(
            Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim();
        let evidence = std::sync::Arc::new(std::sync::Mutex::new(TransferEvidence::default()));
        let server = Builder::address(address)
            .unwrap()
            .serve_at(PATH, SyntheticViewer(evidence.clone()))
            .unwrap()
            .build()
            .unwrap();
        let client = Client {
            connection: Builder::address(address).unwrap().build().unwrap(),
            owner: server.unique_name().unwrap().to_string(),
            capabilities: Capabilities(protocol::VIEWER),
            epoch: 1,
        };
        let len = protocol::viewer_frame_len(3840, 2160).unwrap();
        let frame = Frame {
            width: 3840,
            height: 2160,
            bgra: (0..len).map(|i| (i % 251) as u8).collect(),
        };
        client.viewer_present(7, &frame).unwrap();
        let state = evidence.lock().unwrap();
        assert!(state.valid && state.committed);
        assert_eq!((state.width, state.height, state.strips), (3840, 2160, 2));
        assert_eq!(state.bytes, frame.bgra.len());
    }

    #[test]
    fn four_k_viewers_keep_every_pixel_in_bounded_strips() {
        for (width, height) in [(2880, 1620), (3840, 2160), (4096, 4096), (520, 440)] {
            let len = protocol::viewer_frame_len(width, height).unwrap();
            let frame = Frame {
                width,
                height,
                bgra: (0..len).map(|i| (i % 251) as u8).collect(),
            };
            let strips = viewer_strips(&frame).unwrap();
            let mut next_y = 0;
            let mut next_byte = 0;
            for (y, height, range) in strips {
                assert_eq!(y, next_y);
                assert_eq!(range.start, next_byte);
                assert_eq!(range.len(), protocol::frame_len(width, height).unwrap());
                assert!(range.len() <= protocol::MAX_FRAME_BYTES);
                assert_eq!(frame.bgra[range.clone()].len(), range.len());
                next_y += height;
                next_byte = range.end;
            }
            assert_eq!(next_y, frame.height);
            assert_eq!(next_byte, frame.bgra.len());
        }
    }

    #[test]
    fn malformed_or_oversized_viewers_are_refused_before_ipc() {
        for frame in [
            Frame {
                width: 0,
                height: 8,
                bgra: vec![],
            },
            Frame {
                width: 8,
                height: 8,
                bgra: vec![0; 1],
            },
            Frame {
                width: 4097,
                height: 1,
                bgra: vec![0; 4097 * 4],
            },
            Frame {
                width: 1,
                height: u32::MAX,
                bgra: vec![],
            },
        ] {
            assert!(viewer_strips(&frame).is_err());
        }
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
