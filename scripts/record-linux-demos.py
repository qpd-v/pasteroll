#!/usr/bin/env python3
"""Record the release app for the website's four media slots on private X11.

Run through record-linux-demos.sh. Requires the same existing tools as test-linux-x11.sh.
Frames contain a synthetic terminal, synthetic clipboard examples and action subtitles.
No live profile, clipboard, session bus or display is used. Output is ignored by Git.
"""
import json
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
EXE = ROOT / "target/x86_64-unknown-linux-musl/release/pasteroll"
OUT = ROOT / "target-dev/site-media"
FPS = 8


def main():
    for tool in ("Xvfb", "xdotool", "xterm", "xclip", "xwininfo", "xwd", "convert", "xsetroot"):
        if not shutil.which(tool):
            raise SystemExit(f"Missing local recording tool: {tool}")
    OUT.mkdir(parents=True, exist_ok=True)
    if (OUT / "frames").exists():
        raise SystemExit("Move the previous site-media/frames folder before recording again.")
    processes = []
    metadata = {}
    with tempfile.TemporaryDirectory(prefix="pasteroll-demo-") as scratch:
        temp = Path(scratch)
        env = dict(os.environ, HOME=str(temp / "home"), XDG_DATA_HOME=str(temp / "data"),
                   XDG_CONFIG_HOME=str(temp / "config"), XDG_RUNTIME_DIR=str(temp / "runtime"),
                   PASTEROLL_NO_NOTICE="1", PASTEROLL_NO_SNI="1", LC_ALL="C.UTF-8")
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS"):
            env.pop(name, None)
        (temp / "runtime").mkdir(mode=0o700)
        (temp / "home/Desktop").mkdir(parents=True)

        def call(*args, binary=False, input_data=None):
            result = subprocess.run([str(a) for a in args], env=env, cwd=temp,
                                    input=input_data, capture_output=True, timeout=15)
            if result.returncode:
                raise AssertionError(f"{Path(args[0]).name} failed ({result.returncode})")
            return result.stdout if binary else result.stdout.decode(errors="replace")

        def wait_for(check, message, seconds=8):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                value = check()
                if value:
                    return value
                time.sleep(0.05)
            raise AssertionError(message)

        def key(*keys):
            call("xdotool", "key", "--clearmodifiers", *keys)
            time.sleep(0.2)

        def click(x, y):
            call("xdotool", "mousemove", str(x), str(y), "click", "1")
            time.sleep(0.4)

        def windows():
            tree = call("xwininfo", "-root", "-children")
            sizes = []
            for win in re.findall(r"^\s+(0x[0-9a-f]+)\s", tree, re.M):
                if int(win, 16) == int(target, 0):
                    continue
                info = call("xwininfo", "-id", win)
                if "Map State: IsViewable" in info:
                    sizes.append((int(re.search(r"Width:\s+(\d+)", info)[1]),
                                  int(re.search(r"Height:\s+(\d+)", info)[1])))
            return sizes

        def overlay():
            return (520, 440) in windows()

        def expanded():
            return any(w >= 850 and h >= 500 for w, h in windows())

        def activate():
            call("xdotool", "windowfocus", target, "mousemove", "390", "260",
                 "keydown", "Control_L", "keydown", "Shift_L", "click", "2",
                 "keyup", "Shift_L", "keyup", "Control_L")
            wait_for(overlay, "Carousel did not open")
            time.sleep(0.3)

        def dismiss():
            for _ in range(8):
                if not overlay() and not expanded():
                    return
                key("Escape")
                time.sleep(0.45)
            raise AssertionError("Demo windows did not close")

        def clipboard(text=None, image=None):
            command = ["xclip", "-selection", "clipboard", "-in", "-quiet"]
            if image:
                command += ["-target", "image/png"]
            process = subprocess.Popen(command, env=env, stdin=subprocess.PIPE,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(process)
            process.stdin.write(image.read_bytes() if image else text.encode())
            process.stdin.close()
            time.sleep(0.65)

        class Recording:
            def __init__(self, name, caption):
                self.name, self.caption = name, caption
                self.events = []
                self.stop = threading.Event()
                self.frames = OUT / "frames" / name
                self.frames.mkdir(parents=True)
                self.error = None

            def say(self, caption):
                self.caption = caption
                self.events.append(caption)

            def capture(self):
                index = 0
                try:
                    while not self.stop.is_set():
                        start = time.monotonic()
                        data = call("xwd", "-root", "-silent", binary=True)
                        call("convert", "xwd:-", "-fill", "#11151d", "-draw", "rectangle 0,736 1279,799",
                             "-font", "DejaVu-Sans", "-pointsize", "23", "-fill", "#eef4f6",
                             "-gravity", "South", "-annotate", "+0+22", self.caption,
                             "png:" + str(self.frames / f"{index:05}.png"), input_data=data)
                        index += 1
                        self.stop.wait(max(0, 1 / FPS - (time.monotonic() - start)))
                except BaseException as error:
                    self.error = error
                metadata[self.name] = {"frames": index, "fps": round(index / (time.monotonic() - self.started), 3), "actions": self.events,
                                       "platform": "Linux X11", "synthetic_data_only": True}

            def __enter__(self):
                self.say(self.caption)
                self.started = time.monotonic()
                self.thread = threading.Thread(target=self.capture)
                self.thread.start()
                time.sleep(0.75)
                return self

            def __exit__(self, *exc):
                self.stop.set()
                self.thread.join()
                if self.error:
                    raise self.error
                print("Recorded " + self.name, flush=True)

        rfd, wfd = os.pipe()
        app = None
        try:
            xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(wfd), "-screen", "0", "1280x800x24",
                                     "-nolisten", "tcp"], pass_fds=(wfd,),
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(xvfb)
            os.close(wfd)
            if not select.select([rfd], [], [], 10)[0]:
                raise AssertionError("Private display did not start")
            number = os.read(rfd, 32).decode().strip()
            if not number.isdigit():
                raise AssertionError("Invalid private display")
            env["DISPLAY"] = ":" + number
            os.close(rfd)
            call("xsetroot", "-solid", "#151923")
            call(EXE, "seed")
            sink = temp / "pasted.txt"
            terminal_script = (
                'stty -echo; printf "  PASTEROLL / DEMO WORKSPACE\\n\\n'
                '  Project notes\\n  -------------\\n\\n'
                '  Copy an idea. Find it when you need it.\\n\\n'
                '  Pasted text:\\n\\n  "; exec tee "$1"')
            terminal = subprocess.Popen(["xterm", "-T", "Pasteroll demo workspace", "-geometry", "94x24+55+70",
                                         "-fa", "DejaVu Sans Mono", "-fs", "15", "-bg", "#202630", "-fg", "#e6edf3",
                                         "-e", "sh", "-c", terminal_script, "demo", str(sink)], env=env,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(terminal)

            def find_target():
                found = subprocess.run(["xdotool", "search", "--name", "Pasteroll demo workspace"],
                                       env=env, capture_output=True, text=True, timeout=5)
                return found.stdout.splitlines() if found.returncode == 0 else []

            target = wait_for(find_target, "Demo terminal did not start")[0]
            app = subprocess.Popen([str(EXE), "run", "--max-seconds", "240"], env=env,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(app)
            wait_for(lambda: (temp / "data/pasteroll/app.stop").exists(), "Release app did not start")
            time.sleep(0.5)
            message = "A calmer workspace, one clip at a time."
            clipboard(text=message)

            with Recording("demo", "Copy as usual. Your history stays on this computer.") as recording:
                time.sleep(1)
                recording.say("Ctrl + Shift + middle-click opens Pasteroll.")
                activate()
                time.sleep(1.3)
                recording.say("Scroll through your history. Return to the clip you need.")
                call("xdotool", "mousemove", "500", "360", "click", "5")
                time.sleep(0.8)
                call("xdotool", "click", "4")
                time.sleep(0.8)
                recording.say("Press Enter to paste into the app you were using.")
                key("Return")
                wait_for(lambda: not overlay(), "Paste did not close carousel")
                key("Return")
                wait_for(lambda: sink.exists() and message in sink.read_text(), "Synthetic paste did not arrive")
                time.sleep(2)

            with Recording("carousel", "Open your history wherever you are working.") as recording:
                activate()
                time.sleep(0.8)
                recording.say("Scroll to browse text, images and files.")
                for _ in range(3):
                    call("xdotool", "mousemove", "500", "360", "click", "5")
                    time.sleep(0.65)
                recording.say("Type to search your clipboard history.")
                call("xdotool", "type", "--clearmodifiers", "--delay", "140", "workspace")
                time.sleep(1.4)
                recording.say("Ctrl + P pins a clip so it stays in your history.")
                key("ctrl+p")
                time.sleep(1.2)
                key("ctrl+p")
                recording.say("Delete moves a clip to the recycle bin. Ctrl + Z restores it.")
                key("Delete")
                time.sleep(0.8)
                key("ctrl+z")
                time.sleep(1.3)
                dismiss()

            long_text = ("PROJECT NOTES\n\nA calmer workspace\n\n"
                         "Copy useful ideas without breaking your flow. Pasteroll keeps them ready.\n\n"
                         "PLAN\n- Capture a thought.\n- Find it with a quick search.\n- Paste only what you need.\n\n"
                         "RELEASE CHECKLIST\nTest the installers. Verify every download. Keep private data local.\n\n") * 4
            clipboard(text=long_text)
            with Recording("viewer", "Ctrl + E expands the selected clip.") as recording:
                activate()
                key("ctrl+e")
                wait_for(expanded, "Expanded viewer did not open")
                time.sleep(0.9)
                recording.say("Read long clips in a larger window. Scroll without losing your place.")
                call("xdotool", "mousemove", "650", "430", "click", "--repeat", "4", "--delay", "180", "5")
                time.sleep(0.9)
                recording.say("Ctrl + F finds words inside the clip.")
                key("ctrl+f")
                call("xdotool", "type", "--clearmodifiers", "--delay", "150", "workspace")
                time.sleep(1.3)
                key("F3")
                time.sleep(0.7)
                recording.say("F11 switches to fullscreen.")
                key("F11")
                time.sleep(1.3)
                dismiss()
                time.sleep(0.5)

            # Original synthetic artwork, rendered with existing ImageMagick; no downloaded media.
            picture = temp / "demo-card.png"
            call("convert", "-size", "720x440", "xc:#e9f4f0", "-fill", "#2c9d89", "-draw", "rectangle 0,0 20,439",
                 "-font", "DejaVu-Sans", "-fill", "#183e38", "-pointsize", "22", "-annotate", "+64+90", "DESIGN NOTES",
                 "-pointsize", "42", "-annotate", "+64+175", "A calmer workspace",
                 "-fill", "#4e7069", "-pointsize", "25", "-annotate", "+64+235", "Copy. Scroll. Paste.",
                 "-fill", "#2c9d89", "-draw", "roundrectangle 64,290 256,350 15,15",
                 "-fill", "#ffffff", "-pointsize", "22", "-annotate", "+86+329", "Pasteroll", picture)
            clipboard(image=picture)
            with Recording("editor", "Choose an image. Ctrl + click the desktop to edit before saving.") as recording:
                activate()
                call("xdotool", "mousemove", "1230", "710", "keydown", "Control_L", "click", "1", "keyup", "Control_L")
                wait_for(expanded, "Advanced Image Paste did not open")
                time.sleep(1)
                recording.say("Adjust your image with a live preview.")
                click(1077, 292)  # Rounded corners switch, collapsed panel.
                click(895, 327)   # A modest radius rather than a fully rounded card.
                time.sleep(0.8)
                click(820, 292)   # Collapse the section so subsequent headers stay in place.
                click(1077, 330)  # Outline switch.
                time.sleep(0.8)
                click(820, 330)
                click(1077, 368)  # Drop shadow switch.
                time.sleep(0.8)
                click(820, 368)
                recording.say("Undo and redo changes. The original clip stays intact.")
                key("ctrl+z")
                time.sleep(0.8)
                key("ctrl+y")
                time.sleep(1)
                recording.say("Save the edited copy to your desktop.")
                click(1020, 672)
                wait_for(lambda: list((temp / "home/Desktop").glob("*.png")), "Edited image was not saved")
                time.sleep(1.2)

            call(EXE, "stop")
            if app.wait(timeout=8) != 0:
                raise AssertionError("Release app did not stop cleanly")
            (OUT / "recordings.json").write_text(json.dumps(metadata, indent=2) + "\n")
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


if __name__ == "__main__":
    main()
