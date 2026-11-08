# Pasteroll 0.1.0-beta.19 (Windows and Linux)

Experimental prerelease. Windows adds the local agent brain; Linux catches up from 0.1.0-beta.4
with the shared beta.19 features on X11 and experimental GNOME 46 Wayland.

## In short

- Linux catches up: rolls, favorite tiles, pin-open, dragging, No Bloat and multi-paste.
- A local read-only MCP server lets Claude Code, Codex and OpenCode read shared rolls.
- The GNOME 46 package uses the matched protocol-4 Shell companion.
- Windows keeps its signed updater; Linux packages update by hand.
- Ctrl+F is Favorite everywhere; Ctrl+P still works for Linux beta.4 upgraders.

## New

- **Agent brain (MCP).** A local, read-only MCP server, `pasteroll mcp`, lets Claude Code, Codex
  and OpenCode read the rolls you choose. It is part of the same program: no companion download,
  no port and no network use. Turn it on with `pasteroll brain on`, share rolls with
  `pasteroll brain add Work`, and print the exact entries for all three harnesses with
  `pasteroll mcp-config`. The tools list, read and search the shared rolls; nothing writes,
  copies, pastes or deletes. Images are off by default.
- **Linux catch-up.** The X11 and experimental GNOME 46 Wayland packages now include No Bloat,
  multi-paste with join options, the image editor, favorite tiles and colours, rolls, dragging
  the carousel, pinning it open, the Rolls / appearance settings page and the agent brain.
- **GNOME bridge protocol 4.** The app and its Shell companion must be upgraded together. The
  package carries the matching extension.
- `Ctrl+F` is Favorite everywhere. `Ctrl+P` still works for people upgrading from Linux beta.4.

## Downloads

- `pasteroll-0.1.0-beta.19-windows-x64-setup.exe`
- `pasteroll-0.1.0-beta.19-windows-x64-portable.zip`
- `pasteroll-0.1.0-beta.19-linux-x86_64.tar.gz` and `pasteroll_0.1.0-beta.19_amd64.deb` (X11)
- `pasteroll-0.1.0-beta.19-linux-x86_64-gnome-development.tar.gz` and
  `pasteroll_0.1.0-beta.19_amd64-gnome-development.deb` (experimental GNOME 46)
- `update-manifest.txt`, its signature, and one `SHA256SUMS` for the seven program packages

Windows upgrades keep history, keys and settings. Linux beta.4 history, keys and settings are
read without conversion. The X11 and GNOME Debian packages are alternatives. Linux updates are
installed with the archive installer or the package manager; the in-app updater is Windows-only.
Windows builds are unsigned, so SmartScreen warns for new programs: check the checksum, then
choose More info, Run anyway.

## Testing and known limitations

Automated checks used disposable profiles and synthetic clipboard data. On Windows: the workspace
tests, packaging, the About check, the beta.18 upgrade fixture, and privacy scans of every
artifact. The real-window probe was not run: the only shared platform change adds a default no-op
method that Windows does not implement, so Windows input and window behavior are unchanged.

On Linux: 877 Rust tests, both packaging suites (8 X11 and 7 GNOME checks, including real dpkg
install, upgrade, remove and purge), 13 isolated X11 runtime checks, 54 headless GNOME 46 checks,
and clean scans of all four Linux artifacts with verified checksums. The X11 suite covers
multi-paste, the right-click menu, image editing and Paste Path, No Bloat, Settings, the Rolls /
appearance page, dragging and pinning. The GNOME suite covers the real bridge handshake, capture,
activation, paste, partial paste, viewer, editor, settings confirmation and shutdown.

NOT TESTED: physical GNOME helper devices, login/lock and suspend, mixed-DPI and multi-monitor
layouts, a clean-machine Windows all-users install, start at sign-in across a real login, and the
interactive uninstall data-deletion prompt. Linux OCR quality remains inconsistent. GNOME remains
experimental and supports GNOME 46 only. Report problems at
https://github.com/qpd-v/pasteroll/issues.
