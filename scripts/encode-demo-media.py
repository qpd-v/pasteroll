#!/usr/bin/env python3
"""Encode the four synthetic X11 recordings with an existing local FFmpeg."""
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "target-dev/site-media"
NAMES = ("demo", "carousel", "viewer", "editor")


def main():
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise SystemExit("FFmpeg is missing. This script does not download it.")
    recordings = json.loads((OUT / "recordings.json").read_text())
    for name in NAMES:
        frames = OUT / "frames" / name
        info = recordings[name]
        if len(list(frames.glob("*.png"))) != info["frames"] or info["frames"] < 16:
            raise SystemExit("A recording is missing frames or is incomplete.")
        source = [ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-framerate", str(info["fps"]),
                  "-i", str(frames / "%05d.png"), "-map_metadata", "-1", "-an"]
        subprocess.run(source + ["-vf", "scale=1024:-2:flags=lanczos,fps=16", "-c:v", "libx264",
                                 "-preset", "slow", "-crf", "22", "-pix_fmt", "yuv420p", "-movflags",
                                 "+faststart", str(OUT / f"{name}.mp4")], check=True, capture_output=True)
        palette = ("scale=1024:-2:flags=lanczos,split[a][b];"
                   "[a]palettegen=stats_mode=diff:max_colors=128[p];"
                   "[b][p]paletteuse=dither=bayer:bayer_scale=3")
        subprocess.run(source + ["-filter_complex", palette, "-loop", "0", str(OUT / f"{name}.gif")],
                       check=True, capture_output=True)
        print(f"Encoded {name}.gif and {name}.mp4")
    (OUT / "README.md").write_text(
        "# Website feature recordings\n\n"
        "Actual Pasteroll 0.1.0-beta.2 Linux X11 release recordings, using synthetic clips only.\n"
        "Action subtitles were added to the capture; the app UI is unmodified.\n\n"
        "Copy demo, carousel, viewer and editor GIFs (or the smaller MP4 alternatives) into the\n"
        "website's assets/media/ folder. Its existing loader fills all four Coming soon slots.\n"
        "MP4 takes precedence if both formats exist. No website files were changed or deployed.\n\n"
        "The recordings demonstrate paste, browsing/search/pin/recycle/restore, expanded text\n"
        "view/find/fullscreen, and image effects/undo/redo/save. They do not demonstrate OCR,\n"
        "every editor adjustment, or Windows-specific installer behavior.\n",
        encoding="utf-8")


if __name__ == "__main__":
    main()
