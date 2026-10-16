# Local release process

Current candidate: **0.1.0-beta.2**. Builds are unsigned. All work runs locally; no CI, automatic
updates or installer downloads are introduced. Publishing, committing, pushing and tagging remain
manual actions requiring the user's instruction.

## 1. Review versions and source

- Bump all five `crates/pasteroll-*/Cargo.toml` package versions and their internal path dependency
  versions together. Update the five workspace entries in `Cargo.lock` using Cargo's normal lockfile
  update, then verify with `cargo metadata --locked --offline --no-deps --format-version 1`.
- Update `CHANGELOG.md`, the README beta status, both `packaging/README-*.txt` files, packaging
  guides, and the release-notes filename/content for the chosen version.
- Version, publisher, homepage and license are taken from Cargo metadata for executable/installer
  resources. Keep the Inno AppId in `packaging/windows/pasteroll.iss` unchanged for upgrades.
- For a new icon, regenerate from the approved source with `python scripts/make-icons.py SOURCE`.
  Pillow is needed only for icon generation. Do not replace the approved artwork silently.
- Review the intended diff. Keep `docs/`, `HANDOFF.md`, `NEXT.md`, local history/key files and build
  directories out of commits. No personal names, addresses, folders or session links in public text.
- The source must eventually be committed, reviewed and tagged before publication. The current
  uncommitted working tree is a candidate, not the source of an existing release tag.

The exact README/website filename edits requested for review are in
[`packaging/download-filename-changes.md`](packaging/download-filename-changes.md). The separate
website repository is not edited by this process. README changes have now been applied; the listed
website changes remain a separate task under the user's explicit no-website-edit constraint.

## 2. Tests

Windows, from the repository root:

```powershell
cargo clean -p pasteroll-win --profile dev --target-dir target
cargo test --workspace --exclude pasteroll-linux --target-dir target --locked --offline
python scripts/test-release-tools.py
```

Ubuntu/WSL, from the same repository root:

```sh
cargo test --workspace --exclude pasteroll-win --locked --offline
python3 scripts/test-release-tools.py
```

The workspace contains platform binaries with the same output name. The requested Windows
`cargo test --workspace` was also exercised, but it can build the Linux stub over the Windows test
executable and fail crash tests with `pasteroll-linux only runs on Linux`. Exclusion alone can reuse
that overwritten executable from Cargo's cache. The scoped debug clean above removes only the
Windows crate's debug artifacts before the native run; release artifacts and dependencies remain.
Use this clean plus native-platform exclusion as the release gate; WSL covers Linux separately.
All native unit/integration tests still need to pass. Ignored screenshot/timing tests
do not establish GUI success. Use a separate target directory such as `target-dev/linux` for WSL
tests while building Windows packages. Tests use synthetic/scratch history; never inspect live
clipboard data or the user's real database as a release test.

## 3. Build all four packages

Required Windows tools: Rust/MSVC build tools and an existing **Inno Setup 6** installation.
Release tools require Python **3.11+**. Linux additionally needs the Rust musl target, `musl-tools`,
binutils and `dpkg-deb`. Install prerequisites separately; packaging scripts do not fetch them.

Windows:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-windows-installer.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-packaging.ps1
```

When no Setup-owned Pasteroll is already registered, the optional native current-host check is:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-windows-install.ps1
```

It uses a temporary ProgramData folder and synthetic data, disables all tasks, and preserves the
live process/startup/legacy registration/shortcuts. It tests actual per-user installation, same-version
reinstallation, scoped shutdown and silent removal. It cannot establish clean-machine, older-binary
upgrade, all-users/UAC, shortcut/autostart creation or interactive deletion-dialog success.

Ubuntu/WSL:

```sh
bash scripts/package-linux.sh
python3 scripts/test-linux-packaging.py
```

Cargo uses `--release --locked --offline`, with build paths remapped. The Windows command builds
setup and portable packages; the Linux command builds the static archive and `.deb`. Each command
replaces its current-version packages. Do not ship old unversioned files from `dist/` or use a wildcard
to select assets. For a forced app rebuild, first run `cargo clean -p pasteroll-win --release` on
Windows and `cargo clean -p pasteroll-linux --release --target x86_64-unknown-linux-musl` in WSL.
These clean only build outputs for the named package, not source or user data.

| Asset | Purpose |
|---|---|
| `pasteroll-0.1.0-beta.2-windows-x64-setup.exe` | Windows wizard, per-user/all-users |
| `pasteroll-0.1.0-beta.2-windows-x64-portable.zip` | Portable executable, empty marker, README/licenses |
| `pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz` | Static Linux binary, `install.sh`, desktop entry/icons/licenses |
| `pasteroll_0.1.0-beta.2_amd64.deb` | Debian/Ubuntu; internal version `0.1.0~beta.2` |
| `SHA256SUMS` | SHA-256 hashes of exactly the four files above |

The `.deb` has no maintainer scripts or package dependencies. Optional recommendations are
Tesseract and Zenity. Removal and purge keep user data. AppImage is omitted to avoid another
bundler/runtime; the static archive serves other x86-64 distributions.

