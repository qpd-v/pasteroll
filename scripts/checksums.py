#!/usr/bin/env python3
"""Generate or strictly verify SHA256SUMS for the selected current-version assets."""
import argparse
import sys
sys.dont_write_bytecode = True
from pathlib import Path
from release_artifacts import ROOT, artifact_names, release_version, verify_checksums, write_checksums


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--with-gnome", action="store_true", help="include both GNOME development packages")
    parser.add_argument("--windows-only", action="store_true", help="require only Windows Setup and portable ZIP")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    try:
        names = artifact_names(release_version(), with_gnome=args.with_gnome, windows_only=args.windows_only)
        if args.verify:
            verify_checksums(args.dist, names)
            print(f"PASS: all {len(names)} release checksums match.")
        else:
            print(write_checksums(args.dist, names), end="")
    except (OSError, ValueError):
        parser.exit(1, "Checksum operation failed: check artifact presence, workspace versions and SHA256SUMS.\n")


if __name__ == "__main__":
    main()
