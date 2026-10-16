# Privacy

Pasteroll is a clipboard history, so it necessarily sees what you copy. This is what it does with
that, in plain terms.

## What it records

Text, images and file lists that you copy, plus the time and the name of the program they came
from. A small preview is kept for images.

## What it does not record

- Copies made by programs that mark them as sensitive (password managers use
  `ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory = 0` on Windows
  and `x-kde-passwordManagerHint` on X11).
- Copies made in programs you add to **Settings → Never record copies made in these programs**.
- Anything while recording is paused (tray icon → *Pause recording*).
- Clips larger than the per-clip limit you choose.

## Where it goes

**Nowhere.** There is no network code in Pasteroll: no telemetry, no analytics, no crash upload,
no update check, no cloud sync. You can confirm that by reading the source or watching its
network activity. The only files it writes are in its own data folder
(`%LOCALAPPDATA%\Pasteroll` or `~/.local/share/pasteroll`), plus the files `install` creates
(program, Start menu or application-menu entry, uninstall entry).

## How it is stored

Each clip is encrypted before it is written to the database (see [SECURITY.md](SECURITY.md) for
the design and its limits); so are the time it was copied and the program it came from. History is
removed automatically when it passes the limits you set (count, total size, age); pinned clips are
kept until you delete them.

A clip you delete goes to the recycle bin first, still encrypted, and is removed for good after 30
days (or at once with *Delete permanently*, Shift+Delete, or `pasteroll clear`).

*Extract text* in the expanded viewer recognises text on this computer only (Windows' built-in
recognition, or Tesseract on Linux, which reads the picture through a pipe, not a file). *Save
as* writes a file only where you choose.

## Your controls

- `pasteroll clear --yes` deletes the history; `pasteroll uninstall --remove-data --yes` removes
  the program and all its data.
- Settings → limits, ignored programs, pause.
- Nothing starts with your computer unless you turn that on.

## Contact

Questions about privacy: open an issue at <https://github.com/qpd-v/pasteroll/issues>.
