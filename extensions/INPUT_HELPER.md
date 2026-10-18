# Mouse activation on stock GNOME

This development implementation uses a Pasteroll-owned host input service and
the GNOME companion. It does not patch GNOME, Mutter or another application's
software. Published beta.2 downloads do not include this work.

## Components and permissions

- `pasteroll` remains an ordinary user process. It owns encrypted history and
  never receives raw device descriptors or administrator privileges.
- `pasteroll-input` is an optional system service linked against libevdev2. It
  reads modifier state, grabs supported mice and forwards their ordinary events
  through uinput. Only the matching activation press/release is consumed.
- The existing GNOME extension keeps responsibility for desktop coordinates,
  clipboard handling, UI, target checks and paste. It receives semantic activation
  notifications over the system bus, never a keyboard event stream.

The service accepts requests only from a root-owned `/usr/bin/gnome-shell` process
whose D-Bus credentials match an active, local, unlocked Wayland user session on
logind. The companion checks that the service's unique bus owner is root. A
renewable lease bounds interception to the running application and companion.
Out-of-order renewals cannot override a later revocation.

Keyboard descriptors are not exclusively grabbed. Kernel event masks restrict
their key events to left/right Ctrl, Shift, Alt and Super; the helper does not log
events or forward keyboard input. Device access is still privileged and must be
treated as a security boundary. Do not grant the whole desktop account membership
of the `input` group or make `/dev/input` or `/dev/uinput` world-writable.

Normal events retain their event frames, relative motion and wheel values.
Modifier state is merged across devices in kernel timestamp order. Ownership is
decided on the mouse press: releasing modifiers first cannot leak the claimed
release. Unregistering stops new activation and drains existing button sequences.
Virtual outputs have a distinct kernel identity and cannot be re-grabbed by the
service. Device removal resets that device's state.
If the helper or the last supported mouse disappears, Pasteroll closes its input
UI and waits safely. Reconnection restores activation without restarting the app.

## Supported scope

The current companion targets GNOME 46. The host service initially supports
one active local desktop lease and relative mice with a middle button.
Touchpads/tablets and combined keyboard/mouse
event nodes are left untouched. Another remapper owning the mouse prevents this
helper from taking it; run only one remapping owner for that device.

Physical device behavior, pointer acceleration/DPI, vendor-specific settings,
real login/lock transitions, mixed-DPI/multiple monitors and newer GNOME releases
require separate desktop validation. Software tests and virtual devices do not
establish these results. Forced termination releases kernel grabs; a forced
process kill cannot provide the same button-sequence guarantee as graceful drain.

## Build the development packages

On Linux/WSL, with the existing release prerequisites plus libevdev2 and the GNU
Rust target available:

```sh
bash scripts/package-gnome.sh
```

One command builds a static app, a separate glibc host helper, a GNOME development
archive and a Debian package under `target-dev/wayland-mouse/packages`. The helper
is deliberately not presented as a static portable executable. Build and output
locations can be selected with `PASTEROLL_BUILD_ROOT` and
`PASTEROLL_PACKAGE_OUTPUT`. The script never downloads dependencies or publishes.
For a Windows-managed worktree built through WSL, provide `SOURCE_DATE_EPOCH`
from the Windows checkout's commit timestamp; Linux Git cannot resolve its
Windows `.git` pointer. Ordinary Linux checkouts derive it from Git automatically.
Run `scripts/scan_binary.py` with private terms supplied through
`PASTEROLL_SCAN_WORDS` on every artifact before sharing it.

## Archive installation

Extract the complete GNOME archive. As the desktop account, install the app and
companion:

```sh
./install.sh
./install-gnome-companion.sh
```

The optional input helper needs one administrator-approved installation:

```sh
sudo ./install-input-helper.sh --enable
```

This installs the helper, systemd unit, bus policy and module-load configuration.
It does not run the clipboard application as root. The host must use systemd and
logind, with uinput available. The archive installer checks its own ownership
marker and refuses to overwrite another package's helper.

Log out and back in, then:

```sh
gnome-extensions enable pasteroll@pasteroll.com
pasteroll run --backend gnome
```

Ctrl+Shift+middle-click now requests activation through the helper. A GNOME
session selected automatically by `pasteroll run` uses the same mouse mode.
Missing or unavailable helper support produces an error; it does not silently
substitute a keyboard shortcut. Users who want keyboard mode can choose it:

```sh
pasteroll run --backend gnome --wayland-keyboard
```

Settings and sign-in startup preserve the chosen mode.

## Debian installation

Install the development `.deb` with the package manager so it resolves its
dependencies. The package contains the system extension, helper, service and bus
policy; its GNOME dependency is restricted to the currently supported major
version. The service remains disabled on a new installation. Enable it explicitly:

```sh
sudo systemctl enable --now pasteroll-input.service
```

Then log out/back in, enable the extension and start Pasteroll as above. A
per-user extension copy overrides a system copy: remove an older per-user copy
before using the packaged system extension. Flatpak would still require a host
helper package and separate sandbox integration; it is not implemented here.

## Disable or remove

Stop Pasteroll normally before changing its integration:

```sh
pasteroll stop
sudo systemctl disable --now pasteroll-input.service
```

For the archive, `sudo ./install-input-helper.sh --uninstall` removes only the
marked helper installation. `./install-gnome-companion.sh --uninstall` removes the
per-user companion through GNOME's extension manager. The ordinary app installer
or package manager removes the application. All these paths retain user history,
keys and settings.

## Validation

```sh
cargo test --workspace
```

On WSL/Linux use `cargo test --workspace --exclude pasteroll-win`. The isolated
native checks are:

```sh
bash scripts/wayland-spike/run.sh --mouse
bash scripts/wayland-spike/run.sh --phase3
bash scripts/wayland-spike/run.sh --phase2
```

The mouse suite needs the kernel `uinput` and `evdev` modules, a private namespace
and passwordless sudo for its disposable root fixture. It uses real kernel
devices and the real helper, a private fake logind, stock headless GNOME and the
real app. Ordinary native input is reflected into Clutter by the test driver;
this is not physical libinput or real-session validation. It never attaches to
the live desktop or its clipboard. Test-only raw input injection stays in the
guarded fixture, outside the production service.

Implementation references: [Linux uinput documentation](https://www.kernel.org/doc/html/latest/input/uinput.html),
[libevdev API](https://www.freedesktop.org/software/libevdev/doc/latest/), and
[GNOME 46 display-session lookup](https://github.com/GNOME/gnome-shell/blob/46.0/js/misc/loginManager.js).
