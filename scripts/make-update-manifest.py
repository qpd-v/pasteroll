#!/usr/bin/env python3
"""Writes dist/update-manifest.txt and the bare program file for Pasteroll's updater.

The updater (Windows) downloads two small files, `update-manifest.txt` and `update-manifest.txt.sig`
(a signature of the manifest, made with `scripts/release-keys.ps1 sign`), then the installer or the bare
program file the manifest names, and checks each against the manifest before running anything.

It also writes `dist/update-latest.txt`, the "newest version" note: the version, and the plain lines of
the release notes' "In short" section, which people are shown before they choose to install. That note
is signed too (`update-latest.txt.sig`, in the same signing run) and, after the release is public,
committed as `updates/latest.txt` and `updates/latest.txt.sig`: that is where installed copies learn
which version is the newest, so it cannot be changed by whoever controls the release list.

This script does not sign and never touches a private key. Run it after the packages are built and
before `checksums.py --windows-only`:

    python scripts/make-update-manifest.py            write the manifest, the note and the bare program file
    python scripts/make-update-manifest.py --verify   check that they match dist/ exactly
"""
import argparse
import hashlib
from pathlib import Path
import re
import sys
import zipfile
sys.dont_write_bytecode = True
from release_artifacts import ROOT, release_version

MAX_BYTES = 25 * 1024 * 1024  # the updater refuses anything larger
MAX_NOTES = 5  # the most lines the "newest version" note may carry
MAX_NOTE_CHARS = 160


# Present only in the end-to-end test build (cargo feature e2e-test-server); never in a release.
TEST_BUILD_MARKERS = (b"PASTEROLL_UPDATE_BASE", b"PASTEROLL_UPDATE_TEST_KEY", b"PASTEROLL_UPDATE_ANNOUNCE",
                      b"update-window-e2e", b"PASTEROLL_E2E_RESTART_NOTE")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def summary_lines(version):
    """The bullets under "## In short" in packaging/release-notes-<version>.md, as the plain lines the
    updater shows. They are signed and shown as they are, so they must be plain printable ASCII."""
    path = ROOT / "packaging" / f"release-notes-{version}.md"
    if not path.is_file():
        raise SystemExit(f"{path} is missing: write the release notes first.")
    text = path.read_text(encoding="utf-8").replace("\r\n", "\n")
    match = re.search(r"^## In short[ \t]*\n(.*?)(?=^## |\Z)", text, re.S | re.M)
    if not match:
        raise SystemExit(f'{path.name} needs an "## In short" section: 1 to {MAX_NOTES} bullets ("- ...") of plain text.')
    lines = [line[2:].strip() for line in match.group(1).splitlines() if line.startswith("- ")]
    if not 1 <= len(lines) <= MAX_NOTES:
        raise SystemExit(f'"## In short" in {path.name} needs 1 to {MAX_NOTES} bullets, found {len(lines)}.')
    for line in lines:
        if not line or len(line) > MAX_NOTE_CHARS or not all(0x20 <= ord(c) <= 0x7E for c in line):
            raise SystemExit(f'A bullet under "## In short" must be plain printable ASCII, at most {MAX_NOTE_CHARS} '
                             f"characters: {line!r}")
        if "**" in line or "`" in line:
            raise SystemExit(f"Remove the Markdown formatting from this bullet; it is shown as plain text: {line!r}")
    return lines


def latest_text(version):
    return f"pasteroll-update-latest 1\nversion {version}\n" + "".join(f"note {n}\n" for n in summary_lines(version))


def build(dist):
    version = release_version()
    setup_name = f"pasteroll-{version}-windows-x64-setup.exe"
    zip_name = f"pasteroll-{version}-windows-x64-portable.zip"
    exe_name = f"pasteroll-{version}-windows-x64.exe"
    setup = (dist / setup_name).read_bytes()
    with zipfile.ZipFile(dist / zip_name) as archive:
        matches = [n for n in archive.namelist() if n == "pasteroll.exe"]
        if len(matches) != 1:
            raise SystemExit("The portable ZIP must contain exactly one pasteroll.exe.")
        exe = archive.read("pasteroll.exe")
    leaked = [m.decode() for m in TEST_BUILD_MARKERS if m in exe]
    if leaked:
        raise SystemExit(f"The program file contains test-build code ({', '.join(leaked)}): it was built with "
                         "the e2e-test-server feature. Rebuild the release without it.")
    # Setup carries its own copy of the program, compressed, which the scan above cannot see: it must be
    # byte for byte the file that was scanned (both come from the staged release folder)
    staged = ROOT / "target" / "windows-packaging" / "setup" / "pasteroll.exe"
    if not staged.is_file() or staged.read_bytes() != exe:
        raise SystemExit("The program inside Setup (target/windows-packaging/setup/pasteroll.exe) is missing or is not the "
                         "program in the portable ZIP. Rebuild both packages from the same build.")
    for name, data in ((setup_name, setup), (exe_name, exe)):
        if not 0 < len(data) <= MAX_BYTES:
            raise SystemExit(f"{name} is empty or larger than the updater accepts.")
    text = (
        "pasteroll-update-manifest 1\n"
        f"version {version}\n"
        f"setup {setup_name} {len(setup)} {sha256(setup)}\n"
        f"exe {exe_name} {len(exe)} {sha256(exe)}\n"
    )
    return version, exe_name, exe, text, latest_text(version)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    version, exe_name, exe, text, latest = build(args.dist)
    manifest = args.dist / "update-manifest.txt"
    note = args.dist / "update-latest.txt"
    bare = args.dist / exe_name
    if args.verify:
        if not manifest.is_file() or manifest.read_bytes() != text.encode("ascii"):
            raise SystemExit("update-manifest.txt does not match the packages in dist/.")
        if not note.is_file() or note.read_bytes() != latest.encode("ascii"):
            raise SystemExit("update-latest.txt does not match the release notes (or is missing).")
        if not bare.is_file() or bare.read_bytes() != exe:
            raise SystemExit("The bare program file does not match the one in the portable ZIP.")
        print(f"PASS: update-manifest.txt, update-latest.txt and {exe_name} match the {version} packages and notes.")
        return
    bare.write_bytes(exe)
    manifest.write_bytes(text.encode("ascii"))
    note.write_bytes(latest.encode("ascii"))
    for name in ("update-manifest.txt.sig", "update-latest.txt.sig"):
        stale = args.dist / name
        if stale.exists():
            stale.unlink()  # a signature of an older file must never be uploaded or published with this one
    print(f"Wrote {manifest.name}, {note.name} and {exe_name} for {version}. Now sign both (one passphrase):")
    print(f"  pwsh -NoProfile -File scripts/release-keys.ps1 sign -KeyFile <primary.key.enc> "
          f"-InFile dist/update-manifest.txt,dist/update-latest.txt "
          f"-OutFile dist/update-manifest.txt.sig,dist/update-latest.txt.sig")


if __name__ == "__main__":
    main()
