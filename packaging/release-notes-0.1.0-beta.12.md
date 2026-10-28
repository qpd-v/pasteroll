# Pasteroll 0.1.0-beta.12 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- The Settings window is new: compact, two columns, and it follows the Windows light or dark mode.
- A multi-paste is now kept in history as one clip, with a small badge showing how many clips it joins.
- The Multi-paste button in the carousel is now an icon.

## New

- **A redesigned Settings window.** It now follows Windows' light or dark app mode (including the title
  bar, and it changes when you change the Windows setting). It is much more compact, about two thirds
  of its old height, so the Save and Cancel buttons are no longer cut off on a small or high-scaling
  screen, and it is laid out in two columns of cards. The settings and what they do are the same.
- **A multi-paste is kept in history.** When you paste several clips at once, or copy them with Ctrl+C
  or right-click Copy, the joined text is also added to history as one clip with a small badge showing
  how many clips went into it. It is not kept while recording is paused, follows the usual limits, and
  is encrypted like any clip. Older versions read these clips as ordinary text.
- **The Multi-paste button is an icon** (a checklist) with a tooltip, instead of the words.

## Downloads

- pasteroll-0.1.0-beta.12-windows-x64-setup.exe
- pasteroll-0.1.0-beta.12-windows-x64-portable.zip
- pasteroll-0.1.0-beta.12-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests (including new tests of the history entry and
badge for a multi-paste, the shapes the new Settings window draws, and text contrast in both of its
themes); Windows packaging; privacy scans of all three downloads; the About check; and the upgrade
fixture (upgrading from the beta.11 package keeps the encrypted history, key, settings and presets).
The release signatures were checked with the keys built into the program before publishing.

The real-window self-test (real hooks and clicks, including a real Windows Terminal window and a
real multi-paste that is then found in history with its badge) passed: 83 checks, 0 failed. Run again
with the Settings window checks on, the four Settings checks passed and the 9 overlay focus checks that
are already known to fail in that mode failed again (the other checks passed). Those Settings checks
change values and read them back; they do not click the new controls or look at the window. The new
Settings window's light and dark looks were checked from pictures of the window on this machine only
(one display, one scale), and clicking and hovering its buttons, check boxes and sliders, the Browse
button, and the switch when Windows changes between light and dark while it is open were not tried.

NOT TESTED: an update with a different administrator's credentials, Windows Defender or Controlled
Folder Access during an update, a clean-machine all-users (UAC) install, start-at-sign-in across a real
login, and the interactive uninstall data-deletion prompt.

No Linux package was built or released, and nothing was tried on a real Linux desktop. The shared code
changed (the Linux code compiles under WSL); treat Linux as untested until its catch-up release. The
update check and Download and install are Windows-only.
