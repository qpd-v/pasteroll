# Changelog

## Unreleased

- **Choosing several clips collapses the list to rows.** While Multi-paste is on, or any clip is chosen
  (by Ctrl+click, Shift+click or the keyboard), the tall card of the clip shown shrinks, with a short
  animation, into a one-line row like the others, so about a dozen rows are on screen and easy to choose
  from. Clear the choice and turn Multi-paste off and the card opens again. (Ctrl+E still opens the
  expanded view.)
- **Small pictures in the rows.** A picture clip's one-line row (the neighbours of the clip shown, and
  every row while the list is collapsed) ends in a small picture of it at the right edge.

## 0.1.0-beta.14 (Windows only)

- **Multi-paste from the keyboard.** Ctrl+M turns Multi-paste on or off. With it on, Shift+Down (or Up)
  chooses the clip shown and each one you walk to, and walking back takes them out again; Ctrl+Down (or Up)
  adds each clip you land on; Space chooses the clip shown (while the search box is empty); Ctrl+A chooses
  every clip in the list. The footer and the button's tooltip say so.
- **Options for pasting several clips together.** With several clips chosen, hold Ctrl (the image editor
  key) while you click where they should go, or press Ctrl+E while Multi-paste is on, and a small card
  opens over the carousel: what joins the clips (new line, space, comma, tab or nothing), a list style
  (plain, numbered, bulleted), the order (as chosen, newest first, oldest first) and a preview. Enter or
  Paste pastes (a terminal still asks first, on the card), Copy copies, Esc goes back with the clips still
  chosen. The last choices are remembered until Pasteroll restarts; a plain paste is still one per line.
- Settings has a **Clear history** button (pinned clips are kept; it asks first and cannot be undone).
- The Settings and update windows follow the window to a monitor with a different scale (size, fonts
  and controls), instead of staying the size they opened at.
- **About** and the one-time question about update checks use the new light/dark window style instead of
  the plain Windows dialogs. About opens without holding up the tray.

## 0.1.0-beta.13 (Windows only)

- Checking for updates, downloading and installing now happen in one **update window**: it shows what
  is new, a real progress bar with the size while it downloads, the later steps, and any problem with
  a **Try again** button. A download can be cancelled. It follows Windows' light or dark mode and is
  styled like the new Settings window.
- Far fewer notifications: none while checking or updating (the window shows it, and "Check for updates
  now" shows its answer there too). The notification icon announces a new version once, and a click on
  it opens the update window. After the restart there is one short "updated" line.

## 0.1.0-beta.12 (Windows only)

- A multi-paste is now kept in history as one clip, with a small badge showing how many clips went
  into it (Ctrl+C or right-click Copy of several clips is kept too). It is not kept while recording is
  paused. Older versions still read these clips as ordinary text.
- The **Multi-paste** button in the Complete carousel is now an icon (a checklist) with a tooltip.
- The Settings window is redesigned: it follows Windows' light or dark app mode (and changes when that
  does), is much more compact (about 570 DIP tall instead of 870, so it fits on a small or
  high-scaling screen) and is laid out in two columns of cards.

## 0.1.0-beta.11 (Windows only)

- Settings has a **Save pictures in this folder** choice (with Browse and a Desktop button). Paste Path and
  the image editor save there instead of the Desktop. A folder you pick inside the editor is still
  remembered, until you change the folder in Settings. Clicking the desktop with a picture still saves
  to the desktop.
- **Choose several clips** in the Complete carousel: Ctrl+click to choose, Ctrl+Shift+click for a
  range, then click where to paste (or Enter) to paste them one per line in the order chosen. Ctrl+C or
  right-click and Copy puts them on the clipboard without pasting. Pictures are left out; Esc clears.
- A **Multi-paste** button beside the filter turns plain clicks into choosing: click each clip you want
  (Shift+click for a range), then click where to paste. It starts off each time Pasteroll opens, and is
  not in No Bloat.

