#!/usr/bin/env python3
"""Check public source candidates for locally derived private terms without printing them."""
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
from scan_binary import count_word

ROOT = Path(__file__).resolve().parent.parent


def main():
    terms = [term.strip() for term in os.environ.get("PASTEROLL_SCAN_WORDS", "").split(",") if term.strip()]
    if not terms:
        raise SystemExit("NOT SCANNED: private terms are required.")
    result = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
                            cwd=ROOT, capture_output=True, check=True)
    count = 0
    for name in set(result.stdout.split(b"\0")) - {b""}:
        path = ROOT / os.fsdecode(name)
        if not path.is_file():
            continue
        data = path.read_bytes()
        if any(count_word(data, term) for term in terms):
            raise SystemExit("FAIL: private text found in a public source candidate. Values were not printed.")
        count += 1
    print(f"PASS: {count} public source candidates contain none of the private terms.")


if __name__ == "__main__":
    main()
