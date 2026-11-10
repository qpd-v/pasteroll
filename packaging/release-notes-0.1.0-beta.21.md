# Pasteroll 0.1.0-beta.21 (Windows and Linux)

Experimental prerelease. This release makes the Windows updater recover when GitHub's normal release
download URL fails.

## In short

- Update downloads recover through GitHub's API when the normal link fails.
- A verified mirror on pasteroll.com is used if both GitHub routes fail.
- History, encryption keys, settings and rolls are kept.

## Changes

- **More reliable update downloads.** The Windows updater still tries GitHub's normal release URL first.
  If that fails, it looks up the exact asset name in the public GitHub release API and downloads through
  the asset endpoint. If the API also fails, it downloads the same signed asset from
  `https://pasteroll.com/downloads/`.
- **Verification is unchanged.** The signed update manifest, release-key signature and per-file SHA-256
  checks still decide whether anything is installed. The fallback only changes where the bytes come from.
- **No clipboard behavior changes.** Image editing, multi-paste, rolls, favorites and the MCP agent brain
  are unchanged from beta.20.
- The release process now records that the six packages and update manifest must be mirrored on the
  website alongside the GitHub release.

Existing history, encryption keys, settings and rolls are kept. Linux packages continue to update
manually.

## Downloads

- `pasteroll-0.1.0-beta.21-windows-x64-setup.exe`
- `pasteroll-0.1.0-beta.21-windows-x64-portable.zip`
- `pasteroll-0.1.0-beta.21-linux-x86_64.tar.gz` and `pasteroll_0.1.0-beta.21_amd64.deb` (X11)
- `pasteroll-0.1.0-beta.21-linux-x86_64-gnome-development.tar.gz` and
  `pasteroll_0.1.0-beta.21_amd64-gnome-development.deb` (experimental GNOME 46)
- `update-manifest.txt`, its signature, and one `SHA256SUMS` for the seven program packages

Windows builds are unsigned, so SmartScreen may warn for a new program: check the checksum, then
choose More info, Run anyway.

## Testing and known limitations

The updater tests cover the normal GitHub download, a deliberately broken normal URL recovered through
GitHub's release API, and a second failure recovered from the pasteroll.com mirror. The live fallback
tests downloaded and hashed the current Windows installer. The complete Windows test suite and the
Linux packaging suites must pass for the final artifacts.

Physical GNOME helper devices, login/lock screens, suspend and mixed-DPI layouts remain unverified.
The X11 and GNOME Debian packages are alternatives; the GNOME package is experimental and requires
GNOME 46 with the matched protocol-4 companion.

