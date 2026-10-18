//! Root host service: fixed gesture, authenticated Shell peer, bounded renewable lease.
use super::{device::Device, gesture::Modifiers};
use std::{
    collections::BTreeMap,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use zbus::{
    blocking::{connection::Builder, Connection, Proxy},
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const NAME: &str = "com.pasteroll.Input";
const PATH: &str = "/com/pasteroll/Input";
const LEASE: Duration = Duration::from_millis(500);
const READY: u32 = 1;
const MAX_DEVICES: usize = 64;
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}
pub fn monotonic_us() -> u64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t);
    }
    t.tv_sec as u64 * 1_000_000 + t.tv_nsec as u64 / 1000
}

fn watchdog() {
    use std::os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixDatagram},
    };
    let Some(value) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    let bytes = value.as_encoded_bytes();
    let address = if bytes.first() == Some(&b'@') {
        SocketAddr::from_abstract_name(&bytes[1..])
    } else {
        SocketAddr::from_pathname(Path::new(&value))
    };
    if let (Ok(address), Ok(socket)) = (address, UnixDatagram::unbound()) {
        let _ = socket.send_to_addr(b"WATCHDOG=1", &address);
    }
}

#[derive(Clone)]
struct Owner {
    sender: String,
    seat: String,
    checked: Instant,
}
struct Shared {
    owner: Option<Owner>,
    instance: u64,
    request: u64,
    renewed: Instant,
    enabled: bool,
    accepting: bool,
    ready: bool,
    sequence: u64,
    event_us: u64,
    reported: u64,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            owner: None,
            instance: 0,
            request: 0,
            renewed: Instant::now(),
            enabled: false,
            accepting: false,
            ready: false,
            sequence: 0,
            event_us: 0,
            reported: 0,
        }
    }
}
fn denied() -> zbus::fdo::Error {
    zbus::fdo::Error::AccessDenied(
        "only an active local GNOME Shell session may use this service".into(),
    )
}