## 4. Identity scans and checksums

For unattended verification on Windows, the local wrapper derives terms from the account/profile/display name,
machine and configured Git name/email, merges any existing terms, and sets the variable only in its
own process. It does not save or echo the values:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-release-local.ps1
```

Add `-CheckSource` before committing to scan tracked and non-ignored source candidates for the
same private terms. The source scan does not echo matching values or filenames. Generic profile
path examples in tests are allowed; actual local account/profile/display-name/email terms are not.

The public GitHub owner already used in project URLs/licenses is treated as a public project
identifier when it is also the Git display name; actual local account/profile and configured email
terms are still checked. This covers locally available identity, not unknown alternative emails.
Provide extra environment terms if another private identity must be included.

Set `PASTEROLL_SCAN_WORDS` in the shell running verification: comma-separated private account name,
email and any other builder-specific strings. Never put the values in a repository file, command
argument, log, release note or chat output. Do not echo the variable. A variable set in another
terminal is not automatically inherited by an already running Codex process.

If the variable was deliberately stored in Windows user environment settings, load it without
printing it:

```powershell
$env:PASTEROLL_SCAN_WORDS = [Environment]::GetEnvironmentVariable('PASTEROLL_SCAN_WORDS', 'User')
python scripts/checksums.py
python scripts/verify-release.py
```

If it is already set in this shell, omit the assignment. `scripts/checksums.sh` is the Linux wrapper
for the same strict checksum generator. Re-run it whenever any asset is rebuilt.

`verify-release.py` requires private scan terms and invokes `scripts/scan_binary.py` on **each** of
the four assets. The scanner checks UTF-8 and both UTF-16 byte orders. It inspects ZIP entries,
gzip/tar streams, and Debian control/data archives as well as outer bytes. Terms and matching
content are not printed. Corrupt, unsupported or oversized containers fail closed.

**Inno coverage limit:** the scanner does not decode Inno's proprietary compressed stream. It scans
the setup EXE's outer bytes, the exact staged release inputs, and the `.iss` source; the staged EXE
must match the fresh release EXE. This is not a claim that the complete proprietary stream was
independently extracted and inspected. Clean-machine install tests below remain required.

For a preliminary check while private terms are unavailable:

```powershell
python scripts/verify-release.py --generic-only
```

That mode prints **PRELIMINARY PASS** and is not the final personal-string gate. Standalone scans
can be run as `python scripts/scan_binary.py FILE --require-personal-words`. Use `--include-tree DIR`
for additional known input trees, such as setup staging. No scan extracts files to disk or starts them.

Verify the finished checksum manifest:

```powershell
python scripts/checksums.py --verify
```

On Linux, with all four files present:

```sh
cd dist
sha256sum -c SHA256SUMS
```

On a download containing only one platform's files, `sha256sum -c SHA256SUMS --ignore-missing` is
appropriate. `SHA256SUMS` does not hash itself. The generator requires all four current-version
artifacts and ignores every stale/unrelated file in `dist/`.

## 5. Clean-machine checklist (human verification)

An additional unattended runtime check is available when Xvfb, xterm, xdotool, xclip, xwininfo,
xwd, ImageMagick, unshare/setpriv and unattended sudo are already available in Linux/WSL:

```sh
bash scripts/test-linux-x11.sh
```

It creates private mount/network namespaces and a private X server, drops back to the invoking
Linux UID, and uses only synthetic data. It does not connect to the real WSLg display/clipboard.
The actual release executable is exercised for activation, paste, viewer/fullscreen, image editor,
PNG save, search, pin/recycle/restore and graceful stop. Screenshots/results are under
`target-dev/linux-x11-smoke/`. These checks are supplementary to a clean desktop VM, particularly
for tray integration, system permissions and UAC.

Use Windows 11 Sandbox/VM and an Ubuntu X11 VM with **synthetic** clips. Keep copies of the final
candidate assets/checksums there. Do not use a personal clipboard database. Automated package tests
in scratch directories are separate evidence from this checklist.

### Windows 11

- [ ] Verify the downloaded setup/portable hashes against `SHA256SUMS`.
- [ ] Run setup normally. Only for me is the default; no UAC is requested for that choice.
- [ ] Check welcome, both license texts, destination, tasks, summary and finish pages.
- [ ] Check Start menu defaults on; desktop shortcut and start at sign-in default off.
- [ ] Finish with Run Pasteroll checked. Confirm no self-install prompt, correct icons and tray.
- [ ] Copy synthetic text/image/files; exercise carousel, partial paste, viewer, recycle bin and image editor.
- [ ] Quit gracefully with `pasteroll.exe stop`; repeat with nothing running and confirm success.
- [ ] Install For all users with UAC, then sign in as two accounts. Confirm independent history/settings.
- [ ] Enable optional start at sign-in. Confirm only the intended account has `"<exe>" run` under HKCU Run.
- [ ] Test UAC using separate administrator credentials. Confirm user-specific behavior follows the
  original Setup account, as documented; avoid starting Setup with Run as another user.
- [ ] Extract the portable ZIP into a writable folder. Confirm `data` stays beside the executable,
  no install prompt/registry shortcuts, and the sign-in control explains it is unavailable.
- [ ] Confirm portable `install`, `uninstall`, autostart changes and `--db` are rejected.
- [ ] Copy portable data to a different account/VM. Confirm DPAPI binding remains enforced;
  test explicit key export/import using synthetic data if cross-account migration is wanted.
- [ ] Upgrade over a beta.1 self-install in the default per-user folder. Confirm graceful stop,
  no duplicate Apps entry, retained settings/history/autostart, and no second install prompt.
- [ ] Upgrade over a Setup-owned candidate. Confirm one Apps entry and retained data.
- [ ] Test `/VERYSILENT /CURRENTUSER /NORESTART` in a fresh account; confirm no auto-launch.
- [ ] Test `/VERYSILENT /ALLUSERS /NORESTART` from an elevated terminal in the VM.
- [ ] Uninstall with the default No data-deletion choice. Reinstall and confirm retained data.
- [ ] Uninstall again with Yes using synthetic history. Confirm known data files are removed and
  an unrelated file placed in that data folder survives.
- [ ] Confirm silent uninstall always retains data. Other users' histories always remain.
- [ ] Confirm all-users cleanup affects the account running the elevated uninstaller; with other
  admin credentials, use the documented data-only command from the normal account beforehand.

### Ubuntu X11

- [ ] Confirm the login session is X11, not native Wayland. Verify hashes.
- [ ] Install the archive per-user twice. Check executable, menu/icon, no automatic startup.
- [ ] Run the static binary and test synthetic text/image/files, viewer, image editor and stop.
- [ ] Preserve an enabled per-user autostart preference through an archive upgrade.
- [ ] Remove with `./install.sh --uninstall` twice; confirm history/settings survive.
- [ ] Install system-wide with `sudo ./install.sh --system` twice. Confirm `/usr/local` locations,
  readable icons/licenses, no root history, and independent data for a second account.
- [ ] Remove with `sudo ./install.sh --system --uninstall`; confirm data and unrelated files survive.
- [ ] Install the `.deb` with `sudo dpkg -i pasteroll_0.1.0-beta.2_amd64.deb`; confirm menu/icons and startup.
- [ ] Upgrade/reinstall the `.deb`, remove it, reinstall it, then purge it. Confirm history survives each path.
- [ ] Test portable mode from an extracted archive with `portable.txt`, including disabled installation/autostart.
- [ ] Optionally test local OCR and save dialogs with Tesseract/Zenity installed separately.

## 6. Publish manually after approval

Before publishing, require all of: native tests/package checks pass, the final personal-string scan
passes, hashes match, clean-machine checks are accepted, download wording is updated, and the source
has been explicitly approved for commit/push/tag. The remote tag **v0.1.0-beta.2** must point to
the reviewed source that produced these files. Rebuild if any shipped source, input or packaging
text changes. No tool in this process creates or pushes a tag.

After the user has created/pushed the correct tag, this is the exact PowerShell publication command
from the repository root. **Do not execute it as part of preparation:**

```powershell
gh release create v0.1.0-beta.2 `
  --repo qpd-v/pasteroll `
  --verify-tag `
  --prerelease `
  --title "Pasteroll 0.1.0-beta.2" `
  --notes-file packaging/release-notes-0.1.0-beta.2.md `
  dist/pasteroll-0.1.0-beta.2-windows-x64-setup.exe `
  dist/pasteroll-0.1.0-beta.2-windows-x64-portable.zip `
  dist/pasteroll-0.1.0-beta.2-linux-x86_64.tar.gz `
  dist/pasteroll_0.1.0-beta.2_amd64.deb `
  dist/SHA256SUMS
