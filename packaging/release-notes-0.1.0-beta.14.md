# Pasteroll 0.1.0-beta.14 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## In short

- Multi-paste works from the keyboard: Ctrl+M, Shift or Ctrl with the arrows, Space, and Ctrl+A to choose all.
- Hold Ctrl while you click where to paste, or press Ctrl+E, for options: join with a comma, space or tab, number or bullet them.
- Settings has a Clear history button, and About and the update question match the new look.

## New

- **Multi-paste from the keyboard** (Complete look). **Ctrl+M** turns Multi-paste on or off. With it on,
  **Shift+Down** (or Up) chooses the clip shown and each one you walk to, and walking back takes them out
  again; **Ctrl+Down** (or Up) adds each clip you land on; **Space** chooses the clip shown (while the
  search box is empty); **Ctrl+A** chooses every clip in the list (what matches the search, if you
  searched). Without Multi-paste on, these arrows just move as before.
- **Options for pasting several clips together.** With several clips chosen, **hold Ctrl (the image editor
  key) while you click where they should go**, or press **Ctrl+E** while Multi-paste is on, and a small
  card opens over the carousel instead of pasting at once: what joins the clips (new line, space, comma,
  tab or nothing), a list style (plain, numbered or bulleted), the order (as chosen, newest first or
  oldest first) and a preview of the result. Enter or **Paste** pastes (a terminal still asks first, on the
  card), **Copy** copies, **Esc** goes back with the clips still chosen. The last choices are remembered
  until Pasteroll restarts; a plain paste is still one per line.
- **Clear history** in Settings: deletes every clip except pinned ones, the recycle bin included. It asks
  first and cannot be undone.
- **About and the one-time question about update checks** use the same light or dark window style as
  Settings and the update window, instead of plain Windows dialogs. About opens without holding up the
  tray.
- **Windows with another scale.** Settings and the update window now follow a move to a monitor with a
  different scale (their size, text and controls), instead of staying the size they opened at.

## Downloads

- pasteroll-0.1.0-beta.14-windows-x64-setup.exe
- pasteroll-0.1.0-beta.14-windows-x64-portable.zip
- pasteroll-0.1.0-beta.14-windows-x64.exe (the program file alone, used by portable updates)
- update-manifest.txt and update-manifest.txt.sig (used by the updater)
- SHA256SUMS covering the Setup, the ZIP and the program file

Upgrades preserve history, encryption keys and settings. From beta.9 the tray menu can install this
for you (**Download and install**); beta.7 and beta.8 must install it by hand (quit Pasteroll from its
tray icon and run the new Setup; Setup also asks a running copy to quit). Windows uninstall keeps data
unless deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for
new programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated, on this machine: the Windows workspace tests (including new tests of the keyboard choosing
-- Ctrl+M, Shift and Ctrl with the arrows, Space, Ctrl+A -- of the options card and each way of joining,
numbering and ordering, that a terminal still asks first, and of how the card draws and is hit); Windows
packaging; privacy scans of all three downloads; the About check (against the new About window); and the
upgrade fixture (upgrading from the beta.13 package keeps the encrypted history, key, settings and
presets). An end-to-end test of Download and install against a local test server with a throwaway key
passed all 47 checks. The release signatures were checked with the keys built into the program before
publishing. The Linux crate compiles and its key tests pass under WSL.

The real-window self-test (real hooks and clicks, including a real Windows Terminal window, and now real
Ctrl+M, Shift+Down, Ctrl+A and Ctrl+E presses) passed: 87 checks, 0 failed. Run again with the Settings
window checks on, the four Settings checks passed and the 9 overlay focus checks that are already known
to fail in that mode failed again (the other checks passed).

Not tried: the options card opened by a real Ctrl+click on a window (the unit tests cover that path, the
real-window test opens it with Ctrl+E), clicking the card's buttons and the Clear history button by hand
(its confirmation and what it deletes were not exercised on real history), the About window and the
update question as a person would use them (the About test passes; the question was only looked at in
pictures), and the Settings and update windows being moved to a monitor with a different scale (that
code is untested on a second monitor). The looks were checked from pictures on this machine only.

NOT TESTED: an update with a different administrator's credentials, Windows Defender or Controlled
Folder Access during an update, a clean-machine all-users (UAC) install, start-at-sign-in across a real
login, and the interactive uninstall data-deletion prompt.

No Linux package was built or released, and nothing was tried on a real Linux desktop. The shared code
changed (the Linux code compiles and its key tests pass under WSL; its GNOME extension would also have
to send the new keys); treat Linux as untested until its catch-up release. The update check and
Download and install are Windows-only.
