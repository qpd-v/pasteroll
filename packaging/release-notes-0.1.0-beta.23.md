# Pasteroll 0.1.0-beta.23 (Windows and Linux)

Experimental prerelease for Windows, Linux X11 and GNOME 46 Wayland.

## In short

- Zoom controls use a centered icon toolbar with one read-only percentage display.
- Ctrl+click an inactive clip to select it together with the active clip and start Multi-paste.
- The Multi-paste button starts with no clips selected.
- Unselected clips have no active border or tint while Multi-paste is on.
- Existing history, encryption keys, settings and rolls are kept.

## Changes

- **Zoom toolbar.** Zoom out, Zoom in, Fit and Actual size use centered icons in one rounded toolbar.
  Hover tooltips name each action and its shortcut. The current zoom is shown as a separate,
  centered read-only percentage; there is no second percentage button.
- **Ctrl+click starts a selection.** Outside Multi-paste, Ctrl+clicking an inactive clip selects
  the active clip and the clicked clip, and enables Multi-paste. This also applies to inactive
  image clips. Ctrl+clicking the current image with no selection still opens Advanced Image Paste.
- **The button starts blank.** Clicking the Multi-paste button clears the selection. Clips are
  highlighted only when selected; the active clip has no special border or tint while unselected.

## Downloads

- pasteroll-0.1.0-beta.23-windows-x64-setup.exe
- pasteroll-0.1.0-beta.23-windows-x64-portable.zip
- pasteroll-0.1.0-beta.23-linux-x86_64.tar.gz
- pasteroll_0.1.0-beta.23_amd64.deb
- pasteroll-0.1.0-beta.23-linux-x86_64-gnome-development.tar.gz
- pasteroll_0.1.0-beta.23_amd64-gnome-development.deb

The Windows updater also uses the bare executable and signed update manifest. SHA256SUMS covers
all seven program assets. Windows executables are not Authenticode-signed; update manifests carry
the Pasteroll release-key signature.

## Verification and known limitations

No automated or interactive test suites were run for this release, at the maintainer's request.
Release compilation, package creation, checksums, signature verification and delivery verification
are performed. Functional and visual acceptance is left for manual testing.

Live ChatGPT/document-editor compatibility and physical GNOME helper, login/lock/suspend/mixed-DPI
behavior remain unverified. Image batch limits and temporary attachment retention are unchanged
from beta.22. See PRIVACY.md and IMAGE_FEATURES_TESTING.md.

Linux updates manually. GNOME requires GNOME 46 and the matching protocol-4 companion; reload the
companion after upgrading. X11 and GNOME Debian packages are alternatives.
