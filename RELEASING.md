# Releasing Pasteroll

Current version: **0.1.0-beta.16** (Windows only; Linux stays on **0.1.0-beta.4**), experimental
prerelease. A Windows-only release uses `--windows-only` for checksums and verification, publishes
just Setup, the portable ZIP and their SHA256SUMS, and leaves the Linux links on beta.4.
Use a reviewed source commit and build all six packages fresh. Never select assets
with a dist wildcard. There is no CI. The only network use in the program is the optional update
check and "Download and install" (Windows), so every release also carries a signed update manifest:
`python scripts/make-update-manifest.py` (before `checksums.py --windows-only`; it also writes
`dist/update-latest.txt`, the "newest version" note, from the `## In short` section of the release
notes), then `scripts/release-keys.ps1 sign` run by the maintainer on both files in one run (it asks for
the key passphrase), then `pasteroll update-verify` on the manifest, the note and their signatures (must
print PASS twice) and `make-update-manifest.py --verify`. A Windows-only release publishes six files:
Setup, the portable ZIP, the bare `pasteroll-<version>-windows-x64.exe`, the manifest, its signature and
SHA256SUMS. After the release is public, the note and its signature are committed as `updates/latest.txt`
and `updates/latest.txt.sig` (installed copies read the newest version from there) in a commit whose
subject starts with `Update pointer`.

## Scope and acceptance

The user approved this beta with further manual testing deferred. Record missing
physical/helper/login/lock/suspend and clean-machine Windows UAC/startup/uninstall
checks as NOT TESTED. Existing Pop mixed-display geometry acceptance is limited
to the recorded setup. Linux OCR accuracy and tray/panel/About parity are deferred.
Do not carry this exception forward to a later release without a new decision.

All five crate versions/path dependencies and Cargo.lock must agree. Public
README/packaging text, notes and actual artifacts must agree. Match GNOME app and
companion protocol 3; reload the companion after upgrades. Standard X11 and GNOME
Debian packages are alternatives. Windows builds remain unsigned.

## Automated checks

Use sanitized output and source/home path remapping; never inspect live history.
Windows: cargo test --workspace --exclude pasteroll-linux --locked --offline
Linux: cargo test --workspace --exclude pasteroll-win --locked --offline
Release tools: python scripts/test-release-tools.py
Geometry contract: node scripts/test-gnome-geometry.mjs
Use separate native-platform target directories. No failed test may be hidden by
filtering; ignored tests and physical acceptance are separate evidence.

## Fresh builds and package checks

Windows, with existing Rust/MSVC and Inno Setup 6:

```powershell
pwsh -NoProfile -File scripts/package-windows-installer.ps1
pwsh -NoProfile -File scripts/test-windows-packaging.ps1
python scripts/test-windows-about.py
python scripts/test-windows-update.py
```

Linux needs an existing Rust musl target, musl-gcc, binutils, dpkg-deb, Python 3.11+
and the GNU helper build prerequisites. Scripts never download missing tools.

```sh
bash scripts/package-linux.sh
python3 scripts/test-linux-packaging.py
PASTEROLL_PACKAGE_OUTPUT="$PWD/dist" bash scripts/package-gnome.sh
PASTEROLL_PACKAGE_OUTPUT="$PWD/dist" python3 scripts/test-gnome-packaging.py
```

Use fresh output/build directories or clean only the relevant app release outputs
before building. Windows-managed worktrees built in WSL require SOURCE_DATE_EPOCH
from Windows Git, since Linux Git cannot interpret their Windows .git pointer.
An ordinary native Linux checkout derives it normally.

Run isolated headless/controller/X11 suites against the exact staged release
binary where the environment supports them. They do not certify physical desktop
acceptance. After WSL tests, stop agent-opened GUI/tests, terminate the test distro,
and check elevated powercfg /requests for leaked WSLg display requests. Do not
apply requestsoverride or disrupt unrelated Docker/WSL workloads.

## Scan and freeze

PASTEROLL_SCAN_WORDS supplies private terms only through process environment;
never echo or save them. The local PowerShell wrapper derives available identity
terms without printing them. Scan source plus EVERY final artifact and supported
compressed payload. Inno's proprietary stream is not independently decoded:
scan its raw bytes and exact staged inputs and document that coverage limit.

```powershell
python scripts/checksums.py --with-gnome
python scripts/checksums.py --with-gnome --verify
pwsh -NoProfile -File scripts/verify-release-local.ps1 -WithGnome -CheckSource
```

The manifest covers exactly Windows Setup/ZIP, X11 tar/deb and GNOME-development
tar/deb. SHA256SUMS does not hash itself; scan it too. Keep final full commit,
source input hashes, tool versions, check results and artifact hashes in the
private release report. Rebuild if shipped inputs change.

## Publish after human authorization

Commit/review/integrate the exact source, verify main and the tag target, and
create a draft first. Select all six assets explicitly. Download draft assets to
a fresh directory, verify hashes, then publish the prerelease. Do not replace old
beta assets. Example, after the correct tag exists:

```sh
gh release create v0.1.0-beta.4 --repo qpd-v/pasteroll --verify-tag --draft --prerelease --title "Pasteroll 0.1.0-beta.4" --notes-file packaging/release-notes-0.1.0-beta.4.md dist/pasteroll-0.1.0-beta.4-windows-x64-setup.exe dist/pasteroll-0.1.0-beta.4-windows-x64-portable.zip dist/pasteroll-0.1.0-beta.4-linux-x86_64.tar.gz dist/pasteroll_0.1.0-beta.4_amd64.deb dist/pasteroll-0.1.0-beta.4-linux-x86_64-gnome-development.tar.gz dist/pasteroll_0.1.0-beta.4_amd64-gnome-development.deb dist/SHA256SUMS
```

Verify unauthenticated public downloads after publication. Website changes require
review of existing edits and separate authorization: update versions/downloads/
checksums and main-derived changelog, remove visible videos AND empty slots,
preserve originals locally, test desktop/mobile and verify the deployment.
Do not publish private coordination/handoff documents or candidate build folders.
