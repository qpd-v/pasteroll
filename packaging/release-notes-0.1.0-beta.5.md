# Pasteroll 0.1.0-beta.5 (Windows)

Experimental prerelease for **Windows only**. Linux (X11 and GNOME 46) stays on
[0.1.0-beta.4](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) and catches up in a
later release; nothing here changes the Linux downloads.

## New

- **Two looks for the carousel**, switched from the tray menu (right-click, one click, no
  restart) or in Settings. **Complete** is the carousel as before. **No Bloat** shows only the
  current clip in a smaller window, with no clip list and no buttons on the card. Every feature
  still works from the keyboard, and the controls appear only while you use them: typing brings
  up the search bar, Tab the filter menu, Ctrl+P marks a pinned clip with a star, Delete and
  Ctrl+Z (recycle bin) report in the message line, and Ctrl+E opens the expanded viewer. Recording,
  limits and your history are the same in both. Complete is the default.
- **Advanced Image Paste works on any window**, not only the desktop. With an image chosen, hold
  the editor key (Ctrl by default, changeable in Settings) while you click a window and the picture
  opens in the editor. **Paste Image** then delivers it there. A new **Paste as** choice picks
  what is pasted: the **picture** itself (the default; nothing is saved), the saved file's
  **path** (in quotes when it has spaces), or only its **file name**. Terminals default to the
  path. The desktop still saves a file. A window that needs more rights than Pasteroll is
  refused up front, and nothing is pasted if you switch windows while editing.
- `config.toml` gains `[appearance] mode = "complete" | "no-bloat"`.

Note: the Ctrl+click also reaches the window you click, so it can do what it normally does there
(for example follow a link in a browser). Pick Shift or Alt in Settings if that gets in the way.

## Downloads

- pasteroll-0.1.0-beta.5-windows-x64-setup.exe
- pasteroll-0.1.0-beta.5-windows-x64-portable.zip
- SHA256SUMS covering these two packages

Upgrades preserve history, encryption keys and settings. Windows uninstall keeps data unless
deletion is explicitly chosen. Windows builds are unsigned, so SmartScreen shows a warning for new
programs: check the checksum, then choose More info, Run anyway.

## Testing and known limitations

Automated: the Windows workspace tests, Windows packaging, About and upgrade fixtures, and a
real-window self-test (real hooks and clicks) covering both looks and the editor pasting into a
window all passed. The two looks and Advanced Image Paste in windows were also tried by hand on one
Windows 11 computer.

NOT TESTED: a clean-machine all-users (UAC) install, start-at-sign-in across a real login, and the
interactive uninstall data-deletion prompt. Automated fixtures do not establish those results.
Advanced Image Paste pastes a picture only into programs that accept one; a plain text box
that does not will receive nothing, so choose **Path** or **File name** there.

No Linux code was built or tested for this release. The shared code changed, so Linux behavior
should be treated as untested until its catch-up release.
