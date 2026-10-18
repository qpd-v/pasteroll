"""Run local checks with source remapping and sanitized build output, no installs."""
import getpass
import os
from pathlib import Path
import subprocess
import sys
import time

for stream in (sys.stdout, sys.stderr):
    if hasattr(stream, 'reconfigure'):
        stream.reconfigure(encoding='utf-8', errors='replace')

root = Path(__file__).resolve().parents[2]
out = root / 'target-dev/wayland-phase2'
out.mkdir(parents=True, exist_ok=True)
args = sys.argv[1:]
platform = 'wsl' if args and args[0] == '--wsl' else 'windows'
private = [str(root), root.as_posix(), str(Path.home()), getpass.getuser()]
if args and args[0] == '--wsl':
    args.pop(0)
    linux_root = subprocess.check_output(['wsl', '-d', 'Ubuntu', '--exec', 'wslpath', '-a', str(root)], text=True).strip()
    linux_user = subprocess.check_output(['wsl', '-d', 'Ubuntu', '--exec', 'id', '-un'], text=True).strip()
    linux_home = subprocess.check_output(['wsl', '-d', 'Ubuntu', '--exec', 'bash', '-lc', 'printf %s "$HOME"'], text=True).strip()
    cargo_path = subprocess.check_output(['wsl', '-d', 'Ubuntu', '--exec', 'bash', '-lc', 'command -v cargo'], text=True).strip()
    private.extend([linux_root, linux_user, linux_home])
    command = ['wsl', '-d', 'Ubuntu', '--cd', linux_root, '--exec', 'env',
        '-u', 'CARGO_ENCODED_RUSTFLAGS',
        f'RUSTFLAGS=--remap-path-prefix={linux_root}=/pasteroll --remap-path-prefix={linux_home}=/build-home',
        f'CFLAGS=-ffile-prefix-map={linux_root}=/pasteroll -ffile-prefix-map={linux_home}=/build-home',
        f'CXXFLAGS=-ffile-prefix-map={linux_root}=/pasteroll -ffile-prefix-map={linux_home}=/build-home',
        f'PATH={Path(cargo_path).parent.as_posix()}:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
        'CARGO_TARGET_DIR=target-dev/linux', 'CARGO_BUILD_JOBS=2', *args]
else:
    command = args
env = dict(os.environ)
env['PYTHONIOENCODING'] = 'utf-8'
env.pop('CARGO_ENCODED_RUSTFLAGS', None)
env['RUSTFLAGS'] = f'--remap-path-prefix={root}=/pasteroll --remap-path-prefix={Path.home()}=/build-home'
if platform == 'windows':
    # MSVC otherwise embeds an absolute PDB path even with source remapping.
    env['RUSTFLAGS'] += ' -C link-arg=/PDBALTPATH:pasteroll.pdb'
env['CARGO_TARGET_DIR'] = str(root / 'target-dev/windows')
env['CARGO_BUILD_JOBS'] = '2'
process = subprocess.Popen(command, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
log_name = f'{platform}-{time.time_ns()}.log'
with (out / log_name).open('w', encoding='utf-8') as log:
    for line in iter(process.stdout.readline, b''):
        text = line.decode('utf-8', 'replace')
        for value in sorted(set(private), key=len, reverse=True):
            if value and len(value) > 2:
                text = text.replace(value, '[private]')
        log.write(text)
        print(text, end='', flush=True)
raise SystemExit(process.wait())
