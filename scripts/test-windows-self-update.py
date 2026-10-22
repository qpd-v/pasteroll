#!/usr/bin/env python3
"""End-to-end test of "Download and install update" with a local release server and a throwaway key.

Builds a test-only copy of the program (cargo feature `e2e-test-server`, never shipped) that talks to
a local server instead of GitHub and trusts a key made for this run. Then, on scratch data only:

  1. a Setup install (fixture identity, as in test-windows-update.py) updates itself through a signed
     update: the new Setup is downloaded, verified, run silently, and starts the new program again
  2. a portable copy replaces its own program file, keeping the old one beside it
  3. an update signed by the wrong key, a tampered installer and a tampered manifest are refused, and
     nothing is installed or left behind

No clipboard capture, no input injection, no elevation, nothing from the real installation is touched.
NOT covered: the administrator prompt of an all-users install, and the real GitHub redirect hosts.
"""
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import tomllib
import uuid
import winreg

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "target-dev"
BASE.mkdir(exist_ok=True)
E2E_TARGET = BASE / "e2e-build"
VERSION = tomllib.loads((ROOT / "crates/pasteroll-win/Cargo.toml").read_text())["package"]["version"]
NEW_VERSION = "99.0.0-beta.1"
STAGE_TEMPLATE = ROOT / "target/windows-packaging/setup"
PASSPHRASE = "a throwaway passphrase for tests"
checks = []


def check(label, condition, detail=""):
    if not condition:
        raise AssertionError(f"{label} {detail}")
    checks.append(label)
    print("PASS: " + label, flush=True)


def run(*args, env=None, timeout=240, ok=(0,)):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True, timeout=timeout,
                            creationflags=subprocess.CREATE_NO_WINDOW, env=env)
    if result.returncode not in ok:
        raise AssertionError(f"{Path(str(args[0])).name} failed ({result.returncode}): {result.stdout} {result.stderr}")
    return result


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def tree_hashes(folder):
    return {p.name: sha256(p) for p in Path(folder).iterdir() if p.is_file()}


def pasteroll_processes():
    out = run("pwsh", "-NoProfile", "-Command",
              "@(Get-Process pasteroll -ErrorAction SilentlyContinue | ForEach-Object { '{0}:{1}' -f $_.Id,$_.StartTime.Ticks }) | ConvertTo-Json -Compress").stdout
    return out.strip()


def build():
    env = dict(os.environ, CARGO_TARGET_DIR=str(E2E_TARGET))
    run("cargo", "build", "--release", "-p", "pasteroll-win", "--features", "e2e-test-server",
        "--bin", "pasteroll", "--example", "e2e_stub", env=env, timeout=590)
    return E2E_TARGET / "release/pasteroll.exe", E2E_TARGET / "release/examples/e2e_stub.exe"


def make_key(folder, name):
    env = dict(os.environ, PASTEROLL_KEYS_TEST_PASSPHRASE=PASSPHRASE)
    run("pwsh", "-NoProfile", "-File", ROOT / "scripts/release-keys.ps1", "generate", "-Name", name,
        "-OutDir", folder, "-Test", env=env)
    return Path(folder) / f"{name}.key.enc", (Path(folder) / f"{name}.pub").read_text().strip()


def sign(key_file, manifest, out):
    env = dict(os.environ, PASTEROLL_KEYS_TEST_PASSPHRASE=PASSPHRASE)
    run("pwsh", "-NoProfile", "-File", ROOT / "scripts/release-keys.ps1", "sign", "-KeyFile", key_file,
        "-InFile", manifest, "-OutFile", out, "-Test", env=env)


def manifest_text(setup_name, setup_bytes, exe_name, exe_bytes):
    return ("pasteroll-update-manifest 1\n"
            f"version {NEW_VERSION}\n"
            f"setup {setup_name} {len(setup_bytes)} {hashlib.sha256(setup_bytes).hexdigest()}\n"
            f"exe {exe_name} {len(exe_bytes)} {hashlib.sha256(exe_bytes).hexdigest()}\n")


class Release:
    """A directory served as http://127.0.0.1:PORT/v<NEW_VERSION>/<file>."""

    def __init__(self, folder):
        self.folder = Path(folder)
        (self.folder / f"v{NEW_VERSION}").mkdir(parents=True, exist_ok=True)
        class Quiet(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *args):
                pass
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), lambda *a, **k: Quiet(*a, directory=str(self.folder), **k))
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def publish(self, name, data):
        (self.folder / f"v{NEW_VERSION}" / name).write_bytes(data)

    def stop(self):
        self.server.shutdown()


def update_env(release, public_key):
    return dict(os.environ, PASTEROLL_UPDATE_BASE=release.base, PASTEROLL_UPDATE_TEST_KEY=public_key,
                PASTEROLL_UPDATE_ANNOUNCE=NEW_VERSION)


