#!/usr/bin/env python3
"""Checks a built program for the builder's identity: user name, e-mail, local folders.

Usage: python scripts/scan_binary.py FILE [WORD ...]

It always looks for the generic traces of a build machine ("Users\\" and "/home/"). Put your own
words (your account name, e-mail, project folders) on the command line, or in the environment
variable PASTEROLL_SCAN_WORDS as a comma-separated list, so they never have to be written into
this file."""
import os, re, sys

path = sys.argv[1]
extra = [w for w in os.environ.get("PASTEROLL_SCAN_WORDS", "").split(",") if w]
words = ["Users\\", "/home/"] + extra + sys.argv[2:]
data = open(path, "rb").read()
bad = 0
for w in words:
    b = w.encode()
    u16 = w.encode("utf-16-le")
    n = len(re.findall(re.escape(b), data, re.I)) + len(re.findall(re.escape(u16), data, re.I))
    print(f"{w!r:20} {n}")
    bad += n
print("CLEAN" if bad == 0 else f"FOUND {bad}")
sys.exit(0 if bad == 0 else 1)
