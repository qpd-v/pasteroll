# Pasteroll 0.1.0-beta.19 for Linux (X11 and experimental GNOME 46 Wayland)

Linux catches up from `0.1.0-beta.4` to the shared feature set in Windows `0.1.0-beta.19`.
Both Linux package families are built from the same source and controller as Windows.

## New since Linux beta.4

- **No Bloat** look: only the current clip, with every keyboard feature still available.
- **Multi-paste**: choose several text clips and paste or copy them together, with the join options card.
- **Favorite tiles**: favorites are shown as tiles above the list, with the same colors and `pin-colors.txt` storage as Windows.
- **Rolls**: LEFT and RIGHT rolls, custom rolls, Most used and Images, configurable roll keys and tilt-wheel actions.
- **Pin it open** (`Ctrl+K`): the carousel stays above other windows while the keyboard and wheel return to the application underneath.
- **Drag the carousel** by an empty margin.
- **Advanced image paste and editor** on X11, including the shared editor, presets and Save as work.
- **Agent brain (MCP)**: `pasteroll brain`, `pasteroll mcp` and `pasteroll mcp-config`, reading the rolls you share, locally and read-only.
- **Settings, tray and panel additions**: open history, About/version, rolls and appearance, favorite colors, roll bindings and clear history.
- `Ctrl+F` is Favorite. `Ctrl+P` still works for people upgrading from beta.4.

Linux now writes the same configuration file, history format and MCP transport as Windows.
An existing beta.4 history, key file and settings are read without conversion.

## Packages

- `pasteroll-0.1.0-beta.19-linux-x86_64.tar.gz` and `pasteroll_0.1.0-beta.19_amd64.deb` (X11)
- `pasteroll-0.1.0-beta.19-linux-x86_64-gnome-development.tar.gz` and
  `pasteroll_0.1.0-beta.19_amd64-gnome-development.deb` (experimental GNOME 46 Wayland)
- `SHA256SUMS` covering all four

The X11 and GNOME Debian packages are alternatives, not coinstallable. The GNOME companion
and app must both use protocol 4; reload the extension after upgrading.

## Testing status

Automated checks used only private displays, disposable profiles and synthetic clipboard data:
Rust workspace tests (including new roll-binding and preferences tests), archive and Debian
packaging tests against real `dpkg`, isolated X11 runtime checks, and the headless GNOME 46
native suite. The X11 suite covers multi-paste, the right-click menu, image editing and Paste
Path, No Bloat, the Settings window, dragging and pinning open. The GNOME suite covers the real
bridge handshake, capture, activation, paste, partial paste, viewer, image editor, settings
confirmation, pause/exclusion and shutdown.

Not verified by these checks: physical mouse devices on GNOME Wayland, login/lock and suspend,
mixed-DPI or multi-monitor layouts, a clean machine install outside the test roots, and
desktop-specific tray behavior on every distribution. Report problems at
https://github.com/qpd-v/pasteroll/issues.

The GNOME backend remains experimental and supports GNOME 46 only. Other Wayland compositors
are not supported. X11 needs an X11 or XWayland session.
