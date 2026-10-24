#!/usr/bin/env python3
"""Smoke-test the actual Linux release on a private Xvfb display with synthetic clips only.

Requires existing Xvfb, xdotool, xterm, xclip, xwininfo, xwd and ImageMagick convert.
No connection is made to the user's display, clipboard, D-Bus session or data folder.
"""
import json
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
EXE = Path(os.environ.get("PASTEROLL_EXE", ROOT / "target/x86_64-unknown-linux-musl/release/pasteroll"))
OUT = ROOT / "target-dev/linux-x11-smoke"


def main():
    for command in ("Xvfb", "xdotool", "xterm", "xclip", "xwininfo", "xwd", "convert"):
        if not shutil.which(command):
            raise SystemExit(f"Missing local test tool: {command}")
    OUT.mkdir(parents=True, exist_ok=True)
    results = []
    processes = []
    with tempfile.TemporaryDirectory(prefix="pasteroll-x11-") as scratch:
        temp = Path(scratch)
        env = dict(os.environ, HOME=str(temp / "home"), XDG_DATA_HOME=str(temp / "data"),
                   XDG_CONFIG_HOME=str(temp / "config"), XDG_RUNTIME_DIR=str(temp / "runtime"),
                   PASTEROLL_NO_NOTICE="1", PASTEROLL_NO_SNI="1", LC_ALL="C.UTF-8")
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS"):
            env.pop(key, None)
        (temp / "runtime").mkdir(mode=0o700)
        (temp / "home/Desktop").mkdir(parents=True)

        def call(*args, binary=False, input_data=None):
            run = subprocess.run([str(a) for a in args], env=env, cwd=temp, input=input_data,
                                 capture_output=True, timeout=10)
            if run.returncode:
                raise AssertionError(f"{Path(args[0]).name} returned {run.returncode}")
            return run.stdout if binary else run.stdout.decode(errors="replace")

        def wait_for(check, message, seconds=5):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                value = check()
                if value:
                    return value
                time.sleep(0.05)
            raise AssertionError(message)

        def passed(name):
            results.append(name)
            print("PASS: " + name, flush=True)

        def screenshot(name):
            data = call("xwd", "-root", "-silent", binary=True)
            call("convert", "xwd:-", "png:" + str(OUT / name), input_data=data)

        def visible_windows():
            tree = call("xwininfo", "-root", "-children")
            windows = []
            for win in re.findall(r"^\s+(0x[0-9a-f]+)\s", tree, re.M):
                info = call("xwininfo", "-id", win)
                if "Map State: IsViewable" in info:
                    width = re.search(r"Width:\s+(\d+)", info)
                    height = re.search(r"Height:\s+(\d+)", info)
                    windows.append((win, int(width.group(1)), int(height.group(1))))
            return windows

        def overlay():
            return any(w == 520 and h == 440 for _, w, h in visible_windows())

        def viewer():
            return any(w >= 850 and h >= 500 for _, w, h in visible_windows())

        def activate():
            call("xdotool", "windowfocus", target)
            call("xdotool", "mousemove", "150", "130", "keydown", "Control_L", "keydown", "Shift_L",
                 "click", "2", "keyup", "Shift_L", "keyup", "Control_L")
            wait_for(overlay, "Carousel did not open")

        def key(*keys):
            call("xdotool", "key", "--clearmodifiers", *keys)
            time.sleep(0.15)

        def find_target():
            found = subprocess.run(["xdotool", "search", "--name", "Pasteroll isolated synthetic target"],
                                   env=env, capture_output=True, text=True, timeout=5)
            return found.stdout.splitlines() if found.returncode == 0 else []

        rfd, wfd = os.pipe()
        try:
            # WSL's /tmp/.X11-unix has no sticky bit, so Xvfb refuses its file socket there: use the
            # abstract socket instead (clients find it through the same DISPLAY).
            sticky = not os.path.isdir("/tmp/.X11-unix") or os.stat("/tmp/.X11-unix").st_mode & 0o1000
            xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(wfd), "-screen", "0", "1280x800x24", "-nolisten", "tcp"]
                                    + ([] if sticky else ["-nolisten", "unix"]),
                                    pass_fds=(wfd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(xvfb)
            os.close(wfd)
            if not select.select([rfd], [], [], 10)[0]:
                raise AssertionError("Private X server did not start")
            number = os.read(rfd, 32).decode().strip()
            if not number.isdigit():
                raise AssertionError("Private X server did not supply a display")
            env["DISPLAY"] = ":" + number
        except BaseException:
            if xvfb.poll() is None:
                xvfb.terminate()
                xvfb.wait(timeout=5)
            raise
        finally:
            os.close(rfd)

        app = None
        try:
            # Seed creates only this scratch profile's synthetic examples.
            call(EXE, "seed")
            sink = temp / "pasted.txt"
            terminal = subprocess.Popen(["xterm", "-T", "Pasteroll isolated synthetic target", "-geometry", "90x22+20+20",
                                         "-e", "sh", "-c", 'stty -echo; exec cat > "$1"', "fixture", str(sink)],
                                        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            processes.append(terminal)
            target = wait_for(find_target, "Synthetic target did not start")[0]
            app = subprocess.Popen([str(EXE), "run", "--max-seconds", "100"], env=env,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            processes.append(app)
            wait_for(lambda: (temp / "data/pasteroll/app.stop").exists(), "App control pipe did not start")
            time.sleep(0.5)
            if app.poll() is not None:
                raise AssertionError("Release app exited during startup")
            passed("static release app starts on isolated X11")

            activate()
            screenshot("carousel.png")
            passed("global activation opens the real carousel")
            key("ctrl+e")
            wait_for(viewer, "Expanded image viewer did not open")
            time.sleep(0.4)  # let the opening morph finish before testing another transition
            key("F11")
            wait_for(lambda: any(w == 1280 and h == 800 for _, w, h in visible_windows()), "Fullscreen did not open")
            key("Escape", "Escape")
            wait_for(lambda: overlay() and not viewer(), "Viewer did not return to carousel")
            time.sleep(0.4)
            passed("expanded viewer, fullscreen and return")

            # The newest seeded clip is a picture; Ctrl+click the otherwise empty desktop.
            call("xdotool", "mousemove", "1220", "750", "keydown", "Control_L", "click", "1", "keyup", "Control_L")
            wait_for(viewer, "Advanced Image Paste editor did not open")
            time.sleep(0.4)
            screenshot("image-editor.png")
            key("Escape")
            wait_for(lambda: overlay() and not viewer(), "Editor cancel did not return to carousel")
            time.sleep(0.4)
            passed("Advanced Image Paste opens and cancels")
            call("xdotool", "mousemove", "1220", "750", "click", "1")
            wait_for(lambda: list((temp / "home/Desktop").glob("*.png")), "Desktop image was not saved")
            passed("plain image desktop paste saves PNG")

            token = "PASTEROLL_SYNTHETIC_PASTE_42"
            clipboard = subprocess.Popen(["xclip", "-selection", "clipboard", "-in", "-quiet"], env=env,
                                         stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(clipboard)
            clipboard.stdin.write(token.encode())
            clipboard.stdin.close()
            time.sleep(0.6)
            activate()
            key("ctrl+p")
            status = call(EXE, "status")
            if not re.search(r"stored:\s+\d+ clips, 1 pinned", status):
                raise AssertionError("Pin command did not update synthetic history")
            key("ctrl+p")
            key("Delete", "ctrl+z")
            passed("captured synthetic text can be pinned, recycled and restored")
            key("Return")
            wait_for(lambda: not overlay(), "Confirm did not close carousel")
            key("Return")
            wait_for(lambda: sink.exists() and token in sink.read_text(), "Paste did not reach the synthetic terminal")
            passed("clipboard capture and actual paste keystroke reach target")

            activate()
            call("xdotool", "type", "--clearmodifiers", "--delay", "25", "SYNTHETIC_PASTE")
            key("ctrl+e")
            wait_for(viewer, "Expanded text viewer did not open from search")
            screenshot("text-viewer.png")
            key("Escape", "Escape")
            passed("search and expanded text viewer render")

            call(EXE, "stop")
            if app.wait(timeout=8) != 0:
                raise AssertionError("App did not exit cleanly after stop")
            call(EXE, "stop")
            passed("graceful stop and no-instance stop succeed")
        except BaseException:
            screenshot("failure.png")
            (OUT / "failure-windows.json").write_text(json.dumps(visible_windows()) + "\n")
            raise
        finally:
            if app is not None and app.poll() is None:
                subprocess.run([str(EXE), "stop"], env=env, capture_output=True, timeout=8)
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            (OUT / "results.json").write_text(json.dumps({"passed": results, "private_display": True,
                "synthetic_data_only": True, "clean_desktop_vm": False}, indent=2) + "\n")
    print(f"PASS: {len(results)} isolated X11 runtime checks.")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        message = str(error) if isinstance(error, AssertionError) else type(error).__name__
        print("FAIL: isolated X11 test: " + message, file=sys.stderr)
        sys.exit(1)