## Linux, not released yet (Linux is still on beta.4)

- The picture folder setting and choosing several clips (Ctrl+click, Ctrl+Shift+click, Ctrl+C, right-click
  Copy) are in the Linux code and pass an isolated X11 test (a private Xvfb screen under WSL). Not tried
  yet on a real desktop, on GNOME Wayland or on COSMIC.
- Complete and No Bloat on Linux: a choice in the tray menu, `pasteroll config set mode no-bloat`, and the
  matching window size (checked on X11; the GNOME Wayland size is untested).
- The Desktop folder is found from `$XDG_CONFIG_HOME/user-dirs.dirs`, not only `~/.config`.

## 0.1.0-beta.10 (Windows only)

- Which version is the newest is now **signed** (Windows). The update check no longer trusts GitHub's
  list of releases: it reads a short note from this repository that is signed with the same release
  key as the update itself, and ignores a genuine but older note served again. The note also carries a
  few plain lines about the new version, which the install window shows.
- Settings has a **Check for updates now** button and says how long ago the last check was. The
  install window has **Skip this version** (that version is not mentioned again; a newer one, or a
  check you ask for, brings it back).
- If an update's installer fails, the previous Pasteroll is started again and says so, instead of
  nothing running. After a successful update Pasteroll says which version you now have.
- More tests of the downloader: a server that trickles bytes, an endless chunked answer, a partial
  (206), sign-in (401/407) or compressed answer. Removed unused code.

## 0.1.0-beta.9 (Windows only)

- Add **Download and install** to the update check (Windows). When a newer version is found, the tray
  menu and the notification offer it; nothing is downloaded until you click and confirm. The update
  is downloaded from GitHub, checked against the maintainer's signature and its checksum (a file that
  fails is deleted, never run), and installed: a copy installed by Setup runs the new Setup silently
  and is started again, and a portable or self-installed copy replaces its own program file. An
  all-users copy gets Windows' administrator prompt and is updated from a folder only administrators
  can open, so no file you could change is trusted by the elevated part. Your history and settings are untouched.
  `pasteroll update --yes` does the same from the command line.
- Releases now carry `update-manifest.txt` and its signature, and a bare `pasteroll-<version>-windows-x64.exe`
  for portable updates. `PRIVACY.md` and `SECURITY.md` describe exactly what is downloaded and how it
  is trusted.

## 0.1.0-beta.8 (Windows only)

- Fix the Settings window not showing your answer to the one-time update question. If the question
  appeared while Settings was open and you chose "Check for updates", the checkbox in Settings
  stayed empty. The open window now follows a choice made elsewhere within a second, and the
  question waits until Settings is closed instead of appearing on top of it.

## 0.1.0-beta.7 (Windows only)

- Add an optional update check (Windows). It is off until you say yes: the first time you run this
  version Pasteroll asks once (the default answer is No), and Settings has a checkbox for it. When
  on, about once a day it makes one request to GitHub for the list of releases and tells you if a
  newer version exists, with a menu entry and a notification that open its release page. It never
  downloads or installs anything. The tray menu has *Check for updates now* and the command line
  has `pasteroll update-check`, which check only when you ask. With the check off, Pasteroll makes
  no network request at all. See PRIVACY.md. `config.toml` gains `[updates] check_for_updates`.

## 0.1.0-beta.6 (Windows only)

- Fix Advanced Image Paste never opening in Windows Terminal (and so in PowerShell, Claude Code
  and other programs run inside it). Windows Terminal answers "title bar" to Pasteroll's
  click-area test for its whole window, so the click was refused as "not on a client area".
  A click is now accepted when it lies inside the window's client rectangle, while a standard
  title bar, borders and caption buttons are still refused. A plain click-to-paste into Windows
  Terminal was refused the same way and works now too.
