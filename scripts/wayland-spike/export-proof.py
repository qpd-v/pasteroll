"""Encode the measured compositor frames with an existing local FFmpeg.

No downloads. The title explicitly identifies the diagnostic keyboard path.
The output is an engineering proof, not website or release media.
"""
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'target-dev/wayland-phase1'
capture = json.loads((OUT / 'capture.json').read_text())
frames = capture['frames']
assert not capture['errors'] and len(frames) >= 20, 'No valid recording'
take = capture['directory']
assert Path(take).name == take
manifest = []
for index, frame in enumerate(frames):
    assert Path(frame['file']).name == frame['file']
    manifest.append(f"file '{take}/{frame['file']}'")
    duration = frames[index + 1]['at'] - frame['at'] if index + 1 < len(frames) else .25
    manifest.append(f'duration {duration:.6f}')
manifest.append(f"file '{take}/{frames[-1]['file']}'")
(OUT / 'frames.ffconcat').write_text('\n'.join(manifest) + '\n')
ffmpeg = shutil.which('ffmpeg')
ffprobe = shutil.which('ffprobe')
assert ffmpeg and ffprobe, 'FFmpeg and FFprobe must already be installed'
font = Path('C:/Windows/Fonts/segoeui.ttf')
assert font.is_file(), 'This proof exporter uses the existing Windows Segoe UI font'
caption = 'GNOME 46 Wayland prototype | Diagnostic shortcut - Ctrl + Shift + Space'
filters = ("drawbox=x=0:y=0:w=iw:h=42:color=black@0.90:t=fill,"
           "drawtext=fontfile='C\\:/Windows/Fonts/segoeui.ttf':"
           f"text='{caption}':x=18:y=9:fontsize=20:fontcolor=white")
video = OUT / 'wayland-phase1-proof.mp4'
result = subprocess.run([ffmpeg, '-v', 'error', '-y', '-f', 'concat', '-safe', '0',
    '-i', 'frames.ffconcat', '-vf', filters, '-r', '24', '-c:v', 'libx264', '-crf', '19',
    '-pix_fmt', 'yuv420p', '-movflags', '+faststart', '-an', '-map_metadata', '-1', video.name],
    cwd=OUT, capture_output=True)
assert result.returncode == 0, 'Proof encoding failed (no paths or command output printed)'
result = subprocess.run([ffmpeg, '-v', 'error', '-i', video.name, '-f', 'null', '-'], cwd=OUT,
                        capture_output=True)
assert result.returncode == 0 and not result.stderr, 'Proof decoding failed'
metadata = json.loads(subprocess.check_output([ffprobe, '-v', 'error', '-show_streams',
    '-show_format', '-of', 'json', video.name], cwd=OUT))
stream = metadata['streams'][0]
assert len(metadata['streams']) == 1 and stream['codec_type'] == 'video'
assert (stream['width'], stream['height']) == (1280, 800)
summary = {'captured_frames': len(frames), 'capture_duration': frames[-1]['at'] - frames[0]['at'],
           'video_duration': float(metadata['format']['duration']),
           'encoded_frames': int(stream['nb_frames']), 'width': 1280, 'height': 800,
           'encoded_fps': stream['r_frame_rate'], 'silent': True, 'full_decode': True,
           'label': caption, 'note': 'Timestamped compositor snapshots; encoded holds do not imply 24 fps capture.'}
(OUT / 'video-qa.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
