#!/usr/bin/env python3
"""Test Setup upgrade using synthetic data and a separate fixture AppId/shortcut names.

The actual previous-release (beta.7) and current app payloads and production installer recipe are used.
The fixture identity avoids replacing the user's installed product registration.
No clipboard capture, autostart changes, elevation or live-data access.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import uuid
import winreg
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "target-dev"
BASE.mkdir(exist_ok=True)
VERSION = tomllib.loads((ROOT / "crates/pasteroll-win/Cargo.toml").read_text())["package"]["version"]
CURRENT = ROOT / "target/windows-packaging/setup"
PREVIOUS = "0.1.0-beta.7"
OLD_ZIP = BASE / f"beta7-release/github-download/pasteroll-{PREVIOUS}-windows-x64-portable.zip"
REAL_KEY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{E04386D3-6242-420F-864E-35305B8A20FC}_is1"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
checks = []


def run(*args, timeout=90):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True,
                            timeout=timeout, creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        raise AssertionError(f"{Path(args[0]).name} failed ({result.returncode}): {result.stderr}")
    return result.stdout


def check(label, condition):
    if not condition:
        raise AssertionError(label)
    checks.append(label)
    print("PASS: " + label, flush=True)


def values(path):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path, access=winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
            return [winreg.EnumValue(key, i) for i in range(winreg.QueryInfoKey(key)[1])]
    except FileNotFoundError:
        return None


def live_snapshot():
    locations = [Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/Pasteroll.lnk",
                 Path(os.environ["USERPROFILE"]) / "Desktop/Pasteroll.lnk",
                 Path(os.environ["LOCALAPPDATA"]) / "Programs/Pasteroll/pasteroll.exe"]
    processes = run("pwsh", "-NoProfile", "-Command",
                    "@(Get-Process pasteroll -ErrorAction SilentlyContinue | ForEach-Object { '{0}:{1}' -f $_.Id,$_.StartTime.Ticks }) | ConvertTo-Json -Compress")
    return (values(REAL_KEY), values(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Pasteroll"),
            [v for v in (values(RUN_KEY) or []) if v[0].lower() == "pasteroll"],
            {str(p): hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else None for p in locations}, processes.strip())


def hashes(folder):
    return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in folder.iterdir() if p.is_file()}


def main():
    candidates = [Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Inno Setup 6/ISCC.exe",
                  Path(os.environ["LOCALAPPDATA"]) / "Programs/Inno Setup 6/ISCC.exe"]
    compiler = next((p for p in candidates if p.is_file()), None)
    if compiler is None:
        raise SystemExit("ISCC is missing; no tools were downloaded or installed.")
    assert OLD_ZIP.is_file() and (CURRENT / "pasteroll.exe").is_file(), f"Build {VERSION} and retain the verified {PREVIOUS} ZIP first."
    before = live_snapshot()
    fixture_id = str(uuid.uuid4()).upper()
    key = rf"Software\Microsoft\Windows\CurrentVersion\Uninstall\{{{fixture_id}}}_is1"
    name = "Pasteroll upgrade fixture " + fixture_id[:8]
    metadata = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]
    with tempfile.TemporaryDirectory(prefix="windows-update-", dir=BASE) as temporary:
        scratch = Path(temporary).resolve()
        assert scratch.parent == BASE.resolve() and scratch.name.startswith("windows-update-")
        old_stage = scratch / "old-stage"
        shutil.copytree(CURRENT, old_stage)
        with zipfile.ZipFile(OLD_ZIP) as archive:
            (old_stage / "pasteroll.exe").write_bytes(archive.read("pasteroll.exe"))
        recipe = (ROOT / "packaging/windows/pasteroll.iss").read_text()
        original = "AppId={{E04386D3-6242-420F-864E-35305B8A20FC}"
        assert recipe.count(original) == 1
        recipe = recipe.replace(original, "AppId={{" + fixture_id + "}")
        recipe = recipe.replace("AppName=Pasteroll", "AppName=" + name)
        recipe = recipe.replace(r"{autoprograms}\Pasteroll", "{autoprograms}\\" + name)
        recipe = recipe.replace(r"{autodesktop}\Pasteroll", "{autodesktop}\\" + name)
        # The fixture recipe is outside packaging/windows, so resolve its icon explicitly.
        recipe = recipe.replace(r"SetupIconFile=..\..\crates\pasteroll-win\pasteroll.ico",
                                "SetupIconFile=" + str(ROOT / "crates/pasteroll-win/pasteroll.ico"))
        iss = scratch / "fixture.iss"
        iss.write_text(recipe, encoding="utf-8-sig")
        installers = []
        for version, stage, output in [(PREVIOUS, old_stage, "old"), (VERSION, CURRENT, "new")]:
            run(compiler, "/Qp", "/O" + str(scratch), "/F" + output,
                "/DAppVersion=" + version, "/DAppFileVersion=0.1.0.0",
                "/DAppPublisher=" + metadata["authors"][0], "/DAppURL=" + metadata["homepage"],
                "/DAppLicense=" + metadata["license"], "/DStageDir=" + str(stage), iss)
            installers.append(scratch / (output + ".exe"))
        install = scratch / "program"
        data = scratch / "synthetic-data"
        data.mkdir()
        db = data / "history.db"
        exe = install / "pasteroll.exe"
        args = ["/VERYSILENT", "/CURRENTUSER", "/SUPPRESSMSGBOXES", "/NORESTART",
                "/TASKS=startmenu,desktopicon", "/DIR=" + str(install)]
        def own_registration():
            registered = dict((n, v) for n, v, _ in values(key) or [])
            return str(registered.get("InstallLocation", "")).rstrip("\\") == str(install).rstrip("\\")
        try:
            run(installers[0], *args)
            check(f"{PREVIOUS} fixture Setup installs the exact previous payload", own_registration()
                  and (exe.read_bytes() == (old_stage / "pasteroll.exe").read_bytes())
                  and run(exe, "--version").strip() == f"pasteroll {PREVIOUS}")
            run(exe, "seed", "--db", db)
            run(exe, "config", "init", "--db", db)
            (data / "image-presets.txt").write_text("synthetic presets")
            (data / "unrelated.txt").write_text("keep this unrelated fixture")
            retained = hashes(data)
            run(installers[1], *args)
            check(f"{PREVIOUS} to {VERSION} upgrade retains encrypted history, key, config and presets", hashes(data) == retained)
            check("upgraded program equals the packaged current executable", exe.read_bytes() == (CURRENT / "pasteroll.exe").read_bytes()
                  and run(exe, "--version").strip() == "pasteroll " + VERSION)
            check("About reports Setup installation and the current version", "Installation: Installed (Windows Setup)" in run(exe, "about", "--text"))
            shortcuts = [Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs" / (name + ".lnk"),
                         Path(run("pwsh", "-NoProfile", "-Command", "[Environment]::GetFolderPath('Desktop')").strip()) / (name + ".lnk")]
            check("fixture Start menu and desktop shortcuts are created", all(p.is_file() for p in shortcuts))
            run(installers[1], *args)
            check("same-version reinstall retains synthetic user data", hashes(data) == retained)
            check("live installation, processes, shortcuts and autostart remain unchanged", live_snapshot() == before)
            run(exe, "uninstall", "--data-only", "--remove-data", "--yes", "--db", db)
            check("explicit data-only cleanup removes only the synthetic history and key",
                  not db.exists() and not (data / "history.key").exists() and exe.is_file()
                  and (data / "unrelated.txt").read_text() == "keep this unrelated fixture")
            (data / "history.db").write_bytes(b"synthetic uninstall retention fixture")
            run(install / "unins000.exe", "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
            check("silent uninstall removes fixture program, shortcuts and registration but keeps data",
                  not exe.exists() and values(key) is None and all(not p.exists() for p in shortcuts) and db.is_file())
            check("live installation remains intact after fixture uninstall", live_snapshot() == before)
        finally:
            if own_registration() and (install / "unins000.exe").is_file():
                run(install / "unins000.exe", "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
    (BASE / "windows-update-results.json").write_text(json.dumps({"checks": checks, "passed": True,
        "fixture_identity": True, "production_recipe_and_exact_payloads": True,
        "all_users_uac": "NOT TESTED", "interactive_delete_prompt": "NOT TESTED",
        "enabled_autostart": "NOT TESTED; live preference preserved"}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