fn session(bus: &Connection, pid: u32, uid: u32) -> Result<(String, String), ()> {
    let manager = Proxy::new(
        bus,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(|_| ())?;
    let path: OwnedObjectPath = match manager.call("GetSessionByPID", &(pid,)) {
        Ok(path) => path,
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.login1.NoSessionForPID" =>
        {
            // GNOME Shell may be launched by systemd's user manager, outside a
            // session scope. Follow logind's display session for the verified UID.
            let user: OwnedObjectPath = manager.call("GetUser", &(uid,)).map_err(|_| ())?;
            let proxy = Proxy::new(
                bus,
                "org.freedesktop.login1",
                user.as_str(),
                "org.freedesktop.login1.User",
            )
            .map_err(|_| ())?;
            let (id, path): (String, OwnedObjectPath) =
                proxy.get_property("Display").map_err(|_| ())?;
            if id.is_empty() {
                return Err(());
            }
            path
        }
        Err(_) => return Err(()),
    };
    let props = Proxy::new(
        bus,
        "org.freedesktop.login1",
        path.as_str(),
        "org.freedesktop.DBus.Properties",
    )
    .map_err(|_| ())?;
    let mut map: std::collections::HashMap<String, OwnedValue> = props
        .call("GetAll", &("org.freedesktop.login1.Session",))
        .map_err(|_| ())?;
    let active = bool::try_from(map.remove("Active").ok_or(())?).map_err(|_| ())?;
    let locked = bool::try_from(map.remove("LockedHint").ok_or(())?).map_err(|_| ())?;
    let remote = bool::try_from(map.remove("Remote").ok_or(())?).map_err(|_| ())?;
    let class = String::try_from(map.remove("Class").ok_or(())?).map_err(|_| ())?;
    let kind = String::try_from(map.remove("Type").ok_or(())?).map_err(|_| ())?;
    let (user, _): (u32, OwnedObjectPath) =
        map.remove("User").ok_or(())?.try_into().map_err(|_| ())?;
    let (seat, _): (String, OwnedObjectPath) =
        map.remove("Seat").ok_or(())?.try_into().map_err(|_| ())?;
    if !active
        || locked
        || remote
        || class != "user"
        || kind != "wayland"
        || user != uid
        || seat.is_empty()
    {
        return Err(());
    }
    Ok((path.to_string(), seat))
}
fn shell_executable(pid: u32) -> bool {
    let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
        return false;
    };
    let Ok(expected) = std::fs::canonicalize("/usr/bin/gnome-shell") else {
        return false;
    };
    if exe != expected {
        return false;
    }
    // A user-controlled executable or parent must never acquire root input access.
    let mut p = Some(expected.as_path());
    while let Some(path) = p {
        let Ok(m) = std::fs::metadata(path) else {
            return false;
        };
        if m.uid() != 0 || m.mode() & 0o022 != 0 {
            return false;
        }
        p = path.parent();
    }
    true
}
struct Service {
    state: Arc<Mutex<Shared>>,
    bus: Connection,
}
#[zbus::interface(name = "com.pasteroll.Input1")]
impl Service {
    /// Polling renews a short lease. A false request revokes new activations immediately.
    fn poll(
        &self,
        enabled: bool,
        accepting: bool,
        instance: u64,
        request: u64,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(u32, u32, u64, u64)> {
        let sender = header.sender().ok_or_else(denied)?.to_string();
        let old = { self.state.lock().map_err(|_| denied())?.owner.clone() };
        let owner = if let Some(old) =
            old.filter(|o| o.sender == sender && o.checked.elapsed() < Duration::from_millis(200))
        {
            old
        } else {
            let bus = Proxy::new(
                &self.bus,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )
            .map_err(|_| denied())?;
            let uid: u32 = bus
                .call("GetConnectionUnixUser", &(sender.as_str(),))
                .map_err(|_| denied())?;
            let pid: u32 = bus
                .call("GetConnectionUnixProcessID", &(sender.as_str(),))
                .map_err(|_| denied())?;
            if uid == 0 || !shell_executable(pid) {
                return Err(denied());
            }
            let (_, seat) = session(&self.bus, pid, uid).map_err(|_| denied())?;
            Owner {
                sender: sender.clone(),
                seat,
                checked: Instant::now(),
            }
        };
        let mut state = self.state.lock().map_err(|_| denied())?;
        if state.enabled
            && state.renewed.elapsed() < LEASE
            && state.owner.as_ref().is_some_and(|o| o.sender != sender)
        {
            return Err(denied());
        }
        if state.owner.as_ref().is_none_or(|o| o.sender != sender) {
            state.ready = false;
            state.reported = state.sequence;
            state.request = 0;
            state.instance = 0;
        }
        // The Shell bus connection survives extension disable/re-enable. Scope
        // request ordering to an extension instance, and reject delayed messages
        // from a destroyed instance rather than requiring a root-service restart.
        if instance == 0 || instance < state.instance || instance > monotonic_us() {
            return Err(denied());
        }
        if instance > state.instance {
            state.instance = instance;
            state.request = 0;
            state.reported = state.sequence;
            state.ready = false;
        }
        if request == 0 || request <= state.request {
            return Err(denied());
        }
        state.request = request;
        state.owner = Some(owner);
        state.enabled = enabled;
        state.accepting = enabled && accepting;
        state.renewed = Instant::now();
        let ready = enabled && state.ready;
        // Events are delivered once, only to the current owner, and never after revocation.
        let (seq, at) = if ready && state.sequence != state.reported {
            state.reported = state.sequence;
            (state.sequence, state.event_us)
        } else {
            (0, 0)
        };
        if !enabled {
            state.ready = false;
            state.reported = state.sequence;
        }
        Ok((1, if ready { READY } else { 0 }, seq, at))
    }
}

fn same_device(path: &Path, identity: (u64, u64)) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.rdev() == identity.0 && m.ino() == identity.1)
}
fn seat_for(path: &Path) -> Option<String> {
    let m = std::fs::symlink_metadata(path).ok()?;
    use std::os::unix::fs::FileTypeExt;
    if !m.file_type().is_char_device() {
        return None;
    }
    let dev = m.rdev();
    let data = std::fs::read_to_string(format!(
        "/run/udev/data/c{}:{}",
        libc::major(dev),
        libc::minor(dev)
    ))
    .unwrap_or_default();
    Some(
        data.lines()
            .find_map(|l| l.strip_prefix("E:ID_SEAT="))
            .unwrap_or("seat0")
            .to_string(),
    )
}
struct Entry {
    device: Device,
    identity: (u64, u64),
    sync: bool,
}

