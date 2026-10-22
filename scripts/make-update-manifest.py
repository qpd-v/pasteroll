#!/usr/bin/env python3
"""Writes dist/update-manifest.txt and the bare program file for Pasteroll's updater.

The updater (Windows) downloads two small files, `update-manifest.txt` and `update-manifest.txt.sig`
(a signature of the manifest, made with `scripts/release-keys.ps1 sign`), then the installer or the bare
program file the manifest names, and checks each against the manifest before running anything.

This script does not sign and never touches a private key. Run it after the packages are built and
before `checksums.py --windows-only`:

    python scripts/make-update-manifest.py            write the manifest and the bare program file
    python scripts/make-update-manifest.py --verify   check that the manifest matches dist/ exactly
"""
import argparse
import hashlib
from pathlib import Path
import sys
import zipfile
sys.dont_write_bytecode = True
from release_artifacts import ROOT, release_version

MAX_BYTES = 25 * 1024 * 1024  # the updater refuses anything larger


# Present only in the end-to-end test build (cargo feature e2e-test-server); never in a release.
TEST_BUILD_MARKERS = (b"PASTEROLL_UPDATE_BASE", b"PASTEROLL_UPDATE_TEST_KEY", b"PASTEROLL_UPDATE_ANNOUNCE",
                      b"update-dialog-preview")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


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
    return version, exe_name, exe, text


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    version, exe_name, exe, text = build(args.dist)
    manifest = args.dist / "update-manifest.txt"
    bare = args.dist / exe_name
    if args.verify:
        if not manifest.is_file() or manifest.read_bytes() != text.encode("ascii"):
            raise SystemExit("update-manifest.txt does not match the packages in dist/.")
        if not bare.is_file() or bare.read_bytes() != exe:
            raise SystemExit("The bare program file does not match the one in the portable ZIP.")
        print(f"PASS: update-manifest.txt and {exe_name} match the {version} packages.")
        return
    bare.write_bytes(exe)
    manifest.write_bytes(text.encode("ascii"))
    stale = args.dist / "update-manifest.txt.sig"
    if stale.exists():
        stale.unlink()  # a signature of an older manifest must never be uploaded with this one
    print(f"Wrote {manifest.name} and {exe_name} for {version}. Now sign the manifest:")
    print(f"  pwsh -NoProfile -File scripts/release-keys.ps1 sign -KeyFile <primary.key.enc> "
          f"-InFile dist/update-manifest.txt -OutFile dist/update-manifest.txt.sig")


if __name__ == "__main__":
    main()
