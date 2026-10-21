# Pasteroll GNOME companion

The companion in `pasteroll@pasteroll.com/` is the native GNOME Shell side of
the GNOME Wayland backend. It targets GNOME 46 and uses the local D-Bus interface
`com.pasteroll.Gnome.Bridge2` at `/com/pasteroll/Gnome`.

## What it provides

- Clipboard snapshots, bounded text/file/PNG reads and writes, stale-selection
  checks, sensitive-marker filtering and own-write exclusion.
- A native Shell carousel with renderer BGRA frames and generation-tagged input
  events for search, selection, wheel navigation, dragging and keyboard use.
- The expanded viewer, animation frames, wheel/find/drag input and image-editor
  modal outside-click handling.
- Foreground target context, stable PID/window checks, privilege checks, and a
  bounded left-click replay for paste-at-click. Only the real captured left
  click is replayed, once, after releasing the modal grab.
- Native paste delivery for exactly three chords: Ctrl+V, Shift+Insert and
  Ctrl+Shift+V. No arbitrary key or command method is exported.
- A native panel menu with activate, pause, settings, data-folder and quit
  actions. The settings dialog edits structured options and returns proposals
  through `SettingsResult`; destructive reductions require a real confirmation
  before Rust applies them.
- A first-run notice with a real acknowledgement control. Acknowledgement emits
  Poll kind 25; the notice is never marked acknowledged by state publication
  alone.

The companion owner-pins the D-Bus peer, verifies the peer's UID and executable,
rejects stale generations, releases grabs on owner loss, lock, timeout, disable
and overflow, and keeps event queues bounded. It does not expose shell command
execution, an HTTP endpoint, a general input injector, or clipboard history
persistence; history remains owned by the encrypted Rust store.

## Activation and coordinates

Ctrl+Shift+middle-click activation on stock GNOME uses the optional Pasteroll host
input helper. See [INPUT_HELPER.md](INPUT_HELPER.md) for its permissions,
installation, tests and device limits. With that service installed and enabled:

```sh
pasteroll run --backend gnome
```

Keyboard activation remains an explicit alternative:

```sh
pasteroll run --backend gnome --wayland-keyboard
```

Bridge wire version **3** must be installed with its matching beta.4 app and
companion; older clients/companions are rejected at the handshake. The D-Bus
interface name remains `com.pasteroll.Gnome.Bridge2` for discovery, but its version
check gates the revised Context contract before any data access.

Frame dimensions and BGRA bytes are device pixels. Global Context pointer,
monitor/work rectangles and Show/ViewerShow positions are **Shell stage units**,
which are physical pixels in Mutter physical layout and logical units in logical
layout. Context reports both UI scale (device pixels per DIP) and stage scale
(device pixels per stage unit, read from the matching Clutter stage view).
Show returns the actual UI scale. The adapter converts positions with stage scale
and renders/hit-tests with UI scale; it must not multiply physical monitor geometry
by the UI scale again. Monitor changes during activation are rejected if the
returned UI scale differs from the requested context.

Carousel and individual viewer messages remain capped at 16 MiB and dimensions
at 4096. Viewers larger than one message use ViewerBegin/ViewerStrip/ViewerCommit:
consecutive full-width BGRA strips, each at most 16 MiB, with no resampling. A
complete viewer is at most 4096x4096 (64 MiB of pixel data). The companion retains
at most one displayed frame plus one pending frame, bounding pixel backing to
128 MiB, excluding driver/texture/internal copies. Pending strips stay hidden
until a complete commit; malformed strips discard the pending frame. Every call
checks owner authorization and the unlocked session generation. New begin, close,
lock/revocation and disconnect cleanup discard abandoned transfers. Small frames
retain the single-message path. Rendering and GObject/driver copies can consume
additional memory; verify responsiveness and resource use on the physical setup.

Synthetic checks: `node scripts/test-gnome-geometry.mjs` and native Rust Wayland
unit tests. These cover coordinate conversion, 4K lossless strip coverage,
malformed/stale/incomplete transfers and replacement/cleanup. They do not establish
real GNOME compositor or hardware acceptance. To collect content-free geometry
logs during a future approved candidate run, set `PASTEROLL_GEOMETRY_DIAGNOSTICS=1`.

Headless GNOME 46 checks include a scale-1 virtual monitor and a single 4K virtual
monitor at 200% in both physical and logical layout modes. Synthetic screenshots
verify rendered pixels across the strip boundary. This does not establish
physical mixed-DPI, NVIDIA, hotplug or suspend acceptance. Newer GNOME releases
and non-GNOME Wayland desktops are not advertised as tested.
Mouse activation capability is advertised only after the
authenticated host helper has acquired supported devices.