- Fix the editor not opening when Ctrl is still held a second after the click. The click was
  waited on until every key was released, with a one second limit, to protect the paste
  keystroke; a click that only opens the editor types nothing yet, so it no longer waits.

## 0.1.0-beta.5 (Windows only)

- Add two looks for the carousel, switched from the Windows tray menu (one click, no restart) or
  Settings. **Complete** shows everything, as before. **No Bloat** shows only the current clip in
  a smaller window: no clip list and no buttons on the card. Every feature still works from the
  keyboard (search by typing, Tab for the filters, Ctrl+P, Delete and Ctrl+Z, Ctrl+E for the
  expanded viewer, the image editor), and the search bar, filter menu and pin star appear only
  while you use them. Recording, limits and the history are the same in both; Complete is the
  default.
- Advanced Image Paste now works on any destination, not only the desktop. Hold the editor key
  (Ctrl by default) while you click a window with a picture chosen, edit it, and **Paste Image**
  delivers the result there. A new **Paste as** choice in the editor picks what is pasted: the
  *picture* itself (the default, nothing is saved), the saved file's *path*, or its *file name*.
  Terminals default to the path. The desktop still saves a file.
- `config.toml` gains `[appearance] mode = "complete" | "no-bloat"`. Linux keeps the setting
  in the file and ignores it for now: the Linux tray, settings and window sizing for No Bloat
  wait for the Linux catch-up release.

## 0.1.0-beta.4

- GNOME 46: separate UI density from Shell stage coordinates for mixed-scale monitors.
- Transfer large viewer/editor frames as bounded full-resolution strips; matching
  protocol-3 app and companion are required. No viewer downsampling fallback.
- Correct Linux transparent-icon test expectations. Additional manual acceptance
  is deferred; GNOME remains experimental. Linux OCR quality is inconsistent.

- Replace the boxed application icon with the approved transparent roll. Generate
  the tray, window, shortcut and installer sizes with preserved proportions.
- Single tray click opens history near the icon; double-click opens Settings.
  Right-click keeps the existing menu. Single clicks use the Windows double-click
  interval to distinguish them from double-clicks.
- Add About Pasteroll to the Windows tray menu, showing version, installation type
  and the activation gesture. Tray tooltips show the version and recording state.
- Add image OCR areas: in the expanded viewer, choose the lasso, draw one or more
  freehand outlines, and extract only the selected text. Enter extracts; Ctrl+Z
  undoes an area; Delete clears areas; Escape cancels selection. Zoom and scroll
  preserve image coordinates. The original image is never changed.
- Mask unselected pixels before local OCR and ignore cancelled/stale OCR results.
- Copied OCR text appears in the existing carousel when the viewer closes, including
  saves that finish afterwards; active filters are preserved.
- Copying extracted OCR text or a viewer text selection now records it as a new
  history clip. Repeated copies deduplicate; paused/excluded recording is respected.
  Automatic paste writes remain excluded from clipboard capture.
- Verify Windows upgrades with disposable data and separate installer test identity.
- Ship Windows, X11 and experimental GNOME 46 packages together. Linux
  tray/panel/About parity and OCR accuracy improvements are deferred.

## 0.1.0-beta.3

- Add a native GNOME 46 Wayland backend with a Pasteroll Shell companion: clipboard
  capture, carousel, search, partial paste, expanded viewer, image editor and settings.
- Add an optional administrator-enabled input helper for Ctrl+Shift+middle-click on
  stock GNOME. Keyboard activation is available explicitly without the helper.
- Authenticate the Shell/session, use short renewable input leases, and recover from
  helper/device loss without leaving input grabbed.
- Add separate GNOME development archive and Debian packages. The helper is disabled
  after a fresh package install; enabling it requires an administrator.
- Windows and X11 retain their existing backends.

GNOME validation so far uses isolated automated sessions and synthetic kernel input.
Physical desktop, login/lock/suspend and real compositor input testing has not been performed.
This release exposes GNOME 46 support as experimental with that limitation.
GNOME versions other than 46, other Wayland desktops, Flatpak and Snap are not supported.

