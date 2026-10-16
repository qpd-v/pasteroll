# Changelog

## Unreleased

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
