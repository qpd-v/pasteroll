# Isolated GNOME Wayland validation

## Native mouse helper suite

`run.sh --mouse` tests the real Pasteroll host input service with disposable
kernel evdev/uinput devices, a private logind fixture, stock headless GNOME and
the real app. Build both Linux binaries first. See
[INPUT_HELPER.md](../../extensions/INPUT_HELPER.md) for permissions, packaging
and validation limits. Test results are under `target-dev/wayland-mouse/`.
The driver reflects normal input into Clutter because this is not a physical
libinput seat; kernel button suppression and helper activation are exercised
without replacing the production service.

## Phase three native suite

`phase3.py` drives the real `pasteroll` binary with the GNOME Wayland backend
and native GTK4 clipboard clients. `run.sh --phase3` copies the freshly built
binary to the launcher's private `~/.local/bin/pasteroll`, installs the two
extensions only in the disposable profile, and runs in the existing private
mount, network and PID namespaces. It never attaches to WSLg or the host
display, session bus, clipboard or history.

Build and check with the sanitized Cargo wrapper, then run the native suite:

```sh
python scripts/wayland-spike/check.py --wsl cargo build --offline -p pasteroll-linux --bin pasteroll --example wayland-probe
bash scripts/wayland-spike/run.sh --phase3   # in WSL, from this checkout
```

On a Linux checkout, run `bash scripts/wayland-spike/run.sh --phase3` from the
repository root. Outputs are under the ignored `target-dev/wayland-phase3/`.
Sanitized process logs, `results.json` and `screenshots.json` are at that output
root; PNG evidence is retained in timestamped take directories. A failed required
check returns a nonzero process status. A partial run is never a full-app pass.

The suite covers Bridge2 handshake/capabilities, capture order, exclusions and
pause, explicit `--wayland-keyboard` activation, carousel/search/drag-selected
partial paste, fixed paste chords and terminal warning, wrong-target and held
modifier rejection, expanded viewer scroll/find, image editor save with the
original history entry preserved, capture policies, settings confirmation and
first-run acknowledgement, then graceful stop, extension disable, owner loss
and watchdog release. Settings changes and notice acknowledgement are asserted
through their native controls; `first-run.txt` must remain absent until the
companion reports shell-action kind 25 from the real acknowledgement control.

The test driver is a private injector under `scripts/wayland-spike/extension/`,
never part of the production companion. The fixture has two GTK4 targets so a
foreground change can be tested. The suite uses one software-rendered 1280x800
monitor at scale 1.0 and GNOME Shell 46. Mixed DPI, physical devices, GDM/lock
screen, newer GNOME, KDE and multi-monitor behavior remain untested.

Phase two remains available with `run.sh --phase2`; its probe now exercises the
Bridge2 wire version, generation-preserving `InputActive`/`Show`, viewer
transfer, suspend, and the existing clipboard regression checks.

The production backend now uses the actual app controller, encrypted store and
renderer through the companion. These research scripts stay separate from the
production extension. Tests use only synthetic text, file lists and PNGs; the
published packages and version are unchanged in this phase.

## Historical phase-one experiment

## Result (GNOME Shell 46.0, Ubuntu 24.04)

The real headless Mutter Wayland compositor and a native GTK 3 Wayland client
demonstrate clipboard monitoring, an overlay at the pointer, modal wheel input,
and a real selected-text paste into the native client. XWayland is disabled.

**The original Ctrl+Shift+middle-click activation fails over the native client.**
The native client receives that unclaimed middle-click. The same chord is seen
when the pointer is over the Shell desktop. Both normal stage event handling
and an appended Clutter event filter were investigated. Mutter's existing
filter processes native-client input before the extension's filter sees it.

The experiment uses **Ctrl+Shift+Space as a diagnostic only** to exercise the
remaining workflow. This does not approve or implement a replacement product
gesture. Full mouse-workflow feasibility is unresolved. Do not describe this
as completed Wayland support or promise the same result on another GNOME version.

The asynchronous clipboard reader discards callbacks from superseded selections.
The wheel probe counts discrete events once, ignoring their smooth companions.
The paste probe releases the modal grab, replays the click, validates focus and
held modifiers, then injects Ctrl+V through Mutter's virtual input device.

## Run

In WSL Ubuntu, from the repository root:

```sh
bash scripts/wayland-spike/run.sh
```