```

`--verify-tag` prevents accidental tagging of an older remote branch. This command uploads exactly
the four packages and their manifest. It creates a public prerelease; it is not a local preview.
No release, commit, push or tag has been authorized merely by running preparation scripts.

To stage the same assets for final review, add `--draft` to the creation command. A draft is not
publicly downloadable. Inspect its notes, tag and exactly five uploaded assets, download them into
a separate local directory, and verify their hashes. Publishing an existing draft after explicit
approval uses `gh release edit v0.1.0-beta.2 --repo qpd-v/pasteroll --draft=false --prerelease`.
Do not run `release create` a second time for an existing draft.

## 7. Website demo recordings

The website's four media slots accept `demo.gif`, `carousel.gif`, `viewer.gif` and `editor.gif` in
`assets/media/`; MP4 files with those names take precedence when both formats exist.

`bash scripts/record-linux-demos.sh` records the actual Linux release in a private X server with
synthetic clipboard contents and an isolated profile. It requires the existing tools listed for
the X11 smoke test. The action subtitles are added to the recording; they are not app controls.
Frames and recording metadata go to ignored `target-dev/site-media/`. The recorder refuses to
overwrite existing frames. `python scripts/encode-demo-media.py` makes looping GIFs and smaller
MP4 alternatives with an existing local FFmpeg. No tool downloads software or media.

Inspect each recording before copying it into the separate website repository. That repository
and its deployment require their own authorization; producing recordings does not deploy them.
