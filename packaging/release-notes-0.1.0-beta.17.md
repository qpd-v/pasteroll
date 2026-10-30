# Pasteroll 0.1.0-beta.17 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Fixes "Download and install" refusing to update with "pasteroll-update exists and is not Pasteroll's update folder".
- No other changes since 0.1.0-beta.16.

## Fixed

- **"Download and install" could refuse to update.** After a successful update, the clean-up of the
  downloaded installer could stop halfway (Setup was still running) and leave its folder without the
  mark that tells Pasteroll the folder is its own. The next update then said "pasteroll-update exists
  and is not Pasteroll's update folder, so it was not touched" and did nothing. The mark is now removed
  last, and a leftover folder holding only Pasteroll's own update files is cleared and reused. A folder
  with anything else in it is still never touched.
- If you already see that message (copies before beta.17 cannot fix it themselves), delete the
  `pasteroll-update` folder inside your Pasteroll data folder once (it holds only an old installer), or
  install this version by hand.

## Downloads

- pasteroll-0.1.0-beta.17-windows-x64-setup.exe
- pasteroll-0.1.0-beta.17-windows-x64-portable.zip
- pasteroll-0.1.0-beta.17-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests (including new tests for the leftover folder
and for a folder whose installer is still in use); Windows packaging; privacy scans of all three
downloads; the upgrade fixture (upgrading from the beta.16 package keeps the encrypted history, key,
settings and presets); and the end-to-end test of Download and install against a local release server,
for a copy installed for one account and for a copy installed for all users (through the administrator helper).
The release signatures were checked with the keys built into the program before publishing.

The real-window self-test was not run for this release because it changed only the update clean-up
code. The About check was not run: the About window was not touched.

NOT TESTED: an update with a different administrator's credentials, Windows Defender or Controlled
Folder Access during an update, a clean-machine all-users (UAC) install, start-at-sign-in across a real
login, and the interactive uninstall data-deletion prompt.

No Linux code was built or tested for this release. The update check and Download and install are
Windows-only.