### Executable identity and data protection

The companion verifies the peer's UID and executable through D-Bus credentials
and `/proc/<pid>/exe`. The app authenticates before opening keys or history, then
immediately disables process dumps before accessing those data. It never
temporarily restores dumpability. Subsequent requests are pinned to that unique
D-Bus peer; companion loss closes the app rather than trusting a replacement.

Accepted executable locations are `/usr/bin/pasteroll`,
`/usr/local/bin/pasteroll` and `~/.local/bin/pasteroll`, including their resolved
symlink targets. This GNOME backend requires an installed executable and the
companion; starting an arbitrary portable copy is not supported by its peer check.

Copies are excluded when sensitive MIME markers are present. App identification
uses the foreground process and, if it is hardened, Shell's desktop-app mapping.
When exclusions are configured and the source still cannot be identified, that
copy is skipped. Own clipboard writes are excluded from history.

## Development installation on GNOME 46

This implementation is for the development branch; it is not included in the
published beta.2 downloads. From a Linux source checkout, build the app and copy
the companion into the current account:

```sh
cargo build --release -p pasteroll-linux
install -Dm755 target/release/pasteroll "$HOME/.local/bin/pasteroll"
companion_dir="$HOME/.local/share/gnome-shell/extensions/pasteroll@pasteroll.com"
install -d "$companion_dir"
cp extensions/pasteroll@pasteroll.com/*.js \
   extensions/pasteroll@pasteroll.com/metadata.json \
   extensions/pasteroll@pasteroll.com/stylesheet.css "$companion_dir/"
```

Log out and back in so GNOME discovers the companion, then run:

```sh
gnome-extensions enable pasteroll@pasteroll.com
"$HOME/.local/bin/pasteroll" wayland-check
"$HOME/.local/bin/pasteroll" run --backend gnome --wayland-keyboard
```

Use **Ctrl+Shift+Space** to open the carousel. Mouse-wheel selection,
click-to-paste and Ctrl+desktop-click image editing then work within that explicit
keyboard session. Autostart is off until enabled in Settings and retains the
chosen activation mode. Without keyboard opt-in, GNOME mouse mode requires the
optional host helper; it does not silently switch to XWayland.

To stop and remove only the companion:

```sh
"$HOME/.local/bin/pasteroll" stop
gnome-extensions disable pasteroll@pasteroll.com
gnome-extensions uninstall pasteroll@pasteroll.com
```

History and settings remain in the normal app data folder. The app's uninstall
workflow is separate. Disable sign-in startup in Settings before removing the
app if you previously enabled it.

## Bridge lifecycle

Input generations tag events and frames. Paste authorization survives visible
session teardown for at most three seconds and is consumed once, with final
focus, PID, privilege, held-modifier and clipboard-owner checks. Unknown Shell
surfaces are rejected rather than guessed to be desktop save targets.

Poll actions 20–24 carry panel intent; 25 acknowledges the actual welcome button;
26 coalesces clipboard-change notifications to wake the Rust capture worker.
File dialogs suspend the grab and surfaces; their intentional suspension can
outlast the normal watchdog. Owner loss and extension disable still tear down UI.

## Diagnostics and tests

`pasteroll wayland-check` performs the owner-pinned Bridge2 handshake without
opening history or reading clipboard contents. Native integration runs use the
existing isolated launcher and disposable private profile; results and sanitized
logs are written under the ignored `target-dev/` directories.

From the repository root, the existing native suite entry point is:

```sh
wsl -d Ubuntu -- bash scripts/wayland-spike/run.sh --phase3
```

JavaScript syntax checks can be run without a desktop session:

```sh
for file in extensions/pasteroll@pasteroll.com/*.js; do
  node --input-type=module --check < "$file"
done
```

The production extension does not require X11 or XWayland. Keyboard mode needs
no privileged helper; native mouse mode requires the optional host service.
The test injector in `scripts/wayland-spike` is
separate from this companion and must only be used inside its isolated launcher.

## Beta.4 acceptance scope

Mixed-scale carousel/viewer/editor checks passed by user report on one Pop!_OS
GNOME 46 configuration (4K/200% plus 1080p/100%, app size 100%). Final physical
helper/hotplug/login/lock/suspend acceptance is deferred. Linux panel/About parity
and OCR accuracy improvements are deferred; the panel retains its symbolic icon.
