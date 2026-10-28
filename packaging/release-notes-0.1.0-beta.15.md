# Pasteroll 0.1.0-beta.15 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Choosing several clips now collapses the tall card into a row, so the whole list is easy to choose from.
- Picture clips show a small picture at the right end of their row.

## New

- **The list collapses to rows while you choose clips.** While Multi-paste is on, or any clip is chosen
  (by Ctrl+click, Shift+click or the keyboard), the tall card of the clip shown shrinks, with a short
  animation, into a one-line row like the others. About a dozen rows then fit on screen and are easy to
  click or walk through. Clear the choice and turn Multi-paste off and the card opens again. The card's
  Expand and Delete buttons are hidden while it is one line; Ctrl+E (the expanded view) and the
  right-click menu still work.
- **Small pictures in the rows.** A picture clip's one-line row ends in a small picture of it at the
  right edge, in its own shape (about 22 pixels high). It shows on the rows around the clip shown, and on
  every row while the list is collapsed. A plain box shows until the picture has loaded. The pictures
  are made from the clip's stored preview, kept in memory only while the carousel is open.

## Downloads

- pasteroll-0.1.0-beta.15-windows-x64-setup.exe
- pasteroll-0.1.0-beta.15-windows-x64-portable.zip
- pasteroll-0.1.0-beta.15-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests (including new tests of the collapse -- when it
starts and ends, that it only shrinks on the way, that no row overlaps or leaves the list at any point
of it -- and of the small pictures and how they are drawn); Windows packaging; privacy scans of all
three downloads; the About check; and the upgrade fixture (upgrading from the beta.14 package keeps the
encrypted history, key, settings and presets). The release signatures were checked with the keys built
into the program before publishing. The collapse and the small pictures were also tried by hand on this
machine in a build of this code.

The real-window self-test (real hooks and clicks) was not run for this release because it changed only
the carousel's layout, drawing and logic, which the unit tests drive with a stand-in for the operating
system; nothing the program asks of Windows (hooks, windows, clipboard, keys) changed. The update
installer path was not touched, so the end-to-end test of Download and install was not run either.

Not tried: how the collapse looks on other display scales and on slower machines (the animation is
timed, so a slow machine shows fewer steps), and a very large number of picture clips in one list.

NOT TESTED: an update with a different administrator's credentials, Windows Defender or Controlled
Folder Access during an update, a clean-machine all-users (UAC) install, start-at-sign-in across a real
login, and the interactive uninstall data-deletion prompt.

No Linux package was built or released, and nothing was tried on a real Linux desktop. The shared code
changed (the Linux crates were not compiled for this release); treat Linux as untested until its
catch-up release. The update check and Download and install are Windows-only.