Requires existing `sudo -n`, `unshare`, `setpriv`, Python 3 with PyGObject,
GNOME Shell 46, GTK 3 introspection, GJS, dconf, and `gnome-extensions`.
The launcher does not install or download anything. Test prerequisites were
installed separately with Ubuntu apt, without GDM or a desktop-session manager.

The launcher creates private mount, network and PID namespaces, hides `/tmp`
and `/run`, clears the inherited environment, drops back to the invoking UID,
and creates a disposable home, runtime directory, session bus and empty system
bus. It never attaches to WSLg, another X/Wayland display, the real session bus,
or the live clipboard. A headless virtual monitor uses software rendering.
Test services and profiles are torn down when the namespace exits.

The extension installs only into that temporary profile. Its D-Bus `Input`
method is an intentional test injector on the isolated bus, **not a production
interface**. Never install this experiment into a normal desktop session.

Results, sanitized logs, timestamped PNG frames and JSON evidence are written
under the ignored `target-dev/wayland-phase1/`. Earlier frame takes are preserved.
`results.json` separates required diagnostic checks from the failed mouse gate;
a zero launcher status means the diagnostic harness completed, not that full
mouse parity passed. Inspect `full_mouse_workflow_passed` explicitly.

On Windows, with the existing FFmpeg and FFprobe on PATH:

```sh
python scripts/wayland-spike/export-proof.py
```

The labeled MP4 follows captured frame timestamps. Encoding repeated frames at
24 fps does not mean the compositor was captured at 24 fps. The exporter decodes
the full output and checks dimensions, duration, frame count and absence of audio.
This proof is separate from the approved marketing demos.

## Phase boundary

No Pasteroll controller, renderer, storage, clipboard backend or platform trait
is integrated yet. No production security or desktop compatibility claim is
made. The prototype has no history persistence, image/file transfer, search,
viewer/editor, per-app exclusion or terminal-warning implementation.

Before phase two, decide how to address native-window mouse activation:
investigate another compositor integration, or explicitly approve a GNOME
keyboard fallback. Do not silently substitute pointer polling: it cannot prevent
the original middle-click from also acting in the target application.

## References

### Mouse follow-up before phase two

The corresponding Clutter event-filter API was also inspected in upstream
GNOME 48, 49, 50 and main. No earlier-priority mouse-shortcut registration API
was found. This is source evidence, not a runtime test of those versions.
GNOME's maintainer explains the same native-window routing restriction here:
https://discourse.gnome.org/t/how-to-bind-modifier-mousebutton-in-gjs/3743

These were the original phase-one/two findings. Those probes did not install a
compositor change or device remapper. The later Pasteroll-owned host helper is
now tested separately by `--mouse`. Pointer polling remains unsuitable because
it lets the original click reach the target.

- GNOME extension anatomy and lifecycle: https://gjs.guide/extensions/overview/anatomy.html
- GNOME isolated/nested development guidance: https://gjs.guide/extensions/development/debugging.html
- Mutter 46 event routing: https://github.com/GNOME/mutter/blob/gnome-46/src/core/events.c
- Mutter 46 filter order: https://github.com/GNOME/mutter/blob/gnome-46/clutter/clutter/clutter-event.c
- Mutter 46 virtual input: https://github.com/GNOME/mutter/blob/gnome-46/src/backends/native/meta-virtual-input-device-native.c

The development guide warns that nesting alone is not full isolation; this
launcher additionally uses Linux namespaces and private buses/profiles. These
tests still do not establish physical GPU, real-device, multi-monitor, newer
GNOME or KDE compatibility.
# Rootless geometry checks

On native Linux with bubblewrap/user namespaces available, the keyboard suites
can also run with `bash scripts/wayland-spike/run-rootless.sh --phase2` (or
`--phase3`). Build the requested app/probe under `target-dev/linux` first, as
described below. The rootless launcher hides host home/runtime/device trees and
uses private network, PID, user and mount namespaces. Namespace UID 0 is only the
invoking host user; it is not host administrator access. It does not support the
privileged mouse-helper suite.

`PASTEROLL_GEOMETRY_LAYOUT=physical` or `logical` with `--phase2` configures only
the isolated virtual monitor to 3840x2160 at 200%, verifies the independent UI and
stage scales, and checks a full-resolution tiled viewer and rendered pixels on
both sides of its strip seam. No live GNOME settings or clipboard are accessed.
These synthetic results do not replace physical multi-monitor acceptance.
For an exact staged binary, set `PASTEROLL_TEST_APP_BINARY` to its absolute path
inside this checkout when running `--phase3`; the harness verifies its SHA256
against the private installed copy before starting it.
