#!/usr/bin/env python3
"""Generate or strictly verify SHA256SUMS for the four current-version release assets."""
import argparse
import sys
sys.dont_write_bytecode = True
from pathlib import Path
from release_artifacts import ROOT, artifact_names, release_version, verify_checksums, write_checksums


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    try:
        names = artifact_names(release_version())
        if args.verify:
            verify_checksums(args.dist, names)
            print("PASS: all four release checksums match.")
        else:
            print(write_checksums(args.dist, names), end="")
    except (OSError, ValueError):
        parser.exit(1, "Checksum operation failed: check artifact presence, workspace versions and SHA256SUMS.\n")


if __name__ == "__main__":
    main()
