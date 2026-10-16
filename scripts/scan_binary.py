#!/usr/bin/env python3
"""Scan files and ZIP/tar/Debian payloads without echoing personal search terms.

Usage: python scripts/scan_binary.py FILE [WORD ...] [--require-personal-words]
Prefer comma-separated PASTEROLL_SCAN_WORDS over command-line terms. Inno Setup's proprietary
stream is not decoded: scan the setup EXE plus its exact staged inputs with --include-tree.
Nothing is extracted to disk, executed, or sent over the network.
"""
import argparse
import bz2
import gzip
import io
import lzma
import os
from pathlib import Path
import re
import sys
import tarfile
import zipfile
import zlib

MAX_UNPACKED = 512 * 1024 * 1024


def decompress(data, opener, remaining):
    with opener(io.BytesIO(data), "rb") as compressed:
        expanded = compressed.read(remaining + 1)
    if len(expanded) > remaining:
        raise ValueError("Archive expansion limit exceeded.")
    return expanded


def payloads(data, label, budget, depth=0):
    budget[0] += len(data)
    if budget[0] > MAX_UNPACKED or depth > 8:
        raise ValueError("Archive expansion limit exceeded.")
    yield label, data
    if data.startswith(b"\x1f\x8b"):
        yield from payloads(decompress(data, gzip.open, MAX_UNPACKED - budget[0]), label + "/gzip", budget, depth + 1)
    elif data.startswith(b"\xfd7zXZ\x00"):
        yield from payloads(decompress(data, lzma.LZMAFile, MAX_UNPACKED - budget[0]), label + "/xz", budget, depth + 1)
    elif data.startswith(b"BZh"):
        yield from payloads(decompress(data, bz2.BZ2File, MAX_UNPACKED - budget[0]), label + "/bzip2", budget, depth + 1)
    elif data.startswith(b"!<arch>\n"):
        offset = 8
        while offset < len(data):
            header = data[offset:offset + 60]
            if len(header) != 60 or header[58:60] != b"`\n":
                raise ValueError("Invalid Debian/ar archive.")
            size = int(header[48:58].strip())
            if size < 0 or offset + 60 + size > len(data):
                raise ValueError("Truncated Debian/ar archive.")
            name = header[:16].decode("ascii").strip().rstrip("/")
            if name.startswith(("control.tar.", "data.tar.")) and not name.endswith((".gz", ".xz", ".bz2")):
                raise ValueError("Unsupported Debian compression.")
            yield from payloads(data[offset + 60:offset + 60 + size], label + "/" + name, budget, depth + 1)
            offset += 60 + size + size % 2
    elif zipfile.is_zipfile(io.BytesIO(data)):
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for member in archive.infolist():
                if member.is_dir():
                    continue
                if member.file_size > MAX_UNPACKED - budget[0]:
                    raise ValueError("ZIP expansion limit exceeded.")
                yield from payloads(archive.read(member), label + "/" + member.filename, budget, depth + 1)
    elif data[257:262] == b"ustar":
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
            for member in archive:
                if member.isfile():
                    if member.size > MAX_UNPACKED - budget[0]:
                        raise ValueError("Tar expansion limit exceeded.")
                    content = archive.extractfile(member)
                    yield from payloads(content.read(), label + "/" + member.name, budget, depth + 1)


def count_word(data, word):
    return sum(len(re.findall(re.escape(word.encode(encoding)), data, re.I))
               for encoding in ("utf-8", "utf-16-le", "utf-16-be"))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("file", type=Path)
    parser.add_argument("words", nargs="*")
    parser.add_argument("--require-personal-words", action="store_true")
    parser.add_argument("--include-tree", type=Path, action="append", default=[])
    parser.add_argument("--include-file", type=Path, action="append", default=[])
    args = parser.parse_args(argv)
    personal = [w.strip() for w in os.environ.get("PASTEROLL_SCAN_WORDS", "").split(",") if w.strip()]
    personal.extend(w for w in args.words if w)
    if args.require_personal_words and not personal:
        print("NOT SCANNED: PASTEROLL_SCAN_WORDS is required but unavailable.")
        return 2
    words = [("Windows profile path", "Users\\"), ("Linux home path", "/home/")]
    words.extend((f"personal term {i + 1}", word) for i, word in enumerate(personal))
    inputs = [args.file, *args.include_file]
    for directory in args.include_tree:
        if not directory.is_dir():
            print("NOT SCANNED: an expected input tree is missing.")
            return 2
        inputs.extend(p for p in sorted(directory.rglob("*")) if p.is_file())
    bad = checked = 0
    try:
        for path in inputs:
            if path.is_symlink() or not path.is_file():
                raise ValueError("An expected input is missing or is a symbolic link.")
            for label, data in payloads(path.read_bytes(), path.name, [0]):
                checked += 1
                for name, word in words:
                    count = count_word(data, word)
                    if count:
                        # Member filenames and private terms must never leak through diagnostics.
                        print(f"FOUND: payload {checked}: {name}: {count}")
                        bad += count
    except (OSError, ValueError, RuntimeError, EOFError, tarfile.TarError, zipfile.BadZipFile, zlib.error, lzma.LZMAError):
        print("NOT SCANNED: an input could not be read or unpacked safely.")
        return 2
    print(f"{'CLEAN' if bad == 0 else 'FOUND'}: {checked} payloads; personal terms {'included' if personal else 'NOT PROVIDED'}.")
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
