# Pasteroll 0.1.0-beta.11 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Paste several clips at once: press the new Multi-paste button, click each clip, then click where it should go.
- Ctrl+click and Ctrl+Shift+click also choose clips. Ctrl+C or right-click Copy copies them without pasting.
- Settings has a Save pictures in this folder choice, used by Paste Path and the image editor.

## New

- **Paste several clips at once** (Complete look). Press the **Multi-paste** button beside the filter
  and click each clip you want (Shift+click chooses a range), or hold Ctrl and click (Ctrl+Shift+click
  for a range) without the button. Chosen clips show a tick and the footer counts them. Then click where
  they should go, or press Enter, and they are pasted one per line in the order you chose them. Ctrl+C,
  or right-click and Copy, puts them on the clipboard without pasting anything. Pictures cannot be
  joined into text and are left out; one chosen picture is still pasted as a picture. Esc forgets the
  choice. Multi-paste starts off each time Pasteroll opens, and No Bloat does not have it.
- **Save pictures in this folder.** Settings has a new choice (with Browse and a Desktop button) for
  where Paste Path and the image editor save pictures instead of the Desktop. A folder you pick inside
  the editor is still remembered until you change the folder in Settings. Clicking the desktop with a
  picture still saves to the desktop. A folder that does not exist yet is created, readable only by you.

## Fixed

- When several clips are chosen and you click a terminal, changing what is chosen asks again before the
  multi-line paste.

## Downloads

- pasteroll-0.1.0-beta.11-windows-x64-setup.exe
- pasteroll-0.1.0-beta.11-windows-x64-portable.zip
- pasteroll-0.1.0-beta.11-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests (including new tests of choosing and pasting
several clips, the Multi-paste button and the picture folder); Windows packaging; privacy scans of all
three downloads; the About check; and the upgrade fixture (upgrading from the beta.10 package keeps the
encrypted history, key, settings and presets). The release signatures were checked with the keys built
into the program before publishing.

The real-window self-test (real hooks and clicks, including a real Windows Terminal window and a
Ctrl+click / Ctrl+Shift+click / right-click run through several clips) passed: 82 checks, 0 failed. Run
again with the Settings window checks on, the four Settings checks passed and the 9 overlay focus checks
that are already known to fail in that mode failed again (the other checks passed). Those Settings
checks do not cover the new picture-folder row, which was tested by unit tests only, not by clicking it
in a real Settings window. The Multi-paste button is covered by unit tests; its look and a real click on
it were not checked in the real-window self-test.

NOT TESTED: an update that uses the signed note from a copy older than beta.10, an update with a
different administrator's credentials, Windows Defender or Controlled Folder Access during an update, a
clean-machine all-users (UAC) install, start-at-sign-in across a real login, and the interactive
uninstall data-deletion prompt.

No Linux package was built or released, and nothing was tried on a real Linux desktop. The Linux code in
the repository changed too (it shares the new clip choosing and picture folder); it was only checked in
an isolated X11 session under WSL, so treat Linux as untested until its catch-up release. The update
check and Download and install are Windows-only.
