# Pasteroll 0.1.0-beta.3

**Prerelease. GNOME 46 Wayland support is experimental: physical desktop, login/lock
and suspend testing has not been performed.**

This release adds experimental native GNOME 46 Wayland support. Install the
Pasteroll Shell companion and optionally enable the administrator-installed input
helper for Ctrl+Shift+middle-click. Explicit keyboard mode uses Ctrl+Shift+Space.
The app includes clipboard history, carousel/search, partial paste, viewer, image
editor and settings through the native GNOME backend. Windows and X11 use their
existing backends.

## Packages

- `pasteroll-0.1.0-beta.3-windows-x64-setup.exe`
- `pasteroll-0.1.0-beta.3-windows-x64-portable.zip`
- `pasteroll-0.1.0-beta.3-linux-x86_64.tar.gz` (X11)
- `pasteroll_0.1.0-beta.3_amd64.deb` (X11)
- `pasteroll-0.1.0-beta.3-linux-x86_64-gnome-development.tar.gz`
- `pasteroll_0.1.0-beta.3_amd64-gnome-development.deb`
- `SHA256SUMS` covering every package above

The two Debian packages are alternatives, not coinstallable. The GNOME package
requires GNOME 46, glibc >=2.34, libevdev2 and systemd. Fresh install does not enable
the privileged helper. Follow the included GNOME guide before enabling it.

## Validation and limits

Automated GNOME sessions and synthetic kernel mouse input passed during development.
Windows workspace tests: 566 passed; WSL workspace tests: 577 passed (7 ignored each).
Exact packaged Linux binaries passed 30 synthetic GNOME mouse and 9 X11 runtime checks.
Package checks passed on Windows and Linux, including six actual per-user Windows
setup/reinstall/uninstall checks. These do not establish clean-machine Windows UAC
or physical GNOME desktop acceptance. No physical desktop testing is claimed.
Only GNOME 46 is accepted by the companion; KDE and other Wayland desktops, newer
GNOME versions, touchpads/tablets/combined input devices, Flatpak and Snap are outside
this release's native Wayland support. Windows builds are unsigned.

Checksums cover all six packages. The GNOME development filenames intentionally
identify experimental alternatives; choose the regular Linux packages for X11.

[GNOME setup guide](https://github.com/qpd-v/pasteroll/blob/v0.1.0-beta.3/extensions/INPUT_HELPER.md) ·
[Physical testing checklist](https://github.com/qpd-v/pasteroll/blob/v0.1.0-beta.3/docs/GNOME_ACCEPTANCE.md)
