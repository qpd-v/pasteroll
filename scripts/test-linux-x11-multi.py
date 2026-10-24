#!/usr/bin/env python3
"""Isolated X11 test of the picture-folder setting, choosing several clips, and the No Bloat look.

Runs a private Xvfb display with synthetic clips only (the same way as test-linux-x11.py), against
the program named by PASTEROLL_EXE (default: the static release build). No connection is made to
the user's display, clipboard, D-Bus session or data folder.

Needs Xvfb, xdotool, xterm, xclip, xwininfo, xwd and ImageMagick convert.
PASTEROLL_OUT sets where screenshots go. PASTEROLL_EXPLORE=1 only opens the carousel, saves a
screenshot and the window list, and stops (used to find where the rows are).
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
OUT = Path(os.environ.get("PASTEROLL_OUT", ROOT / "target-dev/linux-x11-multi"))
EXPLORE = bool(os.environ.get("PASTEROLL_EXPLORE"))

# Where the rows of the Complete carousel are, relative to the window's top-left corner (DIP at
# scale 1), with the seeded history: 0 is the picture (shown big), then the file, then the text
# clips newest first. Found with PASTEROLL_EXPLORE=1 and a screenshot.
ROW_X = 150
ROW_Y = {"file": 265, "meeting": 293, "capture": 321, "url": 349, "code": 377}
MEETING = "Meeting notes: ship the spike, measure paste reliability, decide."
CAPTURE = "Capture a thought before it slips away."
URL = "https://example.com/docs/getting-started"


def main():
    for command in ("Xvfb", "xdotool", "xterm", "xclip", "xwininfo", "xwd", "convert"):
        if not shutil.which(command):
            raise SystemExit(f"Missing local test tool: {command}")
    OUT.mkdir(parents=True, exist_ok=True)
    results = []
    processes = []
    with tempfile.TemporaryDirectory(prefix="pasteroll-x11-multi-") as scratch:
        temp = Path(scratch)
        env = dict(os.environ, HOME=str(temp / "home"), XDG_DATA_HOME=str(temp / "data"),
                   XDG_CONFIG_HOME=str(temp / "config"), XDG_RUNTIME_DIR=str(temp / "runtime"),
                   PASTEROLL_NO_NOTICE="1", PASTEROLL_NO_SNI="1", LC_ALL="C.UTF-8")
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS"):
            env.pop(key, None)
        (temp / "runtime").mkdir(mode=0o700)
        (temp / "home/Desktop").mkdir(parents=True)
        pictures = temp / "home/Pictures/pasteroll shots"

        def call(*args, binary=False, input_data=None):
            run = subprocess.run([str(a) for a in args], env=env, cwd=temp, input=input_data,
                                 capture_output=True, timeout=10)
            if run.returncode:
                raise AssertionError(f"{Path(str(args[0])).name} returned {run.returncode}: "
                                     + run.stderr.decode(errors="replace")[:300])
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
                    x = re.search(r"Absolute upper-left X:\s+(-?\d+)", info)
                    y = re.search(r"Absolute upper-left Y:\s+(-?\d+)", info)
                    windows.append((win, int(width.group(1)), int(height.group(1)),
                                    int(x.group(1)), int(y.group(1))))
            return windows

        def carousel(width, height):
            for win, w, h, x, y in visible_windows():
                if w == width and h == height:
                    return x, y
            return None

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
            call(EXE, "seed")
            sink = temp / "pasted.txt"
            terminal = subprocess.Popen(["xterm", "-T", "Pasteroll isolated synthetic target", "-geometry", "90x22+20+20",
                                         "-e", "sh", "-c", 'stty -echo; exec cat > "$1"', "fixture", str(sink)],
                                        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            processes.append(terminal)
            target = wait_for(find_target, "Synthetic target did not start")[0]

            def start_app():
                nonlocal app
                app = subprocess.Popen([str(EXE), "run", "--max-seconds", "200"], env=env,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                processes.append(app)
                wait_for(lambda: (temp / "data/pasteroll/app.stop").exists(), "App control pipe did not start")
                time.sleep(0.5)
                if app.poll() is not None:
                    raise AssertionError("App exited during startup")

            def stop_app():
                call(EXE, "stop")
                if app.wait(timeout=8) != 0:
                    raise AssertionError("App did not exit cleanly after stop")
                (temp / "data/pasteroll/app.stop").unlink(missing_ok=True)

            def activate():
                call("xdotool", "windowfocus", target)
                call("xdotool", "mousemove", "150", "130", "keydown", "Control_L", "keydown", "Shift_L",
                     "click", "2", "keyup", "Shift_L", "keyup", "Control_L")

            def closed():
                return carousel(520, 440) is None

            def row(name, ctrl=False, shift=False, button="1"):
                """Clicks a row of the open carousel, optionally holding Ctrl and/or Shift."""
                x, y = carousel(520, 440)
                held = (["Control_L"] if ctrl else []) + (["Shift_L"] if shift else [])
                args = ["mousemove", str(x + ROW_X), str(y + ROW_Y[name])]
                args += [a for k in held for a in ("keydown", k)] + ["click", button]
                args += [a for k in reversed(held) for a in ("keyup", k)]
                call("xdotool", *args)
                time.sleep(0.25)

            def click_target():
                # an empty part of the terminal, away from the carousel
                call("xdotool", "mousemove", "300", "60", "click", "1")

            def clipboard_text():
                run = subprocess.run(["xclip", "-selection", "clipboard", "-o"], env=env,
                                     capture_output=True, timeout=5)
                return run.stdout.decode(errors="replace") if run.returncode == 0 else None

            mark = [0]  # the terminal's `cat` keeps its file offset, so we note where each step starts

            def mark_sink():
                mark[0] = sink.stat().st_size if sink.exists() else 0

            def sink_new():
                return sink.read_bytes()[mark[0]:].decode(errors="replace") if sink.exists() else ""

            def sink_lines():
                return sink_new().replace("\r\n", "\n").replace("\r", "\n").splitlines()

            # --- the settings file: the picture folder, "no multi-line warning", the mode
            call(EXE, "config", "set", "image_folder", str(pictures))
            call(EXE, "config", "set", "warn_multiline_in_terminals", "false")
            shown = call(EXE, "config", "show")
            if str(pictures) not in shown:
                raise AssertionError("config show does not mention the picture folder")
            passed("config set stores the picture folder and config show names it")
            start_app()
            passed("the program starts with a picture folder in config.toml")

            if EXPLORE:
                activate()
                wait_for(lambda: carousel(520, 440), "Carousel did not open")
                row("meeting", ctrl=True)
                row("capture", ctrl=True)
                screenshot("explore-picked.png")
                x, y = carousel(520, 440)
                call("xdotool", "mousemove", str(x + ROW_X), str(y + ROW_Y["url"]), "click", "3")
                time.sleep(0.4)
                screenshot("explore-menu.png")
                (OUT / "explore-windows.json").write_text(json.dumps(visible_windows()) + "\n")
                print("explore: wrote screenshots in", OUT)
                stop_app()
                return

            # --- two clips chosen with Ctrl+click, pasted one per line by clicking the target
            activate()
            wait_for(lambda: carousel(520, 440), "Carousel did not open")
            row("meeting", ctrl=True)
            row("capture", ctrl=True)
            screenshot("picked.png")
            if sink_new():
                raise AssertionError("choosing clips typed something into the target")
            click_target()
            wait_for(closed, "Clicking the target did not close the carousel")
            time.sleep(0.5)
            key("Return")  # the terminal passes a line on once it is ended
            wait_for(lambda: len(sink_lines()) >= 2, "The chosen clips did not arrive")
            if sink_lines() != [MEETING, CAPTURE]:
                raise AssertionError(f"wrong text in the target: {sink_lines()!r}")
            passed("Ctrl+click chooses clips and a click on the target pastes them one per line")

            # --- a range with Ctrl+Shift+click, in the order of the list
            mark_sink()
            activate()
            wait_for(lambda: carousel(520, 440), "Carousel did not open")
            row("meeting", ctrl=True)
            row("url", ctrl=True, shift=True)
            click_target()
            wait_for(closed, "Clicking the target did not close the carousel")
            time.sleep(0.5)
            key("Return")  # the terminal passes a line on once it is ended
            wait_for(lambda: len(sink_lines()) >= 3, "The range did not arrive")
            if sink_lines() != [MEETING, CAPTURE, URL]:
                raise AssertionError(f"wrong text for the range: {sink_lines()!r}")
            passed("Ctrl+Shift+click chooses a range")

            # --- Ctrl+C copies without pasting
            mark_sink()
            activate()
            wait_for(lambda: carousel(520, 440), "Carousel did not open")
            row("capture", ctrl=True)
            row("url", ctrl=True)
            key("ctrl+c")
            wait_for(closed, "Ctrl+C did not close the carousel")
            wait_for(lambda: clipboard_text() == CAPTURE + "\n" + URL, "The clipboard does not hold both clips")
            time.sleep(0.3)
            if sink_new():
                raise AssertionError("Ctrl+C pasted into the target")
            passed("Ctrl+C copies the chosen clips without pasting")

            # --- the right-click menu
            activate()
            wait_for(lambda: carousel(520, 440), "Carousel did not open")
            x, y = carousel(520, 440)
            call("xdotool", "mousemove", str(x + ROW_X), str(y + ROW_Y["url"]), "click", "3")
            time.sleep(0.4)
            screenshot("menu.png")
            call("xdotool", "mousemove", str(x + ROW_X + 84), str(y + ROW_Y["url"] + 18), "click", "1")
            wait_for(closed, "The Copy menu item did not close the carousel")
            wait_for(lambda: clipboard_text() == URL, "Copy did not reach the clipboard")
            passed("right-click then Copy copies the clip")

            # --- the picture folder: Paste Path from the editor saves into the Settings folder
            mark_sink()
            activate()
            wait_for(lambda: carousel(520, 440), "Carousel did not open")
            call("xdotool", "mousemove", "300", "60", "keydown", "Control_L", "click", "1", "keyup", "Control_L")
            wait_for(lambda: any(w >= 600 and h >= 400 and (w, h) != (520, 440) for _, w, h, _, _ in visible_windows()),
                     "The image editor did not open")
            time.sleep(0.5)
            screenshot("editor.png")
            key("Return")
            wait_for(lambda: list(pictures.glob("*.png")), "No picture was saved in the Settings folder")
            saved = sorted(pictures.glob("*.png"))[0]
            wait_for(lambda: not any(w >= 600 for _, w, h, _, _ in visible_windows()), "The editor did not close")
            time.sleep(0.5)
            key("Return")  # the terminal passes a line on once it is ended
            wait_for(lambda: str(saved) in sink_new(), "The path was not pasted into the target")
            if list((temp / "home/Desktop").glob("*.png")):
                raise AssertionError("a picture was saved on the Desktop as well")
            passed("Paste Path saves into the Settings folder and pastes its path")

            # --- No Bloat
            stop_app()
            call(EXE, "config", "set", "mode", "no-bloat")
            start_app()
            activate()
            wait_for(lambda: carousel(480, 300), "No Bloat window did not open")
            screenshot("no-bloat.png")
            key("Escape")
            wait_for(lambda: carousel(480, 300) is None, "Escape did not close the No Bloat window")
            passed("No Bloat opens the small window")
            stop_app()

            # --- the Settings window: a picture folder typed in its box is saved to config.toml
            elsewhere = temp / "home/Other pictures"
            env["PASTEROLL_OPEN_SETTINGS"] = "1"
            start_app()
            win = wait_for(lambda: carousel(520, 792), "The Settings window did not open")
            time.sleep(0.5)
            screenshot("settings.png")
            call("xdotool", "mousemove", str(win[0] + 16 + 150), str(win[1] + 466 + 14), "click", "1")
            time.sleep(0.2)
            # the box shows the folder now set (typed text is added at the end), so clear it first
            for _ in range(len(str(pictures)) + 2):
                call("xdotool", "key", "BackSpace")
            call("xdotool", "type", "--delay", "10", str(elsewhere))
            time.sleep(0.3)
            screenshot("settings-typed.png")
            call("xdotool", "mousemove", str(win[0] + 330 + 42), str(win[1] + 680 + 64 + 16), "click", "1")
            config_file = temp / "data/pasteroll/config.toml"
            wait_for(lambda: str(elsewhere) in config_file.read_text(), "Save did not write the picture folder")
            passed("the Settings window saves the picture folder")
            env.pop("PASTEROLL_OPEN_SETTINGS")
            time.sleep(1.5)  # it starts again from the saved file
            wait_for(lambda: (temp / "data/pasteroll/app.stop").exists(), "The program did not start again")
            stop_app()
        except BaseException:
            try:
                screenshot("failure.png")
                (OUT / "failure-windows.json").write_text(json.dumps(visible_windows()) + "\n")
            except Exception:
                pass
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
        message = str(error) if isinstance(error, AssertionError) else type(error).__name__ + ": " + str(error)
        print("FAIL: isolated X11 test: " + message, file=sys.stderr)
        sys.exit(1)
