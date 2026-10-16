#!/usr/bin/env python3
"""Exercise built packages in scratch homes and installation roots; never use the live clipboard."""
import hashlib
import io
import os
from pathlib import Path
import stat
import struct
import subprocess
import tarfile
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
SIZES = (16, 24, 32, 48, 64, 128, 256, 512)


class LinuxPackages(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with (ROOT / "crates/pasteroll-linux/Cargo.toml").open("rb") as source:
            cls.version = tomllib.load(source)["package"]["version"]
        cls.tar = ROOT / "dist" / f"pasteroll-{cls.version}-linux-x86_64.tar.gz"
        cls.deb = ROOT / "dist" / f"pasteroll_{cls.version}_amd64.deb"
        if not cls.tar.is_file() or not cls.deb.is_file():
            raise AssertionError("Build the Linux packages before running these tests.")

    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="pasteroll-package-test-")
        self.addCleanup(self.scratch.cleanup)
        self.temp = Path(self.scratch.name)
        self.env = dict(os.environ, HOME=str(self.temp / "home"),
                        XDG_DATA_HOME=str(self.temp / "user-data"),
                        XDG_CONFIG_HOME=str(self.temp / "user-config"))
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "DESTDIR", "PASTEROLL_SCAN_WORDS"):
            self.env.pop(name, None)
        with tarfile.open(self.tar) as archive:
            archive.extractall(self.temp, filter="data")
        self.bundle = self.temp / f"pasteroll-{self.version}-linux-x86_64"

    def command(self, *args, env=None, succeeds=True):
        result = subprocess.run([str(a) for a in args], env=env or self.env,
                                cwd=self.temp, capture_output=True, text=True, timeout=30)
        if succeeds and result.returncode != 0:
            detail = result.stderr.replace(str(ROOT), "/src")
            self.fail(f"{Path(args[0]).name} failed ({result.returncode}): {detail}")
        if not succeeds:
            self.assertNotEqual(result.returncode, 0)
        return result.stdout

    def installer(self, *args, env=None, succeeds=True):
        return self.command("sh", self.bundle / "install.sh", *args, env=env, succeeds=succeeds)

    def make_history_fixture(self, directory):
        directory.mkdir(parents=True, exist_ok=True)
        for name in ("history.db", "history.key", "config.toml", "image-presets.txt", "unrelated.txt"):
            (directory / name).write_bytes(b"synthetic fixture only")
        return {p.name: p.read_bytes() for p in directory.iterdir()}

    def assert_history_kept(self, directory, fixture):
        self.assertEqual({p.name: p.read_bytes() for p in directory.iterdir()}, fixture)

    def test_archive_metadata_static_binary_and_icons(self):
        with tarfile.open(self.tar) as archive:
            for member in archive.getmembers():
                self.assertEqual((member.uid, member.gid, member.uname, member.gname), (0, 0, "", ""))
                self.assertFalse(member.name.startswith("/"))
                self.assertNotIn("..", Path(member.name).parts)
                self.assertTrue(member.isfile() or member.isdir())
        for name in ("pasteroll", "install.sh", "README.txt", "pasteroll.desktop", "install-source.txt",
                     "LICENSE-MIT", "LICENSE-APACHE", "THIRD-PARTY-NOTICES.md"):
            self.assertTrue((self.bundle / name).is_file())
        self.assertFalse((self.bundle / "portable.txt").exists())
        self.assertEqual(stat.S_IMODE((self.bundle / "pasteroll").stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((self.bundle / "install.sh").stat().st_mode), 0o755)
        self.assertNotIn("INTERP", self.command("readelf", "-lW", self.bundle / "pasteroll"))
        self.assertNotIn("(NEEDED)", self.command("readelf", "-dW", self.bundle / "pasteroll"))
        self.assertEqual(self.command(self.bundle / "pasteroll", "--version").strip(), f"pasteroll {self.version}")
        for size in SIZES:
            png = (self.bundle / f"icons/{size}x{size}/apps/pasteroll.png").read_bytes()
            self.assertEqual(struct.unpack_from(">II", png, 16), (size, size))

    def test_per_user_install_upgrade_and_remove_keep_history_and_preferences(self):
        data = Path(self.env["XDG_DATA_HOME"])
        history = data / "pasteroll"
        fixture = self.make_history_fixture(history)
        home = Path(self.env["HOME"])
        self.installer()
        installed = home / ".local/bin/pasteroll"
        self.assertEqual(installed.read_bytes(), (self.bundle / "pasteroll").read_bytes())
        self.assertTrue((data / "applications/pasteroll.desktop").exists())
        self.assertTrue((data / "icons/hicolor/256x256/apps/pasteroll.png").exists())
        autostart = Path(self.env["XDG_CONFIG_HOME"]) / "autostart/pasteroll.desktop"
        self.assertFalse(autostart.exists())
        self.command(installed, "autostart", "on")
        self.installer()
        self.assertIn(str(installed), autostart.read_text())
        self.assert_history_kept(history, fixture)
        self.installer("--uninstall")
        self.installer("--uninstall")
        self.assertFalse(installed.exists())
        self.assertFalse(autostart.exists())
        self.assert_history_kept(history, fixture)

    def test_system_install_upgrade_and_remove_in_a_staged_root(self):
        dest = self.temp / "system-root"
        env = dict(self.env, DESTDIR=str(dest))
        history = Path(self.env["XDG_DATA_HOME"]) / "pasteroll"
        fixture = self.make_history_fixture(history)
        self.installer("--system", env=env)
        self.installer("--system", env=env)
        prefix = dest / "usr/local"
        self.assertEqual((prefix / "bin/pasteroll").read_bytes(), (self.bundle / "pasteroll").read_bytes())
        self.assertIn("Exec=/usr/local/bin/pasteroll run", (prefix / "share/applications/pasteroll.desktop").read_text())
        for size in SIZES:
            self.assertTrue((prefix / f"share/icons/hicolor/{size}x{size}/apps/pasteroll.png").is_file())
        unrelated = prefix / "share/doc/pasteroll/keep.txt"
        unrelated.write_text("keep")
        self.installer("--system", "--uninstall", env=env)
        self.installer("--system", "--uninstall", env=env)
        self.assertFalse((prefix / "bin/pasteroll").exists())
        self.assertEqual(unrelated.read_text(), "keep")
        self.assert_history_kept(history, fixture)

    def test_installer_refuses_unknown_installs_portable_sources_and_bad_options(self):
        dest = self.temp / "unknown-root"
        exe = dest / "usr/local/bin/pasteroll"
        exe.parent.mkdir(parents=True)
        exe.write_bytes(b"not owned by this installer")
        self.installer("--system", "--uninstall", env=dict(self.env, DESTDIR=str(dest)), succeeds=False)
        self.installer("--system", env=dict(self.env, DESTDIR=str(dest)), succeeds=False)
        self.assertEqual(exe.read_bytes(), b"not owned by this installer")
        self.installer("--unexpected", succeeds=False)
        self.installer("--system", env=dict(self.env, DESTDIR="relative"), succeeds=False)
        if os.geteuid() != 0:
            self.installer("--system", succeeds=False)
        (self.bundle / "portable.txt").touch()
        self.installer(succeeds=False)
        self.installer("--system", env=dict(self.env, DESTDIR=str(dest)), succeeds=False)

    def test_interrupted_system_install_can_be_retried(self):
        dest = self.temp / "interrupted-root"
        env = dict(self.env, DESTDIR=str(dest))
        icon = self.bundle / "icons/256x256/apps/pasteroll.png"
        original = icon.read_bytes()
        icon.unlink()
        self.installer("--system", env=env, succeeds=False)
        self.assertTrue((dest / "usr/local/share/pasteroll/install-source.txt").exists())
        icon.write_bytes(original)
        self.installer("--system", env=env)
        self.assertEqual((dest / "usr/local/share/icons/hicolor/256x256/apps/pasteroll.png").read_bytes(), original)
        self.installer("--system", "--uninstall", env=env)
        self.assertFalse((dest / "usr/local/bin/pasteroll").exists())

    def test_portable_mode_has_no_profile_side_effects(self):
        (self.bundle / "portable.txt").touch()
        exe = self.bundle / "pasteroll"
        output = self.command(exe, "config", "path")
        self.assertEqual(Path(output.strip()), self.bundle / "data/config.toml")
        for args in (("install",), ("uninstall",), ("autostart", "on")):
            self.command(exe, *args, succeeds=False)
        self.command(exe, "stop")
        self.assertFalse((self.bundle / "data").exists())
        self.assertFalse(Path(self.env["HOME"]).exists())

    def test_debian_metadata_and_payload(self):
        self.assertEqual(self.command("dpkg-deb", "--field", self.deb, "Package").strip(), "pasteroll")
        deb_version = self.command("dpkg-deb", "--field", self.deb, "Version").strip()
        self.assertEqual(deb_version, self.version.replace("-", "~", 1))
        self.command("dpkg", "--compare-versions", deb_version, "lt", "0.1.0")
        self.assertEqual(self.command("dpkg-deb", "--field", self.deb, "Architecture").strip(), "amd64")
        self.assertEqual(self.command("dpkg-deb", "--field", self.deb, "Depends").strip(), "")
        self.assertEqual(self.command("dpkg-deb", "--field", self.deb, "Recommends").strip(), "tesseract-ocr, zenity")
        control_bytes = subprocess.run(["dpkg-deb", "--ctrl-tarfile", str(self.deb)], capture_output=True, check=True).stdout
        with tarfile.open(fileobj=io.BytesIO(control_bytes)) as archive:
            self.assertEqual({m.name.lstrip("./") for m in archive.getmembers() if m.isfile()}, {"control"})
        root = self.temp / "deb-extracted"
        self.command("dpkg-deb", "--extract", self.deb, root)
        self.assertEqual(hashlib.sha256((root / "usr/bin/pasteroll").read_bytes()).digest(),
                         hashlib.sha256((self.bundle / "pasteroll").read_bytes()).digest())
        self.assertIn("Exec=/usr/bin/pasteroll run", (root / "usr/share/applications/pasteroll.desktop").read_text())
        self.assertFalse((root / "usr/bin/portable.txt").exists())
        self.assertTrue((root / "usr/share/doc/pasteroll/LICENSE-MIT").exists())

    def test_real_dpkg_install_upgrade_remove_and_purge_in_a_private_root(self):
        root = self.temp / "dpkg-root"
        (root / "var/lib/dpkg").mkdir(parents=True)
        (root / "var/lib/dpkg/status").touch()
        (root / "var/log").mkdir(parents=True)
        history = root / "home/person/.local/share/pasteroll"
        fixture = self.make_history_fixture(history)
        command = ("dpkg", f"--root={root}", f"--log={root}/var/log/dpkg.log", "--force-not-root")
        self.command(*command, "--install", self.deb)
        self.command(*command, "--install", self.deb)
        self.assertTrue((root / "usr/bin/pasteroll").is_file())
        self.command(*command, "--remove", "pasteroll")
        self.assertFalse((root / "usr/bin/pasteroll").exists())
        self.assert_history_kept(history, fixture)
        self.command(*command, "--install", self.deb)
        self.command(*command, "--purge", "pasteroll")
        self.assertFalse((root / "usr/bin/pasteroll").exists())
        self.assert_history_kept(history, fixture)


if __name__ == "__main__":
    unittest.main(verbosity=2)