def installed_version(key_path):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key_path, access=winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as k:
            return winreg.QueryValueEx(k, "DisplayVersion")[0]
    except FileNotFoundError:
        return None


def wait_for(predicate, seconds, what):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return
        time.sleep(0.5)
    raise AssertionError(f"timed out waiting for {what}")


def main():
    candidates = [Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Inno Setup 6/ISCC.exe",
                  Path(os.environ["LOCALAPPDATA"]) / "Programs/Inno Setup 6/ISCC.exe"]
    iscc = next((p for p in candidates if p.is_file()), None)
    assert iscc, "ISCC is missing; no tools were downloaded or installed."
    assert (STAGE_TEMPLATE / "pasteroll.exe").is_file(), "Build the packages first (the Setup staging folder is needed)."
    live_exe = Path(os.environ["LOCALAPPDATA"]) / "Programs/Pasteroll/pasteroll.exe"
    live_before = (pasteroll_processes(), sha256(live_exe) if live_exe.is_file() else None)

    e2e_exe, stub_exe = build()
    check("the test-only program and the stand-in program build", e2e_exe.is_file() and stub_exe.is_file())

    fixture_id = str(uuid.uuid4()).upper()
    reg_key = rf"Software\Microsoft\Windows\CurrentVersion\Uninstall\{{{fixture_id}}}_is1"
    name = "Pasteroll self-update fixture " + fixture_id[:8]
    metadata = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]
    with tempfile.TemporaryDirectory(prefix="windows-self-update-", dir=BASE) as temporary:
        scratch = Path(temporary).resolve()
        recipe = (ROOT / "packaging/windows/pasteroll.iss").read_text(encoding="utf-8-sig")
        original = "AppId={{E04386D3-6242-420F-864E-35305B8A20FC}"
        assert recipe.count(original) == 1
        recipe = recipe.replace(original, "AppId={{" + fixture_id + "}")
        recipe = recipe.replace("AppName=Pasteroll", "AppName=" + name)
        recipe = recipe.replace(r"{autoprograms}\Pasteroll", "{autoprograms}\\" + name)
        recipe = recipe.replace(r"{autodesktop}\Pasteroll", "{autodesktop}\\" + name)
        recipe = recipe.replace(r"SetupIconFile=..\..\crates\pasteroll-win\pasteroll.ico",
                                "SetupIconFile=" + str(ROOT / "crates/pasteroll-win/pasteroll.ico"))
        assert "RelaunchRequested" in recipe, "the Setup script must have the relaunch step"
        iss = scratch / "fixture.iss"
        iss.write_text(recipe, encoding="utf-8-sig")

        def build_setup(version, payload, output):
            stage = scratch / ("stage-" + output)
            shutil.copytree(STAGE_TEMPLATE, stage)
            shutil.copyfile(payload, stage / "pasteroll.exe")
            run(iscc, "/Qp", "/O" + str(scratch), "/F" + output, "/DAppVersion=" + version,
                "/DAppFileVersion=0.1.0.0", "/DAppPublisher=" + metadata["authors"][0],
                "/DAppURL=" + metadata["homepage"], "/DAppLicense=" + metadata["license"],
                "/DStageDir=" + str(stage), iss)
            return scratch / (output + ".exe")

        old_setup = build_setup(VERSION, e2e_exe, "old")
        new_setup = build_setup(NEW_VERSION, stub_exe, "new")
        # the key script only accepts test keys in the system temp folder
        keys = Path(tempfile.mkdtemp(prefix="pasteroll-e2e-keys-"))
        _, public = make_key(keys, "release")
        impostor_file, _ = make_key(keys, "impostor")
        key_file = keys / "release.key.enc"

        setup_name = f"pasteroll-{NEW_VERSION}-windows-x64-setup.exe"
        exe_name = f"pasteroll-{NEW_VERSION}-windows-x64.exe"
        setup_bytes, stub_bytes = new_setup.read_bytes(), stub_exe.read_bytes()
        manifest = scratch / "update-manifest.txt"
        manifest.write_bytes(manifest_text(setup_name, setup_bytes, exe_name, stub_bytes).encode("ascii"))
        good_sig = scratch / "good.sig"
        sign(key_file, manifest, good_sig)
        bad_sig = scratch / "impostor.sig"
        sign(impostor_file, manifest, bad_sig)

        release = Release(scratch / "serve")
        program = scratch / "program"
        try:
            # ---- 1. a Setup install updates itself
            data = scratch / "data"
            data.mkdir()
            db = data / "history.db"
            args = ["/VERYSILENT", "/CURRENTUSER", "/SUPPRESSMSGBOXES", "/NORESTART", "/TASKS=", "/DIR=" + str(program)]
            run(old_setup, *args)
            exe = program / "pasteroll.exe"
            check("the old version is installed by Setup", installed_version(reg_key) == VERSION and exe.read_bytes() == e2e_exe.read_bytes())
            run(exe, "seed", "--db", db)
            run(exe, "config", "init", "--db", db)
            retained = tree_hashes(data)
            release.publish("update-manifest.txt", manifest.read_bytes())
            release.publish("update-manifest.txt.sig", good_sig.read_bytes())
            release.publish(setup_name, setup_bytes)
            release.publish(exe_name, stub_bytes)
            result = run(exe, "update", "--yes", "--db", db, env=update_env(release, public), timeout=300)
            check("the update command reports the installer was started", "installer was started" in result.stdout, result.stdout)
            wait_for(lambda: installed_version(reg_key) == NEW_VERSION and (program / "relaunched.txt").exists(), 120,
                     "Setup to finish and start the new program")
            check("Setup installed the downloaded, verified version", exe.read_bytes() == stub_bytes)
            check("the new program was started again by the updater's Setup", (program / "relaunched.txt").read_text().strip().endswith(" run"))
            check("history, key and settings are untouched by the update", all(tree_hashes(data).get(n) == h for n, h in retained.items()))

            # ---- 2. a portable copy replaces its own program file
            portable = scratch / "portable"
            portable.mkdir()
            shutil.copyfile(e2e_exe, portable / "pasteroll.exe")
            (portable / "portable.txt").write_bytes(b"")
            old_bytes = (portable / "pasteroll.exe").read_bytes()
            result = run(portable / "pasteroll.exe", "update", "--yes", env=update_env(release, public), timeout=120)
            check("a portable copy replaces its own program file", "Replaced" in result.stdout and (portable / "pasteroll.exe").read_bytes() == stub_bytes, result.stdout)
            check("the old program is kept beside it until the next start", (portable / "pasteroll.exe.old").read_bytes() == old_bytes)
            check("portable.txt is left alone", (portable / "portable.txt").is_file())
            check("the downloaded file is not left behind", not [p for p in (portable / "data" / "pasteroll-update").glob("*") if p.name != ".pasteroll-update-folder"] if (portable / "data" / "pasteroll-update").exists() else True)

            # ---- 3. refusals
            def refused(label, sig, setup_payload=None, manifest_bytes=None, expect=None):
                folder = scratch / ("refuse-" + str(len(checks)))
                folder.mkdir()
                shutil.copyfile(e2e_exe, folder / "pasteroll.exe")
                (folder / "portable.txt").write_bytes(b"")
                rel = Release(scratch / ("serve-" + folder.name))
                try:
                    m = manifest_bytes if manifest_bytes is not None else manifest.read_bytes()
                    rel.publish("update-manifest.txt", m)
                    rel.publish("update-manifest.txt.sig", Path(sig).read_bytes())
                    rel.publish(setup_name, setup_bytes)
                    rel.publish(exe_name, setup_payload if setup_payload is not None else stub_bytes)
                    result = run(folder / "pasteroll.exe", "update", "--yes", env=update_env(rel, public), timeout=120, ok=(1,))
                finally:
                    rel.stop()
                check(label, (folder / "pasteroll.exe").read_bytes() == e2e_exe.read_bytes()
                      and not (folder / "pasteroll.exe.old").exists()
                      and (expect is None or expect in result.stderr), result.stderr)
                leftovers = [p for p in (folder / "data" / "pasteroll-update").glob("*") if p.name != ".pasteroll-update-folder"] if (folder / "data" / "pasteroll-update").exists() else []
                check(label + " (nothing left behind)", not leftovers, str(leftovers))

            refused("an update signed by the wrong key is refused", bad_sig, expect="not signed")
            refused("a program file that is not what was signed is refused", good_sig,
                    setup_payload=b"M" * len(stub_bytes), expect="checksum")
            tampered = manifest.read_bytes().replace(b"setup ", b"setup ", 1).replace(b"1", b"2", 1)
            refused("a manifest changed after signing is refused", good_sig, manifest_bytes=tampered, expect="not signed")

            check("the real installation, its processes and files were not touched",
                  (pasteroll_processes(), sha256(live_exe) if live_exe.is_file() else None) == live_before)
        finally:
            release.stop()
            shutil.rmtree(keys, ignore_errors=True)
            unins = program / "unins000.exe"
            if unins.is_file():
                subprocess.run([str(unins), "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
                               timeout=120, creationflags=subprocess.CREATE_NO_WINDOW)
        check("the fixture install is removed again", installed_version(reg_key) is None)
    (BASE / "windows-self-update-results.json").write_text(json.dumps({
        "checks": checks, "passed": True, "fixture_identity": True, "local_server": True,
        "NOT_TESTED": ["all-users install (administrator prompt)", "GitHub's real redirect hosts", "a real release signature"]},
        indent=2), encoding="utf-8")
    print(f"{len(checks)} checks passed")


if __name__ == "__main__":
    main()
