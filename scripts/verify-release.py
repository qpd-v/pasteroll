#!/usr/bin/env python3
"""Scan every release artifact and inspect compressed payloads; never publish or create tags."""
import argparse
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
from release_artifacts import ROOT, artifact_names, digest, release_version, verify_checksums


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generic-only", action="store_true", help="preliminary check; never a full personal-string gate")
    parser.add_argument("--with-gnome", action="store_true", help="also require and scan both GNOME development packages")
    parser.add_argument("--windows-only", action="store_true", help="verify only Windows Setup and portable ZIP")
    args = parser.parse_args()
    try:
        names = artifact_names(release_version(), with_gnome=args.with_gnome, windows_only=args.windows_only)
        verify_checksums(ROOT / "dist", names)
        staged = ROOT / "target/windows-packaging/setup"
        if digest(staged / "pasteroll.exe") != digest(ROOT / "target/release/pasteroll.exe"):
            raise ValueError("Setup staged input does not match the release executable.")
        for name in names:
            print(f"Scanning {name}", flush=True)
            command = [sys.executable, str(ROOT / "scripts/scan_binary.py"), str(ROOT / "dist" / name)]
            if not args.generic_only:
                command.append("--require-personal-words")
            if name.endswith("-setup.exe"):
                command += ["--include-tree", str(staged), "--include-file", str(ROOT / "packaging/windows/pasteroll.iss")]
            result = subprocess.run(command, cwd=ROOT)
            if result.returncode:
                return result.returncode
        print("PRELIMINARY PASS: personal-word gate was not required." if args.generic_only else
              f"PASS: personal-string scans completed for all {len(names)} release artifacts and supported payloads.")
        print("Setup coverage: raw installer plus exact staged inputs; its proprietary compressed stream is not decoded.")
        return 0
    except (OSError, ValueError):
        print("Release verification failed: inspect checksums, versions and setup staging.")
        return 1


if __name__ == "__main__":
    sys.exit(main())
