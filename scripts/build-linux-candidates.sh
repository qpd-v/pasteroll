#!/bin/bash
# Produce both Linux candidates and their checksum manifest; no publish/install.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$root"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
bash scripts/package-linux.sh
export PASTEROLL_BUILD_ROOT="$root/target"
export PASTEROLL_PACKAGE_OUTPUT="$root/dist/linux"
bash scripts/package-gnome.sh
python3 - <<'PY'
from pathlib import Path
import shutil, tomllib
root = Path.cwd()
version = tomllib.loads((root/'crates/pasteroll-linux/Cargo.toml').read_text())['package']['version']
for name in (f'pasteroll-{version}-linux-x86_64.tar.gz',f'pasteroll_{version}_amd64.deb'):
    shutil.copyfile(root/'dist'/name,root/'dist/linux'/name)
PY
python3 scripts/test-linux-packaging.py
python3 scripts/test-gnome-packaging.py
python3 scripts/verify-linux-release.py
