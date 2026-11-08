# Pasteroll 0.1.0-beta.20 (Windows and Linux)

Experimental prerelease. This release makes image editing reachable directly from the carousel and
hardens update downloads against transient GitHub gateway failures.

## In short

- Ctrl+click an image clip in the carousel opens Advanced Image Paste.
- Ctrl+click text and file clips still chooses them for Multi-paste.
- Update downloads retry transient GitHub 502, 503 and 504 responses.

## Changes

- **Image clips open the editor from the carousel.** Ctrl+clicking an image row or the large image
  card now makes that picture current and opens Advanced Image Paste. The configured image modifier
  is used, so Ctrl, Shift or Alt remains consistent with destination clicks.
- **Text and file clips keep Multi-paste.** Ctrl+clicking a text or file clip still chooses it, and
  Ctrl+Shift+click still chooses a range. Multi-paste keyboard controls and the Multi-paste button
  are unchanged. Pictures remain excluded from joined text.
- **Clearer in-app guidance.** The one-clip Multi-paste hint and join-card error now say that
  Ctrl+click applies to text and file clips.
- **Transient update downloads retry.** Windows updater downloads now retry HTTP 502, 503 and 504
  responses up to four times with a fresh query key and a short pause. The existing signature and
  checksum checks are unchanged.

Existing history, encryption keys, settings and rolls are kept. Linux packages continue to update
manually.

## Downloads

- `pasteroll-0.1.0-beta.20-windows-x64-setup.exe`
- `pasteroll-0.1.0-beta.20-windows-x64-portable.zip`
- `pasteroll-0.1.0-beta.20-linux-x86_64.tar.gz` and `pasteroll_0.1.0-beta.20_amd64.deb` (X11)
- `pasteroll-0.1.0-beta.20-linux-x86_64-gnome-development.tar.gz` and
  `pasteroll_0.1.0-beta.20_amd64-gnome-development.deb` (experimental GNOME 46)
- `update-manifest.txt`, its signature, and one `SHA256SUMS` for the seven program packages

Windows builds are unsigned, so SmartScreen may warn for a new program: check the checksum, then
choose More info, Run anyway.

## Testing and known limitations

Automated checks used disposable profiles and synthetic clipboard data. The app suites cover
Ctrl+click on image clips and text clips, multi-paste, Advanced Image Paste, and updater retry
behavior. Packaging and release verification cover both Windows and Linux artifacts.

Physical GNOME helper devices, login/lock screens, suspend and mixed-DPI layouts remain unverified.
The X11 and GNOME Debian packages are alternatives; the GNOME package is experimental and requires
GNOME 46 with the matched protocol-4 companion.

