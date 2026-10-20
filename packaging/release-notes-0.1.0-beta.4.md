# Pasteroll 0.1.0-beta.4 — Windows candidate

Unpublished candidate. Linux downloads remain on beta.3.

## Changes

- Transparent roll icon across the tray, taskbar/window, shortcuts and installer.
- Single tray click opens history near the icon; double-click opens Settings;
  right-click retains the existing menu. From tray browsing, choose a destination
  window to paste. Single click waits for the Windows double-click interval.
- About Pasteroll in the tray: version, installation type and activation reminder.
- Tray tooltip shows the current version and recording/paused state.
- Freehand OCR areas in the expanded image viewer: choose the lasso, draw around
  one or more areas, then press Enter or Extract selected text. Ctrl+Z undoes an
  area; Delete clears areas; Escape cancels. Zooming/scrolling keeps outlines
  attached to image pixels. Text outside the outlines is masked before local OCR;
  overlapping regions form a union, read in the OCR engine's normal reading order.
  The source image and saved clip are never changed. Up to16 areas are supported.

- Copied OCR text refreshes the existing carousel when the viewer closes, even
  when its background save completes later. Active filters remain applied.
- Fixed copied OCR text missing from carousel history. Explicitly copied viewer
  text selections are recorded too; duplicates are merged. Paused recording and
  app exclusions still apply, and automatic paste writes remain excluded.

## Packages

- `pasteroll-0.1.0-beta.4-windows-x64-setup.exe`
- `pasteroll-0.1.0-beta.4-windows-x64-portable.zip`
- `SHA256SUMS` covering these two Windows packages

Windows builds are unsigned. Setup upgrades retain history, keys and settings;
uninstall retains them unless data deletion is explicitly requested.

## Acceptance still requiring a separate Windows environment

All-users/UAC installation, enabling autostart across login, and the interactive
uninstall data-deletion prompt need a disposable account/VM. Current-host fixture
tests use a separate AppId/shortcut names and synthetic data to protect the live app.
Review the UI and icon appearance before publication. No Linux runtime tests are
part of this Windows update.
