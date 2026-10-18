//! Narrow RAII bindings to the stable libevdev ABI; all diagnostics are constant strings.
use super::gesture::{Buttons, Decision, Modifiers, MODIFIERS};
use std::{
    collections::BTreeSet,
    ffi::{c_char, c_int, c_uint, c_void, CStr, CString},
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::OpenOptionsExt,
    },
    path::Path,
};

#[link(name = "libevdev.so.2", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn libevdev_new_from_fd(fd: c_int, dev: *mut *mut c_void) -> c_int;
    fn libevdev_free(dev: *mut c_void);
    fn libevdev_next_event(dev: *mut c_void, flags: c_uint, event: *mut libc::input_event)
        -> c_int;
    fn libevdev_has_event_code(dev: *const c_void, ty: c_uint, code: c_uint) -> c_int;
    fn libevdev_get_event_value(dev: *const c_void, ty: c_uint, code: c_uint) -> c_int;
    fn libevdev_get_name(dev: *const c_void) -> *const c_char;
    fn libevdev_get_phys(dev: *const c_void) -> *const c_char;
    fn libevdev_set_clock_id(dev: *mut c_void, clock: c_int) -> c_int;
    fn libevdev_grab(dev: *mut c_void, grab: c_int) -> c_int;
    fn libevdev_uinput_create_from_device(
        dev: *const c_void,
        fd: c_int,
        output: *mut *mut c_void,
    ) -> c_int;
    fn libevdev_uinput_destroy(dev: *mut c_void);
    fn libevdev_uinput_write_event(
        dev: *const c_void,
        ty: c_uint,
        code: c_uint,
        value: c_int,
    ) -> c_int;
}
const EV_KEY: u32 = 1;
const EV_REL: u32 = 2;
pub const MARKER: &str = "pasteroll-input/";
pub struct Device {
    file: File,
    dev: *mut c_void,
    output: *mut c_void,
    output_file: Option<File>,
    pub modifiers: Modifiers,
    pub mouse: bool,
    pub buttons: Buttons,
    forwarded: BTreeSet<u16>,
    pub ready_at: std::time::Instant,
}
// Device is confined to one worker thread; raw pointers never cross a shared boundary.
impl Device {
    pub fn open(path: &Path) -> Result<Self, &'static str> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| "input device unavailable")?;
        let mut dev = std::ptr::null_mut();
        if unsafe { libevdev_new_from_fd(file.as_raw_fd(), &mut dev) } < 0 {
            return Err("input device unsupported");
        }
        let mut d = Self {
            file,
            dev,
            output: std::ptr::null_mut(),
            output_file: None,
            modifiers: Modifiers(0),
            mouse: false,
            buttons: Buttons::default(),
            forwarded: BTreeSet::new(),
            ready_at: std::time::Instant::now(),
        };
        let string = |p: *const c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        let name = string(unsafe { libevdev_get_name(dev) });
        let phys = string(unsafe { libevdev_get_phys(dev) });
        if phys.starts_with(MARKER) || name.starts_with("input-remapper") {
            return Err("virtual input excluded");
        }
        if unsafe { libevdev_set_clock_id(dev, libc::CLOCK_MONOTONIC) } < 0 {
            return Err("input clock unavailable");
        }
        d.mouse = d.has(EV_KEY, 274) && d.has(EV_REL, 0) && d.has(EV_REL, 1) && !d.has(3, 0);
        let keyboard = MODIFIERS.iter().any(|&code| d.has(EV_KEY, u32::from(code)));
        if !d.mouse && !keyboard {
            return Err("input device unsupported");
        }
        // Combined mouse/keyboard nodes require a broader forwarding contract.
        if d.mouse && keyboard {
            return Err("combined input device unsupported");
        }
        if keyboard {
            #[repr(C)]
            struct InputMask {
                ty: u32,
                size: u32,
                ptr: u64,
            }
            let mut mask = [0u8; 96];
            for code in MODIFIERS {
                mask[code as usize / 8] |= 1 << (code % 8);
            }
            let request = 0x4000_0000u64
                | ((std::mem::size_of::<InputMask>() as u64) << 16)
                | (b'E' as u64) << 8
                | 0x93;
            let keys = InputMask {
                ty: 1,
                size: mask.len() as u32,
                ptr: mask.as_ptr() as u64,
            };
            if unsafe { libc::ioctl(d.fd(), request as libc::c_ulong, &keys) } < 0 {
                return Err("modifier filtering unavailable");
            }
            mask.fill(0);
            let scan = InputMask {
                ty: 4,
                size: mask.len() as u32,
                ptr: mask.as_ptr() as u64,
            };
            if unsafe { libc::ioctl(d.fd(), request as libc::c_ulong, &scan) } < 0 {
                return Err("modifier filtering unavailable");
            }
        }
        d.refresh_modifiers();
        Ok(d)
    }
    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
    fn has(&self, ty: u32, code: u32) -> bool {
        unsafe { libevdev_has_event_code(self.dev, ty, code) != 0 }
    }
    pub fn value(&self, code: u16) -> bool {
        unsafe { libevdev_get_event_value(self.dev, EV_KEY, u32::from(code)) != 0 }
    }
    pub fn refresh_modifiers(&mut self) {
        self.modifiers = Modifiers(0);
        for code in MODIFIERS {
            self.modifiers.set(code, self.value(code));
        }
    }
    pub fn start_mouse(&mut self) -> Result<(), &'static str> {
        if !self.mouse {
            return Ok(());
        }
        // Attach only at a neutral boundary, so existing client button pairs are untouched.
        if (256..=767).any(|c| self.value(c)) {
            return Err("mouse buttons are held");
        }
        let phys = CString::new(format!("{MARKER}{}", self.fd()))
            .map_err(|_| "input identity unavailable")?;
        let output = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open("/dev/uinput")
            .map_err(|_| "virtual mouse unavailable")?;
        // libevdev copies capabilities/name/IDs, but does not copy phys. Set the
        // kernel identity before creation; otherwise hotplug would re-grab our output.
        let set_phys = 0x4000_0000u64
            | ((std::mem::size_of::<*const c_char>() as u64) << 16)
            | (b'U' as u64) << 8
            | 108;
        if unsafe { libc::ioctl(output.as_raw_fd(), set_phys as libc::c_ulong, phys.as_ptr()) } < 0
        {
            return Err("virtual identity unavailable");
        }
        if unsafe {
            libevdev_uinput_create_from_device(self.dev, output.as_raw_fd(), &mut self.output)
        } < 0
        {
            return Err("virtual mouse unavailable");
        }
        self.output_file = Some(output);
        if unsafe { libevdev_grab(self.dev, 3) } < 0 {
            return Err("mouse is already captured");
        }
        // Recheck after the grab to close the neutral-boundary race.
        // A fresh kernel key bitmap is used rather than libevdev's cached initial state.
        let mut bitmap = [0u8; 96];
        let request = 0x8000_0000u64 | ((bitmap.len() as u64) << 16) | (b'E' as u64) << 8 | 0x18;
        if unsafe { libc::ioctl(self.fd(), request as libc::c_ulong, bitmap.as_mut_ptr()) } < 0
            || bitmap.iter().any(|&b| b != 0)
        {
            return Err("mouse changed during capture");
        }
        self.ready_at = std::time::Instant::now() + std::time::Duration::from_millis(250);
        Ok(())
    }
    /// Returns a complete normal or synchronization event. The caller disables activation
    /// throughout synchronization and never treats reconstructed presses as gestures.
    pub fn next(&mut self, sync: bool) -> Result<Option<(libc::input_event, bool)>, &'static str> {
        let mut e: libc::input_event = unsafe { std::mem::zeroed() };
        let rc = unsafe { libevdev_next_event(self.dev, if sync { 1 } else { 2 }, &mut e) };
        match rc {
            0 => Ok(Some((e, false))),
            1 => Ok(Some((e, true))),
            n if n == -libc::EAGAIN => Ok(None),
            _ => Err("input device disconnected"),
        }
    }
    pub fn process(
        &mut self,
        e: libc::input_event,
        eligible: bool,
        mods: Modifiers,
    ) -> Result<bool, &'static str> {
        if !self.mouse {
            if e.type_ == 1 {
                self.modifiers.set(e.code, e.value != 0);
            }
            return Ok(false);
        }
        let decision = if e.type_ == 1 {
            self.buttons.process(e.code, e.value, eligible, mods)
        } else {
            Decision::Forward
        };
        if decision == Decision::Forward {
            if unsafe {
                libevdev_uinput_write_event(
                    self.output,
                    u32::from(e.type_),
                    u32::from(e.code),
                    e.value,
                )
            } < 0
            {
                return Err("input forwarding failed");
            }
            if e.type_ == 1 {
                if e.value == 0 {
                    self.forwarded.remove(&e.code);
                } else {
                    self.forwarded.insert(e.code);
                }
            }
        }
        Ok(decision == Decision::Activate)
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            if !self.output.is_null() {
                for &code in &self.forwarded {
                    libevdev_uinput_write_event(self.output, 1, u32::from(code), 0);
                }
                libevdev_uinput_write_event(self.output, 0, 0, 0);
                libevdev_uinput_destroy(self.output);
            }
            libevdev_grab(self.dev, 4);
            libevdev_free(self.dev);
        }
    }
}
