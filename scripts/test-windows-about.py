#!/usr/bin/env python3
"""Verify About in disposable modes and close only the dialog created by this test."""
import ctypes as c
from ctypes import wintypes as w
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "target-dev"
EXE = ROOT / "target/release/pasteroll.exe"
u = c.WinDLL("user32", use_last_error=True)
ENUM = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
u.EnumWindows.argtypes = [ENUM, w.LPARAM]
u.EnumChildWindows.argtypes = [w.HWND, ENUM, w.LPARAM]
u.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
u.GetWindowTextLengthW.argtypes = [w.HWND]
u.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.IsWindowVisible.argtypes = [w.HWND]
u.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]


def text(hwnd):
    n = u.GetWindowTextLengthW(hwnd)
    b = c.create_unicode_buffer(n + 1)
    u.GetWindowTextW(hwnd, b, n + 1)
    return b.value


def main():
    checks = []
    with tempfile.TemporaryDirectory(prefix="windows-about-", dir=BASE) as temporary:
        folder = Path(temporary).resolve()
        assert folder.parent == BASE.resolve() and folder.name.startswith("windows-about-")
        exe = folder / "pasteroll.exe"
        shutil.copyfile(EXE, exe)
        for mode in ("Standalone", "Portable", "Installed (Windows Setup)"):
            portable = folder / "portable.txt"
            setup = folder / "setup.txt"
            if mode == "Portable":
                portable.write_bytes(b"")
                setup.write_bytes(b"")  # Portable must take precedence.
            elif mode.startswith("Installed"):
                portable.unlink()
            output = subprocess.check_output([str(exe), "about", "--text"], text=True,
                                             creationflags=subprocess.CREATE_NO_WINDOW)
            assert f"Installation: {mode}" in output and "Ctrl+Shift+middle-click" in output
            assert not (folder / "data").exists(), "About must not create portable history"
            checks.append("About text reports " + mode + " without creating history")
        process = subprocess.Popen([str(exe), "about"], creationflags=subprocess.CREATE_NO_WINDOW)
        hwnd = None
        try:
            for _ in range(100):
                windows = []
                @ENUM
                def find(handle, _):
                    pid = w.DWORD()
                    u.GetWindowThreadProcessId(handle, c.byref(pid))
                    if pid.value == process.pid and u.IsWindowVisible(handle):
                        windows.append(handle)
                    return True
                u.EnumWindows(find, 0)
                hwnd = next((h for h in windows if text(h) == "About Pasteroll"), None)
                if hwnd is not None:
                    break
                time.sleep(0.05)
            assert hwnd is not None, "native About dialog did not appear"
            contents = []
            @ENUM
            def read_child(handle, _):
                contents.append(text(handle))
                return True
            u.EnumChildWindows(hwnd, read_child, 0)
            assert any("Pasteroll 0.1.0-beta.4" in t and "Installed (Windows Setup)" in t for t in contents)
            checks.append("native About dialog shows version and installation mode")
            u.PostMessageW(hwnd, 0x10, 0, 0)  # Close only this test process's dialog.
            assert process.wait(timeout=5) == 0
            checks.append("About closes normally without launching clipboard capture")
        finally:
            if process.poll() is None:
                if hwnd is not None:
                    u.PostMessageW(hwnd, 0x10, 0, 0)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.terminate()  # Only the standalone test child, never the running app.
                    process.wait(timeout=5)
    (BASE / "windows-about-results.json").write_text(json.dumps({"checks": checks, "passed": True}, indent=2), encoding="utf-8")
    for label in checks:
        print("PASS: " + label)


if __name__ == "__main__":
    main()
