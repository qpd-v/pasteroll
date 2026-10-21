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

**Nowhere.** Your clipboard never leaves your computer: no telemetry, no analytics, no crash
upload, no cloud sync. The one thing in Pasteroll that can use the internet is the optional
**update check** (Windows), described below. Nothing else has any network code, which you can
confirm by reading the source or watching its network activity. The only files it writes are in its own data folder
(`%LOCALAPPDATA%\Pasteroll` or `~/.local/share/pasteroll`), plus the files `install` creates
(program, Start menu or application-menu entry, uninstall entry).

With `portable.txt` beside the program, its application data stays in the adjacent `data` folder.
Portable mode does not install the app or change autostart. On Linux a local control pipe in the
data folder lets `pasteroll stop` request a graceful exit; it does not use the network.

## The update check (Windows, off until you say yes)

Until you have answered, Pasteroll asks whether it may check for updates (once you answer it does
not ask again); "Never check" is the default button and closing the window decides nothing. If you say yes (or tick **Check for updates** in Settings), then about
once a day it makes one HTTPS request to `api.github.com` for the list of releases of this project
and compares the newest version number with its own. GitHub, like any website, sees your IP address
and the program name and version (`Pasteroll/<version>`), plus the usual request headers
(`Accept`, `X-GitHub-Api-Version`); nothing else is sent: no clipboard content, no history, no
settings, no identifier. Windows is told not to sign in to a proxy or server on its own. Your
network may do its own name lookups and proxy discovery for any program. If a newer version exists you get a notification
and a menu entry that opens its release page in your browser. **Nothing is downloaded or installed
by Pasteroll.** You can also check by hand from the tray menu (*Check for updates now*) or with
`pasteroll update-check`; that one request is made only when you ask. Choose No (or untick the box)
and Pasteroll makes no network request at all. The date of the last check and the newest version
you were told about are kept in `update-check.txt` in the data folder. The setting is
`[updates] check_for_updates` in `config.toml`.

## How it is stored

Each clip is encrypted before it is written to the database (see [SECURITY.md](SECURITY.md) for
the design and its limits); so are the time it was copied and the program it came from. History is
removed automatically when it passes the limits you set (count, total size, age); pinned clips are
kept until you delete them.

A clip you delete goes to the recycle bin first, still encrypted, and is removed for good after 30
days (or at once with *Delete permanently*, Shift+Delete, or `pasteroll clear`).

*Advanced Image Paste* works on your computer only. The edited picture is written where you choose
(the Desktop by default) and nowhere else. The presets you save (the settings, never a picture) and
the folder you last saved to are kept in a small plain-text file, `image-presets.txt`, in the data
folder.

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
