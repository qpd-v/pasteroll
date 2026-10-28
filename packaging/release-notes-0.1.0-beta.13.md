# Pasteroll 0.1.0-beta.13 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Updating has its own window now, with a progress bar while it downloads and a Cancel button.
- The update window follows the Windows light or dark mode and looks like the new Settings window.
- Far fewer notifications: none while checking or updating, and one announcement per new version.

## New

- **An update window.** Checking for updates, what is new in the version, the download, installing and
  any problem all happen in one window instead of a stock dialog and a series of notifications. While
  it downloads it shows a progress bar with the size ("3.1 of 4.7 MB") and a percentage; the steps
  after that (checking the signature, installing, waiting for Windows' administrator permission for an
  all-users copy) show a moving bar. A download can be cancelled with the Cancel button, the close button
  or Esc; once installing has started it cannot be stopped. If something goes wrong the window says what
  and offers **Try again**. Enter presses **Not now** on the first page, so a stray key cannot start an
  install. It follows Windows' light or dark mode, including the title bar.
- **Far fewer notifications.** Checking and updating no longer make any. **Check for updates now** (tray
  menu or Settings) shows its answer in the window too. The notification icon announces a new version
  once, and clicking that notification opens the update window. After the restart there is one short
  "updated" line.

## Downloads

- pasteroll-0.1.0-beta.13-windows-x64-setup.exe
- pasteroll-0.1.0-beta.13-windows-x64-portable.zip
- pasteroll-0.1.0-beta.13-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests (including new tests that every page of the
update window fits and its buttons do not overlap, the size and percent text, that only a download can be
cancelled, and that a cancelled download stops part-way); Windows packaging; privacy scans of all three
downloads; the About check; and the upgrade fixture (upgrading from the beta.12 package keeps the
encrypted history, key, settings and presets). An end-to-end test of Download and install against a local
test server with a throwaway key passed all 47 checks. It now also drives the real update window: the
window offers the version with its plain lines, shows the download and the steps after it, ends on
"restarting" with the program file replaced, and reports a file that is not what was signed without
changing anything; the earlier checks (a Setup install, a portable replace, a wrong key, a changed
installer, manifest or note, an old genuine note, and a Setup that fails or crashes leaving the previous
Pasteroll running) still pass. The release signatures were checked with the keys built into the program
before publishing.

The real-window self-test (real hooks and clicks, including a real Windows Terminal window) passed:
83 checks, 0 failed (a first run showed one check voided by keystrokes that were not the test's own; the
repeat was clean). Run again with the Settings window checks on, the four Settings checks passed and the
9 overlay focus checks that are already known to fail in that mode failed again (the other checks passed).

Not tried: clicking and keyboard use of the update window by hand, the moving bar's animation live,
cancelling a real download from GitHub, the update window for an all-users copy (the same install code runs,
but the three Windows administrator prompts were not driven through the window), and an update through
the real GitHub release (the update window is first used by this version, when it looks for a later one).
Its look was checked from pictures of the window on this machine only, in both themes.

NOT TESTED: an update with a different administrator's credentials, Windows Defender or Controlled
Folder Access during an update, a clean-machine all-users (UAC) install, start-at-sign-in across a real
login, and the interactive uninstall data-deletion prompt.

No Linux package was built or released, and nothing was tried on a real Linux desktop. The Linux code
was not touched by this release. The update check and Download and install are Windows-only.
