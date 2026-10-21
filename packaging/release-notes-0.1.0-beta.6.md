# Pasteroll 0.1.0-beta.6 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

This is a fix for [0.1.0-beta.5](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.5).
Nothing else changed.

## Fixed

- **Advanced Image Paste now opens in terminals.** In beta.5, holding Ctrl and clicking a Windows
  Terminal window (and so PowerShell, Claude Code, opencode and anything else run inside it) did
  nothing. Windows Terminal reports its whole window as a title bar, so the click was refused as
  "not on a client area". A click is now accepted when it lies inside the window's client area; a
  standard title bar, borders and caption buttons are still refused. As a side effect, a plain
  click-to-paste into Windows Terminal works too.
- **The editor no longer needs Ctrl released quickly.** The click used to wait until every key was
  up, for at most one second, before it did anything. A click that only opens the editor types
  nothing yet, so it no longer waits and opens even if you keep holding Ctrl.

## Downloads

- pasteroll-0.1.0-beta.6-windows-x64-setup.exe
- pasteroll-0.1.0-beta.6-windows-x64-portable.zip
- SHA256SUMS covering these two packages

Upgrades preserve history, encryption keys and settings. Windows uninstall keeps data unless
deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for new
programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated: the Windows workspace tests, Windows packaging, About and upgrade fixtures (upgrading
from the beta.5 package), and a real-window self-test (real hooks and clicks, including a real
Windows Terminal window) all passed. Advanced Image Paste in Windows Terminal was also tried by
hand on one Windows 11 computer.

NOT TESTED: a clean-machine all-users (UAC) install, start-at-sign-in across a real login, and the
interactive uninstall data-deletion prompt. Automated fixtures do not establish those results.
Advanced Image Paste pastes a picture only into programs that accept one; a plain text box
that does not will receive nothing, so choose **Path** or **File name** there.

No Linux code was built or tested for this release. The shared code changed, so Linux behavior
should be treated as untested until its catch-up release.
