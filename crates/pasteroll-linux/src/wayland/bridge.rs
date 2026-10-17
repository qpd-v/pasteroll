//! Bounded, owner-pinned communication with the companion inside GNOME Shell.
use super::protocol::{self, Capabilities, Error, WireEvent, MAX_BYTES, MAX_EVENTS};
use pasteroll_app::{Captured, ClipboardSource, Payload, Skip};
use pasteroll_render::Frame;
use std::time::Duration;
use zbus::blocking::{connection::Builder, Connection, Proxy};
use zeroize::{Zeroize, Zeroizing};

const NAME: &str = "com.pasteroll.Gnome";
const PATH: &str = "/com/pasteroll/Gnome";
const INTERFACE: &str = "com.pasteroll.Gnome.Bridge1";

#[derive(Clone)]
pub struct Client {
    connection: Connection,
    owner: String,
    pub capabilities: Capabilities,
    pub epoch: u64,
}

#[derive(Clone)]
pub struct Snapshot {
    pub sequence: u64,
    pub types: Vec<String>,
    pub source_app: Option<String>,
    pub excluded: bool,
}

fn transport(error: zbus::Error) -> Error {
    if let zbus::Error::MethodError(name, _, _) = &error {
        if name.as_str().ends_with(".Stale") {
            return Error::Stale;
        }
        if name.as_str().ends_with(".Bounds") {
            return Error::Bounds;
        }
        if name.as_str().ends_with(".AccessDenied") {
            return Error::UntrustedPeer;
        }
    }
    // Do not expose D-Bus errors: a peer may put clipboard bytes in an error message.
    Error::Disconnected
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
        if types.len() > 128 || types.iter().any(|s| s.len() > 256) || source.len() > 256 {
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
        if limit == 0 || limit > MAX_BYTES || mime.len() > 256 {
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

    /// Opt-in applies only to the experiment. It is never inferred from a missing mouse capability.
    pub fn show(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        keyboard_opt_in: bool,
    ) -> Result<(u64, i32, i32, f64), Error> {
        self.capabilities.allow_interaction(keyboard_opt_in)?;
        protocol::frame_len(width, height)?;
        let result: (u64, i32, i32, f64) = self
            .proxy()?
            .call("Show", &(x, y, width, height, keyboard_opt_in))
            .map_err(transport)?;
        if result.0 == 0 || !result.3.is_finite() || result.3 <= 0.0 {
            return Err(Error::Protocol);
        }
        Ok(result)
    }

    pub fn present(&self, generation: u64, frame: &Frame) -> Result<(), Error> {
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

    pub fn poll(&self) -> Result<Vec<WireEvent>, Error> {
        let events: Vec<WireEvent> = self.proxy()?.call("Poll", &()).map_err(transport)?;
        if events.len() > MAX_EVENTS {
            return Err(Error::Protocol);
        }
        Ok(events)
    }

    pub fn hide(&self, generation: u64) -> Result<(), Error> {
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
        "GNOME bridge protocol {}; clipboard/overlay/input available",
        protocol::VERSION
    );
    println!(
        "Global mouse activation: {}",
        client.capabilities.mouse_activation()
    );
    println!(
        "Keyboard activation available for explicit experiments: {}",
        client.capabilities.keyboard_activation()
    );
    println!("Controller integration is not enabled in this phase.");
    Ok(())
}
