#!/usr/bin/env python3
"""Synthetic tests for privacy-safe artifact scans and strict release manifests."""
import gzip
import io
import lzma
import os
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import scan_binary
from release_artifacts import artifact_names, verify_checksums, write_checksums

ROOT = Path(__file__).resolve().parent.parent
TOKEN = "synthetic-private-fixture-token"


def tar_bytes(name, content):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        info = tarfile.TarInfo(name)
        info.size = len(content)
        archive.addfile(info, io.BytesIO(content))
    return stream.getvalue()


def ar_bytes(members):
    result = bytearray(b"!<arch>\n")
    for name, content in members:
        header = f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(content):<10}`\n".encode("ascii")
        assert len(header) == 60
        result.extend(header)
        result.extend(content)
        if len(content) % 2:
            result.extend(b"\n")
    return bytes(result)


class ReleaseTools(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="pasteroll-release-tools-")
        self.addCleanup(temp.cleanup)
        self.temp = Path(temp.name)

    def scan(self, content, *, personal=True, includes=()):
        artifact = self.temp / "artifact.bin"
        artifact.write_bytes(content)
        env = dict(os.environ)
        env.pop("PASTEROLL_SCAN_WORDS", None)
        if personal:
            env["PASTEROLL_SCAN_WORDS"] = TOKEN
        command = [sys.executable, str(ROOT / "scripts/scan_binary.py"), str(artifact), "--require-personal-words"]
        for path in includes:
            command.extend(("--include-tree", str(path)))
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
        self.assertNotIn(TOKEN, result.stdout + result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        return result

    def test_missing_personal_terms_cannot_pass_the_gate(self):
        result = self.scan(b"clean", personal=False)
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("CLEAN", result.stdout)

    def test_detects_private_words_in_utf8_and_both_utf16_orders_without_echoing(self):
        for encoding in ("utf-8", "utf-16-le", "utf-16-be"):
            with self.subTest(encoding=encoding):
                self.assertEqual(self.scan(TOKEN.upper().encode(encoding)).returncode, 1)
        self.assertEqual(self.scan(b"clean file").returncode, 0)

    def test_compressed_zip_and_tar_payloads_are_inspected(self):
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.writestr(TOKEN + ".bin", TOKEN.encode())
        self.assertEqual(self.scan(output.getvalue()).returncode, 1)
        self.assertEqual(self.scan(gzip.compress(tar_bytes("payload", TOKEN.encode()))).returncode, 1)
        self.assertEqual(self.scan(gzip.compress(tar_bytes("payload", b"public fixture"))).returncode, 0)

    def test_debian_control_and_compressed_data_are_inspected(self):
        for name in ("control.tar.xz", "data.tar.xz"):
            content = ar_bytes((("debian-binary", b"2.0\n"),
                                (name, lzma.compress(tar_bytes("payload", TOKEN.encode())))))
            self.assertEqual(self.scan(content).returncode, 1)
        unsupported = ar_bytes((("data.tar.zst", b"unsupported"),))
        self.assertEqual(self.scan(unsupported).returncode, 2)
        clean = ar_bytes((("data.tar.xz", lzma.compress(tar_bytes("payload", b"public fixture"))),))
        self.assertEqual(self.scan(clean).returncode, 0)

    def test_setup_input_tree_is_scanned_and_missing_inputs_fail(self):
        inputs = self.temp / "setup"
        inputs.mkdir()
        (inputs / "README.txt").write_text(TOKEN)
        self.assertEqual(self.scan(b"MZ synthetic setup", includes=(inputs,)).returncode, 1)
        self.assertEqual(self.scan(b"MZ synthetic setup", includes=(self.temp / "absent",)).returncode, 2)

    def test_expansion_limit_and_corrupt_archives_fail(self):
        with patch.object(scan_binary, "MAX_UNPACKED", 64):
            with self.assertRaises(ValueError):
                list(scan_binary.payloads(gzip.compress(b"x" * 128), "fixture", [0]))
        self.assertEqual(self.scan(b"!<arch>\nbroken").returncode, 2)

    def test_manifest_contains_exactly_four_current_files_and_ignores_stale_outputs(self):
        names = artifact_names("0.1.0-beta.2")
        for index, name in enumerate(names):
            (self.temp / name).write_bytes(f"fixture {index}".encode())
        (self.temp / "pasteroll-windows-x64.zip").write_bytes(b"stale archive")
        content = write_checksums(self.temp, names)
        self.assertEqual(len(content.splitlines()), 4)
        self.assertNotIn("  pasteroll-windows-x64.zip", content)
        verify_checksums(self.temp, names)
        (self.temp / names[0]).write_bytes(b"tampered")
        with self.assertRaises(ValueError):
            verify_checksums(self.temp, names)

    def test_missing_artifact_cannot_replace_a_valid_manifest(self):
        names = artifact_names("0.1.0-beta.2")
        (self.temp / "SHA256SUMS").write_text("previous manifest\n")
        with self.assertRaises(ValueError):
            write_checksums(self.temp, names)
        self.assertEqual((self.temp / "SHA256SUMS").read_text(), "previous manifest\n")

    def test_gnome_manifest_requires_both_optional_packages(self):
        names = artifact_names("0.1.0-beta.3", with_gnome=True)
        self.assertEqual(len(names), 6)
        for name in names[:-1]:
            (self.temp / name).write_bytes(b"fixture")
        with self.assertRaises(ValueError):
            write_checksums(self.temp, names)
        (self.temp / names[-1]).write_bytes(b"fixture")
        write_checksums(self.temp, names)
        verify_checksums(self.temp, names)
        with self.assertRaises(ValueError):
            verify_checksums(self.temp, artifact_names("0.1.0-beta.3"))

    def test_windows_only_manifest_does_not_require_or_include_linux(self):
        names = artifact_names("0.1.0-beta.4", windows_only=True)
        self.assertEqual(len(names), 3)
        self.assertIn("pasteroll-0.1.0-beta.4-windows-x64.exe", names)
        for name in names:
            (self.temp / name).write_bytes(b"fixture")
        stale = "pasteroll_0.1.0-beta.3_amd64.deb"
        (self.temp / stale).write_bytes(b"old Linux release")
        content = write_checksums(self.temp, names)
        self.assertNotIn(stale, content)
        verify_checksums(self.temp, names)
        with self.assertRaises(ValueError):
            verify_checksums(self.temp, artifact_names("0.1.0-beta.4"))
        with self.assertRaises(ValueError):
            artifact_names("0.1.0-beta.4", windows_only=True, with_gnome=True)

    def test_manifest_rejects_extra_or_duplicate_entries(self):
        names = artifact_names("0.1.0-beta.2")
        for name in names:
            (self.temp / name).write_bytes(b"fixture")
        original = write_checksums(self.temp, names)
        (self.temp / "SHA256SUMS").write_text(original + original.splitlines()[0] + "\n")
        with self.assertRaises(ValueError):
            verify_checksums(self.temp, names)


if __name__ == "__main__":
    unittest.main(verbosity=2)
