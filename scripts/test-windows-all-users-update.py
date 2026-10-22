#!/usr/bin/env python3
"""End-to-end test of the ADMINISTRATOR path of "Download and install update" (a copy installed for
all users in Program Files). It needs a person at the keyboard: Windows asks for administrator
permission three times and each one must be accepted ("Yes").

Same local release server and throwaway key as test-windows-self-update.py (read that first). On a
fixture identity that is not the real Pasteroll:

  1. install the old fixture for all users (permission prompt 1)
  2. `pasteroll update --yes` from an ordinary program: the elevated helper (prompt 2) downloads and
     checks the signed release in an administrator-only folder, runs Setup from there, the new program
     starts again, and the folder is removed
  3. uninstall the fixture and check nothing of the update is left in Windows' Temp (prompt 3)

Nothing from the real installation is touched. Not covered: GitHub's real redirect hosts.
"""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import uuid
import winreg

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("selfupdate", HERE / "test-windows-self-update.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
check, run, wait_for, ROOT, BASE = base.check, base.run, base.wait_for, base.ROOT, base.BASE
VERSION, NEW_VERSION = base.VERSION, base.NEW_VERSION


def machine_version(key_path):
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key_path, access=winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as k:
            return winreg.QueryValueEx(k, "DisplayVersion")[0]
    except FileNotFoundError:
        return None


def elevated(file, args, timeout=300):
    """Runs `file` with administrator rights (Windows asks the person) and returns its exit code."""
    quoted = ",".join("'" + a.replace("'", "''") + "'" for a in args)
    script = (f"$p = Start-Process -FilePath '{file}' -ArgumentList {quoted} -Verb RunAs -Wait -PassThru; "
              "exit $p.ExitCode")
    result = subprocess.run(["pwsh", "-NoProfile", "-Command", script], capture_output=True, text=True,
                            timeout=timeout, creationflags=subprocess.CREATE_NO_WINDOW)
    return result.returncode, result.stderr


CLEANUP = r"""$u = Join-Path $env:ProgramFiles '@NAME@\unins000.exe'
if (Test-Path $u) { Start-Process $u -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -Wait }
# also any test install an earlier, interrupted run left behind
Get-ChildItem $env:ProgramFiles -Directory -Filter 'Pasteroll all-users fixture *' -ErrorAction SilentlyContinue | ForEach-Object {
  $o = Join-Path $_.FullName 'unins000.exe'
  if (Test-Path $o) { Start-Process $o -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -Wait }
}
Get-ChildItem $env:ProgramFiles -Directory -Filter 'Pasteroll all-users fixture *' -ErrorAction SilentlyContinue | ForEach-Object { Remove-Item $_.FullName -Recurse -Force -ErrorAction SilentlyContinue }
$end = (Get-Date).AddSeconds(60)
do { $left = @(Get-ChildItem (Join-Path $env:SystemRoot 'Temp') -Filter 'pasteroll-update-*' -ErrorAction SilentlyContinue); Start-Sleep 1 } while ($left.Count -gt 0 -and (Get-Date) -lt $end)
if ($left.Count -gt 0) { exit 7 }
exit 0
"""


def remove_fixture(scratch, name):
    """Uninstalls the fixture (one more permission prompt) and waits for the update folder to go."""
    cleanup = scratch / "cleanup.ps1"
    cleanup.write_text(CLEANUP.replace("@NAME@", name.replace("'", "''")), encoding="utf-8")
    return elevated("pwsh", ["-NoProfile", "-File", str(cleanup)])


def main():
    candidates = [Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Inno Setup 6/ISCC.exe",
                  Path(os.environ["LOCALAPPDATA"]) / "Programs/Inno Setup 6/ISCC.exe"]
    iscc = next((p for p in candidates if p.is_file()), None)
    assert iscc, "ISCC is missing; no tools were downloaded or installed."
    assert (base.STAGE_TEMPLATE / "pasteroll.exe").is_file(), "Build the packages first (the Setup staging folder is needed)."
    program_files = Path(os.environ["ProgramFiles"])
    live_exe = Path(os.environ["LOCALAPPDATA"]) / "Programs/Pasteroll/pasteroll.exe"
    live_before = (base.pasteroll_processes(), base.sha256(live_exe) if live_exe.is_file() else None)

    e2e_exe, stub_exe = base.build()
    check("the test-only program and the stand-in program build", e2e_exe.is_file() and stub_exe.is_file())

    fixture_id = str(uuid.uuid4()).upper()
    reg_key = rf"Software\Microsoft\Windows\CurrentVersion\Uninstall\{{{fixture_id}}}_is1"
    name = "Pasteroll all-users fixture " + fixture_id[:8]
    metadata = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]
    with tempfile.TemporaryDirectory(prefix="windows-all-users-update-", dir=BASE) as temporary:
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
        iss = scratch / "fixture.iss"
        iss.write_text(recipe, encoding="utf-8-sig")

        def build_setup(version, payload, output):
            stage = scratch / ("stage-" + output)
            shutil.copytree(base.STAGE_TEMPLATE, stage)
            shutil.copyfile(payload, stage / "pasteroll.exe")
            run(iscc, "/Qp", "/O" + str(scratch), "/F" + output, "/DAppVersion=" + version,
                "/DAppFileVersion=0.1.0.0", "/DAppPublisher=" + metadata["authors"][0],
                "/DAppURL=" + metadata["homepage"], "/DAppLicense=" + metadata["license"],
                "/DStageDir=" + str(stage), iss)
            return scratch / (output + ".exe")

        old_setup = build_setup(VERSION, e2e_exe, "old")
        new_setup = build_setup(NEW_VERSION, stub_exe, "new")
        keys = Path(tempfile.mkdtemp(prefix="pasteroll-e2e-keys-"))
        _, public = base.make_key(keys, "release")
        key_file = keys / "release.key.enc"
        setup_name = f"pasteroll-{NEW_VERSION}-windows-x64-setup.exe"
        exe_name = f"pasteroll-{NEW_VERSION}-windows-x64.exe"
        setup_bytes, stub_bytes = new_setup.read_bytes(), stub_exe.read_bytes()
        manifest = scratch / "update-manifest.txt"
        manifest.write_bytes(base.manifest_text(setup_name, setup_bytes, exe_name, stub_bytes).encode("ascii"))
        # the signing script checks the installer and the program named in the manifest, beside it
        (scratch / setup_name).write_bytes(setup_bytes)
        (scratch / exe_name).write_bytes(stub_bytes)
        note = scratch / "update-latest.txt"
        note.write_bytes(base.latest_text().encode("ascii"))
        signature = scratch / "good.sig"
        note_signature = scratch / "note.sig"
        base.sign(key_file, [manifest, note], [signature, note_signature])

        release = base.Release(scratch / "serve")
        program = program_files / name
        relaunch_note = Path(tempfile.gettempdir()) / f"pasteroll-e2e-relaunched-{name}.txt"
        relaunch_note.unlink(missing_ok=True)
        installed = False
        try:
            print("\n>>> Windows will now ask for administrator permission. Choose Yes. (1 of 3)\n", flush=True)
            code, err = elevated(old_setup, ["/VERYSILENT", "/ALLUSERS", "/SUPPRESSMSGBOXES", "/NORESTART", "/TASKS=",
                                             f'/DIR="{program}"'])
            installed = machine_version(reg_key) is not None
            check("the old version is installed for all users in Program Files", code == 0 and machine_version(reg_key) == VERSION
                  and (program / "pasteroll.exe").read_bytes() == e2e_exe.read_bytes(), f"{code} {err}")

            data = scratch / "data"
            data.mkdir()
            db = data / "history.db"
            exe = program / "pasteroll.exe"
            run(exe, "seed", "--db", db)
            run(exe, "config", "init", "--db", db)
            retained = base.tree_hashes(data)
            release.publish("update-manifest.txt", manifest.read_bytes())
            release.publish("update-manifest.txt.sig", signature.read_bytes())
            release.publish(setup_name, setup_bytes)
            release.publish(exe_name, stub_bytes)
            release.publish_note(note.read_bytes(), note_signature.read_bytes())

            print("\n>>> Windows will ask for administrator permission again. Choose Yes. (2 of 3)\n", flush=True)
            result = run(exe, "update", "--yes", "--db", db, env=base.update_env(release, public), timeout=600)
            check("the update command reports the installer was started", "installer was started" in result.stdout, result.stdout + result.stderr)
            wait_for(lambda: machine_version(reg_key) == NEW_VERSION and relaunch_note.exists(), 180,
                     "Setup to finish and start the new program")
            check("Setup installed the downloaded, verified version", exe.read_bytes() == stub_bytes)
            check("the new program was started again, as the person: no administrator rights",
                  relaunch_note.read_text().strip() == "elevated=0", relaunch_note.read_text())
            check("history, key and settings are untouched by the update", all(base.tree_hashes(data).get(n) == h for n, h in retained.items()))
            check("the plain download folder in the data folder holds no installer",
                  not [p for p in (data / "pasteroll-update").glob("*.exe")] if (data / "pasteroll-update").exists() else True)

            print("\n>>> One more administrator prompt, to remove the test install. Choose Yes. (3 of 3)\n", flush=True)
            code, err = remove_fixture(scratch, name)
            installed = machine_version(reg_key) is not None
            check("the fixture install is removed again", not installed, f"{code} {err}")
            check("the administrator-only update folder was removed from Windows' Temp", code == 0, f"exit {code} {err}")
            check("the real installation, its processes and files were not touched",
                  (base.pasteroll_processes(), base.sha256(live_exe) if live_exe.is_file() else None) == live_before)
        finally:
            release.stop()
            shutil.rmtree(keys, ignore_errors=True)
            relaunch_note.unlink(missing_ok=True)
            if machine_version(reg_key) is not None:
                print("\n>>> The test failed part-way: removing the test install (one more permission prompt).\n", flush=True)
                remove_fixture(scratch, name)
    print(f"{len(base.checks)} checks passed")


if __name__ == "__main__":
    main()