## 0.1.0-beta.2

- Portable mode: place `portable.txt` beside the program to keep history, keys, configuration,
  presets and logs in its `data` folder. Installation and autostart changes are disabled there.
  Windows history remains bound to the Windows account by DPAPI; use key export/import to move it.
- A `stop` command requests a graceful exit and succeeds when no instance is running.
- Windows recognizes Setup-owned installations through `setup.txt` and delegates removal to
  Settings > Apps.
- Updated application icon from the approved artwork.
- Windows Setup wizard with current-user/all-users installation, versioned portable ZIP,
  graceful upgrades, optional shortcuts and autostart, and consent before uninstall data removal.
  Setup stops only the copy in its target folder and leaves another installation's startup entry alone.
- Linux static archive with per-user/system `install.sh`, plus a Debian/Ubuntu package.
  Installation and package removal keep every account's history and settings.

- **Advanced Image Paste**: Ctrl+click the desktop with an image chosen (the key is a setting:
  Ctrl, Shift or Alt) and the card grows into an image editor instead of saving the picture at
  once. A live preview follows every change: outline, resize (pixels or percent), crop (handles,
  numbers, locked shapes), rounded corners, drop shadow, background removal / fill / replace,
  padding, any-angle rotation and flips, and output as PNG, JPG, WebP (lossless) or BMP with
  quality. Choose the file name and folder, undo and redo, reset all, and save presets. Cancel
  returns to the carousel; a plain click on the desktop still saves at once. The clip is never
  changed. `advanced_image_modifier` in `config.toml`; presets live in `image-presets.txt` in the
  data folder.
- **Expanded viewer**: the active card's Expand button (or Ctrl+E) grows the clip into a window
  of three quarters of the screen, F11 or its corner button makes it fullscreen, and Esc shrinks
  it back into the carousel exactly as it was. The carousel stays visible, dimmed and inactive.
  - Text keeps its size in a scrolling container, with find (Ctrl+F, F3), word wrap and
    selection; pictures fit the view and zoom (+/-, Ctrl+wheel, 1:1, double-click) and pan (drag,
    wheel).
  - Toolbar: previous/next (Left/Right, without moving the carousel), copy (Ctrl+C: the
    selection or the whole clip), paste, pin, recycle bin, and a More menu with Save as (Ctrl+S),
    Info (Ctrl+I), reset zoom, Extract text and Delete permanently.
  - Extract text reads text in pictures on this computer: Windows' built-in recognition, or
    Tesseract on Linux.
  - Info shows the type, size, when the clip was copied and from which program.
- **Recycle bin**: deleting (Delete, the card's bin button) moves a clip to the recycle bin for 30
  days; Ctrl+Z brings it back, the "Recycle bin" filter lists it, and copying it again restores it
  too. Shift+Delete or Shift+click deletes for good, after a confirmation.
- Clips now remember (sealed, like the rest) when they were copied and from which program.
- The history database is upgraded on first start (schema 5); earlier versions cannot open it
  afterwards.

## 0.1.0-beta.1

First public beta.

- Windows 10/11 and Linux (X11) clipboard history carousel: Ctrl+Shift+middle-click, wheel or
  arrows to scroll, type to search, filters, pin, delete, partial paste.
- Text, images and file lists; encrypted per clip at rest; nothing leaves the computer.
- Fast image previews (cached encrypted thumbnails, neighbour prefetch).
- Tray icon and settings (history limits, carousel size 50-200 %, ignored programs, custom
  filters, optional start at sign-in). Linux: StatusNotifier or XEmbed tray.
- `install` / `uninstall` for the current user, first-run privacy notice.
- Recovery: passphrase-protected `key-export` / `key-import`.
- Unsigned Windows build; static Linux build for x86-64.