fn worker(state: Arc<Mutex<Shared>>) -> Result<(), &'static str> {
    let mut devices: BTreeMap<PathBuf, Entry> = BTreeMap::new();
    let mut scan = Instant::now();
    let mut owner_key = None;
    let mut notify_at = Instant::now();
    while !STOP.load(Ordering::Relaxed) || devices.values().any(|e| e.device.buttons.held()) {
        if Instant::now() >= notify_at {
            watchdog();
            notify_at = Instant::now() + Duration::from_millis(500);
        }
        let owner = {
            let s = state.lock().map_err(|_| "service state unavailable")?;
            if !STOP.load(Ordering::Relaxed) && s.enabled && s.renewed.elapsed() < LEASE {
                // Session credentials are refreshed by Poll; the independent
                // 500 ms lease is the worker's deadline. A second, shorter
                // deadline would drop a healthy lease during ordinary bus jitter.
                s.owner.clone()
            } else {
                None
            }
        };
        let key = owner.as_ref().map(|o| (o.sender.clone(), o.seat.clone()));
        let mut eligible = owner.is_some();
        if key != owner_key {
            // Do not transfer a grabbed device to another session while any button is held.
            eligible = false;
            devices.retain(|_, e| e.device.mouse && e.device.buttons.held());
            if devices.is_empty() {
                owner_key = key;
                scan = Instant::now();
            }
        }
        if !eligible {
            devices.retain(|_, e| e.device.mouse && e.device.buttons.held());
            let mut s = state.lock().map_err(|_| "service state unavailable")?;
            s.ready = false;
            s.reported = s.sequence;
        } else if Instant::now() >= scan {
            scan = Instant::now() + Duration::from_millis(300);
            devices.retain(|path, e| same_device(path, e.identity));
            let seat = &owner.as_ref().ok_or("owner unavailable")?.seat;
            if let Ok(paths) = std::fs::read_dir("/dev/input") {
                for path in paths.flatten().map(|p| p.path()).take(256) {
                    if devices.len() >= MAX_DEVICES {
                        break;
                    }
                    if !path
                        .file_name()
                        .is_some_and(|s| s.to_string_lossy().starts_with("event"))
                        || devices.contains_key(&path)
                        || seat_for(&path).as_ref() != Some(seat)
                    {
                        continue;
                    }
                    let Ok(mut device) = Device::open(&path) else {
                        continue;
                    };
                    if device.mouse && device.start_mouse().is_err() {
                        continue;
                    }
                    let Ok(m) = std::fs::symlink_metadata(&path) else {
                        continue;
                    };
                    devices.insert(
                        path,
                        Entry {
                            device,
                            identity: (m.rdev(), m.ino()),
                            sync: false,
                        },
                    );
                }
            }
        }
        // Merge by the kernel monotonic event time. Draining keyboards before mice would
        // miss a quick chord whose modifier release is queued after its mouse press.
        // Ordinary keyboard keys never enter this bounded temporary queue.
        let paths: Vec<_> = devices.keys().cloned().collect();
        let mut queued = Vec::new();
        let mut broken = Vec::new();
        let mut recovering = false;
        for path in &paths {
            let e = devices.get_mut(path).ok_or("device unavailable")?;
            for _ in 0..4096 {
                match e.device.next(e.sync) {
                    Ok(Some((event, sync))) => {
                        recovering |= sync || e.sync;
                        e.sync = sync;
                        if e.device.mouse
                            || event.type_ == 1 && super::gesture::MODIFIERS.contains(&event.code)
                        {
                            if queued.len() >= 8192 {
                                return Err("input burst exceeded service capacity");
                            }
                            queued.push((path.clone(), event));
                        }
                    }
                    Ok(None) => {
                        if e.sync {
                            e.sync = false;
                        }
                        break;
                    }
                    Err(_) => {
                        broken.push(path.clone());
                        break;
                    }
                }
            }
        }
        for p in broken.drain(..) {
            devices.remove(&p);
        }
        let has_keyboard = devices.values().any(|e| !e.device.mouse);
        let ready = eligible
            && has_keyboard
            && devices
                .values()
                .any(|e| e.device.mouse && Instant::now() >= e.device.ready_at);
        let accepting = state
            .lock()
            .map_err(|_| "service state unavailable")?
            .accepting;
        queued.sort_by_key(|(_, e)| (e.time.tv_sec, e.time.tv_usec));
        for (path, event) in queued {
            let mods = devices
                .values()
                .filter(|e| !e.device.mouse)
                .fold(Modifiers(0), |m, e| m.union(e.device.modifiers));
            let other_held = devices
                .iter()
                .any(|(other, e)| *other != path && e.device.mouse && e.device.buttons.held());
            let Some(e) = devices.get_mut(&path) else {
                continue;
            };
            let at = (event.time.tv_sec.max(0) as u64)
                .saturating_mul(1_000_000)
                .saturating_add(event.time.tv_usec.max(0) as u64);
            let fresh = monotonic_us()
                .checked_sub(at)
                .is_some_and(|age| age < 100_000);
            if e.device.process(
                event,
                ready && accepting && !recovering && fresh && !other_held,
                mods,
            )? {
                let mut s = state.lock().map_err(|_| "service state unavailable")?;
                if s.enabled && s.accepting && s.renewed.elapsed() < LEASE {
                    s.sequence = s
                        .sequence
                        .checked_add(1)
                        .ok_or("activation sequence exhausted")?;
                    s.event_us = at;
                }
            }
        }
        if recovering {
            for e in devices.values_mut().filter(|e| !e.device.mouse) {
                e.device.refresh_modifiers();
            }
        }
        state.lock().map_err(|_| "service state unavailable")?.ready = ready;
        let mut fds: Vec<_> = devices
            .values()
            .map(|e| libc::pollfd {
                fd: e.device.fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        unsafe {
            libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 5);
        }
    }
    Ok(())
}

