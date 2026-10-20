# Pasteroll

A clipboard history you can scroll with the mouse wheel. Copy as usual; when you need an earlier
clip, press **Ctrl+Shift+middle-click**, scroll through your history right where you are working,
and paste the whole clip, or select just a part of it and paste only that.

**Status: public beta (`0.1.0-beta.3`).** It works and is tested, but it is young. Expect rough
edges and please [report them](https://github.com/qpd-v/pasteroll/issues).

| | Windows 10 / 11 | Linux (X11) |
|---|---|---|
| Download | [Setup EXE](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.3/pasteroll-0.1.0-beta.3-windows-x64-setup.exe) · [Portable ZIP](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.3/pasteroll-0.1.0-beta.3-windows-x64-portable.zip) | [Static archive](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.3/pasteroll-0.1.0-beta.3-linux-x86_64.tar.gz) · [Debian/Ubuntu .deb](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.3/pasteroll_0.1.0-beta.3_amd64.deb) |
| Tray icon | yes | StatusNotifier (KDE, GNOME with AppIndicator, Ubuntu) or XEmbed |
| Wayland | n/a | experimental GNOME 46; separate packages and setup below |
| macOS | not available | |

[Release notes and testing status](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.3) ·
[SHA256SUMS](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.3/SHA256SUMS)

## Next Windows update: 0.1.0-beta.4

The Windows candidate adds the transparent roll icon, About/version information,
and freehand image OCR areas. In the expanded viewer, select the lasso, circle one
or more text areas, and press Enter or Extract selected text. Ctrl+Z undoes an area,
Delete clears areas, and Escape cancels. OCR stays local and leaves the original
image unchanged. Windows candidate builds are local until release approval;
the public downloads above and Linux packages remain beta.3.

## Experimental GNOME 46 Wayland support

Beta.3 adds native GNOME 46 Wayland support through a Shell companion.
Ctrl+Shift+middle-click requires the optional administrator-enabled host input helper;
explicit keyboard mode uses Ctrl+Shift+Space. See [GNOME setup](extensions/INPUT_HELPER.md).
**Physical desktop, login/lock and suspend testing has not been performed.** Automated
headless sessions and synthetic kernel input passed; treat this backend as experimental.
Other GNOME versions and other Wayland compositors are not supported.

- [GNOME 46 experimental archive](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.3/pasteroll-0.1.0-beta.3-linux-x86_64-gnome-development.tar.gz)
- [GNOME 46 experimental Debian/Ubuntu package](https://github.com/qpd-v/pasteroll/releases/download/v0.1.0-beta.3/pasteroll_0.1.0-beta.3_amd64-gnome-development.deb)

The X11 and GNOME Debian packages are alternatives, not coinstallable. The GNOME
helper stays disabled after a fresh package install. The development suffix identifies
the experimental packages; it does not indicate broader Wayland compatibility.

## Features

- **Scroll your history** with the mouse wheel or arrow keys; the carousel opens at the pointer.
- **Search as you type** (file names too) and filter by Text, Document, Image, Video, Audio,
  Compressed, Pinned, or filters you define.
- **Partial paste**: double-click a word or drag across text in the preview, then Enter pastes
  only the selection.
- **Expanded viewer**: the card's Expand button (or Ctrl+E) grows a clip into a large window:
  long text to read and search, pictures to zoom and pan, Save as, Info, and Extract text (OCR, on
  this computer). F11 for fullscreen, Esc to shrink it back.
- **Advanced Image Paste**: Ctrl+click the desktop (the key is a setting) with an image chosen and
  the picture grows into an editor instead of being saved at once: outline, resize, crop, rounded
  corners, drop shadow, background removal or fill, padding, rotate and flip, PNG / JPG / WebP / BMP
  with quality, a file name and folder of your choice, undo and redo, and presets you can reuse. A
  plain click on the desktop still saves the picture immediately. The clip itself is never changed.
- **Pin** clips (Ctrl+P) so they are never removed automatically.
- **Recycle bin**: deleted clips can be brought back (Ctrl+Z, or the Recycle bin filter) for 30
  days; Shift+Delete deletes for good.
- **Images and files** are kept too, with fast cached previews.
- **Private by design**: stored encrypted on your computer, nothing is ever sent anywhere
  (see [PRIVACY.md](PRIVACY.md)).
- **Skips password managers** that mark their copies as sensitive, and programs you exclude.
- **Terminal safe**: asks for a second Enter before pasting several lines into a terminal.
- **Adjustable size**: 50 % to 200 % in Settings.
- Starts with your session only if you turn that on.

## Install

### Windows

1. Download `pasteroll-0.1.0-beta.3-windows-x64-setup.exe` and `SHA256SUMS` from the
   [beta.3 release](https://github.com/qpd-v/pasteroll/releases/tag/v0.1.0-beta.3) and
   [verify the checksum](#verify-your-download).
2. Run the setup wizard. **Only for me** is the default and needs no administrator rights;
   **For all users** needs UAC. Start menu is on by default; desktop shortcut and autostart are off.

For portable use, extract `pasteroll-0.1.0-beta.3-windows-x64-portable.zip` and run `pasteroll.exe`.
Its included `portable.txt` keeps application data in the adjacent `data` folder.

> **"Windows protected your PC"**: the beta is **not code-signed yet**, so SmartScreen shows
> this for new programs. Click **More info → Run anyway** after checking the checksum. Some
> antivirus programs are also wary of anything that watches the keyboard and clipboard (which a
> clipboard manager must); the source is here for you to read and build yourself.

### Linux

```sh
tar xzf pasteroll-0.1.0-beta.3-linux-x86_64.tar.gz
cd pasteroll-0.1.0-beta.3-linux-x86_64
./install.sh              # per-user: ~/.local/bin, menu entry and icon
pasteroll run
```

For all users, use `sudo ./install.sh --system`. On Debian/Ubuntu, an alternative is
`sudo dpkg -i pasteroll_0.1.0-beta.3_amd64.deb`. Neither installer enables autostart.
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

With an image chosen in the carousel, a click on the desktop saves it as a PNG right away. Hold
**Ctrl** while you click (Settings lets you pick Shift or Alt instead) and the picture opens in the
editor. The settings are grouped in the panel on the right, one section open at a time; the
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

The bar at the bottom holds the file name, the folder to save into (the Desktop unless you choose
another; it is remembered) and **Paste Image**. **Cancel**, Esc or the back arrow return to the
carousel without saving. Undo and redo (Ctrl+Z, Ctrl+Y) and **Reset all** are at the top, next to
**Presets**: save the current settings under a name and apply them to any picture later (a
preset keeps everything except the crop). Enter pastes; while you type in a box Enter finishes the
typing instead. The editor is modal: clicks outside it do nothing, and it closes after ten
minutes without input.

Tray icon: pause recording, Settings, open data folder, quit. Settings has the history limits,
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
Get-FileHash .\pasteroll-0.1.0-beta.3-windows-x64-setup.exe -Algorithm SHA256
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
- Not signed yet (Windows), no auto-update: download new versions yourself.
- No macOS build.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. The name "Pasteroll" and its logo are not covered by these licenses; please do not present
a modified build as the official one.
