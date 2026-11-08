# Linux catch-up

This branch brings the Linux packages from beta.4 to the shared beta.19 source used by Windows.
The app, controller, renderer, store, configuration, image editor and MCP server are shared;
the Linux backends provide windows, input, tray, settings and packaging.

## What was added

| Area | X11 | GNOME 46 Wayland |
|---|---|---|
| No Bloat | shared | shared |
| Multi-paste | shared, with join panel | shared, with join panel |
| Favorite tiles and colors | shared, `pin-colors.txt` | shared, `pin-colors.txt` |
| Rolls | Ctrl+Left/Right, tilt wheel, settings | companion shortcuts, tilt wheel, settings |
| Pin open (Ctrl+K) | persistent X11 overlay | companion keeps overlay reactive |
| Drag carousel | window moved by pointer | companion actor moved by pointer |
| Agent brain (MCP) | CLI and stdio server | CLI and stdio server |
| Settings extras | native rolls/appearance page | native GNOME dialog |
| Upgrade from beta.4 | same history, key and config | same history, key and config |

`Ctrl+P` remains accepted for Favorite while `Ctrl+F` is the documented key.

## Build and verify

From a Linux checkout (or Ubuntu/WSL with the build prerequisites):

```sh
bash scripts/build-linux-candidates.sh
```

This builds the static X11 archive and `.deb`, the GNOME development archive and `.deb`,
runs both packaging suites, scans all four artifacts and writes `dist/linux/SHA256SUMS`.
It never installs, publishes or contacts a remote service.

The isolated runtime checks can be run separately:

```sh
PASTEROLL_EXE="$PWD/target/x86_64-unknown-linux-musl/release/pasteroll" bash scripts/test-linux-x11.sh
PASTEROLL_EXE="$PWD/target/x86_64-unknown-linux-musl/release/pasteroll" bash scripts/test-linux-x11.sh multi
PASTEROLL_TEST_APP_BINARY="$PWD/target/x86_64-unknown-linux-musl/release/pasteroll" bash scripts/wayland-spike/run-rootless.sh --phase3
```

The first two use a private Xvfb display, a temporary home and synthetic clips. The third
uses a private mount/network/PID namespace, private session bus, headless GNOME 46 and native
GTK4 targets. None of them touch the live desktop, clipboard or history.

## Release gates

`.github/workflows/linux.yml` runs the workspace tests, both package builds and packaging
checks, the isolated X11 suites and the headless GNOME suite. These automated checks are the
release gate for this beta. Physical-device, login/lock, suspend and mixed-monitor behavior
are documented limitations rather than manual gates; users report problems on GitHub.

The GNOME package stays labeled `gnome-development`, requires GNOME 46, and must be upgraded
with the matching protocol-4 companion.
