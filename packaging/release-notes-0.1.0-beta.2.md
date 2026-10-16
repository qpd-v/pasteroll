# Pasteroll 0.1.0-beta.2

Clipboard history you can scroll with the mouse wheel. Copy as usual, open the carousel with
**Ctrl+Shift+middle-click**, and paste a whole clip or just the part you select.

## New in this beta

- **Windows setup wizard:** install for your account or all users, choose shortcuts and optional
  start at sign-in, and upgrade without losing history. Uninstall keeps data unless you ask to
  delete it.
- **Portable mode:** the Windows portable ZIP includes `portable.txt`. On Linux, create that file
  beside the executable. All app data stays in the adjacent `data` folder.
- **Linux packages:** a static x86-64 archive with per-user/system `install.sh`, plus a Debian/Ubuntu
  `.deb`. Package removal keeps user data.
- **Expanded viewer:** read and search long text, zoom pictures, save clips, and extract text
  locally with Windows OCR or Tesseract on Linux.
- **Recycle bin:** restore deleted clips for 30 days, or delete them permanently.
- **Advanced Image Paste:** edit images before saving: crop, resize, outline, shadow, background,
  padding, rotation and formats, with undo/redo and reusable presets. The original clip stays intact.
- New app icon and a `stop` command that requests a graceful exit.

## Downloads

| Platform | File |
|---|---|
| Windows 10/11 x64 setup | `pasteroll-0.1.0-beta.2-windows-x64-setup.exe` |
| Windows portable | `pasteroll-0.1.0-beta.2-windows-x64-portable.zip` |
| Linux x86-64, X11 | `pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz` |
| Debian/Ubuntu amd64 | `pasteroll_0.1.0-beta.2_amd64.deb` |
| SHA-256 checksums | `SHA256SUMS` |

Windows setup defaults to your account, with no administrator rights. Installing for all users
requires UAC. The beta is unsigned, so SmartScreen may warn; verify the checksum first.

On Linux, extract the archive and run `./install.sh`, or `sudo ./install.sh --system` for all users.
For the Debian package use `sudo dpkg -i pasteroll_0.1.0-beta.2_amd64.deb`. An **X11 session** is
required. OCR and save-dialog helpers are optional and are not downloaded by the installers.

## Before upgrading

History is upgraded to schema 5 on first start; older versions cannot open it afterwards. Keep
a backup of your data folder with Pasteroll stopped, and make a passphrase-protected recovery
export with `key-export` before upgrading an existing history.

On Windows, DPAPI still protects the key for your account. A portable folder copied to another
PC/account cannot open its history there without `key-export` / `key-import`.

Everything stays on your computer. No telemetry, update checks, or cloud sync.

## Beta testing status

Windows and Linux automated tests, package checks, native Windows per-user silent installation,
reinstallation and removal, and isolated Linux X11 runtime checks passed. Download checksums and
local-identity scans passed before uploading.

Clean-machine Windows all-users/UAC, upgrade from the older beta, interactive data-deletion prompts,
and shortcut/sign-in integration still need wider testing. Clean Linux desktop tray integration
and behavior across multiple accounts also need wider testing. Please use synthetic clips when
testing those paths and back up an existing history before upgrading.