pub fn main() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        println!("pasteroll-input: optional host service; run through pasteroll-input.service. No configuration or input injection commands.");
        return Ok(());
    }
    if !args.is_empty() {
        return Err("unknown option; use --help");
    }
    if unsafe { libc::geteuid() } != 0 {
        return Err("start the administrator-installed system service");
    }
    unsafe {
        libc::umask(0o077);
        libc::prctl(libc::PR_SET_DUMPABLE, 0);
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        libc::signal(libc::SIGTERM, stop as *const () as usize);
        libc::signal(libc::SIGINT, stop as *const () as usize);
    }
    let state = Arc::new(Mutex::new(Shared::default()));
    let auth = Builder::system()
        .map_err(|_| "system bus unavailable")?
        .method_timeout(Duration::from_millis(200))
        .build()
        .map_err(|_| "system bus unavailable")?;
    let _service = Builder::system()
        .map_err(|_| "system bus unavailable")?
        .name(NAME)
        .map_err(|_| "service name unavailable")?
        .serve_at(
            PATH,
            Service {
                state: state.clone(),
                bus: auth.clone(),
            },
        )
        .map_err(|_| "service interface unavailable")?
        .build()
        .map_err(|_| "service registration denied")?;
    worker(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn own_process_is_not_a_shell_peer() {
        assert!(!shell_executable(std::process::id()));
    }
    #[test]
    fn lease_starts_disarmed() {
        let s = Shared::default();
        assert!(!s.enabled && !s.ready && s.owner.is_none());
    }
    #[test]
    fn non_device_paths_are_rejected() {
        assert!(seat_for(Path::new("/etc/passwd")).is_none());
    }
}
