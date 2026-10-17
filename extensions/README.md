# GNOME bridge development foundations

The companion in `pasteroll@pasteroll.com/` is a development component for GNOME
46 Wayland. It is not installed by the current packages. The normal `run` command
still uses the existing X11 backend; full controller integration is a later phase.

## What exists

- Session detection distinguishes a Wayland desktop from its optional XWayland DISPLAY.
- Versioned local D-Bus protocol, with the Rust client pinned to the current
  GNOME Shell bus owner and verified to run as the same UID.
- The companion accepts only the owning connection of an explicitly installed
  Pasteroll executable: `/usr/bin/pasteroll`, `/usr/local/bin/pasteroll`, or
  `~/.local/bin/pasteroll`. Symlinks are resolved. Unapproved programs cannot
  attach just by knowing the bus name. This does not protect against a same-user
  attacker replacing the installed executable; the existing threat model still applies.
- UTF-8 text, local file references and PNG clipboard transfers, with explicit
  size bounds, stale-selection checks, sensitive-marker filtering and own-write exclusion.
- Real renderer BGRA frames can be presented on a native Shell surface; input
  is routed as generation-tagged controller events.
- Owner loss, extension disable, stale generations, queue overflow and a stalled
  client release the overlay. The application is never allowed to leave a modal
  grab indefinitely merely by ceasing to poll.

No arbitrary keystroke injection, shell command execution, network endpoint,
remote input interface or clipboard history persistence is exported by the
companion. The separate isolated spike contains a test injector; it is not part
of this companion and must never be installed into a real desktop.

## Mouse integration finding

The original mouse chord works on the Shell desktop in the spike, but GNOME 46
routes native-window clicks before normal extension handlers can consume them.
An appended Clutter filter also runs too late. Upstream branches 48–50 and main
were checked for a supported earlier-priority mouse shortcut API; none was found.
Those newer branches were inspected, not runtime-tested.

The bridge reports **mouse activation unavailable**. It does not implement a
polling substitute or require a privileged global-input reader. Keyboard input
can be armed only by an authenticated client making an explicit opt-in request.
The normal app has not adopted a different default gesture.

## Diagnostics and tests

`pasteroll wayland-check` checks session type and companion protocol availability
without opening the history or reading clipboard content. It explains that the
full controller integration is not enabled yet.

From the repository root, with the existing WSL test prerequisites:

```sh
python scripts/wayland-spike/check.py --wsl cargo build --offline -q -p pasteroll-linux --example wayland-probe
wsl -d Ubuntu -- bash scripts/wayland-spike/run.sh --phase2
```

The launcher copies the compiled synthetic probe to its disposable private
`~/.local/bin/pasteroll`, enables the companion and separate test injector only
inside that profile, and uses a native GTK4 MIME provider/consumer. It uses no
live display, clipboard, history or user D-Bus service. Results and sanitized
logs are under the ignored `target-dev/wayland-phase2/`.

Protocol/session tests also compile on Windows:

```sh
python scripts/wayland-spike/check.py cargo test --offline -q --workspace
python scripts/wayland-spike/check.py --wsl cargo test --offline -q --workspace --exclude pasteroll-win
```

The integration probe is an example binary, not included in release packages.
Its fixed synthetic fixtures and namespace marker prevent accidental live use.

## Current limits

GNOME 46 is the only declared Shell version. The clipboard protocol caps a clip
at 64 MiB, and this initial frame transport caps a BGRA frame at 16 MiB. Frame
presentation currently uses D-Bus byte arrays; high-resolution/fullscreen
throughput requires measurement and likely shared-memory transport before it
can be advertised. Image dimensions are independently checked by the core decoder.

MIME transfer and source-app reporting do not establish compatibility with every
application. Source-app identity is best effort. Full carousel behavior, the
expanded viewer/editor, paste targeting, terminal warnings, per-app policies,
tray/settings/autostart, fractional scaling, installation and store packaging
remain separate integration and validation work.
