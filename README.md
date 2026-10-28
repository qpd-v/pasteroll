# Pasteroll

A clipboard history you can scroll with the mouse wheel. Copy as usual; when you need an earlier
clip, press **Ctrl+Shift+middle-click**, scroll through your history right where you are working,
and paste the whole clip, or select just a part of it and paste only that.

**Status: public beta.** Windows is at `0.1.0-beta.14`; Linux is still at `0.1.0-beta.4` and
catches up in a later release. It works and is tested, but it is young. Expect rough edges and
please [report them](https://github.com/qpd-v/pasteroll/issues).

| | Windows 10 / 11 | Linux (X11) |
|---|---|---|
| Download | [Setup EXE](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.14/pasteroll-0.1.0-beta.14-windows-x64-setup.exe) · [Portable ZIP](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.14/pasteroll-0.1.0-beta.14-windows-x64-portable.zip) | [Static archive](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.4/pasteroll-0.1.0-beta.4-linux-x86_64.tar.gz) · [Debian/Ubuntu .deb](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.4/pasteroll_0.1.0-beta.4_amd64.deb) |
| Tray icon | yes | StatusNotifier (KDE, GNOME with AppIndicator, Ubuntu) or XEmbed |
| Wayland | n/a | experimental GNOME 46; separate packages and setup below |
| macOS | not available | |

Windows: [release notes and testing status](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.14) ·
[SHA256SUMS](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.14/SHA256SUMS)
Linux: [release notes and testing status](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.4) ·
[SHA256SUMS](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.4/SHA256SUMS)

## Beta.5 improvements (Windows only)

A *No Bloat* look that shows only the current clip (every feature still works and its controls
appear when you use them), switched from the tray menu or Settings. Advanced Image Paste now works
on any window, not only the desktop, and can paste the edited picture, its file's path or its
file name. Linux keeps beta.4 until its catch-up release.

## Beta.4 improvements

Windows adds transparent roll branding, About/version information, single tray click
for history and double click for Settings. Both platforms include freehand OCR areas
in the regular expanded image viewer, copied viewer text in history, and immediate
carousel refresh when returning from the viewer. The original image stays unchanged.
GNOME 46 gets corrected mixed-scale monitor geometry and full-resolution viewer
transfers. Linux tray/panel/About interaction parity is deferred to the next version.
Linux OCR accuracy remains inconsistent; recognition improvements are deferred too.

## Experimental GNOME 46 Wayland support

Native GNOME 46 Wayland support uses a matched protocol-3 Shell companion.
Ctrl+Shift+middle-click requires the optional administrator-enabled host input helper;
explicit keyboard mode uses Ctrl+Shift+Space. See [GNOME setup](extensions/INPUT_HELPER.md).
**Mixed-scale display/viewer/editor checks passed on one Pop!_OS GNOME 46 setup.**
Further manual acceptance, including physical mouse-helper, login/lock and suspend
checks, is deferred; this remains experimental. Automated
headless sessions and synthetic kernel input passed; treat this backend as experimental.
Other GNOME versions and other Wayland compositors are not supported.

- [GNOME 46 experimental archive](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.4/pasteroll-0.1.0-beta.4-linux-x86_64-gnome-development.tar.gz)
- [GNOME 46 experimental Debian/Ubuntu package](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.4/pasteroll_0.1.0-beta.4_amd64-gnome-development.deb)

The X11 and GNOME Debian packages are alternatives, not coinstallable. The GNOME
helper stays disabled after a fresh package install. The development suffix identifies
the experimental packages; it does not indicate broader Wayland compatibility.

## Features

- **Two looks**: *Complete* (everything on show) or *No Bloat* (just the current clip, in a
  smaller window; every feature still works from the keyboard and its controls appear when you
  use them). Switch from the tray menu or Settings. Windows only for now.
- **Scroll your history** with the mouse wheel or arrow keys; the carousel opens at the pointer.
- **Search as you type** (file names too) and filter by Text, Document, Image, Video, Audio,
  Compressed, Pinned, or filters you define.
- **Partial paste**: double-click a word or drag across text in the preview, then Enter pastes
  only the selection.
- **Expanded viewer**: the card's Expand button (or Ctrl+E) grows a clip into a large window:
  long text to read and search, pictures to zoom and pan, Save as, Info, and Extract text (OCR, on
  this computer). F11 for fullscreen, Esc to shrink it back.
- **Advanced Image Paste**: Ctrl+click the desktop or any window (the key is a setting) with an
  image chosen and the picture grows into an editor instead of being saved or pasted at once:
  outline, resize, crop, rounded corners, drop shadow, background removal or fill, padding, rotate
  and flip, PNG / JPG / WebP / BMP with quality, a file name and folder of your choice (Settings sets
  the folder pictures go to unless you pick another; it starts as the Desktop), undo and
  redo, and presets you can reuse. In a window you choose what is pasted: the picture, the saved
  file's path, or its name. A plain click still saves (desktop) or pastes (window) the picture
  immediately. The clip itself is never changed.
- **Several clips at once** (Complete mode): Ctrl+click clips in the list to choose them (Ctrl+Shift+click
  chooses everything from the last one you chose), or press the **Multi-paste** button beside the filter
  and simply click each clip (Shift+click for a range; press the button again to go back to normal
  clicks). Ctrl+M turns Multi-paste on from the keyboard; then hold Shift and press Down (or Up) to
  choose the clip shown and the ones you walk to (walking back takes them out again), or hold Ctrl and
  press the arrows to add each clip you land on, Space chooses the clip shown, and Ctrl+A chooses
  everything in the list. Then click where they should go (or press Enter) to
  paste them one per line, in the order you chose them (the result is also kept in history, with a badge).
  **Hold Ctrl while you click where they should go** (or press Ctrl+E with Multi-paste on) to open a
  small options card first: what joins the clips (new line, space, comma, tab or nothing), a list style
  (plain, numbered, bulleted) and the order, with a preview; Enter or Paste pastes, Copy copies, Esc goes
  back. Ctrl+C, or right-click and Copy, puts them on the
  clipboard instead. Pictures cannot be joined into text and are left out. Esc clears the choice.
  While Multi-paste is on, or any clip is chosen, the tall card of the clip shown collapses into a
  one-line row so the whole list is rows (it opens again when the choice is cleared). A picture clip's
  one-line row ends in a small picture of it.
- **Pin** clips (Ctrl+P) so they are never removed automatically. Settings has a **Clear history**
  button (it keeps pinned clips and asks first).
- **Recycle bin**: deleted clips can be brought back (Ctrl+Z, or the Recycle bin filter) for 30
  days; Shift+Delete deletes for good.
- **Images and files** are kept too, with fast cached previews.
- **Private by design**: stored encrypted on your computer; your clipboard is never sent anywhere
  (see [PRIVACY.md](PRIVACY.md)).
- **Update check (Windows, optional)**: off until you say yes. Once a day it asks GitHub whether a
  newer version exists (the answer is a short note signed by the maintainer) and tells you once per
  new version. Turn it on or off in Settings, which also has **Check for updates now** and shows when
  the last check was; or check by hand from the tray menu. Checking, what is new, the download (with a
  progress bar and a Cancel button) and installing all happen in one **update window** that follows
  Windows' light or dark mode, instead of a series of notifications. Nothing is downloaded until you
  click **Download and install** in it (it also offers **Skip this version**); then the update
  is checked against the maintainer's signature before it runs, and your history and settings are
  kept (Setup installs, all-users installs with Windows' administrator prompt, and portable copies
  all update this way). If the installer fails, the previous version is started again and says so.
- **Skips password managers** that mark their copies as sensitive, and programs you exclude.
- **Terminal safe**: asks for a second Enter before pasting several lines into a terminal.
- **Adjustable size**: 50 % to 200 % in Settings.
- Starts with your session only if you turn that on.

## Install

### Windows

1. Download `pasteroll-0.1.0-beta.14-windows-x64-setup.exe` and `SHA256SUMS` from the
   [beta.14 release](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.14) and
   [verify the checksum](#verify-your-download).
2. Run the setup wizard. **Only for me** is the default and needs no administrator rights;
   **For all users** needs UAC. Start menu is on by default; desktop shortcut and autostart are off.

For portable use, extract `pasteroll-0.1.0-beta.14-windows-x64-portable.zip` and run `pasteroll.exe`.
Its included `portable.txt` keeps application data in the adjacent `data` folder.

> **"Windows protected your PC"**: the beta is **not code-signed yet**, so SmartScreen shows
> this for new programs. Click **More info → Run anyway** after checking the checksum. Some
> antivirus programs are also wary of anything that watches the keyboard and clipboard (which a
> clipboard manager must); the source is here for you to read and build yourself.

### Linux

```sh
tar xzf pasteroll-0.1.0-beta.4-linux-x86_64.tar.gz
cd pasteroll-0.1.0-beta.4-linux-x86_64
./install.sh              # per-user: ~/.local/bin, menu entry and icon
pasteroll run
```

For all users, use `sudo ./install.sh --system`. On Debian/Ubuntu, an alternative is
`sudo dpkg -i pasteroll_0.1.0-beta.4_amd64.deb`. Neither installer enables autostart.
Remove an archive install with `./install.sh --uninstall` (add `--system` and sudo for a system
install), or a Debian package with `sudo dpkg -r pasteroll`. These operations keep history/settings.
The app's own `install` and `uninstall` commands manage per-user installs only.

Optional: Extract text needs `tesseract-ocr`, and Save as uses `zenity` or `kdialog` (without one,
clips are saved to the Desktop).

The standard Linux packages above need an **X11 session**. Ubuntu and Fedora log in to Wayland by default: choose
"Ubuntu on Xorg" (or "GNOME on Xorg") from the gear icon on the login screen. For native GNOME 46 Wayland windows, use the experimental packages and companion setup above.
The X11 backend does not provide full native Wayland support.

## Use

| | |
|---|---|
| **Ctrl+Shift+middle-click** | open the carousel at the pointer |
| mouse wheel / ↑ ↓ | move through clips |
| type | search |
| Tab | choose a filter |
| Enter, or click the target | paste |
| double-click / drag in the preview | select part of a clip to paste |
| Ctrl+P | pin or unpin a clip |
| Delete / Ctrl+Z | move a clip to the recycle bin / bring it back |
| Shift+Delete, Shift+click on the bin button | delete for good (asks first) |
| Ctrl+E, or the card's Expand button | open the expanded viewer |
| Esc | close |

**No Bloat** is a look, not a smaller program. The carousel shows only the clip, with no list and
no buttons on the card, and every key above still works: typing brings up the search bar, Tab the
filter menu, Ctrl+P marks the clip with a star, Delete and Ctrl+Z talk in the message line, and
Ctrl+E opens the expanded viewer. Choose it from the tray menu (right-click) or in Settings.

In the expanded viewer:

| | |
|---|---|
| Left / Right | previous / next clip (the carousel keeps its place) |
| wheel, arrows, Page Up/Down, Home/End | scroll or pan |
| Ctrl+wheel, + / -, 0, 1 | zoom pictures, fit, real size |
| Ctrl+F, or just type | find in the clip; Enter or F3 for the next match |
| Ctrl+C / Enter | copy (the selection, or the whole clip) / paste |
| Ctrl+S, Ctrl+I | save as, info |
| F11 | fullscreen |
| Esc | leave fullscreen, then close the viewer |

### Advanced Image Paste

With an image chosen in the carousel, a click on the desktop saves it as a PNG right away, and a
click in a window pastes it. Hold **Ctrl** while you click (Settings lets you pick Shift or Alt
instead) and the picture opens in the editor, on the desktop or in any window. (The click also
reaches the window you click, so a Ctrl+click can do what it normally does there, such as follow a
link; pick another key in Settings if that gets in the way.) The settings are grouped in the panel on the right, one section open at a time; the
switch on a section's row turns the effect on or off, and the preview on the left follows every
change.

| | |
|---|---|
| Crop | drag the handles on the picture, or type a size; lock a shape (1:1, 4:3, 3:2, 16:9, 9:16 or the original's) |
| Resize | by pixels or percent, keeping the proportions or not |
| Rotate and flip | any angle, 90 degree turns, flip sideways or upside down |
| Rounded corners | any radius, square, or fully round |
| Outline | any colour (a palette or a hex code), any thickness |
| Drop shadow | colour, opacity, blur, distance and direction |
| Background | keep, fill see-through parts, remove the background that touches the edges (with a tolerance), or replace it |
| Padding | the same on every side or different on each, with a colour |
| Format and quality | PNG (compression), JPG (quality), WebP (lossless), BMP |

When the destination is a window, the Format and quality section opens first with **Paste as**:
**Image** pastes the edited picture itself (as a PNG, nothing is saved), **Path** saves the file and
pastes its path (in quotes when it has spaces), and **File name** saves it and pastes only the name.
It starts on Image, or on Path in a terminal, and you can change it for each paste. On the desktop
the picture is saved as a file.

The bar at the bottom holds the file name, the folder to save into (the Desktop unless you choose
another; it is remembered until you change the folder in Settings) and **Paste Image** (or **Paste Path** / **Paste Name**). **Cancel**, Esc or the back arrow return to the
carousel without saving. Undo and redo (Ctrl+Z, Ctrl+Y) and **Reset all** are at the top, next to
**Presets**: save the current settings under a name and apply them to any picture later (a
preset keeps everything except the crop). Enter pastes; while you type in a box Enter finishes the
typing instead. The editor is modal: clicks outside it do nothing, and it closes after ten
minutes without input.

Tray icon: pause recording, the two looks, Settings, open data folder, About, check for updates
(Windows) and quit. Settings has the look, the optional update check, the history limits,
carousel size, the key for the image editor, programs to ignore and custom filters.

### Command line

```
pasteroll run | status | config [show|path|init|set KEY VALUE] | exclude PROGRAM
          | clear --yes | repair --yes | rotate-key | key-export --out FILE | key-import --in FILE
          | stop | install | uninstall [--remove-data --yes] | autostart [on|off] | --version
```

## Your data and recovery

### Portable mode

Create an empty `portable.txt` beside the executable. All application data then lives in a `data`
folder beside it, regardless of the working directory. The folder must be writable. Portable mode
does not offer installation or autostart, and rejects `--db` overrides. To remove a portable copy,
quit it and delete its folder; the `uninstall` command is disabled.

On Windows the key is still protected by DPAPI for the signed-in account. Copying a portable folder
to another PC or account does **not** make its history readable there: use `key-export` and
`key-import` to transfer the key securely. Encryption is unchanged.

`pasteroll stop` asks the running copy to quit gracefully and succeeds if none is running. On Linux
it addresses the selected data folder (`--db FILE` outside portable mode); older running versions
without the control pipe must be quit from the tray. On Windows it addresses the current session's
instance. A copy installed by Setup is upgraded using Setup and removed through Settings > Apps.

### Installed data

History is a SQLite database encrypted per clip, in `%LOCALAPPDATA%\Pasteroll` (Windows) or
`~/.local/share/pasteroll` (Linux). **Make a recovery file** so a reinstall or a lost key file
cannot lose the history:

```
pasteroll key-export --out recovery.key     # protected by a passphrase you choose
pasteroll key-import --in recovery.key
```

Read the honest limits in [SECURITY.md](SECURITY.md).

## Verify your download

Each release has a `SHA256SUMS` file.

```powershell
# Windows
Get-FileHash .\pasteroll-0.1.0-beta.14-windows-x64-setup.exe -Algorithm SHA256
```
```sh
# Linux
sha256sum -c SHA256SUMS --ignore-missing
```

The value must match the line in `SHA256SUMS` exactly.

## Build from source

You need a recent stable [Rust](https://rustup.rs) and the platform's build toolchain (MSVC build
tools on Windows). Packaging additionally needs Inno Setup 6 on Windows and Python 3.11+ for the
release tools; Linux packages also need `musl-tools`, binutils and `dpkg-deb`.

```sh
git clone https://github.com/qpd-v/pasteroll
cd pasteroll
cargo build --release -p pasteroll-win      # Windows: target\release\pasteroll.exe
cargo build --release -p pasteroll-linux    # Linux: target/release/pasteroll
cargo clean -p pasteroll-win --profile dev    # removes a possibly stale same-name test binary
cargo test --workspace --exclude pasteroll-linux  # Windows native suite
cargo test --workspace --exclude pasteroll-win   # Linux
```

Release packages: `scripts/package-windows-installer.ps1` builds the setup EXE and portable ZIP;
`scripts/package-linux.sh` builds the static musl archive and `.deb` (the musl Rust target must be
installed beforehand). See [RELEASING.md](RELEASING.md) for tests, private-string scans, checksums
and the manual publication command. No packaging script downloads prerequisites.

The code is split into `pasteroll-core` (storage, crypto, settings), `pasteroll-render` (drawing),
`pasteroll-app` (the controller that is the same on every platform) and one thin crate per
platform (`pasteroll-win`, `pasteroll-linux`).

## Known limits

- Linux: X11 only. Wayland has no global input grabs for ordinary programs.
- Pasting into a program running as administrator (Windows) is blocked by Windows.
- The carousel is not excluded from screen capture (Windows Recall, screen sharing can see it).
- Not Authenticode-signed (Windows). Updates can be installed from the tray menu (optional update
  check, then **Download and install**; see PRIVACY.md and SECURITY.md), or by downloading Setup yourself.
- No macOS build.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. The name "Pasteroll" and its logo are not covered by these licenses; please do not present
a modified build as the official one.
