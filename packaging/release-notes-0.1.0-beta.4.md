# Pasteroll 0.1.0-beta.4

Experimental prerelease for Windows, X11 and GNOME 46 Wayland.

## New and fixed

- Transparent roll branding. Windows tray single click opens history near the
  icon; double click opens Settings; right click keeps its menu. Windows About
  shows version/installation information; tooltip shows recording status.
- Freehand OCR areas in the regular expanded image viewer (Ctrl+E): draw up to
  16 outlines, Enter extracts, Ctrl+Z undoes, Delete clears and Escape cancels.
  Outside pixels are masked and the source image is unchanged.
- Explicitly copied OCR/viewer-selected text enters history, respecting pause,
  exclusions and deduplication. Returning from the viewer refreshes the existing
  carousel even when the background save finishes later; filters remain applied.
- GNOME 46 mixed-scale geometry fixes distinguish UI density from Shell stage
  coordinates. Large viewer/editor frames use bounded full-resolution strips.
  Upgrade the app and companion together: both require wire protocol 3. Log out
  and back in to reload the companion after installation.

## Downloads

- pasteroll-0.1.0-beta.4-windows-x64-setup.exe
- pasteroll-0.1.0-beta.4-windows-x64-portable.zip
- pasteroll-0.1.0-beta.4-linux-x86_64.tar.gz (X11)
- pasteroll_0.1.0-beta.4_amd64.deb (X11)
- pasteroll-0.1.0-beta.4-linux-x86_64-gnome-development.tar.gz
- pasteroll_0.1.0-beta.4_amd64-gnome-development.deb
- SHA256SUMS covering all six packages

The X11 and GNOME Debian packages are alternatives, not coinstallable. The GNOME
package needs GNOME 46, glibc 2.34+, libevdev2 and systemd. Its optional privileged
mouse helper stays disabled after a fresh install; keyboard mode uses
Ctrl+Shift+Space. See extensions/INPUT_HELPER.md for setup and permissions.

## Testing and known limitations

Mixed-scale carousel/viewer/editor tests passed by user report on Pop!_OS GNOME
46 with a 4K/200% built-in display and a 1080p/100% external display. Automated
platform, geometry, transport and package checks supplement that limited result.

Further user/manual testing is deferred. Physical mouse-helper input/hotplug,
lock/unlock, suspend/resume, autostart across login and full upgrade/removal
acceptance have not been completed for this final candidate. Windows all-users
UAC and interactive uninstall choices have not had final clean-machine manual
acceptance. Automated fixtures do not establish those results.

Linux OCR may return empty or garbled text; accuracy improvements are deferred
until the next version. Linux tray/panel/About/version interaction parity with
Windows is also deferred; Linux retains its existing interactions and GNOME
symbolic panel icon. Selection boundaries and source preservation still apply.
GNOME support remains experimental and version-46-only. Native COSMIC/KDE/newer
GNOME, Flatpak and Snap are not supported by this release. Windows is unsigned.

Upgrades preserve history, encryption keys and settings. Windows uninstall keeps
data unless deletion is explicitly chosen; Linux package removal retains data.
