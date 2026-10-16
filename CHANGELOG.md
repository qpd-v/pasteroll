# Changelog

## Unreleased

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
