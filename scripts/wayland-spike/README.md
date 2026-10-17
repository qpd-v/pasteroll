# GNOME Wayland phase-one experiment

This is research tooling, not a backend enabled in Pasteroll. The app, its five
crates, packages and version remain unchanged. Only synthetic text is used.

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

A compositor change or a separately configured device remapper could change
that result. Neither is installed or required by this work. Pointer polling
would allow an unwanted click to reach the target, so it is not used.
Phase two can still implement and test backend foundations while declaring
mouse activation unavailable and keeping keyboard experiments explicitly opt-in.

- GNOME extension anatomy and lifecycle: https://gjs.guide/extensions/overview/anatomy.html
- GNOME isolated/nested development guidance: https://gjs.guide/extensions/development/debugging.html
- Mutter 46 event routing: https://github.com/GNOME/mutter/blob/gnome-46/src/core/events.c
- Mutter 46 filter order: https://github.com/GNOME/mutter/blob/gnome-46/clutter/clutter/clutter-event.c
- Mutter 46 virtual input: https://github.com/GNOME/mutter/blob/gnome-46/src/backends/native/meta-virtual-input-device-native.c

The development guide warns that nesting alone is not full isolation; this
launcher additionally uses Linux namespaces and private buses/profiles. These
tests still do not establish physical GPU, real-device, multi-monitor, newer
GNOME or KDE compatibility.
