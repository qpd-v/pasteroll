# Windows packages

From the repository root, with Rust and **Inno Setup 6** installed:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-windows-installer.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-packaging.ps1
```

The first command builds both `dist/pasteroll-0.1.0-beta.12-windows-x64-setup.exe` and
`dist/pasteroll-0.1.0-beta.12-windows-x64-portable.zip`. It reads version, publisher, website and
license from Cargo metadata, remaps build paths, and builds offline with the lockfile. Rust's
dependencies must already be available locally. The script checks for ISCC and does not install it.
Use `-Iscc PATH` for a nonstandard Inno Setup 6 location.

The portable ZIP alone can be rebuilt with `scripts/package-windows.ps1`. It contains the executable,
an empty `portable.txt`, the README, both licenses and third-party notices. The optional portable
wizard mode is omitted: the ZIP provides extraction without installation or elevation.

## Setup behavior

- The privilege choice defaults to **Only for me**. Its default destination matches the app's
  self-install: `%LOCALAPPDATA%\Programs\Pasteroll`. **For all users** requests elevation and uses
  Program Files. Data remains per-account in either mode.
- The Start menu shortcut defaults on; desktop shortcut and start at sign-in default off.
  Start at sign-in is set by the app's `autostart on` command using the original user's token,
  so the Run value has the exact quoted `"<exe>" run` form the app uses.
- Start Setup normally. If you explicitly run it as another administrator, that administrator is
  its original user; user-specific tasks and the finish-page launch apply to that account.
- The fixed AppId supports upgrades. A new embedded control helper requests graceful shutdown,
  including from older builds without a public `stop` command. It uses Windows-only
  `stop --if-exe FILE` to target only the program being replaced/removed. Another installation's
  running copy and startup preference are left alone. No process is force-killed.
  Quit copies in other logged-in sessions before upgrading or removing an all-users installation.
- Upgrading a self-install in the same folder retires its old Apps entry and preserves autostart.
  `setup.txt` identifies Setup ownership. Setup refuses to overwrite a portable folder.
- **Run Pasteroll** is checked on the finish page and is skipped during silent installs.
- Both license texts appear in the license page and are installed as separate files.

```powershell
.\pasteroll-0.1.0-beta.12-windows-x64-setup.exe /VERYSILENT /CURRENTUSER /NORESTART
.\pasteroll-0.1.0-beta.12-windows-x64-setup.exe /VERYSILENT /ALLUSERS /NORESTART
```

`/CURRENTUSER` needs no administrator rights. `/ALLUSERS` requires UAC approval, or an already
elevated process for unattended use. Tasks can be selected with `/TASKS="startmenu,desktopicon,autostart"`
or all disabled with `/TASKS=""`.

## Removal

Use Settings > Apps. Interactive uninstall asks **Also delete my clipboard history and settings?**,
default **No**. A silent uninstall (`unins000.exe /VERYSILENT /NORESTART`) always keeps data. Data
cleanup invokes `pasteroll.exe uninstall --data-only --remove-data --yes`; it uses the app's own
exclusive history lock, recognizes custom database filenames, and keeps unrelated files.

For all-users removal, the account running the elevated uninstaller is the cleanup account. If UAC
uses different administrator credentials, its data is the administrator's data. Other accounts'
history is always retained. To delete your own data before such a removal, run `pasteroll stop`,
then `pasteroll uninstall --data-only --remove-data --yes` from your normal account. This bypasses
the Setup ownership guard only for data removal; the program remains installed.

The uninstaller removes its Apps registration and shortcuts, and removes the current account's
autostart value only if it points to this installation. Other accounts can disable their per-user
autostart in Settings before system-wide removal; unloaded user registry hives are not modified.

## Signing and verification

These beta artifacts are unsigned. The `.iss` has a disabled signing hook; sign the application
before staging and enable installer/uninstaller signing when a certificate is available.
[SignPath Foundation](https://signpath.org/) offers free signing for eligible open-source projects.
Signing is outside this phase.

The packaging test checks archive contents, markers, payload equality, version resources, and
harmless portable CLI behavior. It does not install or start clipboard capture. Clean-machine
verification still needs per-user and all-users install, wizard defaults, shortcut icons, upgrade
over a self-install and a Setup install, optional autostart, silent flags, portable launch, and
uninstall with both data choices. Final personal-string scans and release checksums are Phase 4.

## Native current-host test

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-install.ps1
```

This test refuses to run if the Setup AppId is already registered, and creates only a new temporary
installation under ProgramData. It disables every task, does not launch capture, and never opens
the live clipboard/history. It verifies real scoped WM_CLOSE delivery with two message-only probe
processes, actual current-user silent installation, Setup ownership, same-version in-place reinstall,
synthetic data-only cleanup and silent uninstall. The live Pasteroll process, autostart value, legacy
registration and shortcut hashes must remain unchanged. Its temporary files and registration are
removed afterwards; only a result report under `target-dev/` remains.

This is a native Windows current-host test, not a clean machine, older-binary upgrade, UAC/all-users,
shortcut/autostart creation, or interactive data-deletion-dialog test. Those remain separate checks.

## Windows package checks

`python scripts/test-windows-about.py` checks portable/standalone/Setup About text
and opens/closes its own native dialog without clipboard capture or history access.
`python scripts/test-windows-update.py` uses the exact previous-release (beta.4) and current
executable payloads and production Inno recipe with a disposable AppId and shortcut names.
It checks retained synthetic history/key/config/presets, upgrades, reinstall,
shortcuts and uninstall while preserving the live installation and autostart.
The verified beta.4 portable ZIP must exist in the documented local release cache
(`target-dev/beta4-release/github-download/`; check its hash against the published SHA256SUMS).
It does not verify all-users/UAC, enabling autostart or the interactive data-deletion
prompt; those still need a disposable Windows account/VM and user acceptance.
Use `python scripts/checksums.py --windows-only` and
`powershell -NoProfile -File scripts/verify-release-local.ps1 -WindowsOnly -CheckSource`.
Windows-only releases keep the Linux downloads on their last published release (beta.4).
