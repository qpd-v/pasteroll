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
upload, no cloud sync. The only things in Pasteroll that can use the internet are the optional
**update check** (Windows) and, only when you ask for it, installing an update, both described below.
Nothing else has any network code, which you can
confirm by reading the source or watching its network activity. The only files it writes are in its own data folder
(`%LOCALAPPDATA%\Pasteroll` or `~/.local/share/pasteroll`), plus the files `install` creates
(program, Start menu or application-menu entry, uninstall entry).

With `portable.txt` beside the program, its application data stays in the adjacent `data` folder.
Portable mode does not install the app or change autostart. On Linux a local control pipe in the
data folder lets `pasteroll stop` request a graceful exit; it does not use the network.

## The update check (Windows, off until you say yes)

Until you have answered, Pasteroll asks whether it may check for updates (once you answer it does
not ask again); "Never check" is the default button and closing the window decides nothing. If you say yes (or tick **Check for updates** in Settings), then about
once a day it makes two small HTTPS requests to `raw.githubusercontent.com` (GitHub's host for the
files in this project's repository): one for the "newest version" note (a few lines of text: the
newest version number and up to five plain lines about it) and one for its signature. The note is
believed only if it is signed with the Pasteroll release key (see SECURITY.md); Pasteroll compares
its version number with its own. GitHub, like any website, sees your IP address
and the program name and version (`Pasteroll/<version>`), plus the usual request headers
(`Accept`); nothing else is sent: no clipboard content, no history, no
settings, no identifier. Windows is told not to sign in to a proxy or server on its own. Your
network may do its own name lookups and proxy discovery for any program. If a newer version exists you get a notification
and a menu entry that opens its release page in your browser or lets you install it; the plain lines from the signed
note are shown in the install window. **The check itself downloads nothing
and installs nothing** (installing is a separate step you must choose, described below). You can also check by hand from the tray menu or from
Settings (*Check for updates now*), or with
`pasteroll update-check`; those two requests are made only when you ask. Choose No (or untick the box)
and Pasteroll makes no network request unless you ask for one (a check by hand, or installing an update). The date of the last check, the newest version
you were told about, the newest version any note has named (so that an old note served again is
ignored), and a version you chose to skip are kept in `update-check.txt` in the data folder (Settings
shows how long ago the last check was). The plain lines of the note are not kept: the install window
fetches the note again when you open it. The setting is
`[updates] check_for_updates` in `config.toml`.

### Download and install (Windows, only when you ask)

When a newer version has been found, the tray menu offers **Download and install**. Nothing is
downloaded until you click **Download and install** in the update window. Then Pasteroll fetches the update from GitHub: a small
signed description of the release and the installer (or, for a portable copy, the program file) from
`github.com`, which hands the download over to GitHub's own file hosts
(`release-assets.githubusercontent.com`, `objects.githubusercontent.com`). GitHub sees your IP address
and the program name and version, as for any download. Before anything is run, Pasteroll checks that the
release description is signed with the Pasteroll release key (its public half is built into the
program) and that the downloaded file matches it exactly; a file that fails is deleted, not run.
`pasteroll update --yes` does the same from the command line, without the update window. The installer is kept in a private `pasteroll-update` folder inside the data folder until the next update (and Setup may write `update-setup.log` there, and its own temporary files in your temp folder). Your
clipboard history, settings and keys are never sent, and the installer does not touch them.

## How it is stored

Each clip is encrypted before it is written to the database (see [SECURITY.md](SECURITY.md) for
the design and its limits); so are the time it was copied and the program it came from. History is
removed automatically when it passes the limits you set (count, total size, age); pinned clips are
kept until you delete them.

A multi-paste (several clips pasted or copied together) is kept as one more clip, encrypted the same
way, unless recording is paused; it follows the same limits and can be deleted like any clip.

A clip you delete goes to the recycle bin first, still encrypted, and is removed for good after 30
days (or at once with *Delete permanently*, Shift+Delete, the **Clear history** button in Settings, or
`pasteroll clear`).

*Advanced Image Paste* works on your computer only. The edited picture is written where you choose
(the Desktop, or the folder chosen in Settings, by default) and nowhere else. That folder's path is kept in plain text in `config.toml`. The presets you save (the settings, never a picture) and
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
