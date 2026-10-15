# Changelog

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
